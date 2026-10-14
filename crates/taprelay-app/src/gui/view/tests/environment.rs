use super::support::{open, settle};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};
use std::time::{Duration, Instant};

#[gpui_kit::test]
fn environment_retry_waits_for_startup_checks(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        for busy in [true, false] {
            let started = Instant::now() - Duration::from_secs(1);
            view.update(cx, |view, cx| {
                view.controller.started = started;
                view.ui.mode = 0;
                view.ui.busy = busy;
                view.ui.problem_busy = false;
                view.ui.problem_retry_allowed = true;
                cx.notify();
            });
            window.render_frame(cx);
            window.click("retry-checks", cx);
            assert_eq!(view.read(cx).controller.started == started, busy);
        }
    })
    .unwrap();
}
