use super::ActionAlert;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{
    ActiveTheme, Disableable, Sizable, Size, StyledExt, Theme, ThemeMode,
    alert::{Alert, AlertVariant},
    button::Button,
    text::{Text, TextView},
};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Context, Entity, FocusHandle, IntoElement, Quad, Render,
    TestAppContext, Window, WindowBounds, WindowOptions, div, prelude::*, px, size,
};

#[derive(Clone, Copy)]
enum Content {
    Plain,
    Markdown,
    Html,
}

impl Content {
    fn text(self) -> Text {
        match self {
            Self::Plain => "Connection failed. 请检查连接后重试。".into(),
            Self::Markdown => TextView::markdown(
                "content",
                "**Connection failed**\n\n- Check device\n- 请重试",
            )
            .into(),
            Self::Html => TextView::html(
                "content",
                "<p><strong>Connection failed</strong></p><p>请重试</p>",
            )
            .into(),
        }
    }
}

struct Comparison {
    variant: AlertVariant,
    size: Size,
    banner: bool,
    title: bool,
    closable: bool,
    custom: bool,
    visible: bool,
    content: Content,
}

impl Default for Comparison {
    fn default() -> Self {
        Self {
            variant: AlertVariant::Default,
            size: Size::Medium,
            banner: false,
            title: false,
            closable: false,
            custom: false,
            visible: true,
            content: Content::Plain,
        }
    }
}

impl Render for Comparison {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        macro_rules! build {
            ($component:ident) => {{
                let message = self.content.text();
                match self.variant {
                    AlertVariant::Default => $component::new("alert", message),
                    AlertVariant::Info => $component::info("alert", message),
                    AlertVariant::Success => $component::success("alert", message),
                    AlertVariant::Warning => $component::warning("alert", message),
                    AlertVariant::Error => $component::error("alert", message),
                }
                .with_size(self.size)
                .visible(self.visible)
                .when(self.title, |alert| {
                    alert.title("Connection status 连接状态")
                })
                .when(self.banner, |alert| alert.banner())
                .when(self.custom, |alert| {
                    alert
                        .with_variant(AlertVariant::Success)
                        .icon(IconName::Download)
                        .p_6()
                        .border_2()
                        .rounded_xl()
                        .bg(cx.theme().secondary)
                        .text_color(cx.theme().primary)
                        .text_lg()
                })
                .when(self.closable, |alert| alert.on_close(|_, _, _| {}))
            }};
        }
        div()
            .size_full()
            .v_flex()
            .gap_4()
            .p_4()
            .child(
                div()
                    .id("native")
                    .flex_shrink_0()
                    .child(build!(Alert))
                    .test_support(),
            )
            .child(
                div()
                    .id("extended")
                    .flex_shrink_0()
                    .child(build!(ActionAlert))
                    .test_support(),
            )
    }
}

fn open<T: Render>(cx: &mut TestAppContext, view: T) -> (AnyWindowHandle, Entity<T>) {
    cx.update(gpui_kit::init);
    cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    size(px(640.), px(800.)),
                ))),
                ..Default::default()
            },
            cx,
            |_, cx| cx.new(|_| view),
        )
        .unwrap()
    })
}

fn surface(window: &Window, id: &'static str) -> Quad {
    let bounds = window.find(id).bounds().scale(window.scale_factor());
    window
        .painted_quads()
        .into_iter()
        .find(|quad| quad.bounds == bounds)
        .unwrap()
}

fn assert_matches_native(window: &Window) {
    assert_eq!(
        window.find("native").bounds().size,
        window.find("extended").bounds().size
    );
    let native = surface(window, "native");
    let extended = surface(window, "extended");
    assert_eq!(native.background, extended.background);
    assert_eq!(native.border_color, extended.border_color);
    assert_eq!(native.border_widths, extended.border_widths);
    assert_eq!(native.border_style, extended.border_style);
    assert_eq!(native.corner_radii, extended.corner_radii);
}

#[gpui_kit::test]
fn matches_native_variants_sizes_banner_and_rich_content_without_action(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, Comparison::default());
    cx.update_window(handle, |_, window, cx| {
        for mode in [ThemeMode::Light, ThemeMode::Dark] {
            Theme::change(mode, Some(window), cx);
            for variant in [
                AlertVariant::Default,
                AlertVariant::Info,
                AlertVariant::Success,
                AlertVariant::Warning,
                AlertVariant::Error,
            ] {
                for size in [
                    Size::XSmall,
                    Size::Small,
                    Size::Medium,
                    Size::Large,
                    Size::Size(px(40.)),
                ] {
                    for banner in [false, true] {
                        for content in [Content::Plain, Content::Markdown, Content::Html] {
                            view.update(cx, |view, cx| {
                                *view = Comparison {
                                    variant,
                                    size,
                                    banner,
                                    content,
                                    title: true,
                                    closable: false,
                                    ..Default::default()
                                };
                                cx.notify();
                            });
                            window.render_frame(cx);
                            assert_matches_native(window);
                        }
                    }
                }
            }
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn preserves_native_style_overrides_title_and_visibility(cx: &mut TestAppContext) {
    let (handle, view) = open(cx, Comparison::default());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_matches_native(window);
        for title in [false, true] {
            for banner in [false, true] {
                view.update(cx, |view, cx| {
                    view.title = title;
                    view.banner = banner;
                    view.custom = true;
                    cx.notify();
                });
                window.render_frame(cx);
                assert_matches_native(window);
            }
        }
        view.update(cx, |view, cx| {
            view.visible = false;
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(window.find("native").bounds().size.height, px(0.));
        assert_eq!(window.find("extended").bounds().size.height, px(0.));
        assert!(window.within("extended").try_find("alert").is_none());
    })
    .unwrap();
}

struct InteractiveAlert {
    focus: FocusHandle,
    banner: bool,
    disabled: bool,
    hidden: bool,
    actions: usize,
    closes: usize,
}

impl Render for InteractiveAlert {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div().track_focus(&self.focus).size_full().p_4().child(
            ActionAlert::warning(
                "alert",
                TextView::markdown(
                    "content",
                    "**Connection failed.**\n\nPlease retry. 请重试。",
                ),
            )
            .title("Connection")
            .icon(IconName::Bluetooth)
            .large()
            .when(self.banner, |alert| alert.banner())
            .visible(!self.hidden)
            .action(
                Button::new("action")
                    .label("Retry")
                    .disabled(self.disabled)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.actions += 1;
                        cx.notify();
                    })),
            )
            .on_close(cx.listener(|this, _, _, cx| {
                this.closes += 1;
                this.hidden = true;
                cx.notify();
            })),
        )
    }
}

#[gpui_kit::test]
fn action_and_close_remain_independent_in_normal_and_banner_modes(cx: &mut TestAppContext) {
    let focus = cx.update(|cx| cx.focus_handle());
    let (handle, view) = open(
        cx,
        InteractiveAlert {
            focus,
            banner: false,
            disabled: false,
            hidden: false,
            actions: 0,
            closes: 0,
        },
    );
    cx.update_window(handle, |_, window, cx| {
        for banner in [false, true] {
            view.update(cx, |view, cx| {
                view.banner = banner;
                view.disabled = true;
                view.hidden = false;
                view.actions = 0;
                view.closes = 0;
                view.focus.focus(window, cx);
                cx.notify();
            });
            window.render_frame(cx);
            let alert = window.find("alert").bounds();
            let action = window.within("alert").find("action").bounds();
            let close = window.within("alert").find("close").bounds();
            assert!(action.left() > alert.left() && action.right() < close.left());
            assert!(action.top() > alert.top() && action.bottom() < alert.bottom());
            window.click("action", cx);
            assert_eq!(view.read(cx).actions, 0);
            view.update(cx, |view, cx| {
                view.disabled = false;
                cx.notify();
            });
            window.render_frame(cx);
            window.click("action", cx);
            assert_eq!(view.read(cx).actions, 1);
            for _ in 0..4 {
                if window.find("action").focused() == Some(true) {
                    break;
                }
                window.press("tab", cx);
            }
            assert_eq!(window.find("action").focused(), Some(true));
            window.press("enter", cx);
            assert_eq!(view.read(cx).actions, 2);
            assert_eq!(view.read(cx).closes, 0);
            window.press("tab", cx);
            assert_eq!(window.find("close").focused(), Some(true));
            assert_eq!(window.find("close").label(), Some("Close alert"));
            window.press("enter", cx);
            assert_eq!(view.read(cx).closes, 1);
            assert!(window.try_find("alert").is_none());
            assert!(window.try_find("action").is_none());
        }
    })
    .unwrap();
}
