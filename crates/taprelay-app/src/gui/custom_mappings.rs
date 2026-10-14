use super::*;
use crate::{MappingOutputOption, MappingRow, platform};
use taprelay_core::{
    function::ModifierSet,
    input::MouseButton,
    mapping::{self, CustomMapping, MappingOutput, RuleId},
};

pub(super) struct MappingDraft {
    pub mapping: CustomMapping,
    original: CustomMapping,
    new: bool,
}
pub(super) enum MappingSave {
    Editor,
    Change,
    Undo,
    Delete(Vec<CustomMapping>),
}

impl Controller {
    fn mapping_pull(&mut self, ui: &mut Presentation) {
        let Some(draft) = &mut self.mapping_draft else {
            return;
        };
        draft.mapping.name = ui.mapping.name.clone().to_string();
        draft.mapping.output = with_ui_modifiers(
            output_from_choice(&ui.mapping.selected_output.clone().value).unwrap_or(
                MappingOutput::Keyboard {
                    usage: 0,
                    modifiers: 0,
                },
            ),
            ui,
        );
    }

    pub(super) fn mapping_dirty(&mut self, ui: &mut Presentation) -> bool {
        self.mapping_pull(ui);
        self.mapping_draft
            .as_ref()
            .is_some_and(|draft| draft.mapping != draft.original)
    }

    pub(super) fn mapping_leave(
        &mut self,
        ui: &mut Presentation,
        next: Option<(Action, String)>,
    ) -> Result<bool> {
        if self.pending_mappings.is_some() {
            return Ok(false);
        }
        if self.recording() {
            self.cancel_capture()?;
        }
        if self.mapping_dirty(ui) {
            self.mapping_navigation = next;
            ui.mapping.leave_open = true;
            return Ok(false);
        }
        self.mapping_close_editor(ui);
        Ok(true)
    }

    fn mapping_close_editor(&mut self, ui: &mut Presentation) {
        self.mapping_draft = None;
        ui.mapping.editor = false;
        ui.mapping.saving = false;
        ui.mapping.error = "".into();
        ui.mapping.conflict_target = "".into();
        ui.mapping.leave_open = false;
    }

    fn mapping_finish_leave(&mut self, ui: &mut Presentation) -> Result<()> {
        self.mapping_close_editor(ui);
        if let Some((action, value)) = self.mapping_navigation.take() {
            self.action(ui, action, &value)?;
        }
        Ok(())
    }

    pub(super) fn mapping_command(
        &mut self,
        ui: &mut Presentation,
        command: &str,
        value: &str,
    ) -> Result<()> {
        match command {
            "conflict" => {
                if let Some(id) = value.strip_prefix("mapping/") {
                    return self.action(ui, Action::EditMapping, id);
                }
                return self.action(ui, Action::Navigate, "1");
            }
            "cancel-capture" => {
                self.cancel_capture()?;
                return Ok(());
            }
            "stay" => {
                ui.mapping.leave_open = false;
                self.mapping_navigation = None;
                return Ok(());
            }
            "discard" => {
                return self.mapping_finish_leave(ui);
            }
            "back" => {
                if ui.mapping.editor {
                    self.mapping_leave(ui, None)?;
                } else {
                    ui.mapping.open = false;
                    ui.mapping.error = "".into();
                }
                return Ok(());
            }
            _ => {}
        }
        anyhow::ensure!(
            !self.runtime.busy() && !self.recording() && !self.runtime.stopped(),
            self.tr(keys::RUNTIME_BUSY)
        );
        self.mapping_pull(ui);
        ui.mapping.error = "".into();
        ui.mapping.conflict_target = "".into();
        match command {
            "open" => {
                ui.mapping.open = true;
            }
            "create" | "edit" => {
                let new = command != "edit";
                let id = new_mapping_id(&self.runtime.config.custom_mappings);
                let rule = if command == "edit" {
                    self.runtime
                        .config
                        .custom_mappings
                        .iter()
                        .find(|rule| rule.id.to_string() == value)
                        .context("Mapping no longer exists")?
                        .clone()
                } else {
                    CustomMapping {
                        id,
                        name: String::new(),
                        enabled: true,
                        shortcuts: Vec::new(),
                        output: MappingOutput::Keyboard {
                            usage: 0,
                            modifiers: 0,
                        },
                        groups: Default::default(),
                    }
                };
                self.mapping_draft = Some(MappingDraft {
                    original: rule.clone(),
                    mapping: rule,
                    new,
                });
                self.last_mapping_capture = self
                    .runtime
                    .custom_capture_result
                    .as_ref()
                    .map_or(0, |result| result.0);
                ui.mapping.open = true;
                ui.mapping.editor = true;
                ui.mapping.new_mapping = new;
                ui.mapping.entry_mode = 0;
                ui.mapping.output_filter = "".into();
                self.sync_output_options(ui);
                ui.mapping.focus_request += 1;
                ui.mapping.name = self.mapping_draft.as_ref().unwrap().mapping.name.clone();
                ui.mapping.ctrl = false;
                ui.mapping.shift = false;
                ui.mapping.alt = false;
                ui.mapping.meta = false;
                self.mapping_output_to_ui(ui);
                self.sync_mapping_draft(ui);
            }
            "filter-output" => {
                ui.mapping.output_filter = value.into();
                self.sync_output_options(ui);
            }
            "select-output" => {
                let output = with_ui_modifiers(
                    output_from_choice(value).context("Unsupported receiver key")?,
                    ui,
                );
                self.mapping_draft
                    .as_mut()
                    .context("Missing mapping draft")?
                    .mapping
                    .output = output;
                self.mapping_output_to_ui(ui);
            }
            "capture-input" | "capture-output" => {
                self.runtime
                    .apply_binding_command(BindingCommand::BeginCustomCapture {
                        output: command == "capture-output",
                        slot: if command == "capture-output" {
                            2
                        } else {
                            value.parse()?
                        },
                        draft: self
                            .mapping_draft
                            .as_ref()
                            .context("Missing mapping draft")?
                            .mapping
                            .clone(),
                    })?;
            }
            "remove-input" => {
                let index: usize = value.parse()?;
                let draft = self
                    .mapping_draft
                    .as_mut()
                    .context("Missing mapping draft")?;
                anyhow::ensure!(index < draft.mapping.shortcuts.len(), "Invalid input slot");
                draft.mapping.shortcuts.remove(index);
                self.sync_mapping_draft(ui);
            }
            "scope" => {
                let draft = self
                    .mapping_draft
                    .as_mut()
                    .context("Missing mapping draft")?;
                if value.is_empty() {
                    draft.mapping.groups.clear();
                } else if !draft.mapping.groups.remove(value) {
                    draft.mapping.groups.insert(value.to_owned());
                }
                self.sync_mapping_draft(ui);
            }
            "save" | "save-leave" => {
                if let Some(draft) = &self.mapping_draft
                    && draft.mapping.name.trim().is_empty()
                    && !draft.mapping.shortcuts.is_empty()
                {
                    let base = format!(
                        "{} → {}",
                        i18n::shortcut_labels(self.locale(), &draft.mapping.shortcuts[0]).join("+"),
                        self.mapping_output_label(draft.mapping.output)
                    );
                    let name = unique_mapping_name(&base, &self.runtime.config.custom_mappings);
                    self.mapping_draft.as_mut().unwrap().mapping.name = name.clone();
                    ui.mapping.name = name;
                }
                let draft = self
                    .mapping_draft
                    .as_mut()
                    .context("Missing mapping draft")?;
                draft.mapping.name = draft.mapping.name.trim().to_string();
                let mut candidate = self.runtime.config.custom_mappings.clone();
                if draft.new {
                    candidate.push(draft.mapping.clone());
                } else {
                    *candidate
                        .iter_mut()
                        .find(|rule| rule.id == draft.mapping.id)
                        .context("Mapping no longer exists")? = draft.mapping.clone();
                }
                if let Err(error) = self.validate_mappings(ui, &candidate) {
                    if command == "save-leave" {
                        ui.mapping.leave_open = false;
                        self.mapping_navigation = None;
                    }
                    return Err(error);
                }
                self.runtime
                    .set_custom_mappings(candidate.clone(), &self.path)?;
                self.pending_mappings = Some((candidate, MappingSave::Editor));
                ui.mapping.leave_open = false;
                ui.mapping.saving = true;
            }
            "toggle" | "delete" => {
                let mut candidate = self.runtime.config.custom_mappings.clone();
                let index = candidate
                    .iter()
                    .position(|rule| rule.id.to_string() == value)
                    .context("Mapping no longer exists")?;
                let purpose = if command == "toggle" {
                    candidate[index].enabled = !candidate[index].enabled;
                    MappingSave::Change
                } else {
                    candidate.remove(index);
                    MappingSave::Delete(self.runtime.config.custom_mappings.clone())
                };
                self.runtime
                    .set_custom_mappings(candidate.clone(), &self.path)?;
                self.pending_mappings = Some((candidate, purpose));
            }
            "undo" => {
                let (before, until) = self.mapping_undo.as_ref().context("Nothing to undo")?;
                anyhow::ensure!(Instant::now() < *until, "Undo expired");
                let mut candidate = self.runtime.config.custom_mappings.clone();
                for (index, mapping) in before.iter().enumerate() {
                    if !candidate.iter().any(|current| current.id == mapping.id) {
                        candidate.insert(index.min(candidate.len()), mapping.clone());
                    }
                }
                self.validate_mappings(ui, &candidate)?;
                self.runtime
                    .set_custom_mappings(candidate.clone(), &self.path)?;
                self.pending_mappings = Some((candidate, MappingSave::Undo));
            }
            _ => bail!("Unknown mapping command"),
        }
        Ok(())
    }

    fn validate_mappings(&self, ui: &mut Presentation, candidate: &[CustomMapping]) -> Result<()> {
        if let Some((first, second)) = mapping::conflict(
            &self.runtime.config.functions,
            &self.runtime.config.foreground_app_rules,
            candidate,
        ) {
            let current = self
                .mapping_draft
                .as_ref()
                .map(|draft| RuleId::Mapping(draft.mapping.id));
            let target = if Some(first) == current {
                second
            } else {
                first
            };
            ui.mapping.conflict_target = if first == second {
                String::new()
            } else {
                match target {
                    RuleId::Mapping(id) => format!("mapping/{id}"),
                    RuleId::Function(id) => format!("function/{}", id.stable_id()),
                }
            };
            let name = |id| match id {
                RuleId::Function(id) => self.tr(function::function_definition(id).name_key),
                RuleId::Mapping(id) => candidate
                    .iter()
                    .find(|mapping| mapping.id == id)
                    .map_or_else(String::new, |mapping| mapping.name.clone()),
            };
            bail!(
                "{}: {} / {}",
                self.tr(keys::CAPTURE_DUPLICATE),
                name(first),
                name(second)
            );
        }
        anyhow::ensure!(
            mapping::valid(
                &self.runtime.config.functions,
                &self.runtime.config.foreground_app_rules,
                candidate
            ),
            self.tr(keys::MAPPINGS_INVALID)
        );
        Ok(())
    }

    fn mapping_output_to_ui(&self, ui: &mut Presentation) {
        let Some(draft) = &self.mapping_draft else {
            return;
        };
        ui.mapping.selected_output = output_option(self.locale(), draft.mapping.output);
        if let MappingOutput::Keyboard { modifiers, .. } = draft.mapping.output {
            ui.mapping.ctrl = modifiers & 1 != 0;
            ui.mapping.shift = modifiers & 2 != 0;
            ui.mapping.alt = modifiers & 4 != 0;
            ui.mapping.meta = modifiers & 8 != 0;
        }
    }

    fn sync_mapping_draft(&self, ui: &mut Presentation) {
        let Some(draft) = &self.mapping_draft else {
            return;
        };
        ui.mapping.inputs = draft
            .mapping
            .shortcuts
            .iter()
            .map(|input| i18n::shortcut_labels(self.locale(), input).join("+"))
            .collect();
        let mut scopes = vec![crate::ForegroundAppScopeOption {
            id: "".into(),
            name: self.tr(keys::COMMON_ALL_APPS),
            checked: draft.mapping.groups.is_empty(),
        }];
        scopes.extend(
            self.runtime
                .config
                .foreground_app_rules
                .groups
                .iter()
                .map(|(id, group)| crate::ForegroundAppScopeOption {
                    id: id.clone(),
                    name: group.name.clone(),
                    checked: draft.mapping.groups.contains(id),
                }),
        );
        ui.mapping.scopes = scopes;
        ui.mapping.scope_label = if draft.mapping.groups.is_empty() {
            self.tr(keys::COMMON_ALL_APPS)
        } else {
            draft
                .mapping
                .groups
                .iter()
                .filter_map(|id| self.runtime.config.foreground_app_rules.groups.get(id))
                .map(|group| group.name.as_str())
                .collect::<Vec<_>>()
                .join(" / ")
        };
    }

    pub(super) fn sync_mapping_operation(&mut self, ui: &mut Presentation) {
        if !self.runtime.busy()
            && let Some((candidate, purpose)) = self.pending_mappings.take()
        {
            if self.runtime.take_custom_mappings_saved() == Some(true)
                && self.runtime.config.custom_mappings == candidate
            {
                match purpose {
                    MappingSave::Editor => {
                        if let Err(error) = self.mapping_finish_leave(ui) {
                            ui.mapping.error = error.to_string();
                        }
                    }
                    MappingSave::Delete(before) => {
                        self.mapping_undo = Some((before, Instant::now() + Duration::from_secs(8)));
                    }
                    MappingSave::Change => {}
                    MappingSave::Undo => {
                        self.mapping_undo = None;
                    }
                }
            } else {
                ui.mapping.saving = false;
                ui.mapping.error = self.tr(keys::COMMON_SAVE_FAILED);
                self.mapping_navigation = None;
            }
        }
        if let Some((serial, output, slot, shortcut)) = self.runtime.custom_capture_result.clone()
            && serial != self.last_mapping_capture
        {
            self.last_mapping_capture = serial;
            self.mapping_pull(ui);
            if let Some(draft) = &mut self.mapping_draft {
                if output {
                    if let Some(output) = MappingOutput::from_shortcut(&shortcut) {
                        draft.mapping.output = with_ui_modifiers(output, ui);
                    }
                    self.mapping_output_to_ui(ui);
                } else if slot < 2 {
                    if slot == draft.mapping.shortcuts.len() {
                        draft.mapping.shortcuts.push(shortcut);
                    } else if let Some(input) = draft.mapping.shortcuts.get_mut(slot) {
                        *input = shortcut;
                    }
                }
                self.sync_mapping_draft(ui);
            }
        }
        if self
            .mapping_undo
            .as_ref()
            .is_some_and(|(_, until)| Instant::now() >= *until)
        {
            self.mapping_undo = None;
        }
        ui.mapping.can_undo = self.mapping_undo.is_some();
        ui.mapping.busy = self.runtime.busy() || self.runtime.stopped() || self.recording();
        let capture = self
            .runtime
            .capture()
            .filter(|capture| !matches!(capture.kind, CaptureKind::Function(_)));
        ui.mapping.capture_slot = capture.map_or(-1, |capture| match capture.kind {
            CaptureKind::MappingInput { slot, .. } => slot as i32,
            CaptureKind::MappingOutput => 2,
            CaptureKind::Function(_) => -1,
        });
        ui.mapping.capture_text = capture
            .map(|capture| {
                if capture.phase == CapturePhase::WaitingForRelease {
                    self.tr(keys::CAPTURE_RELEASE)
                } else if capture.preview.is_empty() {
                    self.tr(keys::CAPTURE_RECORDING)
                } else {
                    i18n::capture_preview(self.locale(), capture)
                }
            })
            .unwrap_or_default();
        ui.mapping.capture_error = capture
            .and_then(|capture| capture.error.as_ref())
            .map(|error| self.mapping_capture_error(error))
            .unwrap_or_default();
        let input_error = self
            .mapping_draft
            .as_ref()
            .and_then(|draft| {
                draft
                    .mapping
                    .shortcuts
                    .iter()
                    .enumerate()
                    .find_map(|(slot, shortcut)| {
                        mapping::shortcut_conflict(
                            &self.runtime.config.functions,
                            &self.runtime.config.foreground_app_rules,
                            &self.runtime.config.custom_mappings,
                            &draft.mapping,
                            slot,
                            shortcut,
                        )
                    })
            })
            .map(|rule| {
                let name = match rule {
                    RuleId::Function(id) => self.tr(function::function_definition(id).name_key),
                    RuleId::Mapping(id) => self
                        .mapping_draft
                        .as_ref()
                        .filter(|draft| draft.mapping.id == id)
                        .map(|draft| draft.mapping.name.clone())
                        .or_else(|| {
                            self.runtime
                                .config
                                .custom_mappings
                                .iter()
                                .find(|mapping| mapping.id == id)
                                .map(|mapping| mapping.name.clone())
                        })
                        .unwrap_or_default(),
                };
                self.mapping_capture_error(&CaptureError::DuplicateMapping(name))
            })
            .unwrap_or_default();
        ui.mapping.input_error = input_error;
    }

    fn mapping_capture_error(&self, error: &CaptureError) -> String {
        match error {
            CaptureError::Invalid => self.tr(keys::CAPTURE_INVALID),
            CaptureError::Duplicate(id) => format!(
                "{}: {}",
                self.tr(keys::CAPTURE_DUPLICATE),
                self.tr(function::function_definition(*id).name_key)
            ),
            CaptureError::DuplicateMapping(name) => format!(
                "{}: {}",
                self.tr(keys::CAPTURE_DUPLICATE),
                if name.trim().is_empty() {
                    self.tr(keys::MAPPINGS_INPUT)
                } else {
                    name.clone()
                }
            ),
            CaptureError::Commit(error) => error.clone(),
        }
    }

    pub(super) fn sync_mapping_rows(&self, ui: &mut Presentation) {
        let rules = &self.runtime.config.foreground_app_rules;
        ui.mapping.rows = self
            .runtime
            .config
            .custom_mappings
            .iter()
            .map(|mapping| MappingRow {
                id: mapping.id.to_string(),
                name: mapping.name.clone(),
                enabled: mapping.enabled,
                input: mapping
                    .shortcuts
                    .iter()
                    .map(|shortcut| i18n::shortcut_labels(self.locale(), shortcut).join("+"))
                    .collect::<Vec<_>>()
                    .join(" / "),
                output: self.mapping_output_label(mapping.output),
                scope: if mapping.groups.is_empty() {
                    self.tr(keys::COMMON_ALL_APPS)
                } else {
                    mapping
                        .groups
                        .iter()
                        .filter_map(|id| rules.groups.get(id))
                        .map(|group| group.name.as_str())
                        .collect::<Vec<_>>()
                        .join(" / ")
                },
            })
            .collect();
        self.sync_output_options(ui);
        if let Some(output) = output_from_choice(&ui.mapping.selected_output.clone().value) {
            ui.mapping.selected_output = output_option(self.locale(), output);
        }
        self.sync_mapping_draft(ui);
    }

    fn sync_output_options(&self, ui: &mut Presentation) {
        ui.mapping.output_options =
            output_options(self.locale(), &ui.mapping.output_filter.clone());
    }

    pub(super) fn mapping_output_label(&self, output: MappingOutput) -> String {
        match output {
            MappingOutput::Keyboard { usage, modifiers } => {
                let key = mapping::output_keys()
                    .iter()
                    .find_map(|(key, value)| (*value == usage).then_some(*key))
                    .unwrap_or(0);
                let mut parts = ModifierSet {
                    ctrl: modifiers & 1 != 0,
                    shift: modifiers & 2 != 0,
                    alt: modifiers & 4 != 0,
                    win: modifiers & 8 != 0,
                }
                .labels();
                for part in &mut parts {
                    if part == "Win" {
                        *part = "Win / Command".into();
                    }
                }
                parts.push(platform::key_name(key));
                parts.join("+")
            }
            MappingOutput::Mouse { button } => i18n::mouse_button_label(self.locale(), button),
        }
    }
}

fn with_ui_modifiers(mut output: MappingOutput, ui: &mut Presentation) -> MappingOutput {
    if let MappingOutput::Keyboard { modifiers, .. } = &mut output {
        *modifiers = u8::from(ui.mapping.ctrl)
            | (u8::from(ui.mapping.shift) << 1)
            | (u8::from(ui.mapping.alt) << 2)
            | (u8::from(ui.mapping.meta) << 3);
    }
    output
}

fn keyboard_option(key: u8, usage: u8) -> MappingOutputOption {
    MappingOutputOption {
        label: platform::key_name(key),
        value: format!("key/{usage}"),
        is_mouse: false,
    }
}

fn output_option(locale: &str, output: MappingOutput) -> MappingOutputOption {
    match output {
        MappingOutput::Keyboard { usage, .. } => mapping::output_keys()
            .into_iter()
            .find_map(|(key, value)| (value == usage).then(|| keyboard_option(key, usage)))
            .unwrap_or_default(),
        MappingOutput::Mouse { button } => MappingOutputOption {
            label: i18n::mouse_button_label(locale, button),
            value: format!("mouse/{}", button as usize),
            is_mouse: true,
        },
    }
}

fn output_options(locale: &str, filter: &str) -> Vec<MappingOutputOption> {
    let filter = filter.trim().to_lowercase();
    let keyboard = rust_i18n::t!(keys::INPUT_KEYBOARD, locale = locale);
    let mouse = rust_i18n::t!(keys::INPUT_MOUSE, locale = locale);
    mapping::output_keys()
        .into_iter()
        .map(|(key, usage)| keyboard_option(key, usage))
        .chain(
            MouseButton::ALL
                .into_iter()
                .map(|button| output_option(locale, MappingOutput::Mouse { button })),
        )
        .filter(|option| {
            let category = if option.is_mouse { &mouse } else { &keyboard };
            format!("{category} {}", option.label)
                .to_lowercase()
                .contains(&filter)
        })
        .collect()
}

fn output_from_choice(value: &str) -> Option<MappingOutput> {
    if let Some(usage) = value.strip_prefix("key/") {
        let output = MappingOutput::Keyboard {
            usage: usage.parse().ok()?,
            modifiers: 0,
        };
        return output.valid().then_some(output);
    }
    let index: usize = value.strip_prefix("mouse/")?.parse().ok()?;
    MouseButton::ALL
        .get(index)
        .map(|button| MappingOutput::Mouse { button: *button })
}

fn new_mapping_id(mappings: &[CustomMapping]) -> u64 {
    (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros()
        .min(u128::from(u64::MAX)) as u64)
        .max(
            mappings
                .iter()
                .map(|mapping| mapping.id)
                .max()
                .unwrap_or(0)
                .saturating_add(1),
        )
}

fn unique_mapping_name(base: &str, mappings: &[CustomMapping]) -> String {
    let base: String = base.trim().chars().take(80).collect();
    let exists = |name: &str| {
        mappings
            .iter()
            .any(|mapping| mapping.name.trim().to_lowercase() == name.to_lowercase())
    };
    if !exists(&base) {
        return base;
    }
    for index in 2.. {
        let suffix = format!(" ({index})");
        let name = format!(
            "{}{}",
            base.chars().take(80 - suffix.len()).collect::<String>(),
            suffix
        );
        if !exists(&name) {
            return name;
        }
    }
    unreachable!()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_choices_cover_supported_outputs_and_can_find_keys_missing_from_a_keyboard() {
        for locale in ["en", "zh-CN"] {
            let choices = output_options(locale, "");
            let mut values = std::collections::BTreeSet::new();
            for choice in choices {
                assert!(values.insert(choice.value.to_string()));
                let output = output_from_choice(&choice.value).unwrap();
                assert_eq!(output_option(locale, output), choice);
            }
            let f24 = output_options(locale, " f24 ");
            assert_eq!(f24.len(), 1);
            assert_eq!(
                output_from_choice(&f24[0].value),
                Some(MappingOutput::Keyboard {
                    usage: 0x73,
                    modifiers: 0
                })
            );
            assert!(output_options(locale, "nonexistent key").is_empty());
        }
        let side2 = output_options("zh-CN", "侧键 2");
        assert_eq!(side2.len(), 1);
        assert_eq!(
            output_from_choice(&side2[0].value),
            Some(MappingOutput::Mouse {
                button: MouseButton::Side2
            })
        );
        for invalid in [
            "key/0", "key/255", "key/no", "mouse/5", "mouse/-1", "other/1",
        ] {
            assert_eq!(output_from_choice(invalid), None);
        }
    }
}
