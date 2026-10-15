use crate::action::Action;
use crate::gui::view::AppView;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{
    StyledExt,
    button::ButtonVariants,
    group_box::{GroupBox, GroupBoxVariants},
};
use gpui_kit::{prelude::*, *};
use std::time::Instant;

impl AppView {
    pub(in crate::gui::view) fn status_card(
        &self,
        window: &mut Window,
        cx: &Context<Self>,
    ) -> GroupBox {
        let frames = self.ripples.frames(Instant::now());
        if !frames.is_empty() {
            window.request_animation_frame();
        }
        let center = std::rc::Rc::new(std::cell::Cell::new(point(px(0.), px(0.))));
        let anchor = center.clone();
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
                        self.status_indicator(
                            "receiver-status-indicator",
                            self.ui.stage.clone(),
                            move |position| anchor.set(position),
                            cx,
                        )
                        .test_support(),
                    )
                    .child(
                        div()
                            .h_flex()
                            .gap_3()
                            .pt_1()
                            .flex_wrap()
                            .child(
                                self.app_button(
                                    "listen",
                                    if self.ui.listening {
                                        "listening.stop"
                                    } else {
                                        "listening.start"
                                    },
                                    Action::Listen,
                                    self.ui.busy,
                                    cx,
                                )
                                .primary(),
                            )
                            .child(
                                self.app_button(
                                    "test-receiver",
                                    "receiver.test",
                                    Action::Test,
                                    !self.ui.ready,
                                    cx,
                                )
                                .secondary(),
                            )
                            .when(self.ui.state_color != 0, |row| {
                                row.child(
                                    self.app_button(
                                        "retry-receiver",
                                        "common.retry",
                                        Action::Retry,
                                        self.ui.problem_busy,
                                        cx,
                                    )
                                    .ghost(),
                                )
                            }),
                    ),
            )
    }
}
