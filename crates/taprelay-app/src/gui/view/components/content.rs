use crate::gui::view::AppView;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{ActiveTheme, StyledExt, scroll::ScrollableElement};
use gpui_kit::{prelude::*, *};

impl AppView {
    pub(in crate::gui::view) fn caption(&self, text: impl Into<SharedString>, cx: &App) -> Div {
        div()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(text.into())
    }

    pub(in crate::gui::view) fn error(&self, text: &str, cx: &App) -> Div {
        div().text_color(cx.theme().danger).child(text.to_owned())
    }

    pub(in crate::gui::view) fn heading(&self, text: String) -> Div {
        div().text_xl().child(text)
    }

    pub(in crate::gui::view) fn scroller(
        &self,
        id: &'static str,
        content: impl IntoElement,
    ) -> AnyElement {
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
}
