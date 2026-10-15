use super::{LiveElement, LiveRegion};
use crate::gui::view::AppView;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{ActiveTheme, StyledExt, scroll::ScrollableElement};
use gpui_kit::{prelude::*, *};
use std::panic::Location;

impl AppView {
    #[track_caller]
    pub(in crate::gui::view) fn text(&self, text: impl Into<SharedString>) -> Stateful<Div> {
        let text = text.into();
        div()
            .id(ElementId::CodeLocation(*Location::caller()))
            .role(Role::Label)
            .aria_label(text.clone())
            .child(text)
    }

    #[track_caller]
    pub(in crate::gui::view) fn caption(
        &self,
        text: impl Into<SharedString>,
        cx: &App,
    ) -> Stateful<Div> {
        self.text(text)
            .text_sm()
            .text_color(cx.theme().muted_foreground)
    }

    #[track_caller]
    pub(in crate::gui::view) fn error(&self, text: &str, cx: &App) -> LiveRegion<Stateful<Div>> {
        self.text(text.to_owned())
            .role(Role::Alert)
            .text_color(cx.theme().danger)
            .live(accesskit::Live::Assertive)
    }

    #[track_caller]
    pub(in crate::gui::view) fn heading(&self, text: String) -> Stateful<Div> {
        self.text(text).role(Role::Heading).aria_level(1).text_xl()
    }

    pub(in crate::gui::view) fn scroller(
        &self,
        id: &'static str,
        content: impl IntoElement,
    ) -> AnyElement {
        if self.scroll_region.replace(id) != id {
            self.content_scroll.set_offset(Point::default());
        }
        div()
            .id(id)
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_y_scroll()
            .track_scroll(&self.content_scroll)
            .child(div().v_flex().gap_4().child(content))
            .vertical_scrollbar(&self.content_scroll)
            .test_support()
            .into_any_element()
    }
}
