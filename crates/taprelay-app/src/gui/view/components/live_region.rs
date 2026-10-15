use gpui_kit::{
    accesskit::{Live, Node},
    prelude::*,
    *,
};

// GPUI's role builder does not set AccessKit's live property.
pub(in crate::gui::view) struct LiveRegion<E> {
    element: E,
    live: Live,
}

pub(in crate::gui::view) trait LiveElement: Element + Sized {
    fn live(self, live: Live) -> LiveRegion<Self> {
        LiveRegion {
            element: self,
            live,
        }
    }
}
impl<E: Element> LiveElement for E {}

impl<E: Element> IntoElement for LiveRegion<E> {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl<E: InteractiveElement> InteractiveElement for LiveRegion<E> {
    fn interactivity(&mut self) -> &mut Interactivity {
        self.element.interactivity()
    }
}
impl<E: StatefulInteractiveElement> StatefulInteractiveElement for LiveRegion<E> {}
impl<E: Styled> Styled for LiveRegion<E> {
    fn style(&mut self) -> &mut StyleRefinement {
        self.element.style()
    }
}
impl<E: ParentElement> ParentElement for LiveRegion<E> {
    fn extend(&mut self, elements: impl IntoIterator<Item = AnyElement>) {
        self.element.extend(elements);
    }
}

impl<E: Element> Element for LiveRegion<E> {
    type RequestLayoutState = E::RequestLayoutState;
    type PrepaintState = E::PrepaintState;
    fn id(&self) -> Option<ElementId> {
        self.element.id()
    }
    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        self.element.source_location()
    }
    fn a11y_role(&self) -> Option<Role> {
        self.element.a11y_role()
    }
    fn write_a11y_info(&self, node: &mut Node) {
        self.element.write_a11y_info(node);
        node.set_live(self.live);
    }
    fn a11y_synthetic_children(
        &mut self,
        prepaint: &mut Self::PrepaintState,
        builder: &mut A11ySubtreeBuilder,
    ) {
        self.element.a11y_synthetic_children(prepaint, builder);
    }
    fn request_layout(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        self.element.request_layout(id, inspector, window, cx)
    }
    fn prepaint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        self.element
            .prepaint(id, inspector, bounds, layout, window, cx)
    }
    fn paint(
        &mut self,
        id: Option<&GlobalElementId>,
        inspector: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        self.element
            .paint(id, inspector, bounds, layout, prepaint, window, cx);
    }
}
