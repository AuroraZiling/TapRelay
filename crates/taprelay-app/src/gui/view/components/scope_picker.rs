use crate::gui::view::{AppView, Command};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{
    Sizable, StyledExt,
    combobox::{Combobox, ComboboxEvent, ComboboxState},
    searchable_list::SearchableListItem,
    tag::Tag,
};
use gpui_kit::{prelude::*, *};
use std::collections::BTreeSet;

#[derive(Clone, PartialEq)]
pub(in crate::gui::view) struct ScopeOption {
    id: String,
    name: String,
}

impl SearchableListItem for ScopeOption {
    type Value = String;

    fn title(&self) -> SharedString {
        self.name.clone().into()
    }

    fn value(&self) -> &String {
        &self.id
    }
}

pub(in crate::gui::view) struct ScopePicker {
    pub(in crate::gui::view) state: Entity<ComboboxState<Vec<ScopeOption>>>,
    items: Vec<ScopeOption>,
    selection: Vec<String>,
}

impl ScopePicker {
    pub(in crate::gui::view) fn element(
        &self,
        visible_limit: usize,
        locale: String,
        all_apps: String,
        disabled: bool,
    ) -> impl IntoElement {
        Combobox::new(&self.state)
            .placeholder(all_apps.clone())
            .w_full()
            .disabled(disabled)
            .render_trigger(move |trigger, _, _| {
                let selection = trigger.selection();
                let mut tags = div().h_flex().w_full().min_w_0().gap_1().overflow_hidden();
                if selection.is_empty() || selection[0].1.id.is_empty() {
                    return tags.child(div().truncate().child(all_apps.clone()));
                }
                for (index, (_, item)) in selection.iter().take(visible_limit).enumerate() {
                    tags = tags.child(
                        div()
                            .id(("scope-tag", index))
                            .min_w_0()
                            .max_w_full()
                            .role(Role::Label)
                            .aria_label(item.name.clone())
                            .child(
                                Tag::new()
                                    .large()
                                    .min_w_0()
                                    .max_w_full()
                                    .child(div().min_w_0().truncate().child(item.title())),
                            )
                            .test_support(),
                    );
                }
                let remaining = selection.len().saturating_sub(visible_limit);
                if remaining > 0 {
                    let label = rust_i18n::t!(
                        "bindings.more_scopes",
                        locale = locale.as_str(),
                        count = remaining
                    )
                    .into_owned();
                    tags = tags.child(
                        div()
                            .id("scope-more")
                            .flex_shrink_0()
                            .role(Role::Label)
                            .aria_label(label.clone())
                            .child(Tag::new().large().child(label))
                            .test_support(),
                    );
                }
                tags
            })
    }

    fn sync(
        &mut self,
        items: &[ScopeOption],
        groups: &BTreeSet<String>,
        window: &mut Window,
        cx: &mut App,
    ) {
        let selection = if groups.is_empty() {
            vec![String::new()]
        } else {
            groups.iter().cloned().collect()
        };
        let items_changed = self.items != items;
        let selected = self.state.read(cx).selected_values();
        if !items_changed && selected.iter().collect::<BTreeSet<_>>() == selection.iter().collect()
        {
            self.selection = selection;
            return;
        }
        self.state.update(cx, |state, cx| {
            if items_changed {
                state.set_items(items.to_vec(), window, cx);
            }
            state.set_selected_values(&selection, window, cx);
        });
        if items_changed {
            self.items = items.to_vec();
        }
        self.selection = selection;
    }
}

impl AppView {
    pub(in crate::gui::view) fn sync_scope_pickers(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let rules = &self.controller.runtime.config.foreground_app_rules;
        let mut items = vec![ScopeOption {
            id: String::new(),
            name: self.tr("common.all_apps"),
        }];
        items.extend(rules.groups.iter().map(|(id, group)| ScopeOption {
            id: id.clone(),
            name: group.name.clone(),
        }));
        let empty = BTreeSet::new();
        for row in self.ui.binding.rows.iter().filter(|row| row.scope_editable) {
            let Some(id) = taprelay_core::function::FunctionId::from_stable_id(&row.id) else {
                continue;
            };
            let picker = self.scope_pickers.entry(id).or_insert_with(|| {
                let state = cx.new(|cx| {
                    ComboboxState::new(items.clone(), Vec::new(), window, cx).multiple(true)
                });
                self._subscriptions.push(cx.subscribe_in(
                    &state,
                    window,
                    move |this, _, event, window, cx| {
                        let ComboboxEvent::Change(values) = event else {
                            return;
                        };
                        let picker = this
                            .scope_pickers
                            .get_mut(&id)
                            .expect("retained scope picker");
                        let groups = if this.ui.groups.busy || this.controller.recording() {
                            picker
                                .selection
                                .iter()
                                .filter(|id| !id.is_empty())
                                .cloned()
                                .collect()
                        } else if values.iter().any(String::is_empty)
                            && !picker.selection.iter().any(String::is_empty)
                        {
                            BTreeSet::new()
                        } else {
                            values.iter().filter(|id| !id.is_empty()).cloned().collect()
                        };
                        picker.sync(&picker.items.clone(), &groups, window, cx);
                        let current = this
                            .controller
                            .runtime
                            .config
                            .foreground_app_rules
                            .assignments
                            .get(&id)
                            .cloned()
                            .unwrap_or_default();
                        if !this.ui.groups.busy && !this.controller.recording() && current != groups
                        {
                            this.dispatch(Command::Scope(id, groups), window, cx);
                        }
                    },
                ));
                ScopePicker {
                    state,
                    items: items.clone(),
                    selection: Vec::new(),
                }
            });
            if !self.ui.groups.busy || picker.selection.is_empty() {
                picker.sync(
                    &items,
                    rules.assignments.get(&id).unwrap_or(&empty),
                    window,
                    cx,
                );
            }
        }
    }
}
