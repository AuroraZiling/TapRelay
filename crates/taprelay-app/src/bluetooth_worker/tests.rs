use super::*;
use crate::platform::Transport;
use std::{collections::VecDeque, sync::Mutex};
use taprelay_core::{command::MediaCommand, passthrough::Event};

#[derive(Debug)]
enum Call {
    Start(usize, Command),
    Send(usize, Command),
    Stop(usize),
}

#[derive(Default)]
struct ScriptWorker {
    incoming: VecDeque<Message>,
    crash: bool,
    backpressure: bool,
    stop_error: bool,
    before_stop: Option<Box<dyn FnOnce() + Send>>,
}

struct ScriptState {
    now: Instant,
    workers: Vec<ScriptWorker>,
    calls: Vec<Call>,
    fail_start: bool,
}

struct Script {
    state: Arc<Mutex<ScriptState>>,
    arrived: mpsc::SyncSender<()>,
    resume: mpsc::Receiver<Duration>,
}

struct ScriptProcess {
    id: usize,
    state: Arc<Mutex<ScriptState>>,
}

impl Execution for Script {
    type Worker = ScriptProcess;

    fn start(&mut self, initial: Command) -> Result<ScriptProcess> {
        let mut state = self.state.lock().unwrap();
        if std::mem::take(&mut state.fail_start) {
            anyhow::bail!("scripted start failure");
        }
        let id = state.workers.len();
        state.calls.push(Call::Start(id, initial));
        state.workers.push(ScriptWorker::default());
        Ok(ScriptProcess {
            id,
            state: self.state.clone(),
        })
    }

    fn now(&self) -> Instant {
        self.state.lock().unwrap().now
    }

    fn wait(&mut self, _: Duration) {
        self.arrived.send(()).unwrap();
        let elapsed = self
            .resume
            .recv_timeout(Duration::from_secs(10))
            .expect("test did not advance supervisor");
        self.state.lock().unwrap().now += elapsed;
    }
}

impl WorkerProcess for ScriptProcess {
    fn send(&self, command: Command) -> Result<()> {
        let mut state = self.state.lock().unwrap();
        anyhow::ensure!(!state.workers[self.id].backpressure, "scripted queue full");
        state.calls.push(Call::Send(self.id, command));
        Ok(())
    }

    fn receive(&self) -> Result<Option<Message>> {
        Ok(self.state.lock().unwrap().workers[self.id]
            .incoming
            .pop_front())
    }

    fn check(&mut self) -> Result<()> {
        anyhow::ensure!(
            !self.state.lock().unwrap().workers[self.id].crash,
            "scripted process exited"
        );
        Ok(())
    }

    fn stop(&mut self) -> Result<()> {
        let callback = self.state.lock().unwrap().workers[self.id]
            .before_stop
            .take();
        if let Some(callback) = callback {
            callback();
        }
        let mut state = self.state.lock().unwrap();
        state.calls.push(Call::Stop(self.id));
        anyhow::ensure!(!state.workers[self.id].stop_error, "scripted stop failure");
        Ok(())
    }
}

struct Harness {
    handle: Handle,
    state: Arc<Mutex<ScriptState>>,
    arrived: mpsc::Receiver<()>,
    resume: mpsc::SyncSender<Duration>,
}

impl Harness {
    fn build(fail_start: bool) -> Self {
        let state = Arc::new(Mutex::new(ScriptState {
            now: Instant::now(),
            workers: vec![],
            calls: vec![],
            fail_start,
        }));
        let (arrived_tx, arrived) = mpsc::sync_channel(1);
        let (resume, resume_rx) = mpsc::sync_channel(1);
        let handle = Handle::start_with(
            None,
            Script {
                state: state.clone(),
                arrived: arrived_tx,
                resume: resume_rx,
            },
        )
        .unwrap();
        Self {
            handle,
            state,
            arrived,
            resume,
        }
    }

    fn new() -> Self {
        let harness = Self::build(false);
        assert!(harness.settle());
        harness
    }

    fn settle(&self) -> bool {
        match self.arrived.recv_timeout(Duration::from_secs(10)) {
            Ok(()) => true,
            Err(mpsc::RecvTimeoutError::Disconnected) => false,
            Err(error) => panic!("supervisor did not reach a boundary: {error}"),
        }
    }

    fn advance(&self, elapsed: Duration) -> bool {
        self.resume.send(elapsed).unwrap();
        self.settle()
    }

    fn step(&self) {
        assert!(self.advance(Duration::ZERO));
    }

    fn snapshot(&self) -> Snapshot {
        self.handle.state.borrow().clone()
    }

    fn current(&self) -> usize {
        self.state.lock().unwrap().workers.len() - 1
    }

    fn message(&self, id: usize, message: Message) {
        self.state.lock().unwrap().workers[id]
            .incoming
            .push_back(message);
    }

    fn status(&self, generation: u64, endpoint: &str, input_available: bool) {
        self.message(
            self.current(),
            status(generation, endpoint, input_available),
        );
        self.step();
    }

    fn connected(&self) {
        self.handle.select("phone".into()).unwrap();
        self.step();
        self.status(3, "phone", false);
        assert!(self.snapshot().ready);
        assert!(!self.handle.link.begin());
    }

    fn full(&self) {
        assert!(self.handle.link.request_profile());
        self.step();
        assert_eq!(self.snapshot().hid_profile, Profile::Full);
        self.status(3, "phone", true);
        assert!(!self.handle.link.begin());
    }

    fn arm(&self) {
        self.message(
            self.current(),
            Message::Armed {
                generation: 3,
                accepted: true,
            },
        );
        self.step();
        assert!(self.handle.link.begin());
    }

    fn send(&self) -> oneshot::Receiver<Result<(), BackendError>> {
        self.handle
            .send(QueuedCommand::press(
                MediaCommand::PlayPause,
                "phone".into(),
                self.snapshot().generation,
                Instant::now(),
            ))
            .unwrap()
    }

    fn media_id(&self) -> u64 {
        let current = self.current();
        self.state
            .lock()
            .unwrap()
            .calls
            .iter()
            .rev()
            .find_map(|call| match call {
                Call::Send(process, Command::Media { id, .. }) if *process == current => Some(*id),
                _ => None,
            })
            .unwrap()
    }

    fn stop(&mut self) {
        self.handle.link.end();
        self.handle.stopping.store(true, Ordering::Release);
        let _ = self.resume.try_send(Duration::ZERO);
        if let Some(worker) = self.handle.worker.take() {
            worker.join().unwrap();
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.stop();
    }
}

fn status(generation: u64, endpoint: &str, input_available: bool) -> Message {
    Message::Status {
        state: Box::new(Snapshot {
            generation,
            ready: true,
            service: true,
            broadcasting: true,
            selected: Some(endpoint.into()),
            targets: vec![Target {
                id: endpoint.into(),
                identity: vec!["physical:phone".into()],
                ..Default::default()
            }],
            ..Default::default()
        }),
        input_available,
    }
}

#[test]
fn replacement_retires_receipts_and_revokes_input_before_stop_then_starts_full() {
    let h = Harness::new();
    h.connected();
    let old = h.current();
    let mut receipt = h.send();
    h.step();
    let link = h.handle.link.clone();
    let state = h.handle.state.clone();
    h.state.lock().unwrap().workers[old].before_stop = Some(Box::new(move || {
        assert!(!link.ready());
        assert_eq!(link.epoch(), 0);
        assert!(!link.request_profile());
        assert!(!state.borrow().ready);
        assert!(matches!(
            receipt.try_recv().unwrap(),
            Err(BackendError::Stale)
        ));
    }));
    h.full();
    let state = h.state.lock().unwrap();
    let stop = state
        .calls
        .iter()
        .position(|call| matches!(call, Call::Stop(id) if *id == old))
        .unwrap();
    assert!(
        matches!(&state.calls[stop + 1], Call::Start(id, Command::Init { profile: Profile::Full, remembered: Some(target), selected: None, .. }) if *id == old + 1 && target.identity == ["physical:phone"])
    );
}

#[test]
fn only_matching_arm_authorizes_capture_and_losing_input_returns_to_media() {
    let h = Harness::new();
    h.connected();
    h.full();
    h.message(
        h.current(),
        Message::Armed {
            generation: 3,
            accepted: false,
        },
    );
    h.step();
    assert!(!h.handle.link.begin());
    h.status(3, "phone", true);
    assert!(
        h.state
            .lock()
            .unwrap()
            .calls
            .iter()
            .any(|call| matches!(call, Call::Send(_, Command::Arm { generation: 3 })))
    );
    h.message(
        h.current(),
        Message::Armed {
            generation: 2,
            accepted: true,
        },
    );
    h.step();
    assert!(!h.handle.link.begin());
    h.arm();
    h.handle.link.set_mouse_percent(50);
    h.handle.link.set_reverse_scroll(true);
    assert!(
        h.handle
            .link
            .submit(Event::Motion { dx: 5, dy: -2 }, Instant::now())
    );
    h.step();
    assert!(h.state.lock().unwrap().calls.iter().any(|call| matches!(
        call,
        Call::Send(
            _,
            Command::Input {
                generation: 3,
                event: Event::Motion { dx: 5, dy: -2 },
                mouse_percent: 50,
                reverse_scroll: true,
                ..
            }
        )
    )));
    h.message(h.current(), status(3, "phone", false));
    h.step();
    assert_eq!(h.handle.link.epoch(), 0);
    assert!(!h.handle.link.profile_requested());
    h.step();
    assert_eq!(h.snapshot().hid_profile, Profile::MediaOnly);
}

#[test]
fn cancellation_and_new_request_cannot_be_undone_by_old_process_arm_or_status() {
    let h = Harness::new();
    h.connected();
    h.full();
    let old = h.current();
    h.handle.link.end();
    h.message(
        old,
        Message::Armed {
            generation: 3,
            accepted: true,
        },
    );
    h.message(old, status(99, "stale-endpoint", true));
    h.step();
    assert_eq!(h.snapshot().hid_profile, Profile::MediaOnly);
    assert!(!h.snapshot().ready);
    assert!(!h.handle.link.begin());
    h.status(3, "phone", false);
    h.full();
    assert!(!h.handle.link.begin());
    h.message(
        old,
        Message::Armed {
            generation: 3,
            accepted: true,
        },
    );
    h.step();
    assert!(!h.handle.link.begin());
    h.arm();
}

#[test]
fn generation_change_rejects_old_status_arm_and_delivery() {
    let h = Harness::new();
    h.connected();
    let mut receipt = h.send();
    h.step();
    let id = h.media_id();
    h.status(4, "phone", false);
    let generation = h.snapshot().generation;
    h.message(h.current(), status(3, "stale-endpoint", false));
    h.message(h.current(), Message::Reply { id, result: Ok(()) });
    h.step();
    assert_eq!(h.snapshot().generation, generation);
    assert_eq!(h.snapshot().selected_target().unwrap().id, "phone");
    assert!(matches!(
        receipt.try_recv().unwrap(),
        Err(BackendError::Stale)
    ));
    h.full();
    h.status(4, "phone", true);
    h.message(
        h.current(),
        Message::Armed {
            generation: 3,
            accepted: true,
        },
    );
    h.step();
    assert!(!h.handle.link.begin());
    h.message(
        h.current(),
        Message::Armed {
            generation: 4,
            accepted: true,
        },
    );
    h.step();
    assert!(h.handle.link.begin());
}

#[test]
fn reconnect_uses_physical_identity_and_maps_media_to_the_new_endpoint() {
    let h = Harness::new();
    h.connected();
    h.full();
    h.status(3, "new-endpoint", true);
    h.arm();
    let snapshot = h.snapshot();
    assert_eq!(snapshot.selected.as_deref(), Some("phone"));
    assert_eq!(snapshot.selected_target().unwrap().id, "new-endpoint");
    let _receipt = h.send();
    h.step();
    assert!(h.state.lock().unwrap().calls.iter().any(|call| matches!(call, Call::Send(_, Command::Media { target, .. }) if target == "new-endpoint")));
}

#[test]
fn full_crash_start_failure_and_timeout_fall_back_to_media_with_observable_error() {
    for failure in 0..3 {
        let h = Harness::new();
        h.connected();
        if failure == 0 {
            h.state.lock().unwrap().fail_start = true;
            assert!(h.handle.link.request_profile());
            h.step();
        } else {
            h.full();
            if failure == 1 {
                h.arm();
                let current = h.current();
                h.state.lock().unwrap().workers[current].crash = true;
                h.step();
            } else {
                assert!(h.advance(Duration::from_secs(29)));
                assert_eq!(h.snapshot().hid_profile, Profile::Full);
                assert!(h.advance(Duration::from_secs(1)));
            }
        }
        assert_eq!(h.snapshot().hid_profile, Profile::MediaOnly);
        assert!(!h.snapshot().ready);
        assert_eq!(h.handle.link.epoch(), 0);
        assert!(!h.handle.link.profile_requested());
        assert!(
            h.handle
                .link
                .take_failure()
                .unwrap()
                .contains("Passthrough stopped")
        );
        h.status(3, "phone", false);
        assert!(h.snapshot().ready);
    }
}

#[test]
fn media_start_and_runtime_failures_publish_terminal_unavailability() {
    let h = Harness::build(true);
    assert!(!h.settle());
    assert!(!h.snapshot().ready && !h.snapshot().service);
    assert!(h.snapshot().last_error.unwrap().contains("start failure"));
    let h = Harness::new();
    h.connected();
    let mut receipt = h.send();
    h.step();
    let current = h.current();
    h.state.lock().unwrap().workers[current].crash = true;
    assert!(!h.advance(Duration::ZERO));
    assert!(!h.snapshot().ready && !h.snapshot().service && !h.snapshot().broadcasting);
    assert!(h.snapshot().selected.is_none());
    assert!(h.snapshot().last_error.unwrap().contains("process exited"));
    assert!(matches!(
        receipt.try_recv().unwrap(),
        Err(BackendError::Stale)
    ));
}

#[test]
fn receipt_deadlines_and_late_replies_do_not_complete_another_request() {
    let h = Harness::new();
    h.connected();
    let mut first = h.send();
    h.step();
    let old_id = h.media_id();
    assert!(h.advance(Duration::from_millis(1999)));
    assert!(matches!(
        first.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    assert!(h.advance(Duration::from_millis(1)));
    assert!(
        matches!(first.try_recv().unwrap(), Err(BackendError::Unavailable(message)) if message.contains("timed out"))
    );
    let mut next = h.send();
    h.step();
    let next_id = h.media_id();
    assert_ne!(old_id, next_id);
    h.message(
        h.current(),
        Message::Reply {
            id: old_id,
            result: Ok(()),
        },
    );
    h.step();
    assert!(matches!(
        next.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    h.message(
        h.current(),
        Message::Reply {
            id: next_id,
            result: Ok(()),
        },
    );
    h.step();
    assert!(next.try_recv().unwrap().is_ok());
}

#[test]
fn pending_limit_and_worker_backpressure_give_requests_a_terminal_result() {
    let h = Harness::new();
    h.connected();
    let mut pending = Vec::new();
    for _ in 0..32 {
        pending.push(h.send());
        h.step();
    }
    let mut overflow = h.send();
    h.step();
    assert!(matches!(
        overflow.try_recv().unwrap(),
        Err(BackendError::Stale)
    ));
    assert!(h.advance(Duration::from_secs(2)));
    assert!(
        pending
            .iter_mut()
            .all(|reply| matches!(reply.try_recv().unwrap(), Err(BackendError::Unavailable(_))))
    );
    let current = h.current();
    h.state.lock().unwrap().workers[current].backpressure = true;
    let mut rejected = h.send();
    assert!(!h.advance(Duration::ZERO));
    assert!(
        matches!(rejected.try_recv().unwrap(), Err(BackendError::Unavailable(message)) if message.contains("queue full"))
    );
    assert!(!h.snapshot().ready);
}

#[test]
fn shutdown_retires_queued_and_pending_requests_without_sending_queued_input() {
    let mut h = Harness::new();
    h.connected();
    h.full();
    h.arm();
    let mut pending = h.send();
    h.step();
    let mut queued = h.send();
    assert!(
        h.handle
            .link
            .submit(Event::Motion { dx: 1, dy: 2 }, Instant::now())
    );
    let old = h.current();
    h.message(
        old,
        Message::Reply {
            id: h.media_id(),
            result: Ok(()),
        },
    );
    h.stop();
    assert!(matches!(
        pending.try_recv().unwrap(),
        Err(BackendError::Stale)
    ));
    assert!(matches!(
        queued.try_recv().unwrap(),
        Err(BackendError::Stale)
    ));
    assert!(!h.snapshot().ready);
    assert_eq!(h.handle.link.epoch(), 0);
    assert!(
        !h.state
            .lock()
            .unwrap()
            .calls
            .iter()
            .any(|call| matches!(call, Call::Send(_, Command::Input { .. })))
    );
}

#[test]
fn stale_passthrough_input_causes_fallback_and_is_not_replayed() {
    let h = Harness::new();
    h.connected();
    h.full();
    h.arm();
    let captured = h.state.lock().unwrap().now;
    assert!(
        h.handle
            .link
            .submit(Event::Motion { dx: 1, dy: 2 }, captured)
    );
    assert!(h.advance(MAX_INPUT_AGE + Duration::from_millis(1)));
    assert_eq!(h.snapshot().hid_profile, Profile::MediaOnly);
    assert!(h.handle.link.take_failure().unwrap().contains("250 ms"));
    assert!(
        !h.state
            .lock()
            .unwrap()
            .calls
            .iter()
            .any(|call| matches!(call, Call::Send(_, Command::Input { .. })))
    );
}

#[test]
fn disconnect_cannot_publish_full_and_restart_completes_when_advertising() {
    let h = Harness::new();
    h.connected();
    h.full();
    h.handle.disconnect().unwrap();
    h.step();
    assert_eq!(h.snapshot().hid_profile, Profile::MediaOnly);
    assert!(h.snapshot().service_paused);
    assert!(!h.handle.link.request_profile());
    h.handle.restart().unwrap();
    h.step();
    assert!(h.snapshot().profile_switching);
    h.message(
        h.current(),
        Message::Status {
            state: Box::new(Snapshot {
                generation: 3,
                service: true,
                broadcasting: true,
                ..Default::default()
            }),
            input_available: false,
        },
    );
    h.step();
    assert!(!h.snapshot().profile_switching);
    assert!(!h.snapshot().ready);
}

#[test]
fn failure_to_stop_old_process_never_starts_a_new_service_owner() {
    let h = Harness::new();
    h.connected();
    let current = h.current();
    h.state.lock().unwrap().workers[current].stop_error = true;
    assert!(h.handle.link.request_profile());
    assert!(!h.advance(Duration::ZERO));
    assert_eq!(h.current(), current);
    assert!(!h.snapshot().ready);
    assert!(h.snapshot().last_error.unwrap().contains("stop failure"));
}

#[test]
fn renewed_full_request_rebuilds_and_discards_old_packets_and_replies() {
    let h = Harness::new();
    h.connected();
    h.full();
    h.arm();
    let old = h.current();
    let mut retired = h.send();
    h.step();
    let old_id = h.media_id();
    assert!(
        h.handle
            .link
            .submit(Event::Motion { dx: 3, dy: 4 }, Instant::now())
    );
    let previous_request = h.handle.link.profile_request();
    h.handle.link.end();
    assert!(h.handle.link.request_profile());
    assert_ne!(h.handle.link.profile_request(), previous_request);
    h.message(
        old,
        Message::Armed {
            generation: 3,
            accepted: true,
        },
    );
    h.step();
    assert_ne!(h.current(), old);
    assert!(!h.handle.link.begin());
    assert!(matches!(
        retired.try_recv().unwrap(),
        Err(BackendError::Stale)
    ));
    h.status(3, "phone", true);
    h.arm();
    let mut next = h.send();
    h.step();
    let next_id = h.media_id();
    assert_ne!(old_id, next_id);
    h.message(
        old,
        Message::Reply {
            id: next_id,
            result: Ok(()),
        },
    );
    h.message(
        h.current(),
        Message::Reply {
            id: old_id,
            result: Ok(()),
        },
    );
    h.step();
    assert!(matches!(
        next.try_recv(),
        Err(oneshot::error::TryRecvError::Empty)
    ));
    assert!(
        !h.state
            .lock()
            .unwrap()
            .calls
            .iter()
            .any(|call| matches!(call, Call::Send(_, Command::Input { .. })))
    );
    h.message(
        h.current(),
        Message::Reply {
            id: next_id,
            result: Ok(()),
        },
    );
    h.step();
    assert!(next.try_recv().unwrap().is_ok());
}

#[test]
fn media_reconnect_and_pairing_deadlines_use_the_same_clock_as_full_switches() {
    let h = Harness::new();
    h.handle.select("phone".into()).unwrap();
    h.step();
    h.handle.pair("phone".into()).unwrap();
    h.step();
    assert!(h.advance(Duration::from_secs(29)));
    assert!(h.snapshot().profile_switching);
    assert!(h.handle.link.take_failure().is_none());
    assert!(h.advance(Duration::from_secs(1)));
    assert!(!h.snapshot().profile_switching);
    assert!(!h.snapshot().ready);
    assert!(
        h.snapshot()
            .last_error
            .unwrap()
            .contains("reconnect within 30 seconds")
    );
    assert!(
        h.handle
            .link
            .take_failure()
            .unwrap()
            .contains("Pairing target was not found")
    );
    h.status(3, "phone", false);
    assert!(h.snapshot().ready);
    assert!(
        !h.state
            .lock()
            .unwrap()
            .calls
            .iter()
            .any(|call| matches!(call, Call::Send(_, Command::Pair(_))))
    );
}

#[test]
fn a_full_control_queue_rejects_a_request_without_sending_it_later() {
    let h = Harness::new();
    h.connected();
    for _ in 0..16 {
        h.handle.refresh().unwrap();
    }
    assert!(
        h.handle
            .send(QueuedCommand::press(
                MediaCommand::PlayPause,
                "phone".into(),
                h.snapshot().generation,
                Instant::now(),
            ))
            .is_err()
    );
    h.step();
    assert!(
        !h.state
            .lock()
            .unwrap()
            .calls
            .iter()
            .any(|call| matches!(call, Call::Send(_, Command::Media { .. })))
    );
    let mut accepted = h.send();
    h.step();
    h.message(
        h.current(),
        Message::Reply {
            id: h.media_id(),
            result: Ok(()),
        },
    );
    h.step();
    assert!(accepted.try_recv().unwrap().is_ok());
}
