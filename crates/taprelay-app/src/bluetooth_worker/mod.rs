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
        Self::start_with(remembered, process::Native)
    }

    fn start_with<E: Execution>(remembered: Option<Target>, execution: E) -> Result<Self> {
        let revision = Arc::new(AtomicU64::new(1));
        let (link, input) = InputLink::channel(revision.clone());
        let (commands, control) = mpsc::sync_channel(16);
        let (updates, state) = watch::channel(Snapshot::default());
        let stopping = Arc::new(AtomicBool::new(false));
        let mut supervisor = Supervisor {
            execution,
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
        let worker = thread::Builder::new()
            .name("taprelay-bluetooth-supervisor".into())
            .spawn(move || {
                if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| supervisor.run()))
                    .is_err()
                {
                    let _ =
                        supervisor.finish(Err(anyhow::anyhow!("Bluetooth supervisor panicked")));
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

trait WorkerProcess: Send {
    fn send(&self, command: Command) -> Result<()>;
    fn receive(&self) -> Result<Option<Message>>;
    fn check(&mut self) -> Result<()>;
    fn stop(&mut self) -> Result<()>;
}

trait Execution: Send + 'static {
    type Worker: WorkerProcess;
    fn start(&mut self, initial: Command) -> Result<Self::Worker>;
    fn now(&self) -> Instant;
    fn wait(&mut self, duration: Duration);
}

struct Supervisor<E: Execution> {
    execution: E,
    process: Option<E::Worker>,
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

impl<E: Execution> Supervisor<E> {
    fn run(&mut self) -> Result<()> {
        let result = self.run_loop();
        self.finish(result)
    }

    fn finish(&mut self, mut result: Result<()>) -> Result<()> {
        self.link.end();
        self.link.set_profile_available(false);
        for (_, pending) in std::mem::take(&mut self.pending) {
            let _ = pending.reply.send(Err(BackendError::Stale));
        }
        while let Ok(command) = self.control.try_recv() {
            if let Control::Send(_, reply) = command {
                let _ = reply.send(Err(BackendError::Stale));
            }
        }
        taprelay_core::devices::revoke_session(&mut self.snapshot);
        self.switching_since = None;
        self.snapshot.service = false;
        self.snapshot.broadcasting = false;
        self.publish_snapshot();
        if let Some(mut worker) = self.process.take()
            && let Err(error) = worker.stop()
            && result.is_ok()
        {
            result = Err(error);
        }
        if let Err(error) = &result {
            let message = format!("{error:#}");
            tracing::error!(%message, "Bluetooth supervisor stopped");
            self.link.fail(message.clone());
            self.snapshot.last_error = Some(message);
            self.publish_snapshot();
        }
        result
    }

    fn run_loop(&mut self) -> Result<()> {
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
            self.execution.wait(Duration::from_millis(5));
        }
        Ok(())
    }

    fn worker(&self) -> Result<&E::Worker> {
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
        self.switching_since = (replacing && publish).then(|| self.execution.now());
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
        self.process = Some(self.execution.start(initial)?);
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
            let Some(message) = self.worker()?.receive()? else {
                break;
            };
            self.message(message)?;
        }
        self.process
            .as_mut()
            .context("Bluetooth worker unavailable")?
            .check()?;
        if self.pairing.as_ref().is_some_and(|(_, since)| {
            self.execution.now().saturating_duration_since(*since) >= Duration::from_secs(30)
        }) {
            self.pairing = None;
            self.link.fail(
                "Pairing target was not found after restarting Bluetooth; refresh and try again",
            );
        }
        if self.switching_since.is_some_and(|since| {
            self.execution.now().saturating_duration_since(since) >= Duration::from_secs(30)
        }) {
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
            if self
                .execution
                .now()
                .saturating_duration_since(packet.captured)
                > MAX_INPUT_AGE
            {
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
            .filter(|(_, pending)| {
                self.execution
                    .now()
                    .saturating_duration_since(pending.created)
                    >= Duration::from_secs(2)
            })
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
                    self.pairing = Some((target, self.execution.now()));
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
                let sent = self.worker()?.send(Command::Media {
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
                });
                if let Err(error) = sent {
                    let _ = reply.send(Err(BackendError::Unavailable(error.to_string())));
                    return Err(error);
                }
                self.pending.insert(
                    self.next_id,
                    Pending {
                        generation: command.generation,
                        created: self.execution.now(),
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
                if self
                    .native_generation
                    .is_some_and(|current| generation < current)
                {
                    return Ok(());
                }
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
mod tests;
