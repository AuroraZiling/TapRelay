use crate::gui::action::Action;
use crate::gui::{
    presentation::ChoiceOption,
    view::{AppView, Command},
};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{
    Selectable, StyledExt,
    button::ButtonVariants,
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
        let autostart_status = self
            .ui
            .capabilities
            .login_approval
            .then_some(self.ui.auto_start_status)
            .flatten()
            .filter(|status| {
                matches!(
                    *status,
                    "settings.autostart.mac.approval"
                        | "settings.autostart.mac.not_found"
                        | "common.status_unavailable"
                )
            });
        let autostart_description = self
            .caption(
                self.tr(autostart_status.unwrap_or("settings.autostart.hint")),
                cx,
            )
            .text_size(rems(0.8125));
        let autostart_description = if autostart_status.is_some() {
            autostart_description
                .id("autostart-status")
                .role(Role::Status)
                .test_support()
                .into_any_element()
        } else {
            autostart_description.into_any_element()
        };
        let mut autostart_controls =
            div()
                .h_flex()
                .gap_2()
                .flex_shrink_0()
                .child(self.setting_switch(
                    "autostart",
                    "settings.autostart",
                    self.ui.auto_start,
                    Action::SetAutostart,
                    cx,
                ));
        if self.ui.capabilities.login_approval {
            autostart_controls = autostart_controls.child(
                self.icon_button(
                    "login-settings",
                    IconName::Settings,
                    self.tr("settings.autostart.mac.settings"),
                    Command::App(Action::LoginSettings, String::new()),
                    false,
                    cx,
                )
                .ghost(),
            );
        }
        let startup = self
            .section("settings.startup", cx)
            .child(
                self.setting_row(
                    "autostart",
                    div()
                        .v_flex()
                        .gap_1()
                        .child(self.text(self.tr("settings.autostart")))
                        .child(autostart_description),
                    autostart_controls,
                ),
            )
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
                                div()
                                    .h_flex()
                                    .gap_2()
                                    .child(self.text(self.tr("permissions.current")).text_sm())
                                    .child(
                                        permission_tag
                                            .gap_1()
                                            .flex_shrink_0()
                                            .child(Icon::new(permission_icon).size_3p5())
                                            .child(self.text(self.tr(permission_label))),
                                    ),
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
                    ("permissions.mac.allowed", Tag::success())
                } else {
                    ("permissions.mac.required", Tag::warning())
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
                        BluetoothPermission::Allowed => allowed(true),
                        BluetoothPermission::NotDetermined => {
                            ("permissions.mac.not_determined", Tag::warning())
                        }
                        BluetoothPermission::Restricted => {
                            ("permissions.mac.restricted", Tag::danger())
                        }
                        BluetoothPermission::Denied => ("permissions.mac.denied", Tag::danger()),
                        BluetoothPermission::Unknown => {
                            ("common.status_unavailable", Tag::secondary())
                        }
                    },
                    Action::BluetoothPermissions,
                ),
            ];
            let mut section = self.section("settings.app.permissions", cx);
            for (id, title, (state, tag), action) in rows {
                section = section.child(
                    self.reveal_focus(
                        (ElementId::from("permission"), id),
                        div()
                            .h_flex()
                            .gap_4()
                            .child(self.text(self.tr(title)).flex_1().min_w_0())
                            .child(
                                div()
                                    .h_flex()
                                    .min_w_0()
                                    .gap_2()
                                    .child(
                                        tag.outline().max_w_full().child(
                                            self.text(self.tr(state))
                                                .id((ElementId::from("permission-status"), id))
                                                .min_w_0()
                                                .role(Role::Status)
                                                .test_support(),
                                        ),
                                    )
                                    .child(
                                        self.icon_button(
                                            (ElementId::from("permission-open"), id),
                                            IconName::Settings,
                                            self.tr("common.open_settings"),
                                            Command::App(action, String::new()),
                                            false,
                                            cx,
                                        )
                                        .ghost()
                                        .flex_shrink_0(),
                                    ),
                            ),
                    ),
                );
            }
            section.child(self.caption(self.tr("permissions.mac.hint"), cx))
        };
        let mut notification_controls = div().h_flex().gap_2().flex_shrink_0();
        if self.ui.capabilities.notification_permission {
            use crate::platform::desktop::NotificationPermission;
            let (status, tag) = match self.ui.notification_permission {
                NotificationPermission::Unknown => ("common.status_unavailable", Tag::secondary()),
                NotificationPermission::NotDetermined => {
                    ("settings.notifications.mac.not_determined", Tag::warning())
                }
                NotificationPermission::Denied => ("permissions.mac.denied", Tag::danger()),
                NotificationPermission::Allowed => ("permissions.mac.allowed", Tag::success()),
                NotificationPermission::Quiet => ("settings.notifications.mac.quiet", Tag::info()),
                NotificationPermission::Unavailable => {
                    ("settings.notifications.mac.unavailable", Tag::secondary())
                }
            };
            notification_controls = notification_controls.child(
                tag.outline().child(
                    self.text(self.tr(status))
                        .id("notification-permission")
                        .role(Role::Status)
                        .test_support(),
                ),
            );
        }
        notification_controls = notification_controls.child(self.setting_switch(
            "notifications",
            "settings.notifications.toggle",
            self.ui.notifications,
            Action::SetNotifications,
            cx,
        ));
        if self.ui.capabilities.notification_permission {
            notification_controls = notification_controls.child(
                self.icon_button(
                    "notification-settings",
                    IconName::Settings,
                    self.tr("settings.notifications.mac.settings"),
                    Command::App(Action::NotificationSettings, String::new()),
                    false,
                    cx,
                )
                .ghost(),
            );
        }
        let notifications = self
            .section("settings.notifications", cx)
            .child(self.setting_row(
                "notifications",
                self.setting_text(
                    "settings.notifications.toggle",
                    "settings.notifications.hint",
                    cx,
                ),
                notification_controls,
            ))
            .child(self.setting(
                "wait-warning",
                "settings.connection.wait.warning",
                "settings.connection.wait.warning.hint",
                self.ui.connection_wait_warning,
                Action::SetConnectionWaitWarning,
                cx,
            ));
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
        self.setting_row(
            id,
            self.setting_text(label, hint, cx),
            self.setting_switch(id, label, value, action, cx),
        )
    }

    fn setting_row(&self, id: &'static str, text: Div, controls: impl IntoElement) -> AnyElement {
        self.reveal_focus(
            (ElementId::from(id), "row"),
            div()
                .h_flex()
                .gap_4()
                .child(text.flex_1().min_w_0())
                .child(controls),
        )
        .into_any_element()
    }

    fn setting_switch(
        &self,
        id: &'static str,
        label: &str,
        value: bool,
        action: Action,
        cx: &Context<Self>,
    ) -> Switch {
        Switch::new(id)
            .accessibility_label(self.tr(label))
            .checked(value)
            .on_change(cx.listener(move |this, checked: &bool, window, cx| {
                this.dispatch(
                    Command::App(action, if *checked { "1" } else { "0" }.into()),
                    window,
                    cx,
                )
            }))
    }

    fn setting_text(&self, label: &str, hint: &str, cx: &Context<Self>) -> Div {
        div()
            .v_flex()
            .gap_1()
            .child(self.text(self.tr(label)))
            .child(self.caption(self.tr(hint), cx).text_size(rems(0.8125)))
    }
}
