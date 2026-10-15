use crate::gui::action::Action;
use crate::gui::view::components::ActionAlert;
use crate::gui::view::{AppView, Command};
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{
    Sizable, StyledExt,
    button::ButtonVariants,
    group_box::{GroupBox, GroupBoxVariants},
    tag::Tag,
};
use gpui_kit::{prelude::*, *};

impl AppView {
    pub(in crate::gui::view) fn overview(
        &self,
        window: &mut Window,
        cx: &Context<Self>,
    ) -> AnyElement {
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
                    .role(Role::Label)
                    .aria_label(format!("{}: {}", binding.function_name, binding.text))
                    .tooltip({
                        let label = binding.function_name.clone();
                        move |window, cx| {
                            gpui_kit::component::tooltip::Tooltip::new(label.clone())
                                .build(window, cx)
                        }
                    })
                    .child(
                        Tag::secondary()
                            .outline()
                            .large()
                            .child(binding.text.clone()),
                    )
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
                            .content_style(self.card_style(cx).flex_1().min_h_0())
                            .child(
                                div()
                                    .h_flex()
                                    .justify_between()
                                    .gap_4()
                                    .child(self.caption(self.tr("overview.bindings"), cx))
                                    .child(
                                        self.icon_button(
                                            "manage-bindings",
                                            IconName::Settings,
                                            self.tr("overview.manage"),
                                            Command::App(Action::Navigate, "1".into()),
                                            false,
                                            cx,
                                        )
                                        .ghost(),
                                    ),
                            )
                            .child(self.scroller("overview-shortcuts", shortcuts)),
                    )
                    .test_support(),
            )
            .into_any_element()
    }
}
