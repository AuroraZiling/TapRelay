mod child;
mod process;
mod protocol;

use anyhow::{Context, Result};
use protocol::{Command, Message};
use std::{
    collections::BTreeMap,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use taprelay_core::{
    command::{CommandPhase, QueuedCommand},
    hid::Profile,
    passthrough::{InputLink, MAX_INPUT_AGE, Packet},
    ports::BackendError,
    state::{Snapshot, Target, TransportActivity},
};
use tokio::sync::{oneshot, watch};

pub use child::run as run_child;

enum Control {
    Select(Option<String>),
    Pair(String),
    Refresh,
    Restart,
    Invalidate,
    Send(QueuedCommand, oneshot::Sender<Result<(), BackendError>>),
}

pub struct Handle {
    commands: mpsc::SyncSender<Control>,
    state: watch::Receiver<Snapshot>,
    link: InputLink,
    revision: Arc<AtomicU64>,
    stopping: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Handle {
    pub fn start(remembered: Option<Target>) -> Result<Self> {
        let revision = Arc::new(AtomicU64::new(1));
        let (link, input) = InputLink::channel(revision.clone());
        let (commands, control) = mpsc::sync_channel(16);
        let (updates, state) = watch::channel(Snapshot::default());
        let stopping = Arc::new(AtomicBool::new(false));
        let mut supervisor = Supervisor {
            process: None,
            control,
            input,
            updates: updates.clone(),
            link: link.clone(),
            revision: revision.clone(),
            stopping: stopping.clone(),
            profile: Profile::MediaOnly,
            publish: true,
            selected: None,
            remembered,
            snapshot: Snapshot::default(),
            native_generation: None,
            native_selected: None,
            pairing: None,
            blocked_native: None,
            arm: None,
            armed: false,
            capture_request: 0,
            switching_since: None,
            pending: BTreeMap::new(),
            next_id: 0,
        };
        let failure_link = link.clone();
        let worker = thread::Builder::new()
            .name("taprelay-bluetooth-supervisor".into())
            .spawn(move || {
                let outcome =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| supervisor.run()));
                drop(supervisor);
                let message = match outcome {
                    Ok(Ok(())) => None,
                    Ok(Err(error)) => Some(format!("{error:#}")),
                    Err(_) => Some("Bluetooth supervisor panicked".into()),
                };
                failure_link.end();
                failure_link.set_profile_available(false);
                if let Some(error) = message {
                    tracing::error!(%error, "Bluetooth supervisor stopped");
                    failure_link.fail(error.clone());
                    updates.send_modify(|state| {
                        taprelay_core::devices::revoke_session(state);
                        state.profile_switching = false;
                        state.service = false;
                        state.broadcasting = false;
                        state.last_error = Some(error);
                    });
                }
            })?;
        link.set_worker(worker.thread().clone());
        Ok(Self {
            commands,
            state,
            link,
            revision,
            stopping,
            worker: Some(worker),
        })
    }

    fn request(&self, command: Control) -> Result<()> {
        self.commands
            .try_send(command)
            .map_err(|_| anyhow::anyhow!("Bluetooth supervisor queue unavailable"))?;
        self.link.wake();
        Ok(())
    }

    fn change(&self, command: Control) -> Result<u64> {
        self.link.end();
        let generation = self.revision.fetch_add(1, Ordering::AcqRel) + 1;
        self.request(command)?;
        Ok(generation)
    }
}

impl crate::platform::Transport for Handle {
    fn input_link(&self) -> Option<InputLink> {
        Some(self.link.clone())
    }
    fn snapshot(&mut self) -> Option<Snapshot> {
        crate::platform::poll_snapshot(&mut self.state)
    }
    fn is_finished(&self) -> bool {
        self.worker
            .as_ref()
            .is_none_or(|worker| worker.is_finished())
    }
    fn refresh(&self) -> Result<()> {
        self.request(Control::Refresh)
    }
    fn restart(&self) -> Result<u64> {
        self.change(Control::Restart)
    }
    fn select(&self, id: String) -> Result<u64> {
        self.change(Control::Select(Some(id)))
    }
    fn pair(&self, id: String) -> Result<()> {
        self.request(Control::Pair(id))
    }
    fn disconnect(&self) -> Result<u64> {
        self.change(Control::Select(None))
    }
    fn bluetooth_settings(&self) -> Result<()> {
        Ok(taprelay_windows::desktop::open("ms-settings:bluetooth")?)
    }
    fn invalidate(&self) {
        if let Err(error) = self.change(Control::Invalidate) {
            self.link.fail(error.to_string());
        }
    }
    fn send(&self, command: QueuedCommand) -> Result<oneshot::Receiver<Result<(), BackendError>>> {
        let (reply, result) = oneshot::channel();
        self.request(Control::Send(command, reply))?;
        Ok(result)
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.link.end();
        self.stopping.store(true, Ordering::Release);
        self.link.wake();
        if let Some(worker) = self.worker.take() {
            let until = Instant::now() + Duration::from_secs(2);
            while !worker.is_finished() && Instant::now() < until {
                thread::sleep(Duration::from_millis(5));
            }
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}

struct Pending {
    generation: u64,
    created: Instant,
    reply: oneshot::Sender<Result<(), BackendError>>,
}

struct Supervisor {
    process: Option<process::Worker>,
    control: mpsc::Receiver<Control>,
    input: mpsc::Receiver<Packet>,
    updates: watch::Sender<Snapshot>,
    link: InputLink,
    revision: Arc<AtomicU64>,
    stopping: Arc<AtomicBool>,
    profile: Profile,
    publish: bool,
    selected: Option<String>,
    remembered: Option<Target>,
    snapshot: Snapshot,
    native_generation: Option<u64>,
    native_selected: Option<String>,
    pairing: Option<(Target, Instant)>,
    blocked_native: Option<u64>,
    arm: Option<(u64, u64)>,
    armed: bool,
    capture_request: u64,
    switching_since: Option<Instant>,
    pending: BTreeMap<u64, Pending>,
    next_id: u64,
}

impl Supervisor {
    fn run(&mut self) -> Result<()> {
        self.replace_worker(Profile::MediaOnly, true)?;
        while !self.stopping.load(Ordering::Acquire) {
            if let Err(error) = self.step() {
                if self.stopping.load(Ordering::Acquire) {
                    break;
                }
                if self.profile == Profile::Full {
                    self.link.fail(format!("Passthrough stopped: {error:#}"));
                    self.replace_worker(Profile::MediaOnly, self.publish)?;
                } else {
                    return Err(error);
                }
            }
            thread::park_timeout(Duration::from_millis(5));
        }
        Ok(())
    }

    fn worker(&self) -> Result<&process::Worker> {
        self.process
            .as_ref()
            .context("Bluetooth worker unavailable")
    }

    fn replace_worker(&mut self, profile: Profile, publish: bool) -> Result<()> {
        let initial = self.initial_command(profile, publish);
        let replacing = self.process.is_some();
        self.link.suspend();
        self.link.set_profile_available(false);
        self.revision.fetch_add(1, Ordering::AcqRel);
        self.profile = profile;
        self.publish = publish;
        self.native_generation = None;
        self.native_selected = None;
        self.blocked_native = None;
        self.arm = None;
        self.armed = false;
        self.capture_request = if profile == Profile::Full {
            self.link.profile_request()
        } else {
            0
        };
        self.switching_since = (replacing && publish).then(Instant::now);
        for (_, pending) in std::mem::take(&mut self.pending) {
            let _ = pending.reply.send(Err(BackendError::Stale));
        }
        self.snapshot.selected = self.selected.clone();
        self.snapshot.ready = false;
        self.snapshot.service = false;
        self.snapshot.broadcasting = false;
        self.snapshot.service_paused = !publish;
        self.snapshot.activity = if publish {
            TransportActivity::Publishing
        } else {
            TransportActivity::Idle
        };
        self.snapshot.last_error = None;
        self.publish_snapshot();
        if let Some(worker) = self.process.as_mut() {
            worker.stop()?;
        }
        self.process = None;
        if self.stopping.load(Ordering::Acquire) {
            return Ok(());
        }
        self.process = Some(process::Worker::start(initial)?);
        Ok(())
    }

    fn initial_command(&self, profile: Profile, publish: bool) -> Command {
        let target = ((profile != self.profile || profile == Profile::Full) && publish)
            .then(|| {
                self.snapshot
                    .targets
                    .iter()
                    .find(|target| {
                        self.selected
                            .as_ref()
                            .is_some_and(|id| target.matches_id(id))
                    })
                    .cloned()
            })
            .flatten();
        Command::Init {
            profile,
            publish,
            selected: if target.is_some() {
                None
            } else {
                self.selected.clone()
            },
            remembered: target.or_else(|| self.remembered.clone()),
        }
    }

    fn publish_snapshot(&mut self) {
        self.snapshot.generation = self.revision.load(Ordering::Acquire);
        self.snapshot.hid_profile = self.profile;
        self.snapshot.profile_switching = self.switching_since.is_some();
        self.link.set_profile_available(
            self.publish
                && self.selected.is_some()
                && self.snapshot.ready
                && self.switching_since.is_none(),
        );
        if *self.updates.borrow() != self.snapshot {
            self.updates.send_replace(self.snapshot.clone());
        }
    }

    fn step(&mut self) -> Result<()> {
        for _ in 0..16 {
            match self.control.try_recv() {
                Ok(command) => self.control(command)?,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.stopping.store(true, Ordering::Release);
                    return Ok(());
                }
            }
        }
        let desired = self.desired_profile();
        if desired != self.profile
            || (desired == Profile::Full && self.capture_request != self.link.profile_request())
        {
            self.replace_worker(desired, self.publish)?;
        }
        for _ in 0..128 {
            let message = self.worker()?.incoming.try_recv();
            match message {
                Ok(message) => self.message(message?)?,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    anyhow::bail!("Bluetooth worker reader stopped")
                }
            }
        }
        self.process
            .as_mut()
            .context("Bluetooth worker unavailable")?
            .check()?;
        if self
            .pairing
            .as_ref()
            .is_some_and(|(_, since)| since.elapsed() >= Duration::from_secs(30))
        {
            self.pairing = None;
            self.link.fail(
                "Pairing target was not found after restarting Bluetooth; refresh and try again",
            );
        }
        if self
            .switching_since
            .is_some_and(|since| since.elapsed() >= Duration::from_secs(30))
        {
            if self.profile == Profile::Full {
                anyhow::bail!("Receiver did not subscribe to keyboard and mouse within 30 seconds");
            }
            self.switching_since = None;
            self.snapshot.last_error =
                Some("Receiver did not reconnect within 30 seconds; connect again".into());
            self.publish_snapshot();
        }
        for _ in 0..256 {
            let Ok(packet) = self.input.try_recv() else {
                break;
            };
            if !self.link.accepts(&packet) || !self.armed {
                continue;
            }
            if packet.captured.elapsed() > MAX_INPUT_AGE {
                anyhow::bail!("Passthrough queue exceeded 250 ms");
            }
            self.worker()?.send(Command::Input {
                generation: self
                    .native_generation
                    .context("Missing Bluetooth generation")?,
                captured_ms: protocol::captured_ms(packet.captured),
                event: packet.event,
                mouse_percent: self.link.mouse_percent(),
                reverse_scroll: self.link.reverse_scroll(),
            })?;
        }
        let expired: Vec<_> = self
            .pending
            .iter()
            .filter(|(_, pending)| pending.created.elapsed() >= Duration::from_secs(2))
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            if let Some(pending) = self.pending.remove(&id) {
                let _ = pending.reply.send(Err(BackendError::Unavailable(
                    "Bluetooth worker delivery timed out".into(),
                )));
            }
        }
        Ok(())
    }

    fn desired_profile(&self) -> Profile {
        if self.publish && self.selected.is_some() && self.link.profile_requested() {
            Profile::Full
        } else {
            Profile::MediaOnly
        }
    }

    fn control(&mut self, command: Control) -> Result<()> {
        match command {
            Control::Select(selected) => {
                self.link.end();
                self.remembered = None;
                self.pairing = None;
                self.selected = selected;
                self.replace_worker(Profile::MediaOnly, self.selected.is_some())?;
            }
            Control::Restart => {
                self.link.end();
                self.remembered = None;
                self.pairing = None;
                self.selected = None;
                self.replace_worker(Profile::MediaOnly, true)?;
            }
            Control::Refresh => self.worker()?.send(Command::Refresh)?,
            Control::Pair(id) => {
                let target = self
                    .snapshot
                    .targets
                    .iter()
                    .find(|target| target.matches_id(&id))
                    .cloned()
                    .unwrap_or(Target {
                        id,
                        ..Default::default()
                    });
                if self.native_generation.is_some() {
                    self.worker()?.send(Command::Pair(target.id))?;
                } else {
                    self.pairing = Some((target, Instant::now()));
                }
            }
            Control::Invalidate => {
                self.link.end();
                self.blocked_native = self.native_generation;
                self.snapshot.ready = false;
                self.worker()?.send(Command::Invalidate)?;
                self.publish_snapshot();
            }
            Control::Send(command, reply) => {
                if command.generation != self.revision.load(Ordering::Acquire)
                    || !self.snapshot.ready
                    || self.switching_since.is_some()
                    || self.selected.as_deref() != Some(&command.target)
                    || self.pending.len() >= 32
                {
                    let _ = reply.send(Err(BackendError::Stale));
                    return Ok(());
                }
                self.next_id = self.next_id.wrapping_add(1);
                self.worker()?.send(Command::Media {
                    id: self.next_id,
                    generation: self
                        .native_generation
                        .context("Missing Bluetooth generation")?,
                    captured_ms: protocol::captured_ms(command.created),
                    target: self
                        .native_selected
                        .clone()
                        .context("Missing native Bluetooth target")?,
                    action: command.action,
                    down: command.phase == CommandPhase::Press,
                })?;
                self.pending.insert(
                    self.next_id,
                    Pending {
                        generation: command.generation,
                        created: Instant::now(),
                        reply,
                    },
                );
            }
        }
        Ok(())
    }

    fn message(&mut self, message: Message) -> Result<()> {
        match message {
            Message::Status {
                mut state,
                input_available,
            } => {
                let generation = state.generation;
                self.native_selected = state.selected.clone();
                if let Some((wanted, _)) = &self.pairing
                    && let Some(target) = state
                        .targets
                        .iter()
                        .find(|target| target.same_device(wanted))
                {
                    self.worker()?.send(Command::Pair(target.id.clone()))?;
                    self.pairing = None;
                }
                if state.adapter_state == taprelay_core::devices::AdapterState::Unknown {
                    state.adapter_state = self.snapshot.adapter_state;
                    state.adapter = self.snapshot.adapter;
                    state.peripheral = self.snapshot.peripheral;
                }
                if self.native_generation != Some(generation) {
                    if self.link.epoch() != 0 {
                        self.link.end();
                    } else {
                        self.link.suspend();
                    }
                    self.native_generation = Some(generation);
                    self.arm = None;
                    self.armed = false;
                    self.revision.fetch_add(1, Ordering::AcqRel);
                }
                if self.blocked_native == Some(generation) {
                    state.ready = false;
                } else {
                    self.blocked_native = None;
                }
                if self.selected.is_none() && self.remembered.is_some() && state.selected.is_some()
                {
                    self.selected = state.selected.clone();
                }
                if let (Some(selected), Some(native)) = (&self.selected, &self.native_selected)
                    && selected != native
                    && let Some(target) = state
                        .targets
                        .iter_mut()
                        .find(|target| target.matches_id(native))
                    && !target.matches_id(selected)
                {
                    target.aliases.push(selected.clone());
                }
                if self.selected.is_some() {
                    state.selected = self.selected.clone();
                }
                if !self.publish {
                    state.peripheral = self.snapshot.peripheral;
                }
                if self.armed && !input_available {
                    self.link.end();
                    self.armed = false;
                }
                if self.profile == Profile::Full
                    && state.ready
                    && input_available
                    && self.link.profile_requested()
                    && self.capture_request == self.link.profile_request()
                    && !self.armed
                    && self.arm.is_none()
                {
                    self.worker()?.send(Command::Arm { generation })?;
                    self.arm = Some((generation, self.revision.load(Ordering::Acquire)));
                }
                if self.profile == Profile::MediaOnly
                    && (state.ready
                        || (self.selected.is_none() && state.service && state.broadcasting))
                {
                    self.switching_since = None;
                }
                self.snapshot = *state;
                self.publish_snapshot();
            }
            Message::Armed {
                generation,
                accepted,
            } => {
                if self.arm == Some((generation, self.revision.load(Ordering::Acquire))) {
                    self.arm = None;
                    if accepted
                        && self.link.profile_requested()
                        && self.capture_request == self.link.profile_request()
                    {
                        self.armed = true;
                        self.link.set_ready(self.revision.load(Ordering::Acquire));
                        self.switching_since = None;
                        self.publish_snapshot();
                    }
                }
            }
            Message::Reply { id, result } => {
                if let Some(pending) = self.pending.remove(&id) {
                    let result = if pending.generation == self.revision.load(Ordering::Acquire) {
                        result.map_err(Into::into)
                    } else {
                        Err(BackendError::Stale)
                    };
                    let _ = pending.reply.send(result);
                }
            }
            Message::Failure(error) => self.link.fail(error),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn supervisor() -> Supervisor {
        let revision = Arc::new(AtomicU64::new(7));
        let (link, input) = InputLink::channel(revision.clone());
        let (_, control) = mpsc::sync_channel(1);
        let (updates, _) = watch::channel(Snapshot::default());
        Supervisor {
            process: None,
            control,
            input,
            updates,
            link,
            revision,
            stopping: Arc::new(AtomicBool::new(false)),
            profile: Profile::MediaOnly,
            publish: true,
            selected: Some("phone".into()),
            remembered: None,
            snapshot: Snapshot::default(),
            native_generation: Some(3),
            native_selected: Some("phone".into()),
            pairing: None,
            blocked_native: None,
            arm: None,
            armed: false,
            capture_request: 0,
            switching_since: None,
            pending: BTreeMap::new(),
            next_id: 0,
        }
    }

    #[test]
    fn profile_reconnect_uses_physical_identity_and_keeps_the_ui_selection() {
        let mut supervisor = supervisor();
        supervisor.snapshot.targets.push(Target {
            id: "phone".into(),
            identity: vec!["physical:phone".into()],
            ..Default::default()
        });
        let Command::Init {
            remembered,
            selected,
            ..
        } = supervisor.initial_command(Profile::Full, true)
        else {
            panic!("wrong command");
        };
        assert!(selected.is_none());
        assert_eq!(remembered.unwrap().identity, vec!["physical:phone"]);
        supervisor
            .message(Message::Status {
                state: Box::new(Snapshot {
                    generation: 3,
                    ready: true,
                    selected: Some("new-endpoint".into()),
                    targets: vec![Target {
                        id: "new-endpoint".into(),
                        identity: vec!["physical:phone".into()],
                        ..Default::default()
                    }],
                    ..Default::default()
                }),
                input_available: false,
            })
            .unwrap();
        assert_eq!(supervisor.snapshot.selected.as_deref(), Some("phone"));
        assert!(supervisor.snapshot.selected_target().is_some());
        assert_eq!(supervisor.native_selected.as_deref(), Some("new-endpoint"));
    }

    #[test]
    fn restarting_without_a_selection_finishes_when_advertising_is_ready() {
        let mut supervisor = supervisor();
        supervisor.selected = None;
        supervisor.switching_since = Some(Instant::now());
        supervisor
            .message(Message::Status {
                state: Box::new(Snapshot {
                    generation: 3,
                    service: true,
                    broadcasting: true,
                    ..Default::default()
                }),
                input_available: false,
            })
            .unwrap();
        assert!(supervisor.switching_since.is_none());
        assert!(!supervisor.snapshot.profile_switching);
        assert!(!supervisor.snapshot.ready);
    }

    #[test]
    fn keyboard_requires_explicit_request_and_disconnect_cannot_publish_it() {
        let mut supervisor = supervisor();
        assert_eq!(supervisor.desired_profile(), Profile::MediaOnly);
        supervisor.link.set_profile_available(true);
        assert!(supervisor.link.request_profile());
        assert_eq!(supervisor.desired_profile(), Profile::Full);
        supervisor.publish = false;
        assert_eq!(supervisor.desired_profile(), Profile::MediaOnly);
    }

    #[test]
    fn late_arm_ack_cannot_undo_cancellation_or_generation_change() {
        for scenario in 0..3 {
            let mut supervisor = supervisor();
            supervisor.profile = Profile::Full;
            supervisor.link.set_profile_available(true);
            supervisor.link.request_profile();
            supervisor.capture_request = supervisor.link.profile_request();
            supervisor.arm = Some((3, 7));
            if scenario != 1 {
                supervisor.link.end();
                if scenario == 2 {
                    supervisor.link.request_profile();
                }
            } else {
                supervisor.revision.fetch_add(1, Ordering::AcqRel);
            }
            supervisor
                .message(Message::Armed {
                    generation: 3,
                    accepted: true,
                })
                .unwrap();
            assert!(!supervisor.link.ready());
            assert!(!supervisor.armed);
        }
    }

    #[test]
    fn capture_is_authorized_only_after_matching_worker_ack() {
        let mut supervisor = supervisor();
        supervisor.profile = Profile::Full;
        supervisor.link.set_profile_available(true);
        supervisor.link.request_profile();
        supervisor.capture_request = supervisor.link.profile_request();
        supervisor.arm = Some((3, 7));
        assert!(!supervisor.link.begin());
        supervisor
            .message(Message::Armed {
                generation: 3,
                accepted: true,
            })
            .unwrap();
        assert!(supervisor.link.begin());
        supervisor
            .message(Message::Status {
                state: Box::new(Snapshot {
                    generation: 3,
                    ..Default::default()
                }),
                input_available: false,
            })
            .unwrap();
        assert_eq!(supervisor.link.epoch(), 0);
        assert!(!supervisor.link.profile_requested());
        assert_eq!(supervisor.desired_profile(), Profile::MediaOnly);
    }

    #[test]
    fn media_subscription_does_not_authorize_keyboard_capture() {
        let mut supervisor = supervisor();
        supervisor
            .message(Message::Status {
                state: Box::new(Snapshot {
                    generation: 3,
                    ready: true,
                    selected: Some("phone".into()),
                    ..Default::default()
                }),
                input_available: false,
            })
            .unwrap();
        assert!(supervisor.snapshot.ready);
        assert!(!supervisor.link.ready());
        assert!(!supervisor.link.begin());
        assert!(supervisor.link.request_profile());
    }

    #[test]
    fn previous_generation_delivery_does_not_complete_new_session() {
        let mut supervisor = supervisor();
        let (reply, mut result) = oneshot::channel();
        supervisor.pending.insert(
            1,
            Pending {
                generation: 6,
                created: Instant::now(),
                reply,
            },
        );
        supervisor
            .message(Message::Reply {
                id: 1,
                result: Ok(()),
            })
            .unwrap();
        assert!(matches!(
            result.try_recv().unwrap(),
            Err(BackendError::Stale)
        ));
    }
}
