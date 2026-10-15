use crate::gui::view::AppView;
use gpui_kit::{prelude::*, *};

#[derive(IntoElement)]
pub(in crate::gui::view) struct FocusReveal {
    id: ElementId,
    content: AnyElement,
    scroll: ScrollHandle,
}

struct RevealState {
    region: FocusHandle,
    previous: Option<FocusHandle>,
}

impl AppView {
    pub(in crate::gui::view) fn reveal_focus(
        &self,
        id: impl Into<ElementId>,
        content: impl IntoElement,
    ) -> FocusReveal {
        FocusReveal {
            id: id.into(),
            content: content.into_any_element(),
            scroll: self.content_scroll.clone(),
        }
    }
}

impl RenderOnce for FocusReveal {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let state = window.use_keyed_state(self.id.clone(), cx, |_, cx| RevealState {
            region: cx.focus_handle(),
            previous: None,
        });
        let region = state.read(cx).region.clone();
        let observed_region = region.clone();
        div()
            .on_children_prepainted(move |bounds, window, cx| {
                let focused = window.focused(cx);
                let changed = state.update(cx, |state, _| {
                    let changed = state.previous != focused;
                    state.previous = focused;
                    changed
                });
                if !changed || !observed_region.contains_focused(window, cx) {
                    return;
                }
                let Some(bounds) = bounds.first() else { return };
                let viewport = self.scroll.bounds();
                let margin = window.rem_size() * 0.25;
                let mut offset = self.scroll.offset();
                if bounds.bottom() + margin > viewport.bottom() {
                    offset.y -= bounds.bottom() + margin - viewport.bottom();
                }
                if bounds.top() - margin < viewport.top() {
                    offset.y += viewport.top() - bounds.top() + margin;
                }
                if offset != self.scroll.offset() {
                    self.scroll.set_offset(offset);
                    window.refresh();
                }
            })
            .id(self.id)
            .w_full()
            .min_w_0()
            .track_focus(&region)
            .child(self.content)
    }
}
