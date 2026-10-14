use super::*;

impl AppView {
    pub(super) fn groups(&self, cx: &Context<Self>) -> AnyElement {
        let group = &self.ui.groups;
        if group.editor_open {
            return self.group_editor(cx);
        }
        let mut rows = div().v_flex().gap_2();
        for row in &group.groups {
            rows = rows.child(
                GroupBox::new()
                    .id(SharedString::from(format!("group-{}", row.id)))
                    .fill()
                    .content_style(StyleRefinement::default().h_flex())
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(rems(0.3125))
                            .child(div().text_ellipsis().child(row.name.clone()))
                            .child(
                                self.caption(
                                    if row.summary.is_empty() {
                                        self.tr("groups.list_empty")
                                    } else {
                                        row.summary.clone()
                                    },
                                    cx,
                                )
                                .text_ellipsis(),
                            ),
                    )
                    .child(self.caption(row.count_label.clone(), cx))
                    .child(self.icon_button(
                        SharedString::from(format!("group-edit-{}", row.id)),
                        IconName::Pencil,
                        self.tr("common.edit"),
                        Command::Group("edit", row.id.clone()),
                        group.busy,
                        cx,
                    ))
                    .child(self.icon_button(
                        SharedString::from(format!("group-delete-{}", row.id)),
                        IconName::Trash,
                        self.tr("common.delete"),
                        Command::Group("request-delete", row.id.clone()),
                        group.busy,
                        cx,
                    )),
            );
        }
        div()
            .v_flex()
            .flex_1()
            .min_h_0()
            .gap_4()
            .child(self.caption(self.tr("groups.help"), cx))
            .when(!group.error.is_empty(), |body| {
                body.child(self.error(&group.error, cx))
            })
            .child(self.scroller("groups", rows))
            .into_any_element()
    }

    fn group_editor(&self, cx: &Context<Self>) -> AnyElement {
        use gpui_kit::component::tab::{Tab, TabBar};
        let group = &self.ui.groups;
        let busy = group.busy || group.saving;
        let tabs = TabBar::new("group-tabs")
            .underline()
            .w_full()
            .last_empty_space(div())
            .selected_index(usize::from(group.choosing_running))
            .child(
                Tab::new()
                    .label(format!(
                        "{} ({})",
                        self.tr("groups.selected"),
                        group.foreground_apps.len()
                    ))
                    .flex_1()
                    .disabled(busy),
            )
            .child(
                Tab::new()
                    .label(self.tr("groups.running"))
                    .flex_1()
                    .disabled(busy),
            )
            .on_click(cx.listener(|this, index: &usize, window, cx| {
                this.dispatch(
                    Command::Group(
                        if *index == 0 {
                            "show-selected"
                        } else {
                            "running"
                        },
                        String::new(),
                    ),
                    window,
                    cx,
                )
            }));
        let mut body = div()
            .v_flex()
            .flex_1()
            .min_h_0()
            .gap_4()
            .child(
                div().h_flex().gap_4().child(self.tr("groups.name")).child(
                    div().flex_1().min_w_0().child(
                        Input::new(&self.group_name)
                            .id("group-name")
                            .aria_label(self.tr("groups.name"))
                            .disabled(busy),
                    ),
                ),
            )
            .child(tabs);
        if group.choosing_running {
            body = body.child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(self.caption(self.tr("groups.running_help"), cx).flex_1())
                    .child(self.icon_button(
                        "group-refresh",
                        IconName::RefreshCw,
                        self.tr("groups.refresh"),
                        Command::Group("running", String::new()),
                        busy,
                        cx,
                    ))
                    .child(self.button(
                        "group-browse",
                        self.tr("groups.browse"),
                        Command::Group("browse", String::new()),
                        busy,
                        cx,
                    )),
            );
        }
        let apps = if group.choosing_running {
            &group.running
        } else {
            &group.foreground_apps
        };
        let mut rows = div().v_flex().gap_2();
        if apps.is_empty() {
            rows = rows.child(self.caption(
                self.tr(if group.choosing_running {
                    "groups.running_empty"
                } else {
                    "groups.no_apps"
                }),
                cx,
            ));
        }
        for app in apps {
            let mut icons = self.controller.foreground_app_icons.borrow_mut();
            icons.request(&app.path);
            let icon = if let Some(image) = icons.image(&app.path) {
                img(image).size_8().into_any_element()
            } else {
                Icon::new(IconName::AppWindow).size_7().into_any_element()
            };
            let mut details = div()
                .v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    div()
                        .text_ellipsis()
                        .child(icons.name(&app.path).unwrap_or(&app.name).to_string()),
                )
                .child(self.caption(app.path.clone(), cx).text_ellipsis());
            if icons.missing(&app.path) {
                details = details.child(self.error(&self.tr("groups.missing"), cx));
            }
            rows = rows.child(
                GroupBox::new()
                    .id(SharedString::from(format!("group-app-row-{}", app.path)))
                    .fill()
                    .content_style(StyleRefinement::default().h_flex().gap_3())
                    .child(div().size_8().flex_shrink_0().child(icon))
                    .child(details)
                    .child(self.icon_button(
                        SharedString::from(format!("group-app-{}", app.path)),
                        if group.choosing_running {
                            IconName::Plus
                        } else {
                            IconName::X
                        },
                        self.tr(if group.choosing_running {
                            "common.add"
                        } else {
                            "common.remove"
                        }),
                        Command::Group(
                            if group.choosing_running {
                                "add"
                            } else {
                                "remove"
                            },
                            app.path.clone(),
                        ),
                        busy,
                        cx,
                    )),
            );
        }
        body.child(self.scroller("group-editor", rows))
            .child(
                div()
                    .h_flex()
                    .justify_end()
                    .gap_3()
                    .flex_shrink_0()
                    .child(div().flex_1().min_w_0().when(
                        !group.form_error.is_empty(),
                        |feedback| {
                            feedback.child(
                                self.error(&group.form_error, cx)
                                    .id("group-form-error")
                                    .test_support(),
                            )
                        },
                    ))
                    .child(self.button(
                        "group-save",
                        self.tr(if group.saving {
                            "common.saving"
                        } else {
                            "groups.save"
                        }),
                        Command::Group("save", String::new()),
                        busy,
                        cx,
                    )),
            )
            .into_any_element()
    }

    pub(super) fn mappings(&self, cx: &Context<Self>) -> AnyElement {
        let mapping = &self.ui.mapping;
        if mapping.editor {
            return self.mapping_editor(cx);
        }
        let mut body = div().v_flex().flex_1().min_h_0().gap_6().child(
            div()
                .h_flex()
                .gap_3()
                .child(self.caption(self.tr("mappings.help"), cx).flex_1())
                .when(mapping.can_undo, |row| {
                    row.child(self.button(
                        "mapping-undo",
                        self.tr("common.undo"),
                        Command::Mapping("undo", String::new()),
                        mapping.busy,
                        cx,
                    ))
                }),
        );
        if !mapping.error.is_empty() {
            body = body.child(self.error(&mapping.error, cx));
        }
        if mapping.rows.is_empty() {
            return body
                .child(
                    div()
                        .v_flex()
                        .flex_1()
                        .justify_center()
                        .items_center()
                        .child(self.caption(self.tr("mappings.empty"), cx)),
                )
                .into_any_element();
        }
        let mut rows = div().v_flex().gap_2().child(
            div()
                .h_flex()
                .h_7()
                .px_4()
                .gap_4()
                .child(
                    self.caption(self.tr("common.enabled"), cx)
                        .w_9()
                        .flex_shrink_0(),
                )
                .child(
                    self.caption(self.tr("common.description"), cx)
                        .flex_grow(2.)
                        .flex_basis(px(0.))
                        .min_w_0(),
                )
                .child(
                    self.caption(self.tr("common.mapping"), cx)
                        .flex_grow(3.)
                        .flex_basis(px(0.))
                        .min_w_0(),
                )
                .child(
                    self.caption(self.tr("common.effective_scope"), cx)
                        .w(rems(7.75))
                        .flex_shrink_0(),
                )
                .child(
                    self.caption(self.tr("common.actions"), cx)
                        .w_20()
                        .flex_shrink_0(),
                ),
        );
        for row in &mapping.rows {
            let id = row.id.clone();
            rows = rows.child(
                GroupBox::new()
                    .id(SharedString::from(format!("mapping-{}", row.id)))
                    .fill()
                    .content_style(StyleRefinement::default().h_flex().min_h_16())
                    .child(
                        div().w_9().flex_shrink_0().child(
                            Switch::new(SharedString::from(format!("mapping-toggle-{id}")))
                                .accessibility_label(row.name.clone())
                                .checked(row.enabled)
                                .disabled(mapping.busy)
                                .on_change(cx.listener(move |this, _, window, cx| {
                                    this.dispatch(
                                        Command::Mapping("toggle", id.clone()),
                                        window,
                                        cx,
                                    )
                                })),
                        ),
                    )
                    .child(
                        div()
                            .flex_grow(2.)
                            .flex_basis(px(0.))
                            .min_w_0()
                            .text_ellipsis()
                            .child(row.name.clone()),
                    )
                    .child(
                        self.caption(format!("{} → {}", row.input, row.output), cx)
                            .flex_grow(3.)
                            .flex_basis(px(0.))
                            .min_w_0(),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "mapping-scope-label-{}",
                                row.id
                            )))
                            .w(rems(7.75))
                            .flex_shrink_0()
                            .text_ellipsis()
                            .tooltip({
                                let text = row.scope.clone();
                                move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(text.clone())
                                        .build(window, cx)
                                }
                            })
                            .child(row.scope.clone()),
                    )
                    .child(
                        div()
                            .h_flex()
                            .w_20()
                            .gap_2()
                            .flex_shrink_0()
                            .child(self.icon_button(
                                SharedString::from(format!("mapping-edit-{}", row.id)),
                                IconName::Pencil,
                                self.tr("common.edit"),
                                Command::Mapping("edit", row.id.clone()),
                                mapping.busy,
                                cx,
                            ))
                            .child(self.icon_button(
                                SharedString::from(format!("mapping-delete-{}", row.id)),
                                IconName::Trash,
                                self.tr("common.delete"),
                                Command::Mapping("delete", row.id.clone()),
                                mapping.busy,
                                cx,
                            )),
                    ),
            );
        }
        body.child(self.scroller("mappings", rows))
            .into_any_element()
    }

    fn mapping_editor(&self, cx: &Context<Self>) -> AnyElement {
        let mapping = &self.ui.mapping;
        let busy = mapping.busy || mapping.saving || self.controller.recording();
        let field = |key: &str| div().v_flex().gap_3().child(self.tr(key));
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
                        .selected(recording),
                    )
                    .child(self.icon_button(
                        ("mapping-remove-input", slot),
                        IconName::X,
                        self.tr(if recording {
                            "capture.cancel_shortcut"
                        } else {
                            "common.remove"
                        }),
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
        output = output.child(modes).child(
            GroupBox::new()
                .id("mapping-output-preview")
                .fill()
                .content_style(StyleRefinement::default().min_h_16())
                .child(if mapping.capture_slot == 2 {
                    mapping.capture_text.clone()
                } else {
                    preview.join(" + ")
                }),
        );
        if mapping.entry_mode == 0 {
            output = output
                .child(
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
                )
                .child(self.caption(self.tr("mappings.mouse_output_help"), cx));
        } else {
            output = output
                .child(self.caption(self.tr("mappings.manual_output_help"), cx))
                .child(self.tr("mappings.search_output"))
                .child(
                    Input::new(&self.output_filter)
                        .id("mapping-output-filter")
                        .aria_label(self.tr("mappings.search_output"))
                        .disabled(busy),
                );
            let options = mapping.output_options.clone();
            let weak = cx.entity().downgrade();
            output = output.child(
                div().child(
                    Button::new("mapping-output-choice")
                        .label(if mapping.selected_output.value.is_empty() {
                            self.tr("mappings.select_output")
                        } else {
                            mapping.selected_output.label.clone()
                        })
                        .disabled(busy)
                        .dropdown_menu(move |mut menu, _, _| {
                            for option in &options {
                                let weak = weak.clone();
                                let value = option.value.clone();
                                menu =
                                    menu.item(PopupMenuItem::new(option.label.clone()).on_click(
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
                                    ));
                            }
                            menu
                        }),
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
            .child(modifiers)
            .child(self.caption(self.tr("mappings.output_help"), cx));
        let options = mapping.scopes.clone();
        let weak = cx.entity().downgrade();
        let scopes = Button::new("mapping-scope-choice")
            .label(mapping.scope_label.clone())
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
                field("common.description").child(
                    Input::new(&self.mapping_name)
                        .id("mapping-name")
                        .aria_label(self.tr("common.description"))
                        .disabled(busy),
                ),
            )
            .child(field("common.effective_scope").child(scopes))
            .child(
                field("mappings.input")
                    .child(input)
                    .when(!mapping.input_error.is_empty(), |section| {
                        section.child(self.error(&mapping.input_error, cx))
                    }),
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

    pub(super) fn update_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = if self.ui.mapping.leave_open {
            1
        } else if self.ui.groups.confirming_delete {
            2
        } else {
            0
        };
        if next == self.dialog {
            return;
        }
        if self.dialog != 0 {
            window.close_dialog(cx);
        }
        self.dialog = next;
        if next == 0 {
            return;
        }
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, window, cx| {
            let Some(entity) = weak.upgrade() else {
                return dialog;
            };
            entity.update(cx, |this, cx| {
                let (title, message, cancel) = if next == 1 {
                    (
                        this.tr("common.save_changes"),
                        this.tr("mappings.unsaved_help"),
                        Command::Mapping("stay", String::new()),
                    )
                } else {
                    (
                        format!(
                            "{} — {}",
                            this.tr("groups.delete"),
                            this.ui.groups.selected_name
                        ),
                        this.ui.groups.deletion_warning.clone(),
                        Command::Group("cancel", String::new()),
                    )
                };
                let busy = if next == 1 {
                    this.ui.mapping.saving
                } else {
                    this.ui.groups.saving
                };
                let mut footer = div().h_flex().justify_end().gap_2().child(this.button(
                    "dialog-cancel",
                    this.tr(if next == 1 {
                        "common.keep_editing"
                    } else {
                        "common.cancel"
                    }),
                    cancel.clone(),
                    busy,
                    cx,
                ));
                if next == 1 {
                    footer = footer
                        .child(this.button(
                            "dialog-discard",
                            this.tr("common.discard_changes"),
                            Command::Mapping("discard", String::new()),
                            busy,
                            cx,
                        ))
                        .child(this.button(
                            "dialog-save",
                            this.tr("mappings.save"),
                            Command::Mapping("save-leave", String::new()),
                            busy,
                            cx,
                        ));
                } else {
                    footer = footer.child(this.button(
                        "dialog-delete",
                        this.tr(if busy {
                            "common.saving"
                        } else {
                            "groups.delete_confirm"
                        }),
                        Command::Group("delete", String::new()),
                        busy,
                        cx,
                    ));
                }
                let weak = cx.entity().downgrade();
                let _ = window;
                dialog
                    .title(title)
                    .child(message)
                    .child(this.error(
                        if next == 1 {
                            &this.ui.mapping.error
                        } else {
                            &this.ui.groups.form_error
                        },
                        cx,
                    ))
                    .footer(footer)
                    .overlay_closable(false)
                    .close_button(!busy)
                    .on_cancel(move |_, _, cx| {
                        if busy {
                            return false;
                        }
                        let _ = weak.update(cx, |this, cx| {
                            match &cancel {
                                Command::Mapping(command, _) => {
                                    let _ =
                                        this.controller.mapping_command(&mut this.ui, command, "");
                                }
                                Command::Group(command, _) => {
                                    let _ =
                                        this.controller.group_command(&mut this.ui, command, "");
                                }
                                _ => {}
                            }
                            this.dialog = 0;
                            cx.notify();
                        });
                        true
                    })
            })
        });
    }
}
