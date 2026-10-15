use crate::gui::view::{AppView, Command};
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{
    ActiveTheme, Disableable, Selectable, StyledExt,
    button::Button,
    checkbox::Checkbox,
    group_box::{GroupBox, GroupBoxVariants},
    input::Input,
    menu::{DropdownMenu, PopupMenuItem},
};
use gpui_kit::{prelude::*, *};

impl AppView {
    pub(super) fn mapping_editor(&self, cx: &Context<Self>) -> AnyElement {
        let mapping = &self.ui.mapping;
        let busy = mapping.busy || mapping.saving || self.controller.recording();
        let field = |key: &str| div().v_flex().gap_3().child(self.text(self.tr(key)));
        let mut input = div().h_flex().gap_2().flex_wrap();
        for (slot, text) in mapping.inputs.iter().enumerate() {
            let recording = mapping.capture_slot == slot as i32;
            input = input.child(
                div()
                    .h_flex()
                    .rounded_md()
                    .border_1()
                    .border_color(cx.theme().border)
                    .child(
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
                    .child(self.icon_button(
                        ("mapping-remove-input", slot),
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
            input = input.child(
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
                    .label(self.tr(key))
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
            if enabled {
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
                GroupBox::new()
                    .id("mapping-output-preview")
                    .fill()
                    .content_style(StyleRefinement::default().min_h_16())
                    .child(
                        self.text(if mapping.capture_slot == 2 {
                            mapping.capture_text.clone()
                        } else {
                            preview.join(" + ")
                        })
                        .role(Role::Status),
                    ),
            );
        if mapping.entry_mode == 0 {
            output = output
                .child(
                    self.reveal_focus(
                        "output-record-focus",
                        div().h_flex().gap_2().child(
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
                            .selected(mapping.capture_slot == 2),
                        ),
                    ),
                )
                .child(self.caption(self.tr("mappings.mouse_output_help"), cx));
        } else {
            output = output
                .child(self.caption(self.tr("mappings.manual_output_help"), cx))
                .child(self.text(self.tr("mappings.search_output")))
                .child(
                    self.reveal_focus(
                        "output-filter-focus",
                        Input::new(&self.output_filter)
                            .id("mapping-output-filter")
                            .aria_label(self.tr("mappings.search_output"))
                            .disabled(busy),
                    ),
                );
            let options = mapping.output_options.clone();
            let weak = cx.entity().downgrade();
            output = output.child(
                self.reveal_focus(
                    "output-choice-focus",
                    div().child(
                        Button::new("mapping-output-choice")
                            .label(if mapping.selected_output.value.is_empty() {
                                self.tr("mappings.select_output")
                            } else {
                                mapping.selected_output.label.clone()
                            })
                            .accessibility_label(
                                self.tr_object("a11y.output", &mapping.selected_output.label),
                            )
                            .disabled(busy)
                            .dropdown_menu(move |mut menu, _, _| {
                                for option in &options {
                                    let weak = weak.clone();
                                    let value = option.value.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(option.label.clone()).on_click(
                                            move |_, window, cx| {
                                                let _ = weak.update(cx, |this, cx| {
                                                    this.dispatch(
                                                        Command::Mapping(
                                                            "select-output",
                                                            value.clone(),
                                                        ),
                                                        window,
                                                        cx,
                                                    )
                                                });
                                            },
                                        ),
                                    );
                                }
                                menu
                            }),
                    ),
                ),
            );
            if mapping.output_options.is_empty() {
                output = output.child(self.caption(self.tr("mappings.no_output_matches"), cx));
            }
        }
        let mut modifiers = div().h_flex().gap_3().flex_wrap();
        for (index, label, checked) in [
            (0, "Ctrl", mapping.ctrl),
            (1, "Shift", mapping.shift),
            (2, "Alt", mapping.alt),
            (3, "Win", mapping.meta),
        ] {
            modifiers = modifiers.child(
                Checkbox::new(("output-modifier", index as usize))
                    .label(label)
                    .checked(checked)
                    .disabled(busy || mapping.selected_output.is_mouse)
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
        output = output
            .child(self.reveal_focus("output-modifiers-focus", modifiers))
            .child(self.caption(self.tr("mappings.output_help"), cx));
        let options = mapping.scopes.clone();
        let weak = cx.entity().downgrade();
        let scopes = Button::new("mapping-scope-choice")
            .label(mapping.scope_label.clone())
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
        let mut body = div().v_flex().gap_4().child(
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
        if self.controller.recording() && mapping.capture_slot < 2 {
            body = body.child(div().child(self.button(
                "mapping-cancel-capture",
                self.tr("capture.cancel_shortcut"),
                Command::Mapping("cancel-capture", String::new()),
                false,
                cx,
            )));
        }
        let mut feedback = div().v_flex().flex_1().min_w_0().gap_2();
        for (id, error) in [
            ("mapping-capture-error", &mapping.capture_error),
            ("mapping-form-error", &mapping.error),
        ] {
            if !error.is_empty() {
                feedback = feedback.child(self.error(error, cx).id(id).test_support());
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
                    .child(self.button(
                        "mapping-save",
                        self.tr(if mapping.saving {
                            "common.saving"
                        } else {
                            "mappings.save"
                        }),
                        Command::Mapping("save", String::new()),
                        busy,
                        cx,
                    )),
            )
            .into_any_element()
    }
}
