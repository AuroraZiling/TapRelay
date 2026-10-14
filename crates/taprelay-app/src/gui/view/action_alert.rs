use std::rc::Rc;

use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{
    alert::AlertVariant,
    button::{Button, ButtonVariants as _},
};

use gpui_kit::{
    App, ClickEvent, ElementId, Empty, InteractiveElement, IntoElement, ParentElement as _,
    RenderOnce, Role, SharedString, StatefulInteractiveElement, StyleRefinement, Styled, Window,
    div, prelude::FluentBuilder as _, px, rems, transparent_white,
};

use gpui_kit::component::{
    ActiveTheme as _, Colorize, Icon, IconName, Sizable, Size, StyledExt, h_flex,
    text::{Text, TextViewStyle},
};

type CloseHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App) + 'static>;

#[derive(IntoElement)]
pub(super) struct ActionAlert {
    id: ElementId,
    style: StyleRefinement,
    variant: AlertVariant,
    icon: Icon,
    title: Option<SharedString>,
    message: Text,
    size: Size,
    banner: bool,
    on_close: Option<CloseHandler>,
    visible: bool,
    action: Option<Button>,
}

#[allow(
    dead_code,
    reason = "Preserve the complete upstream Alert builder API."
)]
impl ActionAlert {
    pub fn new(id: impl Into<ElementId>, message: impl Into<Text>) -> Self {
        Self {
            id: id.into(),
            style: StyleRefinement::default(),
            variant: AlertVariant::default(),
            icon: Icon::new(IconName::Info),
            title: None,
            message: message.into(),
            size: Size::default(),
            banner: false,
            visible: true,
            action: None,
            on_close: None,
        }
    }

    pub fn info(id: impl Into<ElementId>, message: impl Into<Text>) -> Self {
        Self::new(id, message)
            .with_variant(AlertVariant::Info)
            .icon(IconName::Info)
    }

    pub fn success(id: impl Into<ElementId>, message: impl Into<Text>) -> Self {
        Self::new(id, message)
            .with_variant(AlertVariant::Success)
            .icon(IconName::CircleCheck)
    }

    pub fn warning(id: impl Into<ElementId>, message: impl Into<Text>) -> Self {
        Self::new(id, message)
            .with_variant(AlertVariant::Warning)
            .icon(IconName::TriangleAlert)
    }

    pub fn error(id: impl Into<ElementId>, message: impl Into<Text>) -> Self {
        Self::new(id, message)
            .with_variant(AlertVariant::Error)
            .icon(IconName::CircleX)
    }

    pub fn with_variant(mut self, variant: AlertVariant) -> Self {
        self.variant = variant;
        self
    }

    pub fn icon(mut self, icon: impl Into<Icon>) -> Self {
        self.icon = icon.into();
        self
    }

    pub fn title(mut self, title: impl Into<SharedString>) -> Self {
        self.title = Some(title.into());
        self
    }

    pub fn banner(mut self) -> Self {
        self.banner = true;
        self
    }

    pub fn visible(mut self, visible: bool) -> Self {
        self.visible = visible;
        self
    }

    pub fn on_close(
        mut self,
        on_close: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.on_close = Some(Rc::new(on_close));
        self
    }
    pub fn action(mut self, action: Button) -> Self {
        self.action = Some(action.ghost());
        self
    }
}

impl Sizable for ActionAlert {
    fn with_size(mut self, size: impl Into<Size>) -> Self {
        self.size = size.into();
        self
    }
}

impl Styled for ActionAlert {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}

impl RenderOnce for ActionAlert {
    fn render(self, _: &mut Window, cx: &mut App) -> impl IntoElement {
        if !self.visible {
            return Empty.into_any_element();
        }

        let (radius, padding_x, padding_y, gap) = match self.size {
            Size::XSmall => (cx.theme().radius, px(12.), px(6.), px(6.)),
            Size::Small => (cx.theme().radius, px(12.), px(8.), px(6.)),
            Size::Large => (cx.theme().radius_lg, px(20.), px(14.), px(12.)),
            _ => (cx.theme().radius, px(16.), px(10.), px(12.)),
        };

        let fg = match self.variant {
            AlertVariant::Default => cx.theme().foreground,
            AlertVariant::Info => cx.theme().info,
            AlertVariant::Success => cx.theme().success,
            AlertVariant::Warning => cx.theme().warning,
            AlertVariant::Error => cx.theme().danger,
        };
        let (bg, border_color) = if self.variant == AlertVariant::Default {
            (cx.theme().background, cx.theme().border)
        } else {
            (
                fg.mix_oklab(transparent_white(), 0.04),
                fg.mix_oklab(transparent_white(), 0.3),
            )
        };

        h_flex()
            .id(self.id)
            .role(Role::Alert)
            .w_full()
            .text_color(fg)
            .bg(bg)
            .px(padding_x)
            .py(padding_y)
            .gap(gap)
            .justify_between()
            .text_sm()
            .border_1()
            .border_color(border_color)
            .when(!self.banner, |this| this.rounded(radius).items_start())
            .refine_style(&self.style)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .when(self.action.is_some(), |this| this.self_center())
                    .when(self.banner, |this| this.items_center())
                    .overflow_hidden()
                    .gap(gap)
                    .child(
                        div()
                            .when(!self.banner, |this| this.mt(px(5.)))
                            .child(self.icon),
                    )
                    .child(
                        div()
                            .flex_1()
                            .overflow_hidden()
                            .gap_3()
                            .when(!self.banner, |this| {
                                this.when_some(self.title, |this, title| {
                                    this.child(
                                        div().w_full().truncate().font_semibold().child(title),
                                    )
                                })
                            })
                            .child(
                                self.message
                                    .style(TextViewStyle::default().paragraph_gap(rems(0.2))),
                            ),
                    ),
            )
            .when_some(self.action, |this, action| {
                this.child(div().flex_shrink_0().self_center().child(action))
            })
            .when_some(self.on_close, |this, on_close| {
                this.child(
                    div()
                        .id("close")
                        .test_support()
                        .p_0p5()
                        .rounded(cx.theme().radius)
                        .hover(|this| this.bg(bg.opacity(0.8)))
                        .active(|this| this.bg(bg.opacity(0.9)))
                        .on_click(move |ev, window, cx| {
                            on_close(ev, window, cx);
                        })
                        .child(
                            Icon::new(IconName::Close)
                                .with_size(self.size.max(Size::Medium))
                                .flex_shrink_0(),
                        ),
                )
            })
            .test_support()
            .into_any_element()
    }
}

#[cfg(test)]
mod tests;
