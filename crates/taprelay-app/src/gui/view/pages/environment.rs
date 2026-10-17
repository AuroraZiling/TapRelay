use crate::gui::action::Action;
use crate::gui::view::AppView;
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Icon, StyledExt,
    group_box::{GroupBox, GroupBoxVariants},
};
use gpui_kit::{prelude::*, *};

impl AppView {
    pub(in crate::gui::view) fn environment(&self, cx: &Context<Self>) -> AnyElement {
        let mut panel = GroupBox::new()
            .id("environment-checks")
            .fill()
            .content_style(self.card_style(cx));
        for (index, row) in self.ui.checks.iter().enumerate() {
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
                    .id(("environment-check", index))
                    .h_flex()
                    .gap_3()
                    .child(Icon::new(icon).size_4().text_color(color))
                    .child(self.text(row.title.clone()).flex_1())
                    .child(self.caption(self.tr(state), cx)),
            );
        }
        let mut body = div()
            .v_flex()
            .gap(rems(1.375))
            .pr_3()
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(self.heading(self.tr(if self.ui.mode == 3 {
                        "startup.error"
                    } else {
                        "startup.preparing"
                    })))
                    .child(self.caption(self.tr("startup.description"), cx)),
            )
            .child(panel);
        if !self.ui.problem.is_empty() {
            body = body
                .child(self.error(&self.ui.problem, cx))
                .when(self.ui.mode != 3, |body| {
                    body.child(self.caption(self.tr("startup.troubleshooting"), cx))
                });
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
}
