use super::support::{open, settle};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, px};
use std::time::{Duration, Instant};

#[gpui_kit::test]
fn receiver_retry_icon_keeps_its_label_disabled_state_and_keyboard_action(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        for (busy, keyboard) in [(true, false), (false, false), (false, true)] {
            let started = Instant::now() - Duration::from_secs(3);
            view.update(cx, |view, cx| {
                view.controller.started = started;
                view.ui.state_color = 2;
                view.ui.problem_busy = busy;
                view.focus.focus(window, cx);
                cx.notify();
            });
            window.render_frame(cx);
            let retry = window.find("retry-receiver");
            assert_eq!(
                retry.label(),
                Some(view.read(cx).tr("common.retry").as_str())
            );
            assert_eq!(retry.bounds().size.width, px(32.));
            assert_eq!(retry.bounds().size.height, px(32.));
            if keyboard {
                for _ in 0..16 {
                    if window.find("retry-receiver").focused() == Some(true) {
                        break;
                    }
                    window.press("tab", cx);
                }
                assert_eq!(window.find("retry-receiver").focused(), Some(true));
                window.press("enter", cx);
            } else {
                window.click("retry-receiver", cx);
            }
            assert_eq!(view.read(cx).controller.started == started, busy);
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn overview_listener_control_tracks_runtime_and_rejects_testing_without_receiver(
    cx: &mut TestAppContext,
) {
    let (handle, view, _folder) = open(cx);
    for listening in [true, false] {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("listen", cx);
        })
        .unwrap();
        settle(cx, handle, &view);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(view.read(cx).controller.runtime.listening, listening);
            assert_eq!(
                window.find("listen").label(),
                Some(
                    view.read(cx)
                        .tr(if listening {
                            "listening.stop"
                        } else {
                            "listening.start"
                        })
                        .as_str()
                )
            );
            window.click("test-receiver", cx);
            assert!(matches!(
                view.read(cx).controller.runtime.test,
                crate::runtime::TestStatus::Idle
            ));
        })
        .unwrap();
        settle(cx, handle, &view);
        cx.update(|cx| {
            assert!(matches!(
                view.read(cx).controller.runtime.test,
                crate::runtime::TestStatus::Idle
            ))
        });
    }
}

#[gpui_kit::test]
fn overview_group_box_scrolls_long_shortcuts_without_moving_actions(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.ui.elevated = true;
            view.ui.bindings = (0..60)
                .map(|index| crate::gui::presentation::BindingRow {
                    text: format!("Ctrl + Shift + Shortcut {index}"),
                    function_name: format!("Action {index}"),
                    enabled: true,
                    ..Default::default()
                })
                .collect();
            cx.notify();
        });
        window.render_frame(cx);
        let first = "summary-Action 0-Ctrl + Shift + Shortcut 0";
        let last = "summary-Action 59-Ctrl + Shift + Shortcut 59";
        let card = window.find("shortcuts-card").bounds();
        let action = window.find("manage-bindings").bounds();
        assert!(window.find(first).visible());
        assert!(window.find(first).bounds().size.height > px(25.));
        assert!(!window.find(last).visible());
        window.scroll(
            first,
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-5000.))),
            cx,
        );
        assert!(!window.find(first).visible());
        assert!(window.find(last).visible());
        assert!(window.find(last).bounds().is_contained_within(&card));
        assert_eq!(window.find("shortcuts-card").bounds(), card);
        assert_eq!(window.find("manage-bindings").bounds(), action);
        window.click("manage-bindings", cx);
        assert_eq!(view.read(cx).ui.page, 1);
    })
    .unwrap();
}

#[gpui_kit::test]
fn homepage_ripple_tracks_inputs_and_clears_when_page_is_hidden(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.ui.page = 0;
            view.effects(window, cx);
            view.ui.input_count += 1;
            view.effects(window, cx);
            assert_eq!(view.ripples.frames(Instant::now()).len(), 1);
            cx.notify();
        });
        window.render_frame(cx);
        view.update(cx, |view, cx| {
            view.ui.page = 1;
            view.effects(window, cx);
            assert!(view.ripples.frames(Instant::now()).is_empty());
            view.ui.input_count += 10;
            view.ui.page = 0;
            view.effects(window, cx);
            assert!(view.ripples.frames(Instant::now()).is_empty());
            cx.set_reduce_motion(true);
            view.ui.input_count += 1;
            view.effects(window, cx);
            assert!(view.ripples.frames(Instant::now()).is_empty());
        });
    })
    .unwrap();
}
