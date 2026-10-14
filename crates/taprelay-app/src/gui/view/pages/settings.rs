use crate::action::Action;
use crate::gui::view::{AppView, Command};
use gpui_kit::component::{
    Selectable, StyledExt,
    group_box::{GroupBox, GroupBoxVariants},
    switch::Switch,
};
use gpui_kit::{prelude::*, *};

impl AppView {
    pub(in crate::gui::view) fn settings(&self, cx: &Context<Self>) -> AnyElement {
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

    fn section(&self, key: &'static str) -> GroupBox {
        GroupBox::new().id(key).fill().title(self.tr(key))
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
