//! Runtime coordination around the platform input router and transport
//! adapters. The Windows input thread owns the single router instance; this
//! module only applies its parsed outputs to transport and UI state.

use crate::{
    action::{BindingCommand, CaptureTarget},
    config::{Config, Device},
    feedback::TestStatus,
    platform::{self, InputSource, Transport},
};
use anyhow::{Context, Result};
use std::time::Instant;
use taprelay_core::{
    command::{COMMAND_TTL, CommandPhase, MediaCommand, QueuedCommand},
    function::{AppCommand, FunctionAction, FunctionConfigs, FunctionId, Shortcut},
    input::{InputCode, InputEvent, InputState, Recorder},
    input_router::{RouteResult, RoutedInput, RoutedOutput, RouterReason},
    ports::BackendError,
    state::Snapshot,
};
use tokio::sync::{mpsc, oneshot};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapturePhase {
    WaitingForRelease,
    Recording,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureError {
    Invalid,
    Duplicate(FunctionId),
    Commit(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureSession {
    pub target: CaptureTarget,
    pub phase: CapturePhase,
    pub preview: String,
    pub error: Option<CaptureError>,
}

struct Receipt {
    created: Instant,
    test: Option<u64>,
    action: MediaCommand,
    result: oneshot::Receiver<Result<(), BackendError>>,
}

pub struct Runtime {
    pub functions: FunctionConfigs,
    pub remembered_device: Option<Device>,
    pub state: Snapshot,
    pub matched: u64,
    pub delivered: u64,
    pub error: Option<String>,
    pub listening: bool,
    capture: Option<CaptureSession>,
    input: Option<Box<dyn InputSource>>,
    input_initialized: bool,
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
    startup_retry_at: Option<Instant>,
    passthrough_mouse_percent: u16,
    passthrough_reverse_scroll: bool,
}

impl Runtime {
    fn discard_events(&mut self) {
        while let Ok(input) = self.events.try_recv() {
            if let RoutedInput::Control(result) = input {
                self.apply_router_outputs(result);
            }
        }
    }
    pub fn new(config: Config) -> Self {
        let startup_restore = config
            .remembered_device
            .as_ref()
            .map(|device| device.as_target());
        let (sender, events) = mpsc::channel(1024);
        Self {
            functions: config.functions,
            remembered_device: config.remembered_device,
            state: Snapshot::default(),
            matched: 0,
            delivered: 0,
            error: None,
            listening: false,
            capture: None,
            input: None,
            input_initialized: false,
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
            startup_retry_at: None,
            passthrough_mouse_percent: config.options.passthrough_mouse_percent,
            passthrough_reverse_scroll: config.options.passthrough_reverse_scroll,
        }
    }

    pub fn start_bluetooth(&mut self) -> Result<()> {
        match self.start_bluetooth_with(platform::transport) {
            Err(error) if self.startup_restore.is_some() => {
                tracing::warn!(%error, retry_seconds = 5, "Bluetooth startup will retry");
                self.state.last_error = Some(error.to_string());
                Ok(())
            }
            result => result,
        }
    }

    fn start_bluetooth_with(
        &mut self,
        create: impl FnOnce(Option<taprelay_core::state::Target>) -> Result<Box<dyn Transport>>,
    ) -> Result<()> {
        self.startup_retry_at = Some(Instant::now() + std::time::Duration::from_secs(5));
        self.state.ready = false;
        self.state.service = false;
        self.state.broadcasting = false;
        self.state.selected = None;
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
        let remembered = self.startup_restore.clone();
        self.transport = Some(create(remembered)?);
        Ok(())
    }

    fn retry_startup_with(
        &mut self,
        now: Instant,
        create: impl FnOnce(Option<taprelay_core::state::Target>) -> Result<Box<dyn Transport>>,
    ) {
        if self.startup_restore.is_none()
            || self
                .transport
                .as_ref()
                .is_some_and(|transport| !transport.is_finished())
            || self.startup_retry_at.is_none_or(|due| now < due)
        {
            return;
        }
        tracing::info!("Retrying Bluetooth worker for startup restore");
        if let Err(error) = self.start_bluetooth_with(create) {
            tracing::warn!(%error, retry_seconds = 5, "Bluetooth startup will retry");
            self.state.last_error = Some(error.to_string());
        }
        self.startup_retry_at = Some(now + std::time::Duration::from_secs(5));
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
        self.remembered_device = None;
        let result = self
            .transport
            .as_ref()
            .context("Bluetooth not started")?
            .disconnect();
        self.state.ready = false;
        self.state.selected = None;
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
        let restoring = self.startup_restore.take().is_some();
        if !restoring
            && self.state.selected.as_ref() == Some(&id)
            && self.state.selected_target().is_some_and(|target| {
                matches!(
                    target.connection,
                    taprelay_core::devices::Connection::AwaitingHostSubscription
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

    fn input_required(&self) -> bool {
        self.listening
            || self.recording()
            || self.functions.iter().any(|(id, config)| {
                matches!(
                    id,
                    FunctionId::AppToggleListening | FunctionId::AppTogglePassthrough
                ) && config.enabled
                    && !config.shortcuts.is_empty()
            })
    }

    fn sync_input(&mut self) -> Result<()> {
        self.input_initialized = true;
        if self.input_required() {
            self.ensure_input()?;
        }
        self.configure_input();
        if !self.input_required() {
            self.input.take();
        }
        Ok(())
    }

    fn configure_input(&mut self) {
        if let Some(input) = &self.input {
            input.attach_passthrough(
                self.transport
                    .as_ref()
                    .and_then(|transport| transport.input_link())
                    .inspect(|link| {
                        link.set_mouse_percent(self.passthrough_mouse_percent);
                        link.set_reverse_scroll(self.passthrough_reverse_scroll);
                    }),
            );
            if let Some(error) = input.passthrough_error() {
                self.error = Some(error);
            }
        }
        let result = self
            .input
            .as_ref()
            .map_or_else(RouteResult::default, |input| {
                input.configure(
                    &self.functions,
                    self.listening,
                    self.recording(),
                    self.bindings_revision,
                )
            });
        self.apply_router_outputs(result);
    }

    pub fn set_passthrough_reverse_scroll(&mut self, reverse: bool) {
        self.passthrough_reverse_scroll = reverse;
        if let Some(link) = self
            .transport
            .as_ref()
            .and_then(|transport| transport.input_link())
        {
            link.set_reverse_scroll(reverse);
        }
    }

    pub fn set_listening(&mut self, on: bool) -> Result<()> {
        if on {
            self.ensure_input()?;
        }
        self.listening = on;
        self.configure_input();
        if !self.input_required() {
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
        if let Some(capture) = &mut self.capture {
            capture.phase = CapturePhase::WaitingForRelease;
            capture.preview.clear();
            capture.error = None;
            self.capture_state = InputState::default();
            self.recorder.reset();
        }
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
        self.capture = None;
        self.input.take();
        self.transport.take();
        self.state.ready = false;
    }

    fn record(&mut self, target: CaptureTarget) -> Result<()> {
        self.ensure_input()?;
        self.capture = Some(CaptureSession {
            target,
            phase: CapturePhase::WaitingForRelease,
            preview: String::new(),
            error: None,
        });
        self.configure_input();
        self.epoch = Instant::now();
        self.recorder.reset();
        self.capture_state = InputState::default();
        self.window_keys.clear();
        Ok(())
    }

    pub fn apply_binding_command(&mut self, command: BindingCommand) -> Result<()> {
        self.consume_ui_input();
        match command {
            BindingCommand::BeginCapture(target) => {
                let config = self
                    .functions
                    .get(&target.id)
                    .context("Function no longer exists")?;
                anyhow::ensure!(config.enabled, "Function is disabled");
                anyhow::ensure!(
                    target.slot <= config.shortcuts.len() && target.slot < 2,
                    "Shortcut slot unavailable"
                );
                self.finish_recording();
                self.record(target)
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

    fn finish_recording(&mut self) {
        if !self.recording() {
            return;
        }
        self.capture = None;
        self.configure_input();
        self.window_keys.clear();
        self.epoch = Instant::now();
        self.capture_state = InputState::default();
        self.recorder.reset();
        self.discard_events();
        if !self.input_required() {
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
        self.capture.is_some()
    }

    pub fn capture(&self) -> Option<&CaptureSession> {
        self.capture.as_ref()
    }

    fn capture_waiting(&self) -> bool {
        self.capture
            .as_ref()
            .is_some_and(|capture| capture.phase == CapturePhase::WaitingForRelease)
    }

    fn submit_capture(&mut self, shortcut: Shortcut) {
        let Some(capture) = &self.capture else { return };
        let CaptureTarget { id, slot } = capture.target;
        let error = if !shortcut.valid() {
            Some(CaptureError::Invalid)
        } else if let Some(conflict) = self.functions.iter().find_map(|(&other, config)| {
            config
                .shortcuts
                .iter()
                .enumerate()
                .any(|(other_slot, candidate)| {
                    (other, other_slot) != (id, slot) && candidate == &shortcut
                })
                .then_some(other)
        }) {
            Some(CaptureError::Duplicate(conflict))
        } else {
            self.replace_shortcut(id, slot, shortcut)
                .err()
                .map(|error| CaptureError::Commit(error.to_string()))
        };
        if let Some(error) = error {
            self.capture.as_mut().unwrap().error = Some(error);
        } else {
            tracing::info!(function_id = id.stable_id(), slot, "Binding shortcut saved");
            self.finish_recording();
        }
    }

    pub fn bindings_changed(&mut self) {
        self.bindings_revision += 1;
        if let Err(error) = self.sync_input() {
            self.error = Some(error.to_string());
        }
    }

    pub fn set_function_enabled(&mut self, id: FunctionId, enabled: bool) -> Result<()> {
        let function = self.functions.get_mut(&id).context("Unknown function")?;
        if function.enabled == enabled {
            return Ok(());
        }
        function.enabled = enabled;
        self.bindings_changed();
        Ok(())
    }

    fn replace_shortcut(&mut self, id: FunctionId, slot: usize, shortcut: Shortcut) -> Result<()> {
        anyhow::ensure!(slot < 2, "Shortcut slot out of range");
        anyhow::ensure!(shortcut.valid(), "Invalid shortcut");
        let mut candidate = self.functions.clone();
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
        self.functions = candidate;
        self.bindings_changed();
        Ok(())
    }

    pub fn remove_shortcut(&mut self, id: FunctionId, slot: usize) -> Result<()> {
        let function = self.functions.get_mut(&id).context("Unknown function")?;
        anyhow::ensure!(
            slot < function.shortcuts.len(),
            "Shortcut slot out of range"
        );
        function.shortcuts.remove(slot);
        self.bindings_changed();
        Ok(())
    }

    /// Native edges belonging to a GUI control must not also trigger a global shortcut.
    pub fn consume_ui_input(&mut self) {
        self.consume_ui_input_at(Instant::now());
    }

    pub fn consume_ui_input_at(&mut self, captured: Instant) {
        self.ui_input_until = self.ui_input_until.max(captured);
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
        self.tick_with_capture_context(
            !self.recording() || platform::desktop::foreground_is_ours(),
            self.recording() && platform::desktop::any_input_held(),
        );
    }

    fn tick_with_capture_context(&mut self, foreground: bool, input_held: bool) {
        if !self.input_initialized
            && let Err(error) = self.sync_input()
        {
            self.error = Some(error.to_string());
        }
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
                            && self.startup_restore.as_ref().is_some_and(|remembered| {
                                snapshot
                                    .selected_target()
                                    .is_some_and(|target| remembered.same_device(target))
                            })))))
        {
            self.state = snapshot;
            if self.state.selected.is_none() {
                self.state.ready = false;
            }
            if !stopped
                && self.state.ready
                && let Some(target) = self.state.selected_target()
            {
                self.remembered_device = Some(crate::config::Device::from_target(target));
                self.startup_restore = None;
                self.startup_retry_at = None;
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
        self.retry_startup_with(Instant::now(), platform::transport);

        let transport_ready = self.state.ready;
        if self.last_transport_ready && !transport_ready && !self.state.profile_switching {
            self.invalidate(RouterReason::TransportLost);
        }
        self.last_transport_ready = transport_ready;

        self.state.input = self
            .input
            .as_ref()
            .is_some_and(|input| !input.is_finished());
        self.configure_input();
        if self.input_required() && self.input.is_some() && !self.state.input {
            self.error = Some(
                self.input
                    .as_ref()
                    .and_then(|input| input.failure())
                    .unwrap_or_else(|| "Input listener stopped; start listening to retry".into()),
            );
            self.listening = false;
            self.invalidate(RouterReason::ListenerStopped);
            self.finish_recording();
            self.input.take();
        }

        if self.recording() && !foreground {
            self.finish_recording();
        }
        if self.capture_waiting() && !input_held {
            self.discard_events();
            self.capture_state = InputState::default();
            self.capture.as_mut().unwrap().phase = CapturePhase::Recording;
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
                            self.finish_recording();
                            continue;
                        }
                        if !self.capture_state.update(event) {
                            continue;
                        }
                        self.capture.as_mut().unwrap().preview =
                            self.capture_state.description_with(platform::key_name);
                        if let Some(shortcut) = self.recorder.observe(&self.capture_state, event) {
                            self.submit_capture(shortcut);
                        } else if self.recorder.take_invalid() {
                            self.capture.as_mut().unwrap().error = Some(CaptureError::Invalid);
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
                Err(oneshot::error::TryRecvError::Closed) => Some(Err(BackendError::Unavailable(
                    "Transport stopped before delivery".into(),
                ))),
                Err(oneshot::error::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                let receipt = self.receipts.swap_remove(index);
                if let Some(id) = receipt.test
                    && self.test == TestStatus::Pending(id)
                {
                    self.test = match &result {
                        Ok(()) => TestStatus::Succeeded,
                        Err(error) => TestStatus::Failed(error.to_string()),
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
                    Err(BackendError::Stale) => {}
                    Err(error) => self.error = Some(error.to_string()),
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
                    action: FunctionAction::App(AppCommand::ToggleListening),
                    down,
                    created,
                    ..
                } => {
                    if down
                        && !self.recording()
                        && created >= self.epoch
                        && let Err(error) = self.set_listening(!self.listening)
                    {
                        self.error = Some(error.to_string());
                    }
                }
                RoutedOutput::Function {
                    action: FunctionAction::Media(action),
                    down,
                    created,
                    ..
                } => {
                    if down && (!self.listening || created < self.epoch) {
                        continue;
                    }
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
                RoutedOutput::Remote(_) | RoutedOutput::EndPassthrough => {}
                RoutedOutput::Function {
                    action: FunctionAction::App(AppCommand::TogglePassthrough),
                    ..
                } => {}
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
        fn send(&self, _: QueuedCommand) -> Result<oneshot::Receiver<Result<(), BackendError>>> {
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
        fn send(&self, _: QueuedCommand) -> Result<oneshot::Receiver<Result<(), BackendError>>> {
            unreachable!()
        }
    }

    struct StoppedTransport(Option<Snapshot>);
    impl Transport for StoppedTransport {
        fn snapshot(&mut self) -> Option<Snapshot> {
            self.0.take()
        }
        fn is_finished(&self) -> bool {
            true
        }
        fn refresh(&self) -> Result<()> {
            Ok(())
        }
        fn restart(&self) -> Result<u64> {
            unreachable!()
        }
        fn select(&self, _: String) -> Result<u64> {
            unreachable!()
        }
        fn invalidate(&self) {}
        fn send(&self, _: QueuedCommand) -> Result<oneshot::Receiver<Result<(), BackendError>>> {
            unreachable!()
        }
    }

    fn restoring_runtime() -> Runtime {
        Runtime::new(Config {
            remembered_device: Some(crate::config::Device {
                id: "remembered".into(),
                name: "Tablet".into(),
                identity: vec!["container:tablet".into()],
                aliases: vec![],
            }),
            ..Config::default()
        })
    }

    #[test]
    fn startup_retries_creation_and_stopped_worker_at_five_second_intervals() {
        let mut runtime = restoring_runtime();
        assert!(
            runtime
                .start_bluetooth_with(|_| anyhow::bail!("not ready"))
                .is_err()
        );
        let due = runtime.startup_retry_at.unwrap();
        runtime.retry_startup_with(due - std::time::Duration::from_millis(1), |_| {
            panic!("too early")
        });
        runtime.retry_startup_with(due, |remembered| {
            assert_eq!(remembered.unwrap().id, "remembered");
            anyhow::bail!("still not ready")
        });
        runtime.retry_startup_with(due + std::time::Duration::from_secs(4), |_| {
            panic!("too early")
        });
        runtime.retry_startup_with(due + std::time::Duration::from_secs(5), |remembered| {
            assert!(remembered.is_some());
            Ok(Box::new(StoppedTransport(None)))
        });
        runtime.retry_startup_with(due + std::time::Duration::from_secs(10), |remembered| {
            assert!(remembered.is_some());
            Ok(Box::new(FakeTransport))
        });
        runtime.retry_startup_with(due + std::time::Duration::from_secs(100), |_| {
            panic!("worker is running")
        });
    }

    #[test]
    fn stopped_workers_stale_ready_snapshot_does_not_complete_restore() {
        let mut runtime = restoring_runtime();
        let target = runtime.startup_restore.clone().unwrap();
        runtime.transport = Some(Box::new(StoppedTransport(Some(Snapshot {
            selected: Some(target.id.clone()),
            targets: vec![target],
            ready: true,
            ..Default::default()
        }))));
        runtime.tick();
        assert!(!runtime.state.ready);
        assert!(runtime.startup_restore.is_some());
    }

    #[test]
    fn manual_actions_cancel_restore_and_reject_late_automatic_selection() {
        for action in ["select", "disconnect", "pair"] {
            let mut runtime = restoring_runtime();
            let target = runtime.startup_restore.clone().unwrap();
            runtime.transport = Some(Box::new(SnapshotTransport(Some(Snapshot {
                selected: Some(target.id.clone()),
                targets: vec![target],
                ready: true,
                generation: 2,
                ..Default::default()
            }))));
            let _ = match action {
                "select" => runtime.choose("other".into()),
                "disconnect" => runtime.disconnect(),
                _ => runtime.pair("other".into()),
            };
            runtime.tick();
            assert!(runtime.startup_restore.is_none());
            assert!(!runtime.state.ready);
            assert_ne!(runtime.state.selected.as_deref(), Some("remembered"));
            runtime
                .retry_startup_with(Instant::now() + std::time::Duration::from_secs(600), |_| {
                    panic!("cancelled")
                });
        }
    }

    #[test]
    fn new_runtime_keeps_functions_disabled() {
        let runtime = Runtime::new(Config::default());
        assert!(runtime.functions.values().all(|function| !function.enabled));
        assert!(runtime.state.selected.is_none());
    }

    #[test]
    fn startup_restore_survives_transport_creation_failure() {
        let mut runtime = Runtime::new(Config {
            remembered_device: Some(crate::config::Device {
                id: "remembered".into(),
                name: "Tablet".into(),
                identity: vec!["container:tablet".into()],
                aliases: vec![],
            }),
            ..Config::default()
        });
        assert!(
            runtime
                .start_bluetooth_with(|_| anyhow::bail!("not ready"))
                .is_err()
        );
        runtime
            .start_bluetooth_with(|remembered| {
                assert_eq!(remembered.unwrap().id, "remembered");
                Ok(Box::new(FakeTransport))
            })
            .unwrap();
    }

    #[test]
    fn startup_keeps_remembered_device_until_ready() {
        let mut config = Config::default();
        config.remembered_device = Some(crate::config::Device {
            id: "remembered".into(),
            name: "Tablet".into(),
            identity: vec!["container:tablet".into()],
            aliases: vec![],
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
        assert!(runtime.startup_restore.is_some());
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
            targets: vec![target],
            ready: true,
            ..Default::default()
        }))));
        runtime.tick();
        let remembered = runtime.remembered_device.as_ref().unwrap();
        assert_eq!(remembered.id, "gatt-endpoint");
        assert_eq!(remembered.identity, ["container:tablet"]);
        assert!(runtime.disconnect().is_err());
        assert!(runtime.remembered_device.is_none());
    }

    #[test]
    fn startup_snapshot_is_accepted_only_for_the_remembered_physical_device() {
        let remembered = crate::config::Device {
            id: "classic-endpoint".into(),
            name: "Tablet".into(),
            identity: vec!["container:tablet".into()],
            aliases: vec![],
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
            targets: vec![restored],
            ready: true,
            ..Default::default()
        }))));
        runtime.tick();
        assert!(runtime.state.ready);
        assert_eq!(
            runtime.remembered_device.as_ref().unwrap().id,
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
            targets: vec![unrelated],
            ready: true,
            ..Default::default()
        }))));
        runtime.tick();
        assert!(!runtime.state.ready);
        assert_eq!(
            runtime.remembered_device.as_ref().unwrap().id,
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
        runtime.functions.insert(
            id,
            FunctionConfig {
                enabled: true,
                shortcuts: shortcuts.clone(),
            },
        );
        runtime.set_function_enabled(id, false).unwrap();
        assert!(!runtime.functions[&id].enabled);
        assert_eq!(runtime.functions[&id].shortcuts, shortcuts);
        runtime.set_function_enabled(id, true).unwrap();
        assert!(runtime.functions[&id].enabled);
        assert_eq!(runtime.functions[&id].shortcuts, shortcuts);
    }

    #[test]
    fn disabled_function_shortcut_can_be_saved_but_is_not_recognized() {
        let mut runtime = Runtime::new(Config::default());
        runtime.functions.insert(
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
    fn app_runtime() -> Runtime {
        let mut config = Config::default();
        config.functions.insert(
            FunctionId::AppToggleListening,
            FunctionConfig {
                enabled: true,
                shortcuts: vec![Shortcut::keyboard(ModifierSet::empty(), 0x78)],
            },
        );
        let mut runtime = Runtime::new(config);
        runtime.input = Some(Box::new(FakeInput));
        runtime
    }

    fn function_output(action: FunctionAction, down: bool, created: Instant) -> RouteResult {
        RouteResult {
            outputs: vec![RoutedOutput::Function {
                binding: taprelay_core::binding::BindingKey {
                    function: FunctionId::AppToggleListening,
                    slot: 0,
                },
                action,
                down,
                created,
                token: 1,
                revision: 0,
            }],
            ..Default::default()
        }
    }

    #[test]
    fn app_shortcut_keeps_input_alive_at_startup_and_across_listener_changes() {
        let mut runtime = app_runtime();
        runtime.tick();
        assert!(runtime.input_initialized);
        assert!(runtime.state.input);
        assert!(!runtime.listening);
        let action = FunctionAction::App(AppCommand::ToggleListening);
        for expected in [true, false, true] {
            runtime.apply_router_outputs(function_output(action, true, Instant::now()));
            assert_eq!(runtime.listening, expected);
            assert!(runtime.input.is_some());
            runtime.apply_router_outputs(function_output(action, false, Instant::now()));
            assert_eq!(runtime.listening, expected);
        }
        assert!(runtime.receipts.is_empty());
        assert!(runtime.error.is_none());
    }

    #[test]
    fn passthrough_binding_keeps_capture_available_with_listening_paused() {
        let mut runtime = app_runtime();
        let binding = runtime
            .functions
            .get_mut(&FunctionId::AppToggleListening)
            .unwrap();
        let passthrough = binding.clone();
        binding.enabled = false;
        runtime
            .functions
            .insert(FunctionId::AppTogglePassthrough, passthrough);
        runtime.tick();
        assert!(!runtime.listening);
        assert!(runtime.state.input);
        runtime.finish_recording();
        assert!(runtime.input.is_some());
        runtime
            .set_function_enabled(FunctionId::AppTogglePassthrough, false)
            .unwrap();
        assert!(runtime.input.is_none());
    }

    #[test]
    fn removing_or_disabling_last_app_binding_releases_idle_input() {
        let mut runtime = app_runtime();
        runtime
            .set_function_enabled(FunctionId::AppToggleListening, false)
            .unwrap();
        assert!(runtime.input.is_none());
        let mut runtime = app_runtime();
        runtime
            .remove_shortcut(FunctionId::AppToggleListening, 0)
            .unwrap();
        assert!(runtime.input.is_none());
        let mut runtime = app_runtime();
        runtime
            .apply_binding_command(BindingCommand::BeginCapture(CaptureTarget {
                id: FunctionId::AppToggleListening,
                slot: 0,
            }))
            .unwrap();
        runtime.finish_recording();
        assert!(runtime.input.is_some());
    }

    #[test]
    fn paused_or_stale_media_presses_do_not_reach_transport() {
        let mut runtime = app_runtime();
        runtime.transport = Some(Box::new(FakeTransport));
        runtime.state.ready = true;
        runtime.state.selected = Some("receiver".into());
        let action = FunctionAction::Media(MediaCommand::PlayPause);
        runtime.apply_router_outputs(function_output(action, true, Instant::now()));
        assert!(runtime.receipts.is_empty());
        let stale = Instant::now();
        runtime.set_listening(false).unwrap();
        runtime.set_listening(true).unwrap();
        runtime.apply_router_outputs(function_output(action, true, stale));
        assert!(runtime.receipts.is_empty());
        runtime.apply_router_outputs(function_output(action, true, Instant::now()));
        assert_eq!(runtime.receipts.len(), 1);
    }

    #[test]
    fn failed_idle_app_hook_is_reported_without_automatic_restart() {
        struct FailedInput;
        impl InputSource for FailedInput {
            fn is_finished(&self) -> bool {
                true
            }
            fn failure(&self) -> Option<String> {
                Some("hook failed".into())
            }
        }
        let mut runtime = app_runtime();
        runtime.input_initialized = true;
        runtime.input = Some(Box::new(FailedInput));
        runtime.tick();
        assert_eq!(runtime.error.as_deref(), Some("hook failed"));
        assert!(runtime.input.is_none());
        runtime.tick();
        assert!(runtime.input.is_none());
        assert!(!runtime.listening);
    }
    fn capture_runtime() -> Runtime {
        let mut runtime = Runtime::new(Config::default());
        runtime.input = Some(Box::new(FakeInput));
        runtime.input_initialized = true;
        runtime.listening = true;
        for (id, key) in [
            (FunctionId::MediaPlayPause, 0x77),
            (FunctionId::MediaNext, 0x78),
        ] {
            runtime.functions.insert(
                id,
                FunctionConfig {
                    enabled: true,
                    shortcuts: vec![Shortcut::keyboard(ModifierSet::empty(), key)],
                },
            );
        }
        runtime
    }

    fn begin_capture(runtime: &mut Runtime, id: FunctionId, slot: usize) {
        runtime
            .apply_binding_command(BindingCommand::BeginCapture(CaptureTarget { id, slot }))
            .unwrap();
    }

    fn queue_capture_keys(runtime: &mut Runtime, keys: &[(u8, bool)], native: bool) {
        for &(key, down) in keys {
            let event = InputEvent {
                code: InputCode::Key(key),
                down,
                captured: Instant::now(),
            };
            if native {
                runtime
                    .sender
                    .try_send(RoutedInput::Edge {
                        event,
                        result: function_output(
                            FunctionAction::App(AppCommand::ToggleListening),
                            down,
                            event.captured,
                        ),
                    })
                    .unwrap();
            } else {
                runtime.window_keys.push(event);
            }
        }
    }

    #[test]
    fn capture_waits_for_old_input_then_commits_the_target_for_both_sources() {
        for (native, listening) in [(false, false), (false, true), (true, false), (true, true)] {
            let mut runtime = capture_runtime();
            runtime.listening = listening;
            let id = FunctionId::MediaPlayPause;
            begin_capture(&mut runtime, id, 1);
            let original = runtime.functions.clone();
            queue_capture_keys(&mut runtime, &[(0x11, true), (0x77, true)], native);
            runtime.tick_with_capture_context(true, true);
            assert_eq!(
                runtime.capture().unwrap().phase,
                CapturePhase::WaitingForRelease
            );
            assert_eq!(runtime.functions, original);
            queue_capture_keys(&mut runtime, &[(0x77, false), (0x11, false)], native);
            runtime.tick_with_capture_context(true, false);
            assert_eq!(runtime.capture().unwrap().phase, CapturePhase::Recording);
            queue_capture_keys(&mut runtime, &[(0x79, true), (0x79, false)], native);
            runtime.tick_with_capture_context(true, false);
            assert!(runtime.capture().is_none());
            assert_eq!(
                runtime.functions[&id].shortcuts,
                vec![
                    original[&id].shortcuts[0].clone(),
                    Shortcut::keyboard(ModifierSet::empty(), 0x79)
                ]
            );
            assert_eq!(runtime.bindings_revision, 1);
            assert_eq!(runtime.listening, listening);
            assert_eq!(runtime.matched, 0);
            let saved = Config {
                functions: runtime.functions.clone(),
                ..Default::default()
            };
            let restored: Config =
                serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
            assert_eq!(restored.functions, runtime.functions);
        }
    }

    #[test]
    fn capture_invalid_and_duplicate_attempts_preserve_bindings_and_allow_retry() {
        let mut runtime = capture_runtime();
        let id = FunctionId::MediaPlayPause;
        begin_capture(&mut runtime, id, 0);
        runtime.tick_with_capture_context(true, false);
        let original = runtime.functions.clone();
        for (keys, expected) in [
            (vec![(0x11, true), (0x11, false)], CaptureError::Invalid),
            (
                vec![(0x41, true), (0x42, true), (0x42, false), (0x41, false)],
                CaptureError::Invalid,
            ),
            (
                vec![(0x78, true), (0x78, false)],
                CaptureError::Duplicate(FunctionId::MediaNext),
            ),
        ] {
            queue_capture_keys(&mut runtime, &keys, false);
            runtime.tick_with_capture_context(true, false);
            assert_eq!(runtime.capture().unwrap().error, Some(expected));
            assert_eq!(runtime.functions, original);
            assert_eq!(runtime.bindings_revision, 0);
        }
        queue_capture_keys(&mut runtime, &[(0x79, true), (0x79, false)], false);
        runtime.tick_with_capture_context(true, false);
        assert!(runtime.capture().is_none());
        assert_eq!(
            runtime.functions[&id].shortcuts,
            vec![Shortcut::keyboard(ModifierSet::empty(), 0x79)]
        );
        assert_eq!(runtime.bindings_revision, 1);
    }

    #[test]
    fn capture_cancellation_rejects_pending_and_late_input_without_polluting_next_session() {
        for cancel in [0, 1, 2] {
            let mut runtime = capture_runtime();
            let id = FunctionId::MediaPlayPause;
            begin_capture(&mut runtime, id, 0);
            runtime.tick_with_capture_context(true, false);
            let original = runtime.functions.clone();
            if cancel == 0 {
                queue_capture_keys(&mut runtime, &[(0x1b, true)], true);
            }
            queue_capture_keys(&mut runtime, &[(0x79, true), (0x79, false)], true);
            if cancel == 1 {
                runtime
                    .apply_binding_command(BindingCommand::CancelCapture)
                    .unwrap();
            }
            runtime.tick_with_capture_context(cancel != 2, false);
            assert!(runtime.capture().is_none());
            assert_eq!(runtime.functions, original);
            let stale = InputEvent {
                code: InputCode::Key(0x7a),
                down: true,
                captured: Instant::now(),
            };
            begin_capture(&mut runtime, id, 0);
            runtime.tick_with_capture_context(true, false);
            runtime.window_keys.push(stale);
            queue_capture_keys(&mut runtime, &[(0x79, true), (0x79, false)], false);
            runtime.tick_with_capture_context(true, false);
            assert_eq!(
                runtime.functions[&id].shortcuts,
                vec![Shortcut::keyboard(ModifierSet::empty(), 0x79)]
            );
            assert_eq!(runtime.bindings_revision, 1);
            assert_eq!(runtime.matched, 0);
        }
    }

    #[test]
    fn capture_target_changes_and_failed_edits_have_one_authoritative_state() {
        let mut runtime = capture_runtime();
        let id = FunctionId::MediaPlayPause;
        begin_capture(&mut runtime, id, 0);
        runtime.tick_with_capture_context(true, false);
        queue_capture_keys(&mut runtime, &[(0x79, true)], false);
        runtime.tick_with_capture_context(true, false);
        begin_capture(&mut runtime, FunctionId::MediaNext, 0);
        let target = CaptureTarget {
            id: FunctionId::MediaNext,
            slot: 0,
        };
        assert_eq!(runtime.capture().unwrap().target, target);
        assert!(
            runtime
                .apply_binding_command(BindingCommand::BeginCapture(CaptureTarget { id, slot: 2 }))
                .is_err()
        );
        assert_eq!(runtime.capture().unwrap().target, target);
        queue_capture_keys(&mut runtime, &[(0x79, false)], false);
        runtime.tick_with_capture_context(true, false);
        queue_capture_keys(&mut runtime, &[(0x7a, true), (0x7a, false)], false);
        runtime.tick_with_capture_context(true, false);
        assert_eq!(
            runtime.functions[&id].shortcuts,
            vec![Shortcut::keyboard(ModifierSet::empty(), 0x77)]
        );
        assert_eq!(
            runtime.functions[&target.id].shortcuts,
            vec![Shortcut::keyboard(ModifierSet::empty(), 0x7a)]
        );
        begin_capture(&mut runtime, id, 0);
        assert!(
            runtime
                .apply_binding_command(BindingCommand::DeleteShortcut(CaptureTarget {
                    id,
                    slot: 2
                }))
                .is_err()
        );
        assert!(runtime.capture().is_none());
        begin_capture(&mut runtime, id, 0);
        runtime
            .apply_binding_command(BindingCommand::SetFunctionEnabled { id, enabled: false })
            .unwrap();
        assert!(runtime.capture().is_none());
        assert!(
            runtime
                .apply_binding_command(BindingCommand::BeginCapture(CaptureTarget { id, slot: 0 }))
                .is_err()
        );
        assert!(runtime.capture().is_none());
        begin_capture(&mut runtime, target.id, 0);
        runtime
            .apply_binding_command(BindingCommand::DeleteShortcut(target))
            .unwrap();
        assert!(runtime.capture().is_none());
        assert!(runtime.functions[&target.id].shortcuts.is_empty());
    }

    #[test]
    fn interrupted_capture_waits_for_held_keys_before_accepting_a_new_gesture() {
        let mut runtime = capture_runtime();
        begin_capture(&mut runtime, FunctionId::MediaPlayPause, 0);
        runtime.tick_with_capture_context(true, false);
        let original = runtime.functions.clone();
        queue_capture_keys(&mut runtime, &[(0x79, true)], false);
        runtime.tick_with_capture_context(true, true);
        runtime.last_transport_ready = true;
        runtime.tick_with_capture_context(true, true);
        assert_eq!(
            runtime.capture().unwrap().phase,
            CapturePhase::WaitingForRelease
        );
        assert!(runtime.capture().unwrap().preview.is_empty());
        queue_capture_keys(&mut runtime, &[(0x79, false)], false);
        runtime.tick_with_capture_context(true, false);
        assert_eq!(runtime.functions, original);
        assert_eq!(runtime.capture().unwrap().phase, CapturePhase::Recording);
        queue_capture_keys(&mut runtime, &[(0x7a, true), (0x7a, false)], false);
        runtime.tick_with_capture_context(true, false);
        assert_eq!(runtime.bindings_revision, 1);
        assert!(runtime.capture().is_none());
    }

    #[test]
    fn capture_listener_failure_discards_candidate_and_ends_session() {
        struct FailedInput;
        impl InputSource for FailedInput {
            fn is_finished(&self) -> bool {
                true
            }
            fn failure(&self) -> Option<String> {
                Some("capture hook failed".into())
            }
        }
        let mut runtime = capture_runtime();
        begin_capture(&mut runtime, FunctionId::MediaPlayPause, 0);
        runtime.tick_with_capture_context(true, false);
        let original = runtime.functions.clone();
        queue_capture_keys(&mut runtime, &[(0x79, true), (0x79, false)], false);
        runtime.input = Some(Box::new(FailedInput));
        runtime.tick_with_capture_context(true, false);
        assert!(runtime.capture().is_none());
        assert_eq!(runtime.error.as_deref(), Some("capture hook failed"));
        assert_eq!(runtime.functions, original);
        assert!(runtime.input.is_none());
        assert!(!runtime.listening);
    }

    #[test]
    fn capture_deduplicates_window_and_hook_edges_and_does_not_trigger_old_binding() {
        let mut runtime = capture_runtime();
        let id = FunctionId::MediaPlayPause;
        begin_capture(&mut runtime, id, 0);
        runtime.tick_with_capture_context(true, false);
        for down in [true, true, false] {
            queue_capture_keys(&mut runtime, &[(0x77, down)], true);
            queue_capture_keys(&mut runtime, &[(0x77, down)], false);
            runtime.tick_with_capture_context(true, false);
        }
        assert!(runtime.capture().is_none());
        assert_eq!(runtime.bindings_revision, 1);
        assert_eq!(runtime.matched, 0);
        assert!(runtime.listening);
        assert_eq!(
            runtime.functions[&id].shortcuts,
            vec![Shortcut::keyboard(ModifierSet::empty(), 0x77)]
        );
    }
    #[test]
    fn handoff_drains_accepted_hook_input_before_a_queued_listener_edit() {
        use std::sync::{Arc, Mutex, mpsc as sync};
        struct GatedInput {
            gate: Mutex<Option<sync::Receiver<()>>>,
            entered: sync::Sender<()>,
            listening: Arc<Mutex<Vec<bool>>>,
        }
        impl InputSource for GatedInput {
            fn is_finished(&self) -> bool {
                false
            }
            fn configure(
                &self,
                _: &FunctionConfigs,
                listening: bool,
                _: bool,
                _: u64,
            ) -> RouteResult {
                self.listening.lock().unwrap().push(listening);
                if let Some(gate) = self.gate.lock().unwrap().take() {
                    self.entered.send(()).unwrap();
                    gate.recv_timeout(std::time::Duration::from_secs(3))
                        .unwrap();
                }
                RouteResult::default()
            }
        }
        let (release, gate) = sync::channel();
        let (entered, blocked) = sync::channel();
        let (sender, input) = sync::channel();
        let observed = Arc::new(Mutex::new(Vec::new()));
        let listening = observed.clone();
        let mut handle = crate::runtime_worker::RuntimeHandle::spawn(
            Config::default(),
            move |_| {
                let mut runtime = app_runtime();
                runtime.input_initialized = true;
                runtime.input = Some(Box::new(GatedInput {
                    gate: Mutex::new(Some(gate)),
                    entered,
                    listening,
                }));
                sender.send(runtime.sender.clone()).unwrap();
                runtime
            },
            Runtime::tick,
            64,
        )
        .unwrap();
        blocked
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        let created = Instant::now();
        input
            .recv()
            .unwrap()
            .try_send(RoutedInput::Edge {
                event: InputEvent {
                    code: InputCode::Key(0x78),
                    down: true,
                    captured: created,
                },
                result: function_output(
                    FunctionAction::App(AppCommand::ToggleListening),
                    true,
                    created,
                ),
            })
            .unwrap();
        handle.set_listening(false).unwrap();
        release.send(()).unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        while handle.busy() {
            handle.poll();
            assert!(Instant::now() < deadline);
            std::thread::yield_now();
        }
        let states = observed.lock().unwrap();
        assert!(states.contains(&true), "the accepted shortcut must execute");
        assert_eq!(
            states.last(),
            Some(&false),
            "the later edit must take effect last"
        );
        assert!(!handle.listening);
    }
}
