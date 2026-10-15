use crate::gui::action::Action;
use crate::gui::view::AppView;
use gpui_kit::component::{ActiveTheme, StyledExt};
use gpui_kit::{prelude::*, *};

impl AppView {
    pub(in crate::gui::view) fn wizard(
        &self,
        window: &mut Window,
        cx: &Context<Self>,
    ) -> AnyElement {
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
            .child(
                div()
                    .v_flex()
                    .gap_1()
                    .child(header)
                    .when(step < 2, |header| {
                        header.child(self.caption(
                            if step == 0 {
                                self.tr("bindings.help")
                            } else {
                                self.ui.discovery_status.clone()
                            },
                            cx,
                        ))
                    }),
            );
        if step == 2 && !self.ui.elevated {
            body = body.child(self.admin_alert(cx));
        }
        body = body.child(match step {
            0 => self.bindings(window, cx),
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
}
