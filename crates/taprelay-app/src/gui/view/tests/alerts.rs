use super::support::{open, open_sized, settle};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, px};
use std::time::{Duration, Instant};

#[gpui_kit::test]
fn alerts_keep_actions_inside_with_long_messages_and_both_themes(cx: &mut TestAppContext) {
    for (width, height) in [(900., 500.), (1100., 800.)] {
        let (handle, view, _folder) = open_sized(cx, width, height);
        cx.update_window(handle, |_, window, cx| {
            for dark in [false, true] {
                for locale in ["en", "zh-CN"] {
                    view.update(cx, |view, cx| {
                        view.controller.runtime.config.options.language =
                            crate::config::Language::from_name(locale).unwrap();
                        view.ui.page = 0;
                        view.ui.dark = dark;
                        view.ui.locale = locale.into();
                        view.ui.elevated = false;
                        view.ui.problem = format!("{} ", view.tr("permissions.warning")).repeat(4);
                        view.ui.problem_retry_allowed = true;
                        view.effects(window, cx);
                        cx.notify();
                    });
                    window.render_frame(cx);
                    for (alert_id, action_id) in [
                        ("admin-alert", "elevate"),
                        ("overview-alert", "overview-retry"),
                    ] {
                        let alert = window.find(alert_id).bounds();
                        let action = window.within(alert_id).find(action_id).bounds();
                        assert!(action.size.width > px(0.));
                        assert!(action.left() > alert.left());
                        assert!(action.right() < alert.right());
                        assert!(action.top() > alert.top());
                        assert!(action.bottom() < alert.bottom());
                        assert!(alert.right() <= px(width - 28.));
                    }
                    view.update(cx, |view, cx| {
                        view.ui.problem_retry_allowed = false;
                        cx.notify();
                    });
                    window.render_frame(cx);
                    assert!(window.try_find("overview-retry").is_none());
                    assert!(window.find("overview-alert").visible());
                }
            }
        })
        .unwrap();
    }
}

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
