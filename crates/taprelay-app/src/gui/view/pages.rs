use super::*;

impl AppView {
    pub(super) fn scroller(&self, id: &'static str, content: impl IntoElement) -> AnyElement {
        div()
            .id(id)
            .flex_1()
            .min_h_0()
            .test_support()
            .overflow_y_scrollbar()
            .id(id)
            .child(div().v_flex().gap_4().child(content))
            .into_any_element()
    }

    pub(super) fn environment(&self, cx: &Context<Self>) -> AnyElement {
        let mut panel = GroupBox::new().id("environment-checks").fill();
        for row in &self.ui.checks {
            let (icon, state, color) = match row.state {
                1 => (
                    IconName::Loader,
                    "check.running",
                    cx.theme().muted_foreground,
                ),
                2 => (IconName::Check, "check.passed", cx.theme().success),
                3 => (IconName::CircleAlert, "check.failed", cx.theme().danger),
                _ => (
                    IconName::Minus,
                    "check.waiting",
                    cx.theme().muted_foreground,
                ),
            };
            panel = panel.child(
                div()
                    .h_flex()
                    .gap_3()
                    .child(Icon::new(icon).size_4().text_color(color))
                    .child(div().flex_1().child(row.title.clone()))
                    .child(self.caption(self.tr(state), cx)),
            );
        }
        let mut body = div()
            .v_flex()
            .gap(rems(1.375))
            .pr_3()
            .child(self.heading(self.tr(if self.ui.mode == 3 {
                "startup.error"
            } else {
                "startup.preparing"
            })))
            .child(self.caption(self.tr("startup.description"), cx))
            .child(panel);
        if !self.ui.problem.is_empty() {
            body = body
                .child(self.error(&self.ui.problem, cx))
                .child(self.caption(
                    self.tr(if self.ui.mode == 3 {
                        "startup.environment.troubleshooting"
                    } else {
                        "startup.troubleshooting"
                    }),
                    cx,
                ));
        }
        div()
            .v_flex()
            .flex_1()
            .min_h_0()
            .gap(rems(1.375))
            .child(self.scroller("environment", body))
            .child(
                div()
                    .h_flex()
                    .gap_3()
                    .flex_shrink_0()
                    .child(self.app_button(
                        "retry-checks",
                        "startup.retry",
                        Action::Retry,
                        self.ui.busy || self.ui.problem_busy || !self.ui.problem_retry_allowed,
                        cx,
                    ))
                    .when(self.ui.mode != 3, |row| {
                        row.child(self.app_button(
                            "startup-bluetooth",
                            "startup.bluetooth.settings",
                            Action::BluetoothSettings,
                            false,
                            cx,
                        ))
                    })
                    .child(self.app_button("startup-quit", "common.quit", Action::Quit, false, cx)),
            )
            .into_any_element()
    }

    fn status_card(&self, window: &mut Window, cx: &Context<Self>) -> GroupBox {
        let frames = self.ripples.frames(Instant::now());
        if !frames.is_empty() {
            window.request_animation_frame();
        }
        let center = std::rc::Rc::new(std::cell::Cell::new(point(px(0.), px(0.))));
        let anchor = center.clone();
        let signal = self.signal(cx);
        let waves = canvas(
            |_, _, _| (),
            move |bounds, _, window, _| {
                let width: f32 = bounds.size.width.into();
                let height: f32 = bounds.size.height.into();
                let radius = width.hypot(height);
                for frame in frames {
                    let diameter = px(10. + 2. * radius * frame.progress);
                    let bounds = Bounds::new(
                        center.get() - point(diameter / 2., diameter / 2.),
                        size(diameter, diameter),
                    );
                    let [h, s, l, _] = frame.color;
                    window.paint_quad(quad(
                        bounds,
                        diameter / 2.,
                        Hsla::transparent_black(),
                        px(2.),
                        hsla(h, s, l, frame.opacity),
                        BorderStyle::Solid,
                    ));
                }
            },
        )
        .absolute()
        .size_full();
        let dot = canvas(
            move |bounds, _, _| anchor.set(bounds.center()),
            move |bounds, _, window, _| {
                window.paint_quad(quad(
                    bounds,
                    bounds.size.width / 2.,
                    signal,
                    px(0.),
                    signal,
                    BorderStyle::Solid,
                ));
            },
        )
        .size(rems(0.625))
        .flex_shrink_0();
        GroupBox::new()
            .id("receiver-status")
            .fill()
            .h_full()
            .grid()
            .grid_rows(1)
            .content_style(
                StyleRefinement::default()
                    .flex_1()
                    .min_h_0()
                    .relative()
                    .overflow_hidden(),
            )
            .child(waves)
            .child(
                div()
                    .v_flex()
                    .gap_2()
                    .child(self.caption(self.tr("receiver.current"), cx))
                    .child(self.heading(self.ui.device_name.clone()))
                    .child(
                        div()
                            .h_flex()
                            .text_sm()
                            .gap_3()
                            .child(dot)
                            .child(self.ui.stage.clone()),
                    )
                    .child(
                        div()
                            .h_flex()
                            .gap_3()
                            .pt_1()
                            .flex_wrap()
                            .child(self.app_button(
                                "listen",
                                if self.ui.listening {
                                    "listening.stop"
                                } else {
                                    "listening.start"
                                },
                                Action::Listen,
                                self.ui.busy,
                                cx,
                            ))
                            .child(self.app_button(
                                "test-receiver",
                                "receiver.test",
                                Action::Test,
                                !self.ui.ready,
                                cx,
                            ))
                            .when(self.ui.state_color != 0, |row| {
                                row.child(self.app_button(
                                    "retry-receiver",
                                    "common.retry",
                                    Action::Retry,
                                    self.ui.problem_busy,
                                    cx,
                                ))
                            }),
                    ),
            )
    }

    fn admin_alert(&self, cx: &Context<Self>) -> ActionAlert {
        ActionAlert::error("admin-alert", self.tr("permissions.warning")).action(self.app_button(
            "elevate",
            "permissions.restart.short",
            Action::Elevate,
            false,
            cx,
        ))
    }

    pub(super) fn overview(&self, window: &mut Window, cx: &Context<Self>) -> AnyElement {
        let mut content = div().v_flex().flex_1().min_h_0().gap_4();
        if !self.ui.problem.is_empty() {
            content = content.child(
                ActionAlert::error("overview-alert", self.ui.problem.clone()).when(
                    self.ui.problem_retry_allowed,
                    |alert| {
                        alert.action(self.app_button(
                            "overview-retry",
                            "common.retry",
                            Action::Retry,
                            self.ui.problem_busy,
                            cx,
                        ))
                    },
                ),
            );
        }
        if !self.ui.elevated {
            content = content.child(self.admin_alert(cx));
        }
        content = content.child(
            div()
                .id("status-card")
                .h(rems(11.25))
                .flex_shrink_0()
                .child(self.status_card(window, cx))
                .test_support(),
        );
        let mut shortcuts = div().h_flex().flex_wrap().gap_2();
        for binding in self.ui.bindings.iter().filter(|row| row.enabled) {
            shortcuts = shortcuts.child(
                div()
                    .id(SharedString::from(format!(
                        "summary-{}-{}",
                        binding.function_name, binding.text
                    )))
                    .px_2()
                    .py(rems(0.375))
                    .rounded_md()
                    .border_1()
                    .border_color(cx.theme().border)
                    .tooltip({
                        let label = binding.function_name.clone();
                        move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(label.clone())
                                .build(window, cx)
                        }
                    })
                    .child(binding.text.clone())
                    .test_support(),
            );
        }
        content
            .child(
                div()
                    .id("shortcuts-card")
                    .v_flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        GroupBox::new()
                            .id("overview-bindings")
                            .fill()
                            .flex_1()
                            .min_h_0()
                            .grid()
                            .grid_rows(1)
                            .content_style(StyleRefinement::default().flex_1().min_h_0())
                            .child(
                                div()
                                    .h_flex()
                                    .justify_between()
                                    .gap_4()
                                    .child(self.caption(self.tr("overview.bindings"), cx))
                                    .child(self.icon_button(
                                        "manage-bindings",
                                        IconName::Settings,
                                        self.tr("overview.manage"),
                                        Command::App(Action::Navigate, "1".into()),
                                        false,
                                        cx,
                                    )),
                            )
                            .child(self.scroller("overview-shortcuts", shortcuts)),
                    )
                    .test_support(),
            )
            .into_any_element()
    }

    pub(super) fn devices(&self, cx: &Context<Self>) -> AnyElement {
        let mut body = div()
            .v_flex()
            .gap_3()
            .child(self.caption(self.ui.discovery_status.clone(), cx));
        for row in &self.ui.paired_devices {
            let action = Action::try_from(row.action.as_str()).unwrap_or(Action::Device);
            let kind = match row.device_kind.as_str() {
                "tablet" => IconName::Tablet,
                "phone" => IconName::Smartphone,
                "computer" => IconName::Monitor,
                _ => IconName::Bluetooth,
            };
            let (status, color) = match row.status.as_str() {
                "ready" => (IconName::CircleCheck, cx.theme().success),
                "busy" => (IconName::Loader, cx.theme().muted_foreground),
                "failed" => (IconName::CircleAlert, cx.theme().danger),
                "linked" => (IconName::BluetoothConnected, cx.theme().info),
                "paired" => (IconName::Link2, cx.theme().muted_foreground),
                _ => (IconName::BluetoothOff, cx.theme().muted_foreground),
            };
            body = body.child(
                div()
                    .id(SharedString::from(format!("receiver-row-{}", row.id)))
                    .child(
                        GroupBox::new()
                            .id(SharedString::from(format!("receiver-{}", row.id)))
                            .fill()
                            .content_style(StyleRefinement::default().h_flex().gap_3())
                            .child(Icon::new(kind).size_5().flex_shrink_0())
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_ellipsis()
                                    .child(row.name.clone()),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!("receiver-status-{}", row.id)))
                                    .h_flex()
                                    .justify_center()
                                    .size_8()
                                    .flex_shrink_0()
                                    .tooltip({
                                        let text = row.detail.clone();
                                        move |window, cx| {
                                            gpui_kit::component::tooltip::Tooltip::new(text.clone())
                                                .build(window, cx)
                                        }
                                    })
                                    .child(Icon::new(status).size(rems(1.125)).text_color(color)),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!(
                                        "device-action-hint-{}",
                                        row.id
                                    )))
                                    .w(rems(8.25))
                                    .flex_shrink_0()
                                    .tooltip({
                                        let text = row.action_label.clone();
                                        move |window, cx| {
                                            gpui_kit::component::tooltip::Tooltip::new(text.clone())
                                                .build(window, cx)
                                        }
                                    })
                                    .child(
                                        self.button(
                                            SharedString::from(format!("device-{}", row.id)),
                                            if row.action.is_empty() {
                                                self.tr("receiver.connect")
                                            } else {
                                                row.action_label.clone()
                                            },
                                            Command::App(action, row.id.clone()),
                                            !row.enabled || row.action.is_empty(),
                                            cx,
                                        )
                                        .w_full(),
                                    ),
                            ),
                    )
                    .test_support(),
            );
        }
        if self.ui.paired_devices.is_empty() && !self.ui.scanning {
            body = body.child(self.caption(self.tr("receiver.empty"), cx));
        }
        self.scroller("devices", body)
    }

    pub(super) fn bindings(&self, cx: &Context<Self>) -> AnyElement {
        let mut page = div()
            .v_flex()
            .flex_1()
            .min_h_0()
            .gap_6()
            .child(self.caption(self.tr("bindings.help"), cx));
        if !self.ui.groups.error.is_empty() {
            page = page.child(self.error(&self.ui.groups.error, cx));
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
                    self.caption(self.tr("common.shortcut"), cx)
                        .flex_1()
                        .min_w_0(),
                )
                .child(
                    self.caption(self.tr("common.function"), cx)
                        .w(rems(15.))
                        .flex_shrink_0(),
                )
                .child(
                    self.caption(self.tr("common.effective_scope"), cx)
                        .w(rems(7.75))
                        .flex_shrink_0(),
                ),
        );
        let busy = self.controller.recording() || self.ui.groups.busy;
        for row in &self.ui.binding.rows {
            let Some(id) = FunctionId::from_stable_id(&row.id) else {
                continue;
            };
            let capturing = self.ui.binding.capture.function_id == row.id;
            let mut shortcuts = div().h_flex().gap_2().flex_wrap();
            for shortcut in &row.shortcuts {
                let target = CaptureTarget {
                    id,
                    slot: shortcut.slot as usize,
                };
                let recording = capturing && self.ui.binding.capture.slot == shortcut.slot;
                let label = if recording {
                    self.ui.binding.capture.text.clone()
                } else {
                    shortcut.text.clone()
                };
                let mut chip = div()
                    .h_flex()
                    .max_w_full()
                    .rounded_md()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().group_box)
                    .child(
                        self.button(
                            SharedString::from(format!("binding-{}-{}", row.id, shortcut.slot)),
                            label,
                            Command::Binding(BindingCommand::BeginCapture(target)),
                            !row.enabled || busy,
                            cx,
                        )
                        .selected(recording),
                    );
                if row.enabled {
                    chip = chip.child(self.icon_button(
                        SharedString::from(if recording {
                            format!("capture-cancel-{}", row.id)
                        } else {
                            format!("remove-{}-{}", row.id, shortcut.slot)
                        }),
                        IconName::X,
                        self.tr(if recording {
                            "capture.cancel_shortcut"
                        } else {
                            "common.remove"
                        }),
                        Command::Binding(if recording {
                            BindingCommand::CancelCapture
                        } else {
                            BindingCommand::DeleteShortcut(target)
                        }),
                        busy && !recording,
                        cx,
                    ));
                }
                shortcuts = shortcuts.child(chip);
            }
            if row.enabled && row.shortcuts.len() < 2 {
                let recording =
                    capturing && self.ui.binding.capture.slot == row.shortcuts.len() as i32;
                shortcuts = shortcuts.child(
                    self.button(
                        SharedString::from(format!("binding-add-{}", row.id)),
                        if recording {
                            self.ui.binding.capture.text.clone()
                        } else {
                            self.tr("shortcuts.unset")
                        },
                        Command::Binding(BindingCommand::BeginCapture(CaptureTarget {
                            id,
                            slot: row.shortcuts.len(),
                        })),
                        busy,
                        cx,
                    )
                    .icon(IconName::Plus)
                    .accessibility_label(self.tr("shortcuts.add"))
                    .selected(recording),
                );
                if recording {
                    shortcuts = shortcuts.child(self.icon_button(
                        SharedString::from(format!("capture-cancel-{}", row.id)),
                        IconName::X,
                        self.tr("capture.cancel_shortcut"),
                        Command::Binding(BindingCommand::CancelCapture),
                        false,
                        cx,
                    ));
                }
            } else if !row.enabled && row.shortcuts.is_empty() {
                shortcuts =
                    shortcuts.child(self.caption(self.tr("shortcuts.unset"), cx).opacity(0.5));
            }
            let mut shortcut_column = div()
                .id(SharedString::from(format!("binding-shortcuts-{}", row.id)))
                .v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(shortcuts);
            if capturing && !self.ui.binding.capture.error.is_empty() {
                shortcut_column =
                    shortcut_column.child(self.error(&self.ui.binding.capture.error, cx));
            }
            let mut gestures = div()
                .id(SharedString::from(format!("binding-function-{}", row.id)))
                .v_flex()
                .w(rems(15.))
                .flex_shrink_0()
                .when(!row.enabled, |column| column.opacity(0.5));
            for gesture in &row.gestures {
                gestures = gestures.child(
                    div()
                        .h_flex()
                        .h_6()
                        .gap_2()
                        .child(self.caption(gesture.gesture.clone(), cx).w_10())
                        .child(
                            Icon::new(IconName::ArrowRight)
                                .size_4()
                                .text_color(cx.theme().muted_foreground),
                        )
                        .child(
                            div()
                                .px_2()
                                .py(rems(0.125))
                                .rounded_md()
                                .bg(cx.theme().secondary)
                                .child(gesture.action.clone()),
                        ),
                );
            }
            rows = rows.child(
                div()
                    .id(SharedString::from(format!("binding-row-{}", row.id)))
                    .child(
                        GroupBox::new()
                            .id(SharedString::from(format!("binding-{}", row.id)))
                            .fill()
                            .content_style(StyleRefinement::default().h_flex().min_h_16())
                            .child(
                                div().w_9().flex_shrink_0().child(
                                    Switch::new(SharedString::from(format!(
                                        "binding-enabled-{}",
                                        row.id
                                    )))
                                    .accessibility_label(row.label.clone())
                                    .checked(row.enabled)
                                    .disabled(busy)
                                    .on_change(cx.listener(
                                        move |this, enabled: &bool, window, cx| {
                                            this.dispatch(
                                                Command::Binding(
                                                    BindingCommand::SetFunctionEnabled {
                                                        id,
                                                        enabled: *enabled,
                                                    },
                                                ),
                                                window,
                                                cx,
                                            );
                                        },
                                    )),
                                ),
                            )
                            .child(shortcut_column.test_support())
                            .child(gestures.test_support())
                            .child(
                                div()
                                    .id(SharedString::from(format!("binding-scope-{}", row.id)))
                                    .w(rems(7.75))
                                    .flex_shrink_0()
                                    .child(self.scope_menu(id, row, busy, cx))
                                    .test_support(),
                            ),
                    )
                    .test_support(),
            );
        }
        page.child(self.scroller("bindings", rows))
            .into_any_element()
    }

    fn scope_menu(
        &self,
        id: FunctionId,
        row: &crate::FunctionBindingRow,
        busy: bool,
        cx: &Context<Self>,
    ) -> AnyElement {
        if !row.scope_editable {
            return self
                .caption(self.tr("common.all_apps"), cx)
                .into_any_element();
        }
        let rules = &self.controller.runtime.config.foreground_app_rules;
        let selected = rules.assignments.get(&id);
        let mut options = vec![(
            String::new(),
            self.tr("common.all_apps"),
            selected.is_none_or(|s| s.is_empty()),
        )];
        options.extend(rules.groups.iter().map(|(key, group)| {
            (
                key.clone(),
                group.name.clone(),
                selected.is_some_and(|s| s.contains(key)),
            )
        }));
        let weak = cx.entity().downgrade();
        Button::new(SharedString::from(format!("scope-{}", row.id)))
            .label(row.scope_label.clone())
            .tooltip(row.scope_label.clone())
            .w_full()
            .disabled(busy)
            .dropdown_menu(move |mut menu, _, _| {
                for (group, name, checked) in &options {
                    let weak = weak.clone();
                    let group = group.clone();
                    menu = menu.item(PopupMenuItem::new(name.clone()).checked(*checked).on_click(
                        move |_, window, cx| {
                            let _ = weak.update(cx, |this, cx| {
                                this.dispatch(Command::Scope(id, group.clone()), window, cx)
                            });
                        },
                    ));
                }
                menu
            })
            .into_any_element()
    }

    pub(super) fn wizard(&self, window: &mut Window, cx: &Context<Self>) -> AnyElement {
        let step = if self.ui.wizard_page <= 1 {
            0
        } else {
            self.ui.wizard_page - 1
        };
        let key = ["wizard.bindings", "wizard.receiver", "wizard.test"][step as usize];
        let mut header = div()
            .h_flex()
            .gap_3()
            .flex_shrink_0()
            .child(self.heading(self.tr(key)));
        if step == 1 {
            header = header
                .child(self.bluetooth_status(cx))
                .child(div().flex_1())
                .children(self.receiver_actions(cx));
        }
        let mut body = div()
            .v_flex()
            .flex_1()
            .min_h_0()
            .gap_4()
            .child(
                div()
                    .h_1()
                    .w_full()
                    .flex_shrink_0()
                    .rounded_full()
                    .bg(cx.theme().muted)
                    .child(
                        div()
                            .h_full()
                            .w(relative(step as f32 / 2.))
                            .rounded_full()
                            .bg(cx.theme().primary),
                    ),
            )
            .child(header);
        if step == 2 && !self.ui.elevated {
            body = body.child(self.admin_alert(cx));
        }
        body = body.child(match step {
            0 => self.bindings(cx),
            1 => self.devices(cx),
            _ => self.scroller(
                "wizard-test",
                div()
                    .v_flex()
                    .gap_4()
                    .child(self.caption(
                        format!("{}{}", self.tr("wizard.input.count"), self.ui.input_count),
                        cx,
                    ))
                    .child(div().h(rems(11.25)).child(self.status_card(window, cx))),
            ),
        });
        body.child(
            div()
                .h_flex()
                .justify_end()
                .gap_3()
                .flex_shrink_0()
                .child(self.app_button(
                    "wizard-skip",
                    "common.skip",
                    Action::WizardFinish,
                    false,
                    cx,
                ))
                .child(self.app_button(
                    "wizard-back",
                    "common.previous_step",
                    Action::WizardBack,
                    step == 0 || self.controller.recording(),
                    cx,
                ))
                .child(self.app_button(
                    "wizard-next",
                    if step == 2 {
                        "common.finish"
                    } else {
                        "common.next_step"
                    },
                    if step == 2 {
                        Action::WizardFinish
                    } else {
                        Action::WizardNext
                    },
                    self.controller.recording() || (step == 1 && !self.ui.receiver_next_allowed),
                    cx,
                )),
        )
        .into_any_element()
    }

    pub(super) fn settings(&self, cx: &Context<Self>) -> AnyElement {
        let mut appearance = self.section("settings.appearance");
        let mut themes = div().h_flex().gap_2();
        for (index, name) in ["system", "light", "dark"].iter().enumerate() {
            themes = themes.child(
                self.button(
                    ("theme", index),
                    self.tr(&format!("theme.{name}")),
                    Command::App(Action::Theme, name.to_string()),
                    false,
                    cx,
                )
                .selected(self.ui.theme_choice == index as i32),
            );
        }
        let mut languages = div().h_flex().gap_2();
        for (index, option) in self.ui.language_options.iter().enumerate() {
            languages = languages.child(
                self.button(
                    ("language", index),
                    option.label.clone(),
                    Command::App(Action::Language, option.value.clone()),
                    false,
                    cx,
                )
                .selected(self.ui.language_choice == index as i32),
            );
        }
        appearance = appearance.child(themes).child(languages);
        let mut rates = div().h_flex().gap_2().flex_wrap();
        for rate in [0, 60, 125, 250, 500, 1000] {
            rates = rates.child(
                self.button(
                    ("report-rate", rate as usize),
                    if rate == 0 {
                        self.tr("common.auto")
                    } else {
                        format!("{rate} Hz")
                    },
                    Command::App(Action::SetPassthroughMouseReportRate, rate.to_string()),
                    false,
                    cx,
                )
                .selected(self.ui.passthrough_mouse_report_rate == rate),
            );
        }
        let passthrough = self
            .section("settings.passthrough")
            .child(self.setting(
                "overlay",
                "settings.passthrough_overlay",
                "settings.passthrough_overlay_hint",
                self.ui.passthrough_overlay,
                Action::SetPassthroughOverlay,
                cx,
            ))
            .child(self.tr("settings.passthrough.mouse.report.rate"))
            .child(rates)
            .child(self.caption(self.tr("settings.passthrough.mouse.report.rate.hint"), cx))
            .child(self.setting(
                "reverse-scroll",
                "settings.passthrough.reverse.scroll",
                "settings.passthrough.reverse.scroll.hint",
                self.ui.passthrough_reverse_scroll,
                Action::SetPassthroughReverseScroll,
                cx,
            ));
        let startup = self
            .section("settings.startup")
            .child(self.setting(
                "autostart",
                "settings.autostart",
                "",
                self.ui.auto_start,
                Action::SetAutostart,
                cx,
            ))
            .child(self.setting(
                "start-hidden",
                "settings.hidden",
                "",
                self.ui.start_hidden,
                Action::SetStartHidden,
                cx,
            ))
            .child(self.setting(
                "auto-listen",
                "settings.auto.listen",
                "",
                self.ui.auto_listen,
                Action::SetAutoListen,
                cx,
            ))
            .child(self.setting(
                "close-to-tray",
                "settings.close.tray",
                "settings.close.hint",
                self.ui.close_to_tray,
                Action::SetCloseToTray,
                cx,
            ));
        let permissions = self
            .section("settings.app.permissions")
            .child(self.caption(
                self.tr(if self.ui.elevated {
                    "permissions.badge.administrator"
                } else {
                    "permissions.badge.user"
                }),
                cx,
            ))
            .child(self.setting(
                "always-admin",
                "settings.always.admin",
                "settings.always.admin.hint",
                self.ui.always_admin,
                Action::SetAlwaysAdmin,
                cx,
            ))
            .child(div().child(self.app_button(
                "settings-elevate",
                "permissions.restart",
                Action::Elevate,
                self.ui.elevated,
                cx,
            )));
        let notifications = self
            .section("settings.notifications")
            .child(self.setting(
                "wait-warning",
                "settings.connection.wait.warning",
                "settings.connection.wait.warning.hint",
                self.ui.connection_wait_warning,
                Action::SetConnectionWaitWarning,
                cx,
            ))
            .child(self.setting(
                "notifications",
                "settings.notifications.toggle",
                "settings.notifications.hint",
                self.ui.notifications,
                Action::SetNotifications,
                cx,
            ));
        let licensing = self
            .section("settings.licensing")
            .child("GPL-3.0-only")
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(self.app_button(
                        "license",
                        "settings.license",
                        Action::License,
                        false,
                        cx,
                    ))
                    .child(self.app_button(
                        "notices",
                        "settings.third.party.notices",
                        Action::ThirdPartyNotices,
                        false,
                        cx,
                    )),
            );
        let about = self
            .section("settings.about")
            .child(format!("TapRelay {}", self.ui.app_version))
            .child(self.caption(self.ui.data_directory.clone(), cx))
            .child(
                div()
                    .h_flex()
                    .gap_2()
                    .flex_wrap()
                    .child(self.app_button(
                        "repository",
                        "settings.repository",
                        Action::Repository,
                        false,
                        cx,
                    ))
                    .child(self.app_button(
                        "data-folder",
                        "settings.data.folder",
                        Action::DataFolder,
                        false,
                        cx,
                    ))
                    .child(self.app_button(
                        "logs-folder",
                        "logs.folder",
                        Action::LogsFolder,
                        false,
                        cx,
                    ))
                    .child(self.app_button("wizard", "settings.wizard", Action::Wizard, false, cx)),
            );
        self.scroller(
            "settings",
            div().v_flex().gap_7().pr_3().children([
                appearance,
                passthrough,
                startup,
                permissions,
                notifications,
                licensing,
                about,
            ]),
        )
    }

    fn setting(
        &self,
        id: &'static str,
        label: &str,
        hint: &str,
        value: bool,
        action: Action,
        cx: &Context<Self>,
    ) -> Div {
        div()
            .h_flex()
            .gap_4()
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .flex_1()
                    .child(self.tr(label))
                    .when(!hint.is_empty(), |row| {
                        row.child(self.caption(self.tr(hint), cx))
                    }),
            )
            .child(
                Switch::new(id)
                    .accessibility_label(self.tr(label))
                    .checked(value)
                    .on_change(cx.listener(move |this, checked: &bool, window, cx| {
                        this.dispatch(
                            Command::App(action, if *checked { "1" } else { "0" }.into()),
                            window,
                            cx,
                        )
                    })),
            )
    }
}
