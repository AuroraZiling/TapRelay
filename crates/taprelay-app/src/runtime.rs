//! Independent input and transport lifetimes. Configuration and navigation do not gate either.
use crate::{
    config::Config,
    feedback::TestStatus,
    platform::{self, InputSource, Transport},
};
use anyhow::{Context, Result};
use std::time::Instant;
use taprelay_core::{
    binding::BindingIndex,
    command::{COMMAND_TTL, MediaCommand, QueuedCommand},
    input::{InputCode, InputEvent, InputState, Recorder, Trigger},
    state::Snapshot,
};
use tokio::sync::{mpsc, oneshot};

#[derive(Clone, Copy, PartialEq, Eq)]
enum CapturePhase {
    Idle,
    WaitingForRelease,
    Recording,
}
struct Receipt {
    created: Instant,
    test: Option<u64>,
    result: oneshot::Receiver<Result<(), String>>,
}

pub struct Runtime {
    pub config: Config,
    pub state: Snapshot,
    pub learned: Option<Trigger>,
    pub matched: u64,
    pub delivered: u64,
    pub error: Option<String>,
    pub listening: bool,
    capture: CapturePhase,
    pub capture_preview: String,
    pub capture_cancelled: bool,
    input: Option<Box<dyn InputSource>>,
    transport: Option<Box<dyn Transport>>,
    events: mpsc::Receiver<InputEvent>,
    sender: mpsc::Sender<InputEvent>,
    gate: InputState,
    recorder: Recorder,
    pub window_keys: Vec<InputEvent>,
    keyboard_from_window: Option<bool>,
    epoch: Instant,
    required_generation: u64,
    ui_input_until: Instant,
    receipts: Vec<Receipt>,
    pub test: TestStatus,
    next_test: u64,
    bindings: BindingIndex,
    pub bindings_revision: u64,
}
impl Runtime {
    pub fn new(mut config: Config) -> Self {
        // Legacy configs may contain disabled rows; every listed binding is now active.
        for binding in &mut config.bindings {
            binding.enabled = true;
        }
        // Device selection is intentionally session-only. Older config files may still
        // deserialize a device record, but it must never be restored or used.
        config.device = None;
        let (sender, events) = mpsc::channel(256);
        let bindings = BindingIndex::new(&config.bindings);
        Self {
            config,
            state: Snapshot {
                ..Default::default()
            },
            learned: None,
            matched: 0,
            delivered: 0,
            error: None,
            listening: false,
            capture: CapturePhase::Idle,
            capture_preview: String::new(),
            capture_cancelled: false,
            input: None,
            transport: None,
            events,
            sender,
            gate: InputState::default(),
            recorder: Recorder::default(),
            window_keys: vec![],
            keyboard_from_window: None,
            epoch: Instant::now(),
            required_generation: 0,
            ui_input_until: Instant::now(),
            receipts: vec![],
            test: TestStatus::Idle,
            next_test: 0,
            bindings,
            bindings_revision: 0,
        }
    }
    pub fn start_bluetooth(&mut self) -> Result<()> {
        self.start_bluetooth_with(platform::transport)
    }
    fn start_bluetooth_with(
        &mut self,
        create: impl FnOnce() -> Result<Box<dyn Transport>>,
    ) -> Result<()> {
        self.state.ready = false;
        self.state.service = false;
        self.state.broadcasting = false;
        self.state.selected = None;
        self.state.target_status = None;
        self.state.last_error = None;
        self.error = None;
        self.invalidate();
        if self.transport.as_ref().is_some_and(|t| t.is_finished()) {
            self.transport.take();
        }
        if let Some(t) = &self.transport {
            self.required_generation = t.restart()?;
            return Ok(());
        }
        self.required_generation = 0;
        self.transport = Some(create()?);
        Ok(())
    }
    pub fn pair(&mut self, id: String) -> Result<()> {
        if self.state.selected.as_ref() == Some(&id)
            && self.state.pairing_handoff == taprelay_core::devices::PairingHandoff::WaitingForOS
        {
            return Ok(());
        }
        self.choose(id.clone())?;
        self.transport
            .as_ref()
            .context("Bluetooth not started")?
            .pair(id)?;
        self.state.pairing_handoff = taprelay_core::devices::PairingHandoff::WaitingForOS;
        Ok(())
    }
    pub fn disconnect(&mut self) -> Result<()> {
        let result = self
            .transport
            .as_ref()
            .context("Bluetooth not started")?
            .disconnect();
        self.state.ready = false;
        self.state.selected = None;
        self.state.target_status = None;
        self.invalidate();
        self.required_generation = result?;
        Ok(())
    }
    pub fn bluetooth_settings(&self) -> Result<()> {
        self.transport
            .as_ref()
            .context("Bluetooth not started")?
            .bluetooth_settings()
    }
    pub fn refresh(&self) -> Result<()> {
        self.transport
            .as_ref()
            .context("Bluetooth not started")?
            .refresh()
    }
    pub fn choose(&mut self, id: String) -> Result<()> {
        if self.state.selected.as_ref() == Some(&id)
            && self.state.target_status.as_ref().is_some_and(|t| {
                matches!(
                    t.connection,
                    taprelay_core::devices::Connection::Connecting
                        | taprelay_core::devices::Connection::AwaitingHostSubscription
                        | taprelay_core::devices::Connection::Synchronizing
                        | taprelay_core::devices::Connection::Connected
                )
            })
        {
            return Ok(());
        }
        self.required_generation = self
            .transport
            .as_ref()
            .context("Bluetooth not started")?
            .select(id.clone())?;
        self.state.ready = false;
        self.state.selected = Some(id);
        self.state.target_status = None;
        self.invalidate();
        Ok(())
    }
    fn ensure_input(&mut self) -> Result<()> {
        if self.input.as_ref().is_none_or(|i| i.is_finished()) {
            self.input.take();
            while self.events.try_recv().is_ok() {}
            self.gate = InputState::default();
            self.epoch = Instant::now();
            self.input = Some(platform::input(self.sender.clone())?);
        }
        Ok(())
    }
    pub fn set_listening(&mut self, on: bool) -> Result<()> {
        if on {
            self.ensure_input()?;
        }
        self.listening = on;
        if !on && !self.recording() {
            self.input.take();
        }
        if !on {
            self.invalidate();
        }
        Ok(())
    }
    fn invalidate(&mut self) {
        self.epoch = Instant::now();
        if matches!(self.test, TestStatus::Pending(_)) {
            self.test = TestStatus::Failed(
                "Test cancelled because the input or connection session changed".into(),
            );
        }
        if let Some(t) = &self.transport {
            t.invalidate();
        }
    }
    pub fn record(&mut self) -> Result<()> {
        self.ensure_input()?;
        self.invalidate();
        self.capture = CapturePhase::WaitingForRelease;
        self.learned = None;
        self.capture_cancelled = false;
        self.capture_preview.clear();
        self.recorder = Recorder::default();
        self.window_keys.clear();
        self.keyboard_from_window = None;
        Ok(())
    }
    pub fn finish_recording(&mut self) {
        if !self.recording() {
            return;
        }
        self.capture = CapturePhase::Idle;
        self.window_keys.clear();
        self.learned = None;
        self.capture_preview.clear();
        self.invalidate();
        self.gate = InputState::default();
        while self.events.try_recv().is_ok() {}
        if !self.listening {
            self.input.take();
        }
    }
    pub fn send(&mut self) -> Result<()> {
        anyhow::ensure!(
            !matches!(self.test, TestStatus::Pending(_)),
            "A test is already pending"
        );
        self.next_test += 1;
        let id = self.next_test;
        match self.enqueue(Instant::now(), Some(id)) {
            Ok(()) => {
                self.test = TestStatus::Pending(id);
                Ok(())
            }
            Err(e) => {
                self.test = TestStatus::Failed(e.to_string());
                Err(e)
            }
        }
    }
    pub fn recording(&self) -> bool {
        self.capture != CapturePhase::Idle
    }
    pub fn capture_waiting(&self) -> bool {
        self.capture == CapturePhase::WaitingForRelease
    }
    /// Call once after an accepted binding edit; options/navigation never rebuild matching.
    pub fn bindings_changed(&mut self) {
        self.bindings = BindingIndex::new(&self.config.bindings);
        self.bindings_revision += 1;
    }
    /// Native edges belonging to a GUI control must not also trigger a global shortcut.
    pub fn consume_ui_input(&mut self) {
        self.ui_input_until = Instant::now();
    }
    pub fn capture_key_labels(&self) -> Vec<String> {
        if self.recording() && !self.capture_waiting() {
            self.gate.key_labels()
        } else {
            vec![]
        }
    }
    fn send_at(&mut self, created: Instant) -> Result<()> {
        self.enqueue(created, None)
    }
    fn enqueue(&mut self, created: Instant, test: Option<u64>) -> Result<()> {
        anyhow::ensure!(self.state.ready, "Device is not ready");
        anyhow::ensure!(
            created >= self.epoch && created.elapsed() <= COMMAND_TTL,
            "Input expired"
        );
        let c = QueuedCommand {
            action: MediaCommand::PlayPause,
            target: self.state.selected.clone().context("No selected device")?,
            generation: self.state.generation,
            created,
        };
        let receipt = self
            .transport
            .as_ref()
            .context("Bluetooth not started")?
            .send(c)?;
        self.receipts.push(Receipt {
            created,
            test,
            result: receipt,
        });
        Ok(())
    }
    pub fn tick(&mut self) {
        let stopped = self.transport.as_ref().is_some_and(|t| t.is_finished());
        if let Some(s) = self.transport.as_mut().and_then(|t| t.snapshot())
            && (stopped
                || (s.generation >= self.required_generation
                    && (s.selected == self.state.selected || s.selected.is_none())))
        {
            self.state = s;
            if self.state.selected.is_none() {
                self.state.ready = false;
                self.state.target_status = None;
            }
        }
        if stopped {
            taprelay_core::devices::revoke_session(&mut self.state);
            self.state.service = false;
            self.state.broadcasting = false;
            self.state
                .last_error
                .get_or_insert_with(|| "Bluetooth worker stopped; retry to restart".into());
        }
        self.state.input = self.input.as_ref().is_some_and(|i| !i.is_finished());
        if (self.listening || self.recording()) && !self.state.input {
            self.error = Some(
                self.input
                    .as_ref()
                    .and_then(|i| i.failure())
                    .unwrap_or_else(|| "Input listener stopped; start listening to retry".into()),
            );
            self.listening = false;
            self.capture_cancelled = true;
            self.finish_recording();
        }
        if self.recording() && self.capture_waiting() {
            // Native snapshot covers keys held before the hook was created and the activating click.
            if !platform::desktop::any_input_held() {
                while self.events.try_recv().is_ok() {}
                self.gate = InputState::default();
                self.capture = CapturePhase::Recording;
                self.epoch = Instant::now();
            }
        }
        let mut pending = Vec::new();
        for _ in 0..256 {
            let Ok(event) = self.events.try_recv() else {
                break;
            };
            pending.push((event, false));
        }
        pending.extend(self.window_keys.drain(..).map(|event| (event, true)));
        pending.sort_by_key(|(event, _)| event.captured);
        for (event, from_window) in pending {
            if from_window && !self.recording() {
                continue;
            }
            if event.captured < self.epoch {
                continue;
            }
            if self.recording() && matches!(event.code, InputCode::Key(_)) {
                if self.capture_waiting() {
                    continue;
                }
                // Choose one source for the entire gesture: native hooks normally
                // arrive first, while the focused field works if the hook is silent.
                let source = self.keyboard_from_window.get_or_insert(from_window);
                if *source != from_window {
                    continue;
                }
            }
            let release_match = !event.down && matches!(event.code, InputCode::Mouse(_));
            let matched = if release_match {
                self.best_match(event)
            } else {
                None
            };
            if event.captured < self.epoch || !self.gate.update(event) {
                continue;
            }
            if !self.recording() && event.captured <= self.ui_input_until {
                continue;
            }
            if self.recording() {
                if self.capture_waiting() {
                    continue;
                }
                if event.code == InputCode::Key(0x1b) && event.down {
                    self.capture_cancelled = true;
                    continue;
                }
                self.capture_preview = self.gate.description();
                if self.learned.is_none() {
                    self.learned = self.recorder.observe(&self.gate, event);
                }
                continue;
            }
            let matched = if event.down && matches!(event.code, InputCode::Key(_)) {
                self.best_match(event)
            } else {
                matched
            };
            if self.listening
                && let Some(matched) = matched
            {
                self.matched += 1;
                tracing::info!("Input recognized: {}", self.config.bindings[matched].tap);
                // Disconnected input is observed but never queued for later delivery.
                if self.state.ready
                    && let Err(e) = self.send_at(event.captured)
                {
                    self.error = Some(e.to_string());
                }
            }
        }
        let mut i = 0;
        while i < self.receipts.len() {
            let r = match self.receipts[i].result.try_recv() {
                Ok(r) => Some(r),
                Err(oneshot::error::TryRecvError::Closed) => {
                    Some(Err("Transport stopped before delivery".into()))
                }
                Err(_) => None,
            };
            if let Some(r) = r {
                let receipt = self.receipts.swap_remove(i);
                if let Some(id) = receipt.test
                    && self.test == TestStatus::Pending(id)
                {
                    self.test = match &r {
                        Ok(()) => TestStatus::Succeeded,
                        Err(e) => TestStatus::Failed(e.clone()),
                    };
                }
                if receipt.created < self.epoch {
                    continue;
                }
                match r {
                    Ok(()) => {
                        self.delivered += 1;
                        tracing::info!(
                            "Windows accepted Play/Pause notifications; receiver playback is not acknowledged"
                        );
                    }
                    Err(e) => self.error = Some(e),
                }
            } else {
                i += 1;
            }
        }
    }
    fn best_match(&self, event: InputEvent) -> Option<usize> {
        self.bindings.best_match(event.code, &self.gate)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use taprelay_core::{binding::Binding, input::MouseButton};
    struct FakeTransport;
    struct FakeInput;
    type Reply = oneshot::Sender<Result<(), String>>;
    struct DeferredTransport {
        replies: std::rc::Rc<std::cell::RefCell<Vec<Reply>>>,
        stopped: bool,
    }
    impl Transport for DeferredTransport {
        fn disconnect(&self) -> Result<u64> {
            Ok(2)
        }
        fn snapshot(&mut self) -> Option<Snapshot> {
            None
        }
        fn is_finished(&self) -> bool {
            self.stopped
        }
        fn refresh(&self) -> Result<()> {
            Ok(())
        }
        fn restart(&self) -> Result<u64> {
            anyhow::ensure!(!self.stopped, "dead worker was reused");
            Ok(1)
        }
        fn select(&self, _: String) -> Result<u64> {
            Ok(1)
        }
        fn invalidate(&self) {}
        fn send(&self, _: QueuedCommand) -> Result<oneshot::Receiver<Result<(), String>>> {
            let (tx, rx) = oneshot::channel();
            self.replies.borrow_mut().push(tx);
            Ok(rx)
        }
    }
    #[test]
    fn receiver_selection_is_session_only_and_legacy_config_is_not_restored() {
        let mut config = Config::default();
        config.device = Some(crate::config::Device {
            id: "old-endpoint".into(),
            name: "Old tablet".into(),
            identity: vec![],
            aliases: vec![],
            legacy_verified: false,
        });
        let mut runtime = Runtime::new(config);
        assert!(runtime.config.device.is_none());
        assert!(runtime.state.selected.is_none());
        runtime.transport = Some(Box::new(DeferredTransport {
            replies: Default::default(),
            stopped: false,
        }));
        runtime.choose("candidate".into()).unwrap();
        assert_eq!(runtime.state.selected.as_deref(), Some("candidate"));
        runtime.disconnect().unwrap();
        assert!(runtime.state.selected.is_none());
        assert!(!runtime.state.ready);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        runtime.config.save(&path).unwrap();
        assert!(!std::fs::read_to_string(path).unwrap().contains("device"));
    }
    #[test]
    fn manual_failure_is_terminal_and_automatic_receipts_do_not_override_it() {
        let replies = std::rc::Rc::new(std::cell::RefCell::new(vec![]));
        let mut r = Runtime::new(Config::default());
        r.transport = Some(Box::new(DeferredTransport {
            replies: replies.clone(),
            stopped: false,
        }));
        r.state.ready = true;
        r.state.selected = Some("receiver".into());
        r.send().unwrap();
        r.send_at(Instant::now()).unwrap();
        replies.borrow_mut().remove(1).send(Ok(())).unwrap();
        r.tick();
        assert!(matches!(r.test, TestStatus::Pending(_)));
        replies
            .borrow_mut()
            .remove(0)
            .send(Err("Receiver disconnected".into()))
            .unwrap();
        r.tick();
        assert_eq!(r.test, TestStatus::Failed("Receiver disconnected".into()));
        r.send_at(Instant::now()).unwrap();
        replies.borrow_mut().remove(0).send(Ok(())).unwrap();
        r.tick();
        assert_eq!(r.test, TestStatus::Failed("Receiver disconnected".into()));
        r.send().unwrap();
        replies.borrow_mut().clear();
        r.tick();
        assert_eq!(
            r.test,
            TestStatus::Failed("Transport stopped before delivery".into())
        );
    }
    #[test]
    fn retry_recreates_dead_worker_and_resets_generation_floor() {
        let mut r = Runtime::new(Config::default());
        r.transport = Some(Box::new(DeferredTransport {
            replies: Default::default(),
            stopped: true,
        }));
        r.required_generation = 99;
        let mut recreated = false;
        r.start_bluetooth_with(|| {
            recreated = true;
            Ok(Box::new(FakeTransport))
        })
        .unwrap();
        assert!(recreated);
        assert_eq!(r.required_generation, 0);
        assert!(!r.transport.as_ref().unwrap().is_finished());
    }
    #[test]
    fn changing_device_cancels_test_even_if_old_receipt_later_succeeds() {
        let replies = std::rc::Rc::new(std::cell::RefCell::new(vec![]));
        let mut r = Runtime::new(Config::default());
        r.transport = Some(Box::new(DeferredTransport {
            replies: replies.clone(),
            stopped: false,
        }));
        r.state.ready = true;
        r.state.selected = Some("old".into());
        r.send().unwrap();
        r.choose("new".into()).unwrap();
        replies.borrow_mut().remove(0).send(Ok(())).unwrap();
        r.tick();
        assert!(matches!(r.test, TestStatus::Failed(_)));
        assert_eq!(r.delivered, 0);
    }
    impl InputSource for FakeInput {
        fn is_finished(&self) -> bool {
            false
        }
    }
    #[test]
    fn keyboard_recording_previews_and_confirms_on_release() {
        for keys in [vec![0x77], vec![0xa2, 0x4b]] {
            let mut r = Runtime::new(Config::default());
            r.input = Some(Box::new(FakeInput));
            r.record().unwrap();
            r.capture = CapturePhase::Recording;
            for &key in &keys {
                r.sender
                    .try_send(InputEvent {
                        code: InputCode::Key(key),
                        down: true,
                        captured: Instant::now(),
                    })
                    .unwrap();
                r.tick();
                assert!(!r.capture_preview.is_empty());
                assert!(r.learned.is_none());
            }
            for &key in keys.iter().rev() {
                r.sender
                    .try_send(InputEvent {
                        code: InputCode::Key(key),
                        down: false,
                        captured: Instant::now(),
                    })
                    .unwrap();
                r.tick();
            }
            let mut expected = keys;
            expected.sort_unstable();
            assert_eq!(r.learned, Some(Trigger::Keyboard { keys: expected }));
        }
    }
    #[test]
    fn focused_keyboard_records_when_hook_is_silent_and_deduplicates_when_present() {
        for native in [false, true] {
            let mut r = Runtime::new(Config::default());
            r.input = Some(Box::new(FakeInput));
            r.record().unwrap();
            r.capture = CapturePhase::Recording;
            for (key, down) in [(0xa2, true), (0x4b, true), (0x4b, false), (0xa2, false)] {
                if native {
                    r.sender
                        .try_send(InputEvent {
                            code: InputCode::Key(key),
                            down,
                            captured: Instant::now(),
                        })
                        .unwrap();
                }
                r.window_keys.push(InputEvent {
                    code: InputCode::Key(key),
                    down,
                    captured: Instant::now(),
                });
                r.tick();
                if down {
                    assert!(!r.capture_preview.is_empty());
                    assert!(r.learned.is_none());
                }
            }
            assert_eq!(
                r.learned,
                Some(Trigger::Keyboard {
                    keys: vec![0x4b, 0xa2]
                })
            );
            assert_eq!(r.keyboard_from_window, Some(!native));
            r.finish_recording();
            assert!(r.learned.is_none());
            assert!(r.window_keys.is_empty());
        }
    }
    impl Transport for FakeTransport {
        fn snapshot(&mut self) -> Option<Snapshot> {
            None
        }
        fn refresh(&self) -> Result<()> {
            Ok(())
        }
        fn restart(&self) -> Result<u64> {
            Ok(1)
        }
        fn select(&self, _: String) -> Result<u64> {
            Ok(1)
        }
        fn invalidate(&self) {}
        fn send(&self, _: QueuedCommand) -> Result<oneshot::Receiver<Result<(), String>>> {
            let (tx, rx) = oneshot::channel();
            tx.send(Ok(())).unwrap();
            Ok(rx)
        }
    }
    #[test]
    fn manual_test_needs_neither_bindings_nor_listener() {
        let mut r = Runtime::new(Config::default());
        r.transport = Some(Box::new(FakeTransport));
        r.state.ready = true;
        r.state.selected = Some("receiver".into());
        r.send().unwrap();
        r.tick();
        assert_eq!(r.delivered, 1);
        assert!(!r.listening);
        assert!(r.config.bindings.is_empty());
    }
    #[test]
    fn gui_clicks_do_not_send_a_second_media_command() {
        let mut r = Runtime::new(Config::default());
        r.input = Some(Box::new(FakeInput));
        r.listening = true;
        r.config.bindings.push(Binding {
            tap: Trigger::Mouse {
                button: MouseButton::Left,
            },
            relay: MediaCommand::PlayPause,
            enabled: true,
        });
        for down in [true, false] {
            r.sender
                .try_send(InputEvent {
                    code: InputCode::Mouse(MouseButton::Left),
                    down,
                    captured: Instant::now(),
                })
                .unwrap();
        }
        r.bindings_changed();
        r.consume_ui_input();
        r.tick();
        assert_eq!(r.matched, 0);
        for down in [true, false] {
            r.sender
                .try_send(InputEvent {
                    code: InputCode::Mouse(MouseButton::Left),
                    down,
                    captured: Instant::now(),
                })
                .unwrap();
        }
        r.tick();
        assert_eq!(r.matched, 1);
    }
    #[test]
    fn backend_clearing_selection_stops_waiting_for_the_old_receiver() {
        struct DisconnectingTransport;
        impl Transport for DisconnectingTransport {
            fn snapshot(&mut self) -> Option<Snapshot> {
                Some(Snapshot {
                    generation: 1,
                    selected: None,
                    ready: false,
                    ..Default::default()
                })
            }
            fn refresh(&self) -> Result<()> {
                Ok(())
            }
            fn restart(&self) -> Result<u64> {
                Ok(1)
            }
            fn select(&self, _: String) -> Result<u64> {
                Ok(1)
            }
            fn invalidate(&self) {}
            fn send(&self, _: QueuedCommand) -> Result<oneshot::Receiver<Result<(), String>>> {
                unreachable!()
            }
        }

        let mut runtime = Runtime::new(Config::default());
        runtime.transport = Some(Box::new(DisconnectingTransport));
        runtime.choose("ipad".into()).unwrap();
        assert_eq!(runtime.state.selected.as_deref(), Some("ipad"));
        runtime.tick();
        assert!(runtime.state.selected.is_none());
        assert!(runtime.state.target_status.is_none());
        assert!(!runtime.state.ready);
    }
    #[test]
    fn stale_snapshot_cannot_restore_previous_receiver_after_selection() {
        struct StaleTransport;
        impl Transport for StaleTransport {
            fn disconnect(&self) -> Result<u64> {
                Ok(2)
            }
            fn snapshot(&mut self) -> Option<Snapshot> {
                Some(Snapshot {
                    ready: true,
                    selected: Some("old".into()),
                    generation: 99,
                    ..Default::default()
                })
            }
            fn refresh(&self) -> Result<()> {
                Ok(())
            }
            fn restart(&self) -> Result<u64> {
                Ok(1)
            }
            fn select(&self, _: String) -> Result<u64> {
                Ok(1)
            }
            fn invalidate(&self) {}
            fn send(&self, _: QueuedCommand) -> Result<oneshot::Receiver<Result<(), String>>> {
                unreachable!()
            }
        }
        let mut r = Runtime::new(Config::default());
        r.transport = Some(Box::new(StaleTransport));
        r.choose("new".into()).unwrap();
        r.tick();
        assert_eq!(r.state.selected.as_deref(), Some("new"));
        assert!(!r.state.ready);
        r.disconnect().unwrap();
        r.tick();
        assert!(r.state.selected.is_none());
        assert!(!r.state.ready);
    }
    #[test]
    fn changing_receiver_and_stopping_input_preserve_transport() {
        let mut r = Runtime::new(Config::default());
        r.transport = Some(Box::new(FakeTransport));
        r.listening = true;
        r.choose("next".into()).unwrap();
        assert!(r.listening);
        assert!(!r.state.ready);
        r.set_listening(false).unwrap();
        assert!(r.transport.is_some());
        assert!(!r.listening);
    }
    #[test]
    fn overlapping_mouse_bindings_fire_once_and_disabled_bindings_do_not_fire() {
        let mut r = Runtime::new(Config::default());
        r.listening = true;
        struct Fake;
        impl InputSource for Fake {
            fn is_finished(&self) -> bool {
                false
            }
        }
        r.input = Some(Box::new(Fake));
        r.config.bindings = vec![
            Binding {
                tap: Trigger::Mouse {
                    button: MouseButton::Side1,
                },
                relay: MediaCommand::PlayPause,
                enabled: true,
            },
            Binding {
                tap: Trigger::Mixed {
                    modifiers: vec![0xa2],
                    button: MouseButton::Side1,
                },
                relay: MediaCommand::PlayPause,
                enabled: true,
            },
        ];
        r.bindings_changed();
        for (code, down) in [
            (InputCode::Key(0xa2), true),
            (InputCode::Mouse(MouseButton::Side1), true),
            (InputCode::Mouse(MouseButton::Side1), true),
            (InputCode::Mouse(MouseButton::Side1), false),
        ] {
            r.sender
                .try_send(InputEvent {
                    code,
                    down,
                    captured: Instant::now(),
                })
                .unwrap();
        }
        r.tick();
        assert_eq!(r.matched, 1);
        assert_eq!(r.delivered, 0);
        for b in &mut r.config.bindings {
            b.enabled = false;
        }
        r.bindings_changed();
        for down in [true, false] {
            r.sender
                .try_send(InputEvent {
                    code: InputCode::Mouse(MouseButton::Side1),
                    down,
                    captured: Instant::now(),
                })
                .unwrap();
        }
        r.tick();
        assert_eq!(r.matched, 1);
    }
}
