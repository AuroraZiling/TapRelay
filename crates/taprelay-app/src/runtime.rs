//! Runtime coordination around the platform input router and transport
//! adapters. The Windows input thread owns the single router instance; this
//! module only applies its parsed outputs to transport and UI state.

use crate::{
    action::BindingCommand,
    config::Config,
    feedback::TestStatus,
    platform::{self, InputSource, Transport},
};
use anyhow::{Context, Result};
use std::time::{Duration, Instant};
use taprelay_core::{
    command::{COMMAND_TTL, CommandPhase, MediaCommand, QueuedCommand},
    function::{FunctionAction, FunctionId, Shortcut, complete_configs},
    input::{InputCode, InputEvent, InputState, Recorder},
    input_router::{RouteResult, RoutedInput, RoutedOutput, RouterReason},
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
    action: MediaCommand,
    result: oneshot::Receiver<Result<(), String>>,
}

pub struct Runtime {
    pub config: Config,
    pub state: Snapshot,
    pub learned: Option<Shortcut>,
    pub matched: u64,
    pub delivered: u64,
    pub error: Option<String>,
    pub listening: bool,
    capture: CapturePhase,
    pub capture_preview: String,
    pub capture_cancelled: bool,
    pub capture_invalid: bool,
    input: Option<Box<dyn InputSource>>,
    transport: Option<Box<dyn Transport>>,
    events: mpsc::Receiver<RoutedInput>,
    sender: mpsc::Sender<RoutedInput>,
    capture_state: InputState,
    recorder: Recorder,
    pub window_keys: Vec<InputEvent>,
    epoch: Instant,
    required_generation: u64,
    ui_input_until: Instant,
    receipts: Vec<Receipt>,
    pub test: TestStatus,
    next_test: u64,
    pub bindings_revision: u64,
    last_transport_ready: bool,
    invalidating: bool,
    startup_restore: Option<taprelay_core::state::Target>,
}

impl Runtime {
    pub fn wake_delay(&self) -> Duration {
        Duration::from_millis(50)
    }
    fn discard_events(&mut self) {
        while let Ok(input) = self.events.try_recv() {
            if let RoutedInput::Control(result) = input {
                self.apply_router_outputs(result);
            }
        }
    }
    pub fn new(mut config: Config) -> Self {
        complete_configs(&mut config.functions);
        // Device selection is session-only. Older config files may still
        // deserialize a device record, but it must never be restored or used.
        config.device = None;
        let startup_restore = config
            .remembered_device
            .as_ref()
            .map(|device| device.as_target());
        let (sender, events) = mpsc::channel(1024);
        Self {
            config,
            state: Snapshot::default(),
            learned: None,
            matched: 0,
            delivered: 0,
            error: None,
            listening: false,
            capture: CapturePhase::Idle,
            capture_preview: String::new(),
            capture_cancelled: false,
            capture_invalid: false,
            input: None,
            transport: None,
            events,
            sender,
            capture_state: InputState::default(),
            recorder: Recorder::default(),
            window_keys: vec![],
            epoch: Instant::now(),
            required_generation: 0,
            ui_input_until: Instant::now(),
            receipts: vec![],
            test: TestStatus::Idle,
            next_test: 0,
            bindings_revision: 0,
            last_transport_ready: false,
            invalidating: false,
            startup_restore,
        }
    }

    pub fn start_bluetooth(&mut self) -> Result<()> {
        self.start_bluetooth_with(platform::transport)
    }

    fn start_bluetooth_with(
        &mut self,
        create: impl FnOnce(Option<taprelay_core::state::Target>) -> Result<Box<dyn Transport>>,
    ) -> Result<()> {
        self.state.ready = false;
        self.state.service = false;
        self.state.broadcasting = false;
        self.state.selected = None;
        self.state.target_status = None;
        self.state.last_error = None;
        self.error = None;
        self.invalidate(RouterReason::SessionChanged);
        if self
            .transport
            .as_ref()
            .is_some_and(|transport| transport.is_finished())
        {
            self.transport.take();
        }
        if let Some(transport) = &self.transport {
            self.required_generation = transport.restart()?;
            return Ok(());
        }
        self.required_generation = 0;
        let remembered = self.startup_restore.take();
        self.transport = Some(create(remembered)?);
        Ok(())
    }

    pub fn pair(&mut self, id: String) -> Result<()> {
        self.startup_restore = None;
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
        self.startup_restore = None;
        self.config.remembered_device = None;
        let result = self
            .transport
            .as_ref()
            .context("Bluetooth not started")?
            .disconnect();
        self.state.ready = false;
        self.state.selected = None;
        self.state.target_status = None;
        self.invalidate(RouterReason::SessionChanged);
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
        self.startup_restore = None;
        if self.state.selected.as_ref() == Some(&id)
            && self.state.target_status.as_ref().is_some_and(|target| {
                matches!(
                    target.connection,
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
        self.invalidate(RouterReason::SessionChanged);
        Ok(())
    }

    fn ensure_input(&mut self) -> Result<()> {
        if self.input.as_ref().is_none_or(|input| input.is_finished()) {
            self.input.take();
            self.discard_events();
            self.capture_state = InputState::default();
            self.epoch = Instant::now();
            self.input = Some(platform::input(self.sender.clone())?);
        }
        Ok(())
    }

    fn configure_input(&mut self) {
        let result = self
            .input
            .as_ref()
            .map_or_else(RouteResult::default, |input| {
                input.configure(
                    &self.config.functions,
                    self.listening,
                    self.recording(),
                    self.bindings_revision,
                )
            });
        self.apply_router_outputs(result);
    }

    pub fn set_listening(&mut self, on: bool) -> Result<()> {
        if on {
            self.ensure_input()?;
        }
        self.listening = on;
        self.configure_input();
        if !on && !self.recording() {
            self.input.take();
        }
        if !on {
            self.invalidate(RouterReason::ListenerStopped);
        }
        Ok(())
    }

    fn invalidate(&mut self, reason: RouterReason) {
        if self.invalidating {
            return;
        }
        self.invalidating = true;
        let mut cleanup = self
            .input
            .as_ref()
            .map_or_else(RouteResult::default, |input| input.terminate(reason));
        cleanup.revision = self.bindings_revision;
        self.apply_router_outputs(cleanup);
        self.epoch = Instant::now();
        if matches!(self.test, TestStatus::Pending(_)) {
            self.test = TestStatus::Failed(
                "Test cancelled because the input or connection session changed".into(),
            );
        }
        if (reason == RouterReason::TransportLost
            || (reason == RouterReason::ListenerStopped && !self.state.input))
            && let Some(transport) = &self.transport
        {
            transport.invalidate();
        }
        self.invalidating = false;
    }

    /// Finish the input session before the transport is dropped. This is a
    /// best-effort release: the BLE worker still owns the native notification
    /// boundary, so a physically disconnected receiver cannot be promised a
    /// final packet.
    pub fn shutdown(&mut self) {
        self.invalidate(RouterReason::ApplicationExit);
        self.listening = false;
        self.capture = CapturePhase::Idle;
        self.input.take();
        self.transport.take();
        self.state.ready = false;
    }

    fn record(&mut self) -> Result<()> {
        self.ensure_input()?;
        self.capture = CapturePhase::WaitingForRelease;
        self.configure_input();
        self.learned = None;
        self.capture_cancelled = false;
        self.capture_invalid = false;
        self.capture_preview.clear();
        self.recorder.reset();
        self.capture_state = InputState::default();
        self.window_keys.clear();
        Ok(())
    }

    /// Apply one binding edit as a single worker command. This keeps input
    /// suppression, capture transitions and configuration mutation ordered on
    /// the runtime thread instead of exposing that sequence to the UI.
    pub fn apply_binding_command(&mut self, command: BindingCommand) -> Result<()> {
        self.consume_ui_input();
        match command {
            BindingCommand::BeginCapture(target) => {
                let config = self
                    .config
                    .functions
                    .get(&target.id)
                    .context("Function no longer exists")?;
                anyhow::ensure!(config.enabled, "Function is disabled");
                anyhow::ensure!(
                    target.slot <= config.shortcuts.len() && target.slot < 2,
                    "Shortcut slot unavailable"
                );
                self.finish_recording();
                self.record()
            }
            BindingCommand::CancelCapture => {
                self.finish_recording();
                Ok(())
            }
            BindingCommand::DeleteShortcut(target) => {
                self.finish_recording();
                self.remove_shortcut(target.id, target.slot)
            }
            BindingCommand::SetFunctionEnabled { id, enabled } => {
                self.finish_recording();
                self.set_function_enabled(id, enabled)
            }
        }
    }

    pub fn finish_recording(&mut self) {
        if !self.recording() {
            return;
        }
        self.capture = CapturePhase::Idle;
        self.configure_input();
        self.window_keys.clear();
        self.learned = None;
        self.capture_preview.clear();
        self.capture_invalid = false;
        self.epoch = Instant::now();
        self.capture_state = InputState::default();
        self.recorder.reset();
        self.discard_events();
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
        match self.enqueue_media(
            MediaCommand::PlayPause,
            CommandPhase::Press,
            Instant::now(),
            Some(id),
        ) {
            Ok(()) => {
                self.test = TestStatus::Pending(id);
                Ok(())
            }
            Err(error) => {
                self.test = TestStatus::Failed(error.to_string());
                Err(error)
            }
        }
    }

    pub fn recording(&self) -> bool {
        self.capture != CapturePhase::Idle
    }

    pub fn capture_waiting(&self) -> bool {
        self.capture == CapturePhase::WaitingForRelease
    }

    pub fn bindings_changed(&mut self) {
        self.bindings_revision += 1;
        self.configure_input();
    }

    pub fn set_function_enabled(&mut self, id: FunctionId, enabled: bool) -> Result<()> {
        let function = self
            .config
            .functions
            .get_mut(&id)
            .context("Unknown function")?;
        if function.enabled == enabled {
            return Ok(());
        }
        function.enabled = enabled;
        self.bindings_changed();
        Ok(())
    }

    pub fn replace_shortcut(
        &mut self,
        id: FunctionId,
        slot: usize,
        shortcut: Shortcut,
    ) -> Result<()> {
        anyhow::ensure!(slot < 2, "Shortcut slot out of range");
        anyhow::ensure!(shortcut.valid(), "Invalid shortcut");
        let mut candidate = self.config.functions.clone();
        let function = candidate.get_mut(&id).context("Unknown function")?;
        anyhow::ensure!(function.enabled, "Function is disabled");
        if slot == function.shortcuts.len() {
            anyhow::ensure!(
                function.shortcuts.len() < 2,
                "A function may have at most two shortcuts"
            );
            function.shortcuts.push(shortcut);
        } else {
            *function
                .shortcuts
                .get_mut(slot)
                .context("Shortcut slot out of range")? = shortcut;
        }
        anyhow::ensure!(
            taprelay_core::binding::valid(&candidate),
            "Shortcut conflicts with another function"
        );
        self.config.functions = candidate;
        self.bindings_changed();
        Ok(())
    }

    pub fn remove_shortcut(&mut self, id: FunctionId, slot: usize) -> Result<()> {
        let mut candidate = self.config.functions.clone();
        let function = candidate.get_mut(&id).context("Unknown function")?;
        anyhow::ensure!(
            slot < function.shortcuts.len(),
            "Shortcut slot out of range"
        );
        function.shortcuts.remove(slot);
        self.config.functions = candidate;
        self.bindings_changed();
        Ok(())
    }

    /// Native edges belonging to a GUI control must not also trigger a global shortcut.
    pub fn consume_ui_input(&mut self) {
        self.ui_input_until = Instant::now();
    }

    fn enqueue_media(
        &mut self,
        action: MediaCommand,
        phase: CommandPhase,
        created: Instant,
        test: Option<u64>,
    ) -> Result<()> {
        anyhow::ensure!(self.state.ready, "Device is not ready");
        anyhow::ensure!(
            created >= self.epoch
                && (phase == CommandPhase::Release || created.elapsed() <= COMMAND_TTL),
            "Input expired"
        );
        let command = match phase {
            CommandPhase::Press => QueuedCommand::press(
                action,
                self.state.selected.clone().context("No selected device")?,
                self.state.generation,
                created,
            ),
            CommandPhase::Release => QueuedCommand::release(
                action,
                self.state.selected.clone().context("No selected device")?,
                self.state.generation,
                created,
            ),
        };
        let receipt = self
            .transport
            .as_ref()
            .context("Bluetooth not started")?
            .send(command)?;
        self.receipts.push(Receipt {
            created,
            test,
            action,
            result: receipt,
        });
        Ok(())
    }

    pub fn tick(&mut self) {
        let stopped = self
            .transport
            .as_ref()
            .is_some_and(|transport| transport.is_finished());
        if let Some(snapshot) = self
            .transport
            .as_mut()
            .and_then(|transport| transport.snapshot())
            && (stopped
                || (snapshot.generation >= self.required_generation
                    && (snapshot.selected == self.state.selected
                        || snapshot.selected.is_none()
                        || (self.state.selected.is_none()
                            && self.config.remembered_device.as_ref().is_some_and(
                                |remembered| {
                                    snapshot
                                        .target_status
                                        .as_ref()
                                        .is_some_and(|target| remembered.matches(target))
                                },
                            )))))
        {
            self.state = snapshot;
            if self.state.selected.is_none() {
                self.state.ready = false;
                self.state.target_status = None;
            }
            if self.state.ready
                && let Some(target) = self.state.target_status.as_ref()
            {
                self.config.remembered_device = Some(crate::config::Device::from_target(target));
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

        let transport_ready = self.state.ready;
        if self.last_transport_ready && !transport_ready {
            self.invalidate(RouterReason::TransportLost);
        }
        self.last_transport_ready = transport_ready;

        self.state.input = self
            .input
            .as_ref()
            .is_some_and(|input| !input.is_finished());
        self.configure_input();
        if (self.listening || self.recording()) && !self.state.input {
            self.error = Some(
                self.input
                    .as_ref()
                    .and_then(|input| input.failure())
                    .unwrap_or_else(|| "Input listener stopped; start listening to retry".into()),
            );
            self.listening = false;
            self.capture_cancelled = true;
            self.invalidate(RouterReason::ListenerStopped);
            self.finish_recording();
        }

        if self.recording() && self.capture_waiting() && !platform::desktop::any_input_held() {
            self.discard_events();
            self.capture_state = InputState::default();
            self.capture = CapturePhase::Recording;
            self.epoch = Instant::now();
        }

        let mut pending = Vec::new();
        for _ in 0..1024 {
            let Ok(input) = self.events.try_recv() else {
                break;
            };
            pending.push(input);
        }
        let window_keys = std::mem::take(&mut self.window_keys);
        if self.recording() {
            pending.extend(window_keys.into_iter().map(|event| RoutedInput::Edge {
                event,
                result: RouteResult::default(),
            }));
        }
        for input in pending {
            match input {
                RoutedInput::Control(result) => self.apply_router_outputs(result),
                RoutedInput::Edge { event, result } => {
                    if event.captured < self.epoch {
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
                        if !self.capture_state.update(event) {
                            continue;
                        }
                        self.capture_preview = self.capture_state.description();
                        if self.learned.is_none() {
                            self.learned = self.recorder.observe(&self.capture_state, event);
                            if self.recorder.take_invalid() {
                                self.capture_invalid = true;
                            }
                        }
                        continue;
                    }
                    if event.captured > self.ui_input_until {
                        self.apply_router_outputs(result);
                    }
                }
            }
        }

        let mut index = 0;
        while index < self.receipts.len() {
            let result = match self.receipts[index].result.try_recv() {
                Ok(result) => Some(result),
                Err(oneshot::error::TryRecvError::Closed) => {
                    Some(Err("Transport stopped before delivery".into()))
                }
                Err(oneshot::error::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                let receipt = self.receipts.swap_remove(index);
                if let Some(id) = receipt.test
                    && self.test == TestStatus::Pending(id)
                {
                    self.test = match &result {
                        Ok(()) => TestStatus::Succeeded,
                        Err(error) => TestStatus::Failed(error.clone()),
                    };
                }
                if receipt.created < self.epoch {
                    continue;
                }
                match result {
                    Ok(()) => {
                        self.delivered += 1;
                        tracing::info!(
                            action = ?receipt.action,
                            "Windows accepted HID notification; receiver execution is not acknowledged"
                        );
                    }
                    Err(error) => self.error = Some(error),
                }
            } else {
                index += 1;
            }
        }
    }

    fn apply_router_outputs(&mut self, result: taprelay_core::input_router::RouteResult) {
        // Hook decisions and configuration barriers share one ordered queue.
        // A release accepted just before a config edit still belongs to its
        // earlier press; discarding it by revision would leave the peer stuck.
        // Connection/input invalidation is handled separately by epoch/target.

        for output in result.outputs {
            match output {
                RoutedOutput::Feedback { binding, action } => {
                    self.matched += 1;
                    tracing::info!(?binding, ?action, "Input recognized");
                }
                RoutedOutput::Function {
                    action: FunctionAction::Media(action),
                    activation: _,
                    down,
                    created,
                    ..
                } => {
                    if let Err(error) = self.enqueue_media(
                        action,
                        if down {
                            CommandPhase::Press
                        } else {
                            CommandPhase::Release
                        },
                        created,
                        None,
                    ) {
                        // Recognition remains real even while the receiver is
                        // unavailable; no command is queued for later replay.
                        if self.state.ready {
                            self.error = Some(error.to_string());
                            if !self.invalidating {
                                self.invalidate(RouterReason::TransportLost);
                                return;
                            }
                        }
                    }
                }

                RoutedOutput::Replay(input) => {
                    if let Some(source) = &self.input
                        && let Err(error) = source.replay(input)
                    {
                        self.error = Some(error.to_string());
                    }
                }
                RoutedOutput::Local(_) => {}
            }
        }
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.invalidate(RouterReason::ApplicationExit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::config::Config;
    use taprelay_core::{
        function::{FunctionConfig, FunctionId, ModifierSet, Shortcut},
        input::MouseButton,
    };

    struct FakeInput;
    impl InputSource for FakeInput {
        fn is_finished(&self) -> bool {
            false
        }
    }

    struct FakeTransport;
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

    struct SnapshotTransport(Option<Snapshot>);
    impl Transport for SnapshotTransport {
        fn snapshot(&mut self) -> Option<Snapshot> {
            self.0.take()
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

    #[test]
    fn new_runtime_keeps_functions_disabled_and_does_not_restore_device() {
        let mut config = Config::default();
        config.device = Some(crate::config::Device {
            id: "old-endpoint".into(),
            name: "Old tablet".into(),
            identity: vec![],
            aliases: vec![],
            legacy_verified: false,
        });
        let runtime = Runtime::new(config);
        assert!(runtime.config.device.is_none());
        assert!(
            runtime
                .config
                .functions
                .values()
                .all(|function| !function.enabled)
        );
        assert!(runtime.state.selected.is_none());
    }

    #[test]
    fn startup_offers_remembered_device_only_once() {
        let mut config = Config::default();
        config.remembered_device = Some(crate::config::Device {
            id: "remembered".into(),
            name: "Tablet".into(),
            identity: vec!["container:tablet".into()],
            aliases: vec![],
            legacy_verified: false,
        });
        let mut runtime = Runtime::new(config);
        let mut offered = None;
        runtime
            .start_bluetooth_with(|remembered| {
                offered = remembered;
                Ok(Box::new(FakeTransport))
            })
            .unwrap();
        assert_eq!(offered.unwrap().id, "remembered");
        runtime.start_bluetooth().unwrap();
        assert!(runtime.startup_restore.is_none());
    }

    #[test]
    fn ready_session_updates_memory_and_explicit_disconnect_clears_it() {
        let target = taprelay_core::state::Target {
            id: "gatt-endpoint".into(),
            name: "Tablet".into(),
            identity: vec!["container:tablet".into()],
            aliases: vec!["classic-endpoint".into()],
            ..Default::default()
        };
        let mut runtime = Runtime::new(Config::default());
        runtime.state.selected = Some(target.id.clone());
        runtime.transport = Some(Box::new(SnapshotTransport(Some(Snapshot {
            generation: 1,
            selected: Some(target.id.clone()),
            target_status: Some(target.clone()),
            targets: vec![target],
            ready: true,
            ..Default::default()
        }))));
        runtime.tick();
        let remembered = runtime.config.remembered_device.as_ref().unwrap();
        assert_eq!(remembered.id, "gatt-endpoint");
        assert_eq!(remembered.identity, ["container:tablet"]);
        assert!(runtime.disconnect().is_err());
        assert!(runtime.config.remembered_device.is_none());
    }

    #[test]
    fn startup_snapshot_is_accepted_only_for_the_remembered_physical_device() {
        let remembered = crate::config::Device {
            id: "classic-endpoint".into(),
            name: "Tablet".into(),
            identity: vec!["container:tablet".into()],
            aliases: vec![],
            legacy_verified: false,
        };
        let restored = taprelay_core::state::Target {
            id: "gatt-endpoint".into(),
            name: "Renamed tablet".into(),
            identity: remembered.identity.clone(),
            ..Default::default()
        };
        let config = Config {
            remembered_device: Some(remembered),
            ..Default::default()
        };
        let mut runtime = Runtime::new(config);
        runtime.transport = Some(Box::new(SnapshotTransport(Some(Snapshot {
            generation: 1,
            selected: Some(restored.id.clone()),
            target_status: Some(restored.clone()),
            targets: vec![restored],
            ready: true,
            ..Default::default()
        }))));
        runtime.tick();
        assert!(runtime.state.ready);
        assert_eq!(
            runtime.config.remembered_device.as_ref().unwrap().id,
            "gatt-endpoint"
        );

        let unrelated = taprelay_core::state::Target {
            id: "other".into(),
            name: "Tablet".into(),
            identity: vec!["container:other".into()],
            ..Default::default()
        };
        runtime.state = Snapshot::default();
        runtime.transport = Some(Box::new(SnapshotTransport(Some(Snapshot {
            generation: 2,
            selected: Some(unrelated.id.clone()),
            target_status: Some(unrelated.clone()),
            targets: vec![unrelated],
            ready: true,
            ..Default::default()
        }))));
        runtime.tick();
        assert!(!runtime.state.ready);
        assert_eq!(
            runtime.config.remembered_device.as_ref().unwrap().id,
            "gatt-endpoint"
        );
    }

    #[test]
    fn explicit_test_needs_no_bindings_or_listener() {
        let mut runtime = Runtime::new(Config::default());
        runtime.transport = Some(Box::new(FakeTransport));
        runtime.state.ready = true;
        runtime.state.selected = Some("receiver".into());
        runtime.send().unwrap();
        runtime.tick();
        assert_eq!(runtime.delivered, 1);
        assert!(!runtime.listening);
    }

    #[test]
    fn toggling_function_preserves_saved_shortcuts() {
        let mut runtime = Runtime::new(Config::default());
        let id = FunctionId::MediaNext;
        let shortcuts = vec![Shortcut::mouse(ModifierSet::empty(), MouseButton::Side1)];
        runtime.config.functions.insert(
            id,
            FunctionConfig {
                enabled: true,
                shortcuts: shortcuts.clone(),
            },
        );
        runtime.set_function_enabled(id, false).unwrap();
        assert!(!runtime.config.functions[&id].enabled);
        assert_eq!(runtime.config.functions[&id].shortcuts, shortcuts);
        runtime.set_function_enabled(id, true).unwrap();
        assert!(runtime.config.functions[&id].enabled);
        assert_eq!(runtime.config.functions[&id].shortcuts, shortcuts);
    }

    #[test]
    fn binding_command_keeps_capture_and_config_changes_atomic() {
        let id = FunctionId::MediaPlayPause;
        let shortcut = Shortcut::mouse(ModifierSet::empty(), MouseButton::Side1);
        let mut config = Config::default();
        config.functions.insert(
            id,
            FunctionConfig {
                enabled: true,
                shortcuts: vec![shortcut.clone()],
            },
        );
        let mut runtime = Runtime::new(config);
        runtime.input = Some(Box::new(FakeInput));

        runtime
            .apply_binding_command(BindingCommand::BeginCapture(crate::action::CaptureTarget {
                id,
                slot: 1,
            }))
            .unwrap();
        assert!(runtime.recording());

        let error = runtime
            .apply_binding_command(BindingCommand::BeginCapture(crate::action::CaptureTarget {
                id,
                slot: 2,
            }))
            .unwrap_err();
        assert_eq!(error.to_string(), "Shortcut slot unavailable");
        assert!(
            runtime.recording(),
            "validation must happen before replacing the active capture"
        );

        runtime
            .apply_binding_command(BindingCommand::DeleteShortcut(
                crate::action::CaptureTarget { id, slot: 0 },
            ))
            .unwrap();
        assert!(!runtime.recording());
        assert!(runtime.config.functions[&id].shortcuts.is_empty());

        runtime
            .apply_binding_command(BindingCommand::SetFunctionEnabled { id, enabled: false })
            .unwrap();
        assert!(!runtime.config.functions[&id].enabled);
    }

    #[test]
    fn disabled_function_shortcut_can_be_saved_but_is_not_recognized() {
        let mut runtime = Runtime::new(Config::default());
        runtime.config.functions.insert(
            FunctionId::MediaNext,
            FunctionConfig {
                enabled: false,
                shortcuts: vec![Shortcut::mouse(ModifierSet::empty(), MouseButton::Side1)],
            },
        );
        runtime.bindings_changed();
        runtime.input = Some(Box::new(FakeInput));
        runtime.listening = true;
        runtime
            .sender
            .try_send(RoutedInput::Edge {
                event: InputEvent {
                    code: InputCode::Mouse(MouseButton::Side1),
                    down: true,
                    captured: Instant::now(),
                },
                result: RouteResult::default(),
            })
            .unwrap();
        runtime.tick();
        assert_eq!(runtime.matched, 0);
    }
}
