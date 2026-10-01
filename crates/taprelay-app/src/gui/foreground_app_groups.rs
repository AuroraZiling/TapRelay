use super::*;
use crate::{ForegroundAppGroupRow, ForegroundAppRow, ForegroundAppUi};
use taprelay_core::foreground_app::{ForegroundAppRules, executable_identity};
mod draft;
mod icons;
pub(super) use draft::GroupDraft;
pub(super) use icons::IconCache;

pub(super) fn connect(controller: &Rc<RefCell<Controller>>, ui: &AppWindow) {
    let cache = controller.borrow().foreground_app_icons.clone();
    ui.global::<ForegroundAppUi>()
        .on_request_icon(move |path| cache.borrow_mut().request(&path));
    let cache = controller.borrow().foreground_app_icons.clone();
    ui.global::<ForegroundAppUi>()
        .on_app_icon(move |path, _| cache.borrow().image(&path).unwrap_or_default());
    let cache = controller.borrow().foreground_app_icons.clone();
    ui.global::<ForegroundAppUi>()
        .on_has_icon(move |path, _| cache.borrow().image(&path).is_some());
    let cache = controller.borrow().foreground_app_icons.clone();
    ui.global::<ForegroundAppUi>()
        .on_app_name(move |path, _| cache.borrow().name(&path).unwrap_or_default().into());
    let cache = controller.borrow().foreground_app_icons.clone();
    ui.global::<ForegroundAppUi>()
        .on_app_missing(move |path, _| cache.borrow().missing(&path));
    let c = controller.clone();
    let window = ui.as_weak();
    ui.global::<ForegroundAppUi>()
        .on_command(move |command, value| {
            dispatch_controller(&c, &window, |c, ui| {
                c.runtime.consume_ui_input();
                if let Err(error) = c.group_command(ui, &command, &value) {
                    let view = ui.global::<ForegroundAppUi>();
                    if view.get_editor_open() || view.get_confirming_delete() {
                        view.set_form_error(error.to_string().into());
                    } else {
                        view.set_error(error.to_string().into());
                    }
                }
                c.sync(ui);
                Ok(())
            });
        });
    let c = controller.clone();
    let window = ui.as_weak();
    ui.global::<ForegroundAppUi>()
        .on_open_scope(move |function| {
            dispatch_controller(&c, &window, |c, ui| {
                ui.global::<ForegroundAppUi>().set_scope_function(function);
                c.sync_scope_choices(ui);
                Ok(())
            });
        });
    let c = controller.clone();
    let window = ui.as_weak();
    ui.global::<ForegroundAppUi>()
        .on_set_scope(move |function, group| {
            dispatch_controller(&c, &window, |c, ui| {
                c.runtime.consume_ui_input();
                let result = (|| {
                    anyhow::ensure!(
                        !c.runtime.busy() && !c.recording(),
                        c.tr(keys::RUNTIME_BUSY)
                    );
                    let id = FunctionId::from_stable_id(&function).context("Unknown function")?;
                    let mut rules = c.runtime.config.foreground_app_rules.clone();
                    rules.toggle_scope(id, &group);
                    c.save_foreground_app_rules(ui, rules)
                })();
                if let Err(error) = result {
                    ui.global::<ForegroundAppUi>()
                        .set_error(error.to_string().into());
                }
                c.sync(ui);
                Ok(())
            });
        });
}

impl Controller {
    fn save_foreground_app_rules(
        &mut self,
        ui: &AppWindow,
        rules: ForegroundAppRules,
    ) -> Result<()> {
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
            taprelay_core::mapping::conflict(&self.runtime.config.functions, &rules, &mappings)
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
        self.runtime.set_foreground_app_rules(rules, &self.path)?;
        ui.global::<ForegroundAppUi>().set_error("".into());
        Ok(())
    }

    fn group_command(&mut self, ui: &AppWindow, command: &str, value: &str) -> Result<()> {
        let view = ui.global::<ForegroundAppUi>();
        if command == "cancel" {
            if self.pending_group_rules.is_none() {
                self.close_group_view(ui);
            }
            return Ok(());
        }
        anyhow::ensure!(
            !self.runtime.busy() && !self.recording(),
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
                view.set_selected_id(draft.id.clone().unwrap_or_default().into());
                view.set_selected_name(draft.group.name.clone().into());
                self.group_draft = Some(draft);
                self.sync_group_draft(ui);
                self.open_group_view(ui, false);
                if command == "create" {
                    view.set_running(ModelRc::default());
                    view.set_choosing_running(true);
                    self.group_command(ui, "running", "")?;
                }
            }
            "save" => {
                let name = view.get_selected_name();
                anyhow::ensure!(!name.trim().is_empty(), self.tr(keys::GROUPS_NAME_REQUIRED));
                let draft = self
                    .group_draft
                    .as_ref()
                    .context(self.tr(keys::GROUPS_INVALID))?;
                let candidate = draft.candidate(rules, &name);
                self.save_foreground_app_rules(ui, candidate.clone())?;
                self.pending_group_rules = Some(candidate);
                view.set_saving(true);
                view.set_form_error("".into());
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
                view.set_selected_id(value.into());
                view.set_selected_name(rules.groups[value].name.clone().into());
                view.set_deletion_warning(
                    if names.is_empty() {
                        self.tr(keys::GROUPS_DELETE_UNUSED)
                    } else {
                        format!(
                            "{}\n{}",
                            self.tr(keys::GROUPS_DELETE_WARNING),
                            names.join(" / ")
                        )
                    }
                    .into(),
                );
                self.group_draft = None;
                self.open_group_view(ui, true);
            }
            "delete" => {
                anyhow::ensure!(view.get_confirming_delete(), "Delete confirmation required");
                let mut candidate = rules.clone();
                candidate.remove_group(&view.get_selected_id());
                self.save_foreground_app_rules(ui, candidate.clone())?;
                self.pending_group_rules = Some(candidate);
                view.set_saving(true);
                view.set_form_error("".into());
            }
            "running" => {
                let draft = self
                    .group_draft
                    .as_ref()
                    .context(self.tr(keys::GROUPS_INVALID))?;
                let foreground_apps =
                    crate::platform::foreground_apps::running()?
                        .into_iter()
                        .filter(|path| {
                            !draft.group.foreground_apps.iter().any(|other| {
                                executable_identity(other) == executable_identity(path)
                            })
                        })
                        .map(foreground_app_row)
                        .collect::<Vec<_>>();
                view.set_running(ModelRc::new(VecModel::from(foreground_apps)));
                view.set_choosing_running(true);
            }
            "show-selected" => view.set_choosing_running(false),
            "browse" | "add" => {
                let path = if command == "browse" {
                    let Some(path) = crate::platform::foreground_apps::browse()? else {
                        return Ok(());
                    };
                    path
                } else {
                    value.to_string()
                };
                anyhow::ensure!(
                    taprelay_core::foreground_app::valid_executable(&path),
                    self.tr(keys::GROUPS_INVALID)
                );
                let error = self.tr(keys::GROUPS_INVALID);
                self.group_draft.as_mut().context(error)?.add(path.clone());
                self.sync_group_draft(ui);
                // Keep choosing open to support adding several running applications.
                if command == "browse" {
                    view.set_choosing_running(false);
                }
                use slint::Model;
                let remaining = view
                    .get_running()
                    .iter()
                    .filter(|app| executable_identity(&app.path) != executable_identity(&path))
                    .collect::<Vec<_>>();
                view.set_running(ModelRc::new(VecModel::from(remaining)));
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

    fn open_group_view(&self, ui: &AppWindow, deleting: bool) {
        let view = ui.global::<ForegroundAppUi>();
        view.set_error("".into());
        view.set_form_error("".into());
        view.set_confirming_delete(deleting);
        view.set_choosing_running(false);
        view.set_saving(false);
        if deleting {
            ui.invoke_show_group_delete();
        } else {
            ui.invoke_show_group_editor();
        }
    }

    fn close_group_view(&mut self, ui: &AppWindow) {
        if ui.global::<ForegroundAppUi>().get_confirming_delete() {
            ui.invoke_close_group_delete();
        }
        ui.invoke_close_group_editor();
        self.group_draft = None;
        let view = ui.global::<ForegroundAppUi>();

        view.set_saving(false);
        view.set_confirming_delete(false);
        view.set_form_error("".into());
        view.set_foreground_apps(ModelRc::default());
        view.set_running(ModelRc::default());
    }

    pub(super) fn sync_group_operation(&mut self, ui: &AppWindow) {
        if !self.runtime.busy()
            && let Some(expected) = self.pending_group_rules.take()
        {
            if self.runtime.take_foreground_app_rules_saved() == Some(true)
                && self.runtime.config.foreground_app_rules == expected
            {
                self.close_group_view(ui);
            } else {
                let view = ui.global::<ForegroundAppUi>();
                view.set_saving(false);
                view.set_form_error(self.tr(keys::COMMON_SAVE_FAILED).into());
            }
        }
    }

    fn sync_group_draft(&self, ui: &AppWindow) {
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
        ui.global::<ForegroundAppUi>()
            .set_foreground_apps(ModelRc::new(VecModel::from(foreground_apps)));
    }

    pub(super) fn sync_foreground_app_groups(&self, ui: &AppWindow) {
        self.sync_scope_choices(ui);
        let rows = self
            .runtime
            .config
            .foreground_app_rules
            .groups
            .iter()
            .map(|(id, group)| ForegroundAppGroupRow {
                id: id.clone().into(),
                name: group.name.clone().into(),
                count: group.foreground_apps.len() as i32,
                count_label: i18n::foreground_app_count(self.locale(), group.foreground_apps.len())
                    .into(),
                summary: group
                    .foreground_apps
                    .iter()
                    .map(|path| path.rsplit(['\\', '/']).next().unwrap_or(path))
                    .collect::<Vec<_>>()
                    .join(" · ")
                    .into(),
            })
            .collect::<Vec<_>>();
        ui.global::<ForegroundAppUi>()
            .set_groups(ModelRc::new(VecModel::from(rows)));
    }

    fn sync_scope_choices(&self, ui: &AppWindow) {
        let view = ui.global::<ForegroundAppUi>();
        let rules = &self.runtime.config.foreground_app_rules;
        let selected = FunctionId::from_stable_id(&view.get_scope_function())
            .and_then(|id| rules.assignments.get(&id));
        let mut options = vec![crate::ForegroundAppScopeOption {
            id: "".into(),
            name: self.tr(keys::COMMON_ALL_APPS).into(),
            checked: selected.is_none(),
        }];
        options.extend(
            rules
                .groups
                .iter()
                .map(|(id, group)| crate::ForegroundAppScopeOption {
                    id: id.clone().into(),
                    name: group.name.clone().into(),
                    checked: selected.is_some_and(|groups| groups.contains(id)),
                }),
        );
        view.set_scope_options(update_model(view.get_scope_options(), options));
    }
}

fn foreground_app_row(path: String) -> ForegroundAppRow {
    let name = path.rsplit(['\\', '/']).next().unwrap_or(&path).to_string();
    ForegroundAppRow {
        path: path.into(),
        name: name.into(),
    }
}
