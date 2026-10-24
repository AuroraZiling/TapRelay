use super::{LiveElement, LiveRegion};
use crate::gui::view::AppView;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{
    ActiveTheme, StyledExt,
    scroll::{ScrollableElement, Scrollbar, ScrollbarMode},
};
use gpui_kit::{prelude::*, *};
use std::panic::Location;

impl AppView {
    pub(in crate::gui::view) fn card_style(&self, cx: &App) -> StyleRefinement {
        let style = StyleRefinement::default().rounded(cx.theme().radius_lg);
        if cx.theme().is_dark() {
            style
        } else {
            style.border_1().border_color(cx.theme().border)
        }
    }

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
        let scroller = div()
            .id(id)
            .flex_1()
            .min_h_0()
            .min_w_0()
            .when(id == "mapping-editor", |scroller| {
                scroller.mx(rems(-0.25)).px_1()
            })
            .overflow_y_scroll()
            .track_scroll(&self.content_scroll)
            .child(
                div()
                    .v_flex()
                    .gap_4()
                    .when(id == "bindings", |content| content.pr_3())
                    .child(content),
            );
        let viewport = div()
            .relative()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .child(scroller.size_full().test_support());
        if id == "bindings" {
            viewport
                .child(
                    div().absolute().inset_0().child(
                        Scrollbar::vertical(&self.content_scroll)
                            .id("bindings-scrollbar")
                            .mode(ScrollbarMode::Always)
                            .viewport_from_layout(),
                    ),
                )
                .into_any_element()
        } else {
            viewport
                .vertical_scrollbar(&self.content_scroll)
                .into_any_element()
        }
    }
}
