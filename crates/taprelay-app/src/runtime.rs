//! Runtime coordination around the platform input router and transport
//! adapters. The Windows input thread owns the single router instance; this
//! module only applies its parsed outputs to transport and UI state.

use crate::{
    config::{Config, Device},
    platform::{self, InputSource, Transport},
    runtime::commands::{BindingCommand, CaptureTarget},
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

mod bindings;
mod capture;
pub(crate) mod commands;
mod connection;
pub(crate) mod passthrough;
pub(crate) mod worker;

pub use capture::{CaptureError, CaptureKind, CapturePhase, CaptureSession};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum TestStatus {
    #[default]
    Idle,
    Pending(u64),
    Succeeded,
    Failed(String),
}

struct Receipt {
    created: Instant,
    test: Option<u64>,
    action: MediaCommand,
    result: oneshot::Receiver<Result<(), BackendError>>,
}

pub struct Runtime {
    pub functions: FunctionConfigs,
    pub custom_mappings: Vec<taprelay_core::mapping::CustomMapping>,
    pub custom_capture_result: Option<(u64, bool, usize, Shortcut)>,
    next_capture_result: u64,
    pub foreground_app_rules: taprelay_core::foreground_app::ForegroundAppRules,
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
    passthrough_mouse_report_rate: u16,
    passthrough_reverse_scroll: bool,
}

impl Runtime {
    #[cfg(test)]
    pub(crate) fn set_headless_transport(&mut self, transport: Box<dyn Transport>) {
        self.transport = Some(transport);
    }

    #[cfg(test)]
    pub(crate) fn headless_tick(&mut self) {
        struct Input;
        impl crate::platform::InputSource for Input {
            fn is_finished(&self) -> bool {
                false
            }
        }
        self.input = Some(Box::new(Input));
        self.tick_with_capture_context(true, false);
        self.input = Some(Box::new(Input));
    }

    pub fn passthrough_epoch(&self) -> u64 {
        self.transport
            .as_ref()
            .and_then(|transport| transport.input_link())
            .map_or(0, |link| link.epoch())
    }

    pub fn passthrough_disconnected_epoch(&self) -> u64 {
        self.transport
            .as_ref()
            .and_then(|transport| transport.input_link())
            .map_or(0, |link| link.disconnected_epoch())
    }

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
            custom_mappings: config.custom_mappings,
            custom_capture_result: None,
            next_capture_result: 0,
            foreground_app_rules: config.foreground_app_rules,
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
            passthrough_mouse_report_rate: config.options.passthrough_mouse_report_rate,
            passthrough_reverse_scroll: config.options.passthrough_reverse_scroll,
        }
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
        let mut input_error = None;
        let link = self
            .transport
            .as_ref()
            .and_then(|transport| transport.input_link());
        if let Some(link) = &link {
            link.set_mapping_profile(
                self.listening && self.custom_mappings.iter().any(|mapping| mapping.enabled),
            );
            link.set_mouse_percent(self.passthrough_mouse_percent);
            link.set_mouse_report_rate(self.passthrough_mouse_report_rate);
            link.set_reverse_scroll(self.passthrough_reverse_scroll);
        }
        if let Some(input) = &self.input {
            input.attach_passthrough(link);
            if let Some(error) = input.passthrough_error() {
                input_error = Some(error);
            }
        }
        if let Some(error) = input_error {
            self.listening = false;
            self.error = Some(error);
            if let Some(link) = self
                .transport
                .as_ref()
                .and_then(|transport| transport.input_link())
            {
                link.set_mapping_profile(false);
            }
            self.invalidate(RouterReason::TransportLost);
        }
        let result = self
            .input
            .as_ref()
            .map_or_else(RouteResult::default, |input| {
                input.configure(
                    &self.functions,
                    &self.custom_mappings,
                    &self.foreground_app_rules,
                    self.listening,
                    self.recording(),
                    self.bindings_revision,
                )
            });
        self.apply_router_outputs(result);
    }

    pub fn set_passthrough_mouse_report_rate(&mut self, hz: u16) {
        self.passthrough_mouse_report_rate = hz;
        if let Some(link) = self
            .transport
            .as_ref()
            .and_then(|transport| transport.input_link())
        {
            link.set_mouse_report_rate(hz);
        }
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
            self.test = if matches!(
                reason,
                RouterReason::TransportLost | RouterReason::SessionChanged
            ) {
                TestStatus::Idle
            } else {
                TestStatus::Failed(
                    "Test cancelled because the input or connection session changed".into(),
                )
            };
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
                RoutedInput::Edge { event, mut result } => {
                    if event.captured < self.epoch {
                        continue;
                    }
                    if self.recording() {
                        if self.capture_waiting() {
                            continue;
                        }
                        let output = self
                            .capture
                            .as_ref()
                            .is_some_and(|capture| capture.kind == CaptureKind::MappingOutput);
                        if event.code == InputCode::Key(0x1b) && event.down {
                            self.finish_recording();
                            continue;
                        }
                        if output
                            && matches!(event.code, InputCode::Key(key) if taprelay_core::input::modifier(key))
                        {
                            continue;
                        }
                        if !self.capture_state.update(event) {
                            continue;
                        }
                        self.capture.as_mut().unwrap().preview =
                            self.capture_state.pressed_inputs();
                        if let Some(shortcut) = self.recorder.observe(&self.capture_state, event) {
                            self.submit_capture(shortcut);
                        } else if self.recorder.take_invalid() {
                            self.capture.as_mut().unwrap().error = Some(CaptureError::Invalid);
                        } else if let Some(shortcut) = self.recorder.candidate()
                            && self.capture.as_ref().is_some_and(|capture| {
                                !matches!(capture.kind, CaptureKind::Function(_))
                            })
                        {
                            let error = self.custom_capture_error(&shortcut);
                            self.capture.as_mut().unwrap().error = error;
                        }
                        continue;
                    }
                    if event.captured <= self.ui_input_until {
                        // GUI input must still release commands already held by the receiver.
                        result.outputs.retain(|output| {
                            matches!(output, RoutedOutput::Function { down: false, .. })
                        });
                    }
                    self.apply_router_outputs(result);
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
                        Err(BackendError::Stale | BackendError::TimedOut) => TestStatus::Idle,
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
                    Err(BackendError::Stale | BackendError::TimedOut) => {}
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
                RoutedOutput::Mapping { down, .. } => {
                    if down {
                        self.matched += 1;
                    }
                }
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
mod tests;
