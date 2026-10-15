use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CapturePhase {
    WaitingForRelease,
    Recording,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureError {
    Invalid,
    Duplicate(FunctionId),
    DuplicateMapping(String),
    Commit(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CaptureKind {
    Function(CaptureTarget),
    MappingInput {
        slot: usize,
        draft: taprelay_core::mapping::CustomMapping,
    },
    MappingOutput,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaptureSession {
    pub kind: CaptureKind,
    pub phase: CapturePhase,
    pub preview: Vec<InputCode>,
    pub error: Option<CaptureError>,
}

impl Runtime {
    pub(super) fn record(&mut self, kind: CaptureKind) -> Result<()> {
        self.ensure_input()?;
        self.capture = Some(CaptureSession {
            kind,
            phase: CapturePhase::WaitingForRelease,
            preview: Vec::new(),
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
            BindingCommand::BeginCustomCapture {
                output,
                slot,
                draft,
            } => {
                anyhow::ensure!(
                    output || (slot <= draft.shortcuts.len() && slot < 2),
                    "Shortcut slot unavailable"
                );
                self.finish_recording();
                self.record(if output {
                    CaptureKind::MappingOutput
                } else {
                    CaptureKind::MappingInput { slot, draft }
                })
            }
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
                self.record(CaptureKind::Function(target))
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

    pub(super) fn finish_recording(&mut self) {
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

    pub fn recording(&self) -> bool {
        self.capture.is_some()
    }

    pub fn capture(&self) -> Option<&CaptureSession> {
        self.capture.as_ref()
    }

    pub(super) fn capture_waiting(&self) -> bool {
        self.capture
            .as_ref()
            .is_some_and(|capture| capture.phase == CapturePhase::WaitingForRelease)
    }

    pub(super) fn custom_capture_error(&self, shortcut: &Shortcut) -> Option<CaptureError> {
        use taprelay_core::mapping::{self, MappingOutput, RuleId};
        let capture = self.capture.as_ref()?;
        if capture.kind == CaptureKind::MappingOutput {
            return MappingOutput::from_shortcut(shortcut)
                .is_none()
                .then_some(CaptureError::Invalid);
        }
        let CaptureKind::MappingInput { slot, draft } = &capture.kind else {
            return None;
        };
        if !shortcut.valid() {
            return Some(CaptureError::Invalid);
        }
        mapping::shortcut_conflict(
            &self.functions,
            &self.foreground_app_rules,
            &self.custom_mappings,
            draft,
            *slot,
            shortcut,
        )
        .map(|rule| match rule {
            RuleId::Function(id) => CaptureError::Duplicate(id),
            RuleId::Mapping(id) => CaptureError::DuplicateMapping(if id == draft.id {
                draft.name.clone()
            } else {
                self.custom_mappings
                    .iter()
                    .find(|mapping| mapping.id == id)
                    .map_or_else(String::new, |mapping| mapping.name.clone())
            }),
        })
    }

    pub(super) fn submit_capture(&mut self, shortcut: Shortcut) {
        let Some(capture) = &self.capture else { return };
        let (function, output, slot) = match capture.kind {
            CaptureKind::Function(target) => (Some(target.id), false, target.slot),
            CaptureKind::MappingInput { slot, .. } => (None, false, slot),
            CaptureKind::MappingOutput => (None, true, 2),
        };
        let Some(id) = function else {
            if let Some(error) = self.custom_capture_error(&shortcut) {
                self.capture.as_mut().unwrap().error = Some(error);
                return;
            }
            self.next_capture_result += 1;
            self.custom_capture_result = Some((self.next_capture_result, output, slot, shortcut));
            self.finish_recording();
            return;
        };
        let error = if !shortcut.valid() {
            Some(CaptureError::Invalid)
        } else if let Some(conflict) = self.functions.iter().find_map(|(&other, config)| {
            config
                .shortcuts
                .iter()
                .enumerate()
                .any(|(other_slot, candidate)| {
                    (other, other_slot) != (id, slot)
                        && candidate == &shortcut
                        && self.foreground_app_rules.overlaps(id, other)
                })
                .then_some(other)
        }) {
            Some(CaptureError::Duplicate(conflict))
        } else if let Some(mapping) = self.custom_mappings.iter().find(|mapping| {
            mapping.shortcuts.contains(&shortcut)
                && taprelay_core::mapping::scopes_overlap(
                    &self.foreground_app_rules,
                    &self
                        .foreground_app_rules
                        .assignments
                        .get(&id)
                        .cloned()
                        .unwrap_or_default(),
                    &mapping.groups,
                )
        }) {
            Some(CaptureError::DuplicateMapping(mapping.name.clone()))
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
}
