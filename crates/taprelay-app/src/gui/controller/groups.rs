use super::*;
use crate::gui::presentation::{ForegroundAppGroupRow, ForegroundAppRow};
use taprelay_core::foreground_app::{ForegroundAppRules, executable_identity};
mod draft;
mod icons;
pub(in crate::gui) use draft::GroupDraft;
pub(in crate::gui) use icons::IconCache;

impl Controller {
    pub(in crate::gui) fn save_foreground_app_rules(
        &mut self,
        ui: &mut Presentation,
        rules: ForegroundAppRules,
    ) -> Result<()> {
        self.validate_foreground_app_rules(&rules)?;
        self.runtime.set_foreground_app_rules(rules, &self.path)?;
        ui.groups.error = "".into();
        Ok(())
    }

    pub(in crate::gui) fn save_function_scope(
        &mut self,
        ui: &mut Presentation,
        id: FunctionId,
        groups: std::collections::BTreeSet<String>,
    ) -> Result<()> {
        let mut rules = self.runtime.config.foreground_app_rules.clone();
        if groups.is_empty() {
            rules.assignments.remove(&id);
        } else {
            rules.assignments.insert(id, groups.clone());
        }
        self.validate_foreground_app_rules(&rules)?;
        self.runtime.set_function_scope(id, groups, &self.path)?;
        ui.groups.error = "".into();
        Ok(())
    }

    fn validate_foreground_app_rules(&self, rules: &ForegroundAppRules) -> Result<()> {
        anyhow::ensure!(rules.valid(), self.tr(keys::GROUPS_INVALID));
        if let Some((first, second)) = rules.conflict(&self.runtime.config.functions) {
            bail!(
                "{}: {} / {}",
                self.tr(keys::GROUPS_CONFLICT),
                self.tr(function::function_definition(first).name_key),
                self.tr(function::function_definition(second).name_key)
            );
        }
        let mut mappings = self.runtime.config.custom_mappings.clone();
        for mapping in &mut mappings {
            mapping.groups.retain(|id| rules.groups.contains_key(id));
        }
        if let Some((first, second)) =
            taprelay_core::mapping::conflict(&self.runtime.config.functions, rules, &mappings)
        {
            let name = |id| match id {
                taprelay_core::mapping::RuleId::Function(id) => {
                    self.tr(function::function_definition(id).name_key)
                }
                taprelay_core::mapping::RuleId::Mapping(id) => mappings
                    .iter()
                    .find(|mapping| mapping.id == id)
                    .map_or_else(String::new, |mapping| mapping.name.clone()),
            };
            bail!(
                "{}: {} / {}",
                self.tr(keys::GROUPS_CONFLICT),
                name(first),
                name(second)
            );
        }
        Ok(())
    }

    pub(in crate::gui) fn group_command(
        &mut self,
        ui: &mut Presentation,
        command: &str,
        value: &str,
    ) -> Result<()> {
        if command == "cancel" {
            if self.pending_group_rules.is_none() {
                self.close_group_view(ui);
            }
            return Ok(());
        }
        anyhow::ensure!(
            !self.runtime.edits_blocked() && !self.recording(),
            self.tr(keys::RUNTIME_BUSY)
        );
        let rules = &self.runtime.config.foreground_app_rules;
        match command {
            "create" | "edit" => {
                self.foreground_app_icons.borrow_mut().refresh();
                let draft = if command == "create" {
                    GroupDraft::new(rules, &self.tr(keys::GROUPS_DEFAULT_NAME))
                } else {
                    GroupDraft::edit(rules, value).context(self.tr(keys::GROUPS_INVALID))?
                };
                ui.groups.selected_id = draft.id.clone().unwrap_or_default();
                ui.groups.selected_name = draft.group.name.clone();
                self.group_draft = Some(draft);
                self.sync_group_draft(ui);
                self.open_group_view(ui, false);
                if command == "create" {
                    ui.groups.running = Vec::new();
                    ui.groups.choosing_running = true;
                    self.group_command(ui, "running", "")?;
                }
            }
            "save" => {
                let name = ui.groups.selected_name.clone();
                anyhow::ensure!(!name.trim().is_empty(), self.tr(keys::GROUPS_NAME_REQUIRED));
                let draft = self
                    .group_draft
                    .as_ref()
                    .context(self.tr(keys::GROUPS_INVALID))?;
                let candidate = draft.candidate(rules, &name);
                self.save_foreground_app_rules(ui, candidate.clone())?;
                self.pending_group_rules = Some(candidate);
                ui.groups.saving = true;
                ui.groups.form_error = "".into();
            }
            "request-delete" => {
                anyhow::ensure!(
                    rules.groups.contains_key(value),
                    self.tr(keys::GROUPS_INVALID)
                );
                let mut names = rules
                    .assignments
                    .iter()
                    .filter(|(_, groups)| groups.contains(value))
                    .map(|(id, _)| self.tr(function::function_definition(*id).name_key))
                    .collect::<Vec<_>>();
                names.extend(
                    self.runtime
                        .config
                        .custom_mappings
                        .iter()
                        .filter(|mapping| mapping.groups.contains(value))
                        .map(|mapping| mapping.name.clone()),
                );
                ui.groups.selected_id = value.into();
                ui.groups.selected_name = rules.groups[value].name.clone();
                ui.groups.deletion_warning = if names.is_empty() {
                    self.tr(keys::GROUPS_DELETE_UNUSED)
                } else {
                    format!(
                        "{}\n{}",
                        self.tr(keys::GROUPS_DELETE_WARNING),
                        names.join(" / ")
                    )
                };
                self.group_draft = None;
                self.open_group_view(ui, true);
            }
            "delete" => {
                anyhow::ensure!(ui.groups.confirming_delete, "Delete confirmation required");
                let mut candidate = rules.clone();
                candidate.remove_group(&ui.groups.selected_id.clone());
                self.save_foreground_app_rules(ui, candidate.clone())?;
                self.pending_group_rules = Some(candidate);
                ui.groups.saving = true;
                ui.groups.form_error = "".into();
            }
            "running" => {
                let draft = self
                    .group_draft
                    .as_ref()
                    .context(self.tr(keys::GROUPS_INVALID))?;
                let foreground_apps =
                    self.desktop
                        .running_apps()?
                        .into_iter()
                        .filter(|path| {
                            !draft.group.foreground_apps.iter().any(|other| {
                                executable_identity(other) == executable_identity(path)
                            })
                        })
                        .map(foreground_app_row)
                        .collect::<Vec<_>>();
                ui.groups.running = foreground_apps;
                ui.groups.choosing_running = true;
            }
            "show-selected" => ui.groups.choosing_running = false,
            "picked" | "add" => {
                #[cfg(target_os = "macos")]
                let path = crate::platform::foreground_apps::identity_for_selection(value)?;
                #[cfg(windows)]
                let path = value.to_string();
                anyhow::ensure!(
                    taprelay_core::foreground_app::valid_executable(&path),
                    self.tr(keys::GROUPS_INVALID)
                );
                let error = self.tr(keys::GROUPS_INVALID);
                self.group_draft.as_mut().context(error)?.add(path.clone());
                self.sync_group_draft(ui);
                // Keep choosing open to support adding several running applications.
                if command == "picked" {
                    ui.groups.choosing_running = false;
                    ui.groups.form_error.clear();
                }
                let remaining = ui
                    .groups
                    .running
                    .clone()
                    .iter()
                    .filter(|app| executable_identity(&app.path) != executable_identity(&path))
                    .cloned()
                    .collect::<Vec<_>>();
                ui.groups.running = remaining;
            }
            "remove" => {
                let error = self.tr(keys::GROUPS_INVALID);
                self.group_draft
                    .as_mut()
                    .context(error)?
                    .group
                    .foreground_apps
                    .retain(|path| path != value);
                self.sync_group_draft(ui);
            }
            _ => bail!("Unknown application group command"),
        }
        Ok(())
    }

    fn open_group_view(&self, ui: &mut Presentation, deleting: bool) {
        ui.groups.error = "".into();
        ui.groups.form_error = "".into();
        ui.groups.confirming_delete = deleting;
        ui.groups.choosing_running = false;
        ui.groups.saving = false;
        if deleting {
            ui.groups.confirming_delete = true;
        } else {
            {
                ui.page = 4;
                ui.groups.editor_open = true;
                ui.groups.focus_request += 1;
            };
        }
    }

    fn close_group_view(&mut self, ui: &mut Presentation) {
        if ui.groups.confirming_delete {
            ui.groups.confirming_delete = false;
        }
        ui.groups.editor_open = false;
        self.group_draft = None;

        ui.groups.saving = false;
        ui.groups.confirming_delete = false;
        ui.groups.form_error = "".into();
        ui.groups.foreground_apps = Vec::new();
        ui.groups.running = Vec::new();
    }

    pub(in crate::gui) fn sync_group_operation(&mut self, ui: &mut Presentation) {
        if !self.runtime.busy()
            && let Some(expected) = self.pending_group_rules.take()
        {
            if self.runtime.take_foreground_app_rules_saved() == Some(true)
                && self.runtime.config.foreground_app_rules == expected
            {
                self.close_group_view(ui);
            } else {
                ui.groups.saving = false;
                ui.groups.form_error = self.tr(keys::COMMON_SAVE_FAILED);
            }
        }
    }

    fn sync_group_draft(&self, ui: &mut Presentation) {
        let foreground_apps = self
            .group_draft
            .as_ref()
            .map(|draft| {
                draft
                    .group
                    .foreground_apps
                    .iter()
                    .cloned()
                    .map(foreground_app_row)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        ui.groups.foreground_apps = foreground_apps;
    }

    pub(in crate::gui) fn sync_foreground_app_groups(&self, ui: &mut Presentation) {
        self.sync_scope_choices(ui);
        let rows = self
            .runtime
            .config
            .foreground_app_rules
            .groups
            .iter()
            .map(|(id, group)| ForegroundAppGroupRow {
                id: id.clone(),
                name: group.name.clone(),
                count: group.foreground_apps.len() as i32,
                count_label: rust_i18n::t!(
                    keys::GROUPS_COUNT,
                    locale = self.locale(),
                    count = group.foreground_apps.len()
                )
                .into_owned(),
                summary: group
                    .foreground_apps
                    .iter()
                    .map(|path| path.rsplit(['\\', '/']).next().unwrap_or(path))
                    .collect::<Vec<_>>()
                    .join(" · "),
            })
            .collect::<Vec<_>>();
        ui.groups.groups = rows;
    }

    fn sync_scope_choices(&self, ui: &mut Presentation) {
        let rules = &self.runtime.config.foreground_app_rules;
        let selected = FunctionId::from_stable_id(&ui.groups.scope_function.clone())
            .and_then(|id| rules.assignments.get(&id));
        let mut options = vec![crate::gui::presentation::ForegroundAppScopeOption {
            id: "".into(),
            name: self.tr(keys::COMMON_ALL_APPS),
            checked: selected.is_none(),
        }];
        options.extend(rules.groups.iter().map(|(id, group)| {
            crate::gui::presentation::ForegroundAppScopeOption {
                id: id.clone(),
                name: group.name.clone(),
                checked: selected.is_some_and(|groups| groups.contains(id)),
            }
        }));
        ui.groups.scope_options = options;
    }
}

fn foreground_app_row(path: String) -> ForegroundAppRow {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(&path).to_string();
    ForegroundAppRow { path, name }
}
