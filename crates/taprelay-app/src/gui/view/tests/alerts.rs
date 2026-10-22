use super::support::{open, settle};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};
use std::time::{Duration, Instant};

#[gpui_kit::test]
fn alert_retry_respects_disabled_state_and_accepts_pointer_and_keyboard(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        for (busy, keyboard) in [(true, false), (false, false), (false, true)] {
            let started = Instant::now() - Duration::from_secs(3);
            view.update(cx, |view, cx| {
                view.controller.started = started;
                view.ui.mode = 2;
                view.ui.page = 0;
                view.ui.problem = "Connection failed".into();
                view.ui.problem_busy = busy;
                view.ui.problem_retry_allowed = true;
                view.focus.focus(window, cx);
                cx.notify();
            });
            window.render_frame(cx);
            if keyboard {
                for _ in 0..16 {
                    if window.find("overview-retry").focused() == Some(true) {
                        break;
                    }
                    window.press("tab", cx);
                }
                assert_eq!(window.find("overview-retry").focused(), Some(true));
                window.press("enter", cx);
            } else {
                window.click("overview-retry", cx);
            }
            assert_eq!(view.read(cx).controller.started == started, busy);
        }
    })
    .unwrap();
}
