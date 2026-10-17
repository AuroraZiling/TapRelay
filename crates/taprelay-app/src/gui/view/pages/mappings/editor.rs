use crate::gui::presentation::MappingOutputOption;
use crate::gui::view::components::ShortcutButton;
use crate::gui::view::{AppView, Command};
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Selectable, StyledExt,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    group_box::{GroupBox, GroupBoxVariants},
    input::Input,
    menu::{DropdownMenu, PopupMenuItem},
    searchable_list::{SearchableListItem, SearchableVec},
    select::Select,
};
use gpui_kit::{prelude::*, *};

impl MappingOutputOption {
    fn element(&self) -> Div {
        div()
            .h_flex()
            .min_w_0()
            .gap_2()
            .child(
                Icon::new(if self.is_mouse {
                    IconName::Mouse
                } else {
                    IconName::Keyboard
                })
                .size_4(),
            )
            .child(div().min_w_0().truncate().child(self.label.clone()))
    }
}

impl SearchableListItem for MappingOutputOption {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }
    fn value(&self) -> &String {
        &self.value
    }
    fn matches(&self, query: &str) -> bool {
        format!("{} {}", self.category, self.label)
            .to_lowercase()
            .contains(&query.trim().to_lowercase())
    }
    fn display_title(&self) -> Option<AnyElement> {
        Some(self.element().into_any_element())
    }
    fn render(&self, _: &mut Window, _: &mut App) -> impl IntoElement {
        self.element()
    }
}

impl AppView {
    pub(in crate::gui::view) fn sync_output_choice(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let items = &self.ui.mapping.output_options;
        let value = &self.ui.mapping.selected_output.value;
        let items_changed = self.output_choice_items != *items;
        if !items_changed
            && self.output_choice.read(cx).selected_value() == (!value.is_empty()).then_some(value)
        {
            return;
        }
        self.output_choice.update(cx, |state, cx| {
            if items_changed {
                state.set_items(SearchableVec::new(items.clone()), window, cx);
            }
            state.set_selected_value(value, window, cx);
        });
        if items_changed {
            self.output_choice_items = items.clone();
        }
    }

    pub(super) fn mapping_editor(&self, cx: &Context<Self>) -> AnyElement {
        let mapping = &self.ui.mapping;
        let busy = mapping.busy || mapping.saving || self.controller.recording();
        let field = |key: &str| div().v_flex().gap_3().child(self.text(self.tr(key)));
        let mut input = div().h_flex().gap_2().flex_wrap();
        for (slot, text) in mapping.inputs.iter().enumerate() {
            let recording = mapping.capture_slot == slot as i32;
            input = input.child(
                ShortcutButton::new(
                    self.button(
                        ("mapping-input", slot),
                        if recording {
                            mapping.capture_text.clone()
                        } else {
                            text.clone()
                        },
                        Command::Mapping("capture-input", slot.to_string()),
                        busy,
                        cx,
                    )
                    .accessibility_label(self.tr_object("a11y.record_shortcut", text))
                    .selected(recording),
                )
                .clear_button(self.plain_icon_button(
                    if recording {
                        ElementId::from("mapping-cancel-capture")
                    } else {
                        ("mapping-remove-input", slot).into()
                    },
                    IconName::X,
                    if recording {
                        self.tr("capture.cancel_shortcut")
                    } else {
                        self.tr_object("a11y.remove_shortcut", text)
                    },
                    Command::Mapping(
                        if recording {
                            "cancel-capture"
                        } else {
                            "remove-input"
                        },
                        slot.to_string(),
                    ),
                    busy && !recording,
                    cx,
                )),
            );
        }
        if mapping.inputs.len() < 2 {
            let recording = mapping.capture_slot == mapping.inputs.len() as i32;
            let mut chip = ShortcutButton::new(
                self.button(
                    "mapping-add-input",
                    if recording {
                        mapping.capture_text.clone()
                    } else {
                        self.tr("shortcuts.unset")
                    },
                    Command::Mapping("capture-input", mapping.inputs.len().to_string()),
                    busy,
                    cx,
                )
                .icon(IconName::Plus)
                .accessibility_label(self.tr("shortcuts.add"))
                .selected(recording),
            );
            if recording {
                chip = chip.clear_button(self.plain_icon_button(
                    "mapping-cancel-capture",
                    IconName::X,
                    self.tr("capture.cancel_shortcut"),
                    Command::Mapping("cancel-capture", String::new()),
                    false,
                    cx,
                ));
            }
            input = input.child(chip);
        }
        let mut output = field("mappings.output");
        let mut modes = div()
            .h_flex()
            .gap_1()
            .p_1()
            .rounded_md()
            .bg(cx.theme().group_box);
        for (mode, key) in [(0, "mappings.entry_record"), (1, "mappings.entry_manual")] {
            modes = modes.child(
                Button::new(("entry-mode", mode as usize))
                    .accessibility_label(self.tr(key))
                    .child(self.button_text(self.tr(key)))
                    .flex_1()
                    .selected(mapping.entry_mode == mode)
                    .toggled(mapping.entry_mode == mode)
                    .disabled(busy)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.ui.mapping.entry_mode = mode;
                        cx.notify();
                    })),
            );
        }
        let mut preview = Vec::new();
        for (label, enabled) in [
            ("Ctrl", mapping.ctrl),
            ("Shift", mapping.shift),
            ("Alt", mapping.alt),
            ("Win", mapping.meta),
        ] {
            if enabled && !mapping.selected_output.is_mouse {
                preview.push(label.to_owned());
            }
        }
        preview.push(if mapping.selected_output.value.is_empty() {
            self.tr("mappings.select_output")
        } else {
            mapping.selected_output.label.clone()
        });
        output = output
            .child(self.reveal_focus("output-mode-focus", modes))
            .child(
                div()
                    .id("mapping-output-preview")
                    .child(
                        GroupBox::new()
                            .fill()
                            .content_style(self.card_style(cx).h_flex().min_h_16())
                            .child(
                                self.text(if mapping.capture_slot == 2 {
                                    mapping.capture_text.clone()
                                } else {
                                    preview.join(" + ")
                                })
                                .id("mapping-output-preview-text")
                                .role(Role::Status)
                                .test_support(),
                            ),
                    )
                    .test_support(),
            );
        let output_control = if mapping.entry_mode == 0 {
            self.reveal_focus(
                "output-record-focus",
                self.button(
                    "mapping-record-output",
                    if mapping.capture_slot == 2 {
                        self.tr("capture.cancel_shortcut")
                    } else {
                        self.tr("mappings.record_output")
                    },
                    Command::Mapping(
                        if mapping.capture_slot == 2 {
                            "cancel-capture"
                        } else {
                            "capture-output"
                        },
                        String::new(),
                    ),
                    busy && mapping.capture_slot != 2,
                    cx,
                )
                .w_full()
                .selected(mapping.capture_slot == 2),
            )
            .into_any_element()
        } else {
            let empty = self.tr("mappings.no_output_matches");
            self.reveal_focus(
                "output-choice-focus",
                div()
                    .id("mapping-output-choice")
                    .role(Role::Group)
                    .aria_label(self.tr_object("a11y.output", &mapping.selected_output.label))
                    .child(
                        Select::new(&self.output_choice)
                            .icon(IconName::Search)
                            .accessibility_label(
                                self.tr_object("a11y.output", &mapping.selected_output.label),
                            )
                            .w_full()
                            .placeholder(self.tr("mappings.select_output"))
                            .search_placeholder(self.tr("mappings.search_output"))
                            .menu_width(rems(20.))
                            .disabled(busy)
                            .empty(move |_, _| div().p_3().text_sm().child(empty.clone())),
                    )
                    .test_support(),
            )
            .into_any_element()
        };
        let mut modifiers = div().h_flex().gap_2().flex_shrink_0();
        for (index, label, checked) in [
            (0, "Ctrl", mapping.ctrl),
            (1, "Shift", mapping.shift),
            (2, "Alt", mapping.alt),
            (3, "Win", mapping.meta),
        ] {
            modifiers = modifiers.child(
                Checkbox::new(("output-modifier", index as usize))
                    .gap_1()
                    .text_size(rems(0.875))
                    .label(label)
                    .checked(checked)
                    .disabled(busy)
                    .on_change(cx.listener(move |this, checked: &bool, _, cx| {
                        match index {
                            0 => this.ui.mapping.ctrl = *checked,
                            1 => this.ui.mapping.shift = *checked,
                            2 => this.ui.mapping.alt = *checked,
                            _ => this.ui.mapping.meta = *checked,
                        }
                        cx.notify();
                    })),
            );
        }
        output = output.child(
            self.reveal_focus(
                "output-controls-focus",
                div()
                    .id("mapping-output-controls")
                    .h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .max_w(rems(12.))
                            .child(output_control),
                    )
                    .when(!mapping.selected_output.is_mouse, |row| {
                        row.child(modifiers)
                    })
                    .test_support(),
            ),
        );
        if mapping.entry_mode == 1 {
            output = output.child(
                self.caption(self.tr("mappings.manual_output_help"), cx)
                    .id("mapping-output-help")
                    .test_support(),
            );
        }
        let options = mapping.scopes.clone();
        let weak = cx.entity().downgrade();
        let scopes = Button::new("mapping-scope-choice")
            .child(self.button_text(mapping.scope_label.clone()))
            .accessibility_label(self.tr_object("a11y.scope", &mapping.scope_label))
            .w_full()
            .disabled(busy)
            .dropdown_menu(move |mut menu, _, _| {
                for option in &options {
                    let weak = weak.clone();
                    let id = option.id.clone();
                    menu = menu.item(
                        PopupMenuItem::new(option.name.clone())
                            .checked(option.checked)
                            .on_click(move |_, window, cx| {
                                let _ = weak.update(cx, |this, cx| {
                                    this.dispatch(Command::Mapping("scope", id.clone()), window, cx)
                                });
                            }),
                    );
                }
                menu
            });
        let left = div()
            .id("mapping-input-column")
            .v_flex()
            .flex_1()
            .min_w_0()
            .gap_6()
            .child(
                self.reveal_focus(
                    "mapping-name-focus",
                    field("common.description").child(
                        Input::new(&self.mapping_name)
                            .id("mapping-name")
                            .aria_label(self.tr("common.description"))
                            .disabled(busy),
                    ),
                ),
            )
            .child(self.reveal_focus(
                "mapping-scope-focus",
                field("common.effective_scope").child(scopes),
            ))
            .child(
                self.reveal_focus(
                    "mapping-input-focus",
                    field("mappings.input")
                        .child(input)
                        .when(!mapping.input_error.is_empty(), |section| {
                            section.child(self.error(&mapping.input_error, cx))
                        }),
                ),
            );
        let body = div().v_flex().gap_4().child(
            div()
                .h_flex()
                .items_start()
                .gap_6()
                .child(left.test_support())
                .child(
                    div()
                        .id("mapping-output-column")
                        .flex_1()
                        .min_w_0()
                        .child(output)
                        .test_support(),
                ),
        );
        let mut feedback = div().v_flex().flex_1().min_w_0().gap_2();
        for (id, error) in [
            ("mapping-capture-error", &mapping.capture_error),
            ("mapping-form-error", &mapping.error),
        ] {
            if !error.is_empty() {
                feedback = feedback.child(
                    self.error(error, cx)
                        .id(id)
                        .when(id == "mapping-capture-error", |error| {
                            error.text_xs().line_height(relative(1.25))
                        })
                        .test_support(),
                );
            }
        }
        if !mapping.conflict_target.is_empty() {
            feedback = feedback.child(div().child(self.button(
                "mapping-conflict",
                self.tr("mappings.edit_conflict"),
                Command::Mapping("conflict", mapping.conflict_target.clone()),
                busy,
                cx,
            )));
        }
        div()
            .v_flex()
            .flex_1()
            .min_h_0()
            .gap_4()
            .child(self.scroller("mapping-editor", body))
            .child(
                div()
                    .h_flex()
                    .justify_end()
                    .items_end()
                    .gap_3()
                    .flex_shrink_0()
                    .child(feedback)
                    .child(
                        self.button(
                            "mapping-save",
                            self.tr(if mapping.saving {
                                "common.saving"
                            } else {
                                "common.save"
                            }),
                            Command::Mapping("save", String::new()),
                            busy,
                            cx,
                        )
                        .primary()
                        .icon(
                            Icon::new(IconName::Save)
                                .transform(Transformation::scale(size(0.875, 0.875))),
                        ),
                    ),
            )
            .into_any_element()
    }
}
