use crate::gui::view::AppView;
use gpui_kit::component::{ActiveTheme, StyledExt};
use gpui_kit::{prelude::*, *};

impl AppView {
    pub(in crate::gui::view) fn status_indicator(
        &self,
        id: &'static str,
        text: impl Into<SharedString>,
        on_position: impl Fn(Point<Pixels>) + 'static,
        cx: &App,
    ) -> Stateful<Div> {
        let text = text.into();
        let signal = self.signal(cx);
        let dot = canvas(
            move |bounds, _, _| on_position(bounds.center()),
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
        div()
            .id(id)
            .h_flex()
            .text_sm()
            .gap_3()
            .min_w_0()
            .role(Role::Status)
            .aria_label(text.clone())
            .child(dot)
            .child(div().min_w_0().child(text))
    }

    pub(in crate::gui::view) fn signal(&self, cx: &App) -> Hsla {
        match self.ui.state_color {
            0 => cx.theme().success,
            1 => cx.theme().muted_foreground,
            2 => cx.theme().warning,
            _ => cx.theme().danger,
        }
    }
}
