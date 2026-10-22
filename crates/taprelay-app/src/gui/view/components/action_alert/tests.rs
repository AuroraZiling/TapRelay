use super::ActionAlert;
use gpui_kit::assets::IconName;
use gpui_kit::component::{Disableable, Sizable, button::Button, text::TextView};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Context, Entity, FocusHandle, IntoElement, Render,
    TestAppContext, Window, WindowBounds, WindowOptions, div, prelude::*, px, size,
};

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
