use crate::gui::action::Action;
use crate::gui::{
    presentation::ChoiceOption,
    view::{AppView, Command},
};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{
    Selectable, StyledExt,
    combobox::Combobox,
    group_box::{GroupBox, GroupBoxVariants},
    searchable_list::SearchableListItem,
    switch::Switch,
    tag::Tag,
};
use gpui_kit::{assets::IconName, component::Icon};
use gpui_kit::{prelude::*, *};

impl SearchableListItem for ChoiceOption {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &String {
        &self.value
    }
}

impl AppView {
    pub(in crate::gui::view) fn sync_report_rate(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let auto_label = self.tr("common.auto");
        let labels_changed = self.report_rate_auto_label != auto_label;
        let value = self.ui.passthrough_mouse_report_rate.to_string();
        if !labels_changed && self.report_rate.read(cx).selected_value().as_ref() == Some(&value) {
            return;
        }
        self.report_rate.update(cx, |state, cx| {
            if labels_changed {
                let items = [0, 60, 125, 250, 500, 1000]
                    .into_iter()
                    .map(|rate| ChoiceOption {
                        label: if rate == 0 {
                            auto_label.clone()
                        } else {
                            format!("{rate} Hz")
                        },
                        value: rate.to_string(),
                    })
                    .collect();
                state.set_items(items, window, cx);
            }
            state.set_selected_values(&[value], window, cx);
        });
        self.report_rate_auto_label = auto_label;
    }

    pub(in crate::gui::view) fn settings(&self, cx: &Context<Self>) -> AnyElement {
        let mut appearance = self.section("settings.appearance", cx);
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
                .selected(self.ui.theme_choice == index as i32)
                .toggled(self.ui.theme_choice == index as i32),
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
                .selected(self.ui.language_choice == index as i32)
                .toggled(self.ui.language_choice == index as i32),
            );
        }
        appearance = appearance
            .child(
                self.reveal_focus(
                    "theme-options",
                    div()
                        .v_flex()
                        .gap_2()
                        .child(self.setting_text("settings.theme", "settings.theme.hint", cx))
                        .child(themes.flex_wrap()),
                ),
            )
            .child(
                self.reveal_focus(
                    "language-options",
                    div()
                        .v_flex()
                        .gap_2()
                        .child(self.setting_text("settings.language", "settings.language.hint", cx))
                        .child(languages.flex_wrap()),
                ),
            );
        let passthrough = self
            .section("settings.passthrough", cx)
            .child(self.setting(
                "overlay",
                "settings.passthrough_overlay",
                "settings.passthrough_overlay_hint",
                self.ui.passthrough_overlay,
                Action::SetPassthroughOverlay,
                cx,
            ))
            .child(
                self.reveal_focus(
                    "rate-options",
                    div()
                        .h_flex()
                        .gap_4()
                        .child(
                            self.setting_text(
                                "settings.passthrough.mouse.report.rate",
                                "settings.passthrough.mouse.report.rate.hint",
                                cx,
                            )
                            .flex_1()
                            .min_w_0(),
                        )
                        .child(
                            div()
                                .id("report-rate")
                                .role(Role::Group)
                                .aria_label(self.tr("settings.passthrough.mouse.report.rate"))
                                .w_32()
                                .flex_shrink_0()
                                .child(Combobox::new(&self.report_rate).w_full())
                                .test_support(),
                        ),
                ),
            )
            .child(self.setting(
                "reverse-scroll",
                "settings.passthrough.reverse.scroll",
                "settings.passthrough.reverse.scroll.hint",
                self.ui.passthrough_reverse_scroll,
                Action::SetPassthroughReverseScroll,
                cx,
            ));
        let startup = self
            .section("settings.startup", cx)
            .child(self.setting(
                "autostart",
                "settings.autostart",
                "settings.autostart.hint",
                self.ui.auto_start,
                Action::SetAutostart,
                cx,
            ))
            .child(self.setting(
                "start-hidden",
                "settings.hidden",
                "settings.hidden.hint",
                self.ui.start_hidden,
                Action::SetStartHidden,
                cx,
            ))
            .child(self.setting(
                "auto-listen",
                "settings.auto.listen",
                "settings.auto.listen.hint",
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
        let startup = if self.ui.capabilities.login_approval {
            startup
                .child(
                    self.caption(
                        self.tr(self
                            .ui
                            .auto_start_status
                            .unwrap_or("common.status_unavailable")),
                        cx,
                    )
                    .id("autostart-status")
                    .role(Role::Status)
                    .test_support(),
                )
                .child(self.reveal_focus(
                    "login-settings-action",
                    self.app_button(
                        "login-settings",
                        "common.open_settings",
                        Action::LoginSettings,
                        false,
                        cx,
                    ),
                ))
        } else {
            startup
        };
        let (permission_tag, permission_icon, permission_label) = if self.ui.elevated {
            (
                Tag::success(),
                IconName::ShieldCheck,
                "permissions.badge.administrator",
            )
        } else {
            (Tag::info(), IconName::UserRound, "permissions.badge.user")
        };
        let permissions = if self.ui.capabilities.elevation {
            self.section("settings.app.permissions", cx)
                .child(
                    self.reveal_focus(
                        "admin-action",
                        div()
                            .h_flex()
                            .justify_between()
                            .gap_3()
                            .flex_wrap()
                            .child(
                                permission_tag
                                    .gap_1()
                                    .flex_shrink_0()
                                    .child(Icon::new(permission_icon).size_3p5())
                                    .child(self.text(self.tr(permission_label))),
                            )
                            .child(
                                self.app_button(
                                    "settings-elevate",
                                    "permissions.restart",
                                    Action::Elevate,
                                    self.ui.elevated,
                                    cx,
                                )
                                .flex_shrink_0(),
                            ),
                    ),
                )
                .child(self.setting(
                    "always-admin",
                    "settings.always.admin",
                    "settings.always.admin.hint",
                    self.ui.always_admin,
                    Action::SetAlwaysAdmin,
                    cx,
                ))
        } else {
            use crate::platform::desktop::BluetoothPermission;
            let input = self.ui.input_permissions;
            let allowed = |allowed| {
                if allowed {
                    "permissions.mac.allowed"
                } else {
                    "permissions.mac.required"
                }
            };
            let rows = [
                (
                    "accessibility",
                    "permissions.mac.accessibility",
                    allowed(input.accessibility),
                    Action::Elevate,
                ),
                (
                    "monitoring",
                    "permissions.mac.monitoring",
                    allowed(input.monitoring),
                    Action::InputMonitoringSettings,
                ),
                (
                    "posting",
                    "permissions.mac.posting",
                    allowed(input.posting),
                    Action::Elevate,
                ),
                (
                    "bluetooth",
                    "permissions.mac.bluetooth",
                    match self.ui.bluetooth_permission {
                        BluetoothPermission::Allowed => "permissions.mac.allowed",
                        BluetoothPermission::NotDetermined => "permissions.mac.not_determined",
                        BluetoothPermission::Restricted => "permissions.mac.restricted",
                        BluetoothPermission::Denied => "permissions.mac.denied",
                        BluetoothPermission::Unknown => "common.status_unavailable",
                    },
                    Action::BluetoothPermissions,
                ),
            ];
            let mut section = self.section("settings.app.permissions", cx);
            for (id, title, state, action) in rows {
                section = section.child(
                    self.reveal_focus(
                        (ElementId::from("permission"), id),
                        div()
                            .h_flex()
                            .gap_4()
                            .child(
                                div()
                                    .v_flex()
                                    .flex_1()
                                    .gap_1()
                                    .child(self.text(self.tr(title)))
                                    .child(
                                        self.caption(self.tr(state), cx)
                                            .id((ElementId::from("permission-status"), id))
                                            .role(Role::Status)
                                            .test_support(),
                                    ),
                            )
                            .child(self.button(
                                (ElementId::from("permission-open"), id),
                                self.tr("common.open_settings"),
                                Command::App(action, String::new()),
                                false,
                                cx,
                            )),
                    ),
                );
            }
            section.child(self.caption(self.tr("permissions.mac.hint"), cx))
        };
        let notifications = self
            .section("settings.notifications", cx)
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
        let notifications = if self.ui.capabilities.notification_permission {
            use crate::platform::desktop::NotificationPermission;
            let status = match self.ui.notification_permission {
                NotificationPermission::Unknown => "common.status_unavailable",
                NotificationPermission::NotDetermined => {
                    "settings.notifications.mac.not_determined"
                }
                NotificationPermission::Denied => "permissions.mac.denied",
                NotificationPermission::Allowed => "permissions.mac.allowed",
                NotificationPermission::Quiet => "settings.notifications.mac.quiet",
                NotificationPermission::Unavailable => "settings.notifications.mac.unavailable",
            };
            notifications
                .child(
                    self.caption(self.tr(status), cx)
                        .id("notification-permission")
                        .role(Role::Status)
                        .test_support(),
                )
                .child(self.caption(self.tr("settings.notifications.mac.hint"), cx))
                .child(
                    self.reveal_focus(
                        "notification-actions",
                        div()
                            .h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(self.app_button(
                                "notification-settings",
                                "common.open_settings",
                                Action::NotificationSettings,
                                false,
                                cx,
                            ))
                            .child(self.app_button(
                                "test-notification",
                                "settings.notifications.mac.test",
                                Action::TestNotification,
                                !self.ui.notifications,
                                cx,
                            )),
                    ),
                )
        } else {
            notifications
        };
        let licensing = self
            .section("settings.licensing", cx)
            .child(self.text("GPL-3.0-only"))
            .child(
                self.reveal_focus(
                    "license-actions",
                    div()
                        .h_flex()
                        .gap_2()
                        .flex_wrap()
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
                ),
            );
        let about = self
            .section("settings.about", cx)
            .child(self.text(format!("TapRelay {}", self.ui.app_version)))
            .child(self.caption(self.ui.data_directory.clone(), cx))
            .child(
                self.reveal_focus(
                    "about-actions",
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
                        .child(self.app_button(
                            "wizard",
                            "settings.wizard",
                            Action::Wizard,
                            false,
                            cx,
                        )),
                ),
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

    fn section(&self, key: &'static str, cx: &App) -> GroupBox {
        GroupBox::new()
            .id(key)
            .fill()
            .text_sm()
            .content_style(self.card_style(cx))
            .title(
                self.text(self.tr(key))
                    .role(Role::Heading)
                    .aria_level(2)
                    .font_weight(FontWeight::MEDIUM),
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
    ) -> AnyElement {
        self.reveal_focus(
            (ElementId::from(id), "row"),
            div()
                .h_flex()
                .gap_4()
                .child(self.setting_text(label, hint, cx).flex_1().min_w_0())
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
                ),
        )
        .into_any_element()
    }

    fn setting_text(&self, label: &str, hint: &str, cx: &Context<Self>) -> Div {
        div()
            .v_flex()
            .gap_1()
            .child(self.text(self.tr(label)))
            .child(self.caption(self.tr(hint), cx).text_size(rems(0.8125)))
    }
}
