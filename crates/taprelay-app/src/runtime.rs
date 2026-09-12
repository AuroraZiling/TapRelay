//! Runtime coordination around the platform input router and transport
//! adapters. The Windows input thread owns the single router instance; this
//! module only applies its parsed outputs to transport and UI state.

use crate::{
    config::Config,
    feedback::TestStatus,
    platform::{self, InputSource, Transport},
};
use anyhow::{Context, Result};
use std::time::Instant;
use taprelay_core::{
    command::{COMMAND_TTL, CommandPhase, MediaCommand, QueuedCommand, QueuedReport},
    function::{FunctionAction, FunctionId, Shortcut, complete_configs},
    hid::{self, ConsumerState, KeyboardState, MouseReport, ReportKind, split_relative},
    input::{InputCode, InputEvent, InputState, Recorder},
    input_router::{PhysicalInput, RouteResult, RoutedInput, RoutedOutput, RouterReason},
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
    remote_keyboard: KeyboardState,
    remote_consumer: ConsumerState,
    remote_mouse_buttons: u8,
    last_transport_ready: bool,
}

impl Runtime {
    pub fn new(mut config: Config) -> Self {
        complete_configs(&mut config.functions);
        // Device selection is session-only. Older config files may still
        // deserialize a device record, but it must never be restored or used.
        config.device = None;
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
            remote_keyboard: KeyboardState::default(),
            remote_consumer: ConsumerState::default(),
            remote_mouse_buttons: 0,
            last_transport_ready: false,
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
            while self.events.try_recv().is_ok() {}
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
                    self.state.passthrough_ready,
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
        let cleanup = self
            .input
            .as_ref()
            .map_or_else(RouteResult::default, |input| input.terminate(reason));
        self.apply_router_outputs(cleanup);
        self.epoch = Instant::now();
        if matches!(self.test, TestStatus::Pending(_)) {
            self.test = TestStatus::Failed(
                "Test cancelled because the input or connection session changed".into(),
            );
        }
        if let Some(transport) = &self.transport {
            transport.invalidate();
        }
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
    }

    pub fn record(&mut self) -> Result<()> {
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

    pub fn unbind_function(&mut self, id: FunctionId) -> Result<()> {
        let function = self
            .config
            .functions
            .get_mut(&id)
            .context("Unknown function")?;
        function.shortcuts.clear();
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

    pub fn shortcut_conflict(
        &self,
        id: FunctionId,
        slot: usize,
        shortcut: &Shortcut,
    ) -> Option<FunctionId> {
        self.config.functions.iter().find_map(|(&other, config)| {
            config
                .shortcuts
                .iter()
                .enumerate()
                .any(|(other_slot, candidate)| {
                    (other, other_slot) != (id, slot) && candidate == shortcut
                })
                .then_some(other)
        })
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

    fn enqueue_report(
        &mut self,
        kind: ReportKind,
        bytes: Vec<u8>,
        created: Instant,
        must_deliver: bool,
    ) -> Result<()> {
        let ready = match kind {
            ReportKind::Consumer => self.state.ready,
            ReportKind::Keyboard | ReportKind::Mouse => self.state.passthrough_ready,
        };
        anyhow::ensure!(ready, "HID report channel is not ready");
        let report = QueuedReport {
            kind,
            bytes,
            target: self.state.selected.clone().context("No selected device")?,
            generation: self.state.generation,
            created,
            must_deliver,
        };
        anyhow::ensure!(
            report.valid(
                self.state.selected.as_deref(),
                self.state.generation,
                ready,
                Instant::now(),
                self.epoch,
            ),
            "HID input report expired"
        );
        self.transport
            .as_ref()
            .context("Bluetooth not started")?
            .send_report(report)
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
                    && (snapshot.selected == self.state.selected || snapshot.selected.is_none())))
        {
            self.state = snapshot;
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
            while self.events.try_recv().is_ok() {}
            self.capture_state = InputState::default();
            self.capture = CapturePhase::Recording;
            self.epoch = Instant::now();
        }

        let mut pending = Vec::new();
        for _ in 0..1024 {
            let Ok(input) = self.events.try_recv() else {
                break;
            };
            if let Some(previous) = pending.last_mut()
                && merge_motion(previous, &input)
            {
                continue;
            }
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
                RoutedInput::Motion { event, result } => {
                    if !self.recording() && event.captured >= self.epoch {
                        self.apply_router_outputs(result);
                    }
                }
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
        if result.revision != self.bindings_revision && !result.outputs.is_empty() {
            tracing::debug!(
                result_revision = result.revision,
                current_revision = self.bindings_revision,
                "Dropping input result from an older configuration revision"
            );
            return;
        }
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
                        }
                    }
                }
                RoutedOutput::Function { .. } => {}
                RoutedOutput::PassthroughChanged(enabled) => {
                    tracing::info!(enabled, "Passthrough route changed");
                }
                RoutedOutput::ResetRemote => {
                    self.remote_keyboard.clear();
                    self.remote_consumer.clear();
                    self.remote_mouse_buttons = 0;
                    if self.state.ready {
                        let now = Instant::now();
                        if let Err(error) = self.enqueue_report(
                            ReportKind::Consumer,
                            self.remote_consumer.report(),
                            now,
                            true,
                        ) {
                            self.error = Some(error.to_string());
                        }
                    }
                    if self.state.passthrough_ready {
                        let now = Instant::now();
                        if let Err(error) = self.enqueue_report(
                            ReportKind::Keyboard,
                            vec![0; hid::KEYBOARD_REPORT_LENGTH],
                            now,
                            true,
                        ) {
                            self.error = Some(error.to_string());
                        }
                        if let Err(error) = self.enqueue_report(
                            ReportKind::Mouse,
                            MouseReport::neutral().encode().to_vec(),
                            now,
                            true,
                        ) {
                            self.error = Some(error.to_string());
                        }
                    }
                }
                RoutedOutput::Remote(input) => {
                    if let Err(error) = self.send_remote(input) {
                        self.error = Some(error.to_string());
                        self.invalidate(RouterReason::TransportLost);
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

    fn send_remote(&mut self, input: PhysicalInput) -> Result<()> {
        let now = Instant::now();
        match input {
            PhysicalInput::Edge { code, down } => match code {
                InputCode::Key(key) => {
                    if let Some(usage) = hid::consumer_usage(key) {
                        if self.state.ready {
                            let owner = 0x8000_0000u64 | u64::from(key);
                            if down {
                                self.remote_consumer.press_usage(owner, usage);
                            } else {
                                self.remote_consumer.release_usage(owner, usage);
                            }
                            self.enqueue_report(
                                ReportKind::Consumer,
                                self.remote_consumer.report(),
                                now,
                                !down,
                            )?;
                        }
                        return Ok(());
                    }
                    if let Some(bit) = hid::keyboard_modifier_bit(key) {
                        self.remote_keyboard.set_modifier(bit, down);
                    } else if let Some(usage) = hid::keyboard_usage(key) {
                        self.remote_keyboard.set_key(usage, down).map_err(|_| {
                            anyhow::anyhow!("More than six keyboard keys are held; input released")
                        })?;
                    }
                    self.enqueue_report(
                        ReportKind::Keyboard,
                        self.remote_keyboard
                            .report()
                            .map_err(|_| anyhow::anyhow!("Keyboard rollover overflow"))?
                            .to_vec(),
                        now,
                        !down,
                    )?;
                }
                InputCode::Mouse(button) => {
                    let bit = 1 << button as u8;
                    if down {
                        self.remote_mouse_buttons |= bit;
                    } else {
                        self.remote_mouse_buttons &= !bit;
                    }
                    self.enqueue_report(
                        ReportKind::Mouse,
                        MouseReport {
                            buttons: self.remote_mouse_buttons,
                            ..MouseReport::neutral()
                        }
                        .encode()
                        .to_vec(),
                        now,
                        !down,
                    )?;
                }
            },
            PhysicalInput::Motion { dx, dy } => {
                let xs: Vec<_> = split_relative(dx).collect();
                let ys: Vec<_> = split_relative(dy).collect();
                let count = xs.len().max(ys.len());
                for index in 0..count {
                    self.enqueue_report(
                        ReportKind::Mouse,
                        MouseReport {
                            buttons: self.remote_mouse_buttons,
                            x: xs.get(index).copied().unwrap_or_default(),
                            y: ys.get(index).copied().unwrap_or_default(),
                            ..MouseReport::neutral()
                        }
                        .encode()
                        .to_vec(),
                        now,
                        false,
                    )?;
                }
            }
            PhysicalInput::Wheel {
                vertical,
                horizontal,
            } => {
                let verticals: Vec<_> = split_wheel(vertical).collect();
                let horizontals: Vec<_> = split_wheel(horizontal).collect();
                let count = verticals.len().max(horizontals.len());
                for index in 0..count {
                    self.enqueue_report(
                        ReportKind::Mouse,
                        MouseReport {
                            buttons: self.remote_mouse_buttons,
                            wheel: verticals.get(index).copied().unwrap_or_default(),
                            horizontal_wheel: horizontals.get(index).copied().unwrap_or_default(),
                            ..MouseReport::neutral()
                        }
                        .encode()
                        .to_vec(),
                        now,
                        false,
                    )?;
                }
            }
        }
        Ok(())
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        self.invalidate(RouterReason::ApplicationExit);
    }
}

/// Collapse only adjacent motion reports with the same route and report kind.
/// Keyboard/button edges, route changes, and pending-modifier replays remain
/// hard ordering boundaries. The accumulator is deliberately bounded by the
/// per-tick dispatch drain rather than growing with pointer rate.
fn merge_motion(previous: &mut RoutedInput, next: &RoutedInput) -> bool {
    let RoutedInput::Motion {
        event: previous_event,
        result: previous_result,
    } = previous
    else {
        return false;
    };
    let RoutedInput::Motion {
        event: next_event,
        result: next_result,
    } = next
    else {
        return false;
    };
    if previous_result.consume != next_result.consume
        || previous_result.outputs.len() != 1
        || next_result.outputs.len() != 1
    {
        return false;
    }
    let (route, previous_input, next_input) =
        match (&previous_result.outputs[0], &next_result.outputs[0]) {
            (RoutedOutput::Local(previous), RoutedOutput::Local(next)) => (0u8, previous, next),
            (RoutedOutput::Remote(previous), RoutedOutput::Remote(next)) => (1u8, previous, next),
            _ => return false,
        };
    let Some(input) = add_motion(*previous_input, *next_input) else {
        return false;
    };
    previous_event.input = input;
    previous_event.captured = previous_event.captured.max(next_event.captured);
    previous_result.outputs[0] = if route == 0 {
        RoutedOutput::Local(input)
    } else {
        RoutedOutput::Remote(input)
    };
    true
}

fn add_motion(left: PhysicalInput, right: PhysicalInput) -> Option<PhysicalInput> {
    match (left, right) {
        (PhysicalInput::Motion { dx, dy }, PhysicalInput::Motion { dx: rx, dy: ry }) => {
            Some(PhysicalInput::Motion {
                dx: dx.checked_add(rx)?,
                dy: dy.checked_add(ry)?,
            })
        }
        (
            PhysicalInput::Wheel {
                vertical,
                horizontal,
            },
            PhysicalInput::Wheel {
                vertical: rv,
                horizontal: rh,
            },
        ) => Some(PhysicalInput::Wheel {
            vertical: vertical.checked_add(rv)?,
            horizontal: horizontal.checked_add(rh)?,
        }),
        _ => None,
    }
}

fn split_wheel(mut value: i32) -> impl Iterator<Item = i8> {
    std::iter::from_fn(move || {
        if value == 0 {
            return None;
        }
        let part = value.clamp(i8::MIN as i32, i8::MAX as i32) as i8;
        value -= i32::from(part);
        Some(part)
    })
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
