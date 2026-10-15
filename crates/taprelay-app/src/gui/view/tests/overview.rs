use super::support::{open, open_sized, settle};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, px};
use std::time::Instant;

#[gpui_kit::test]
fn preserves_overview_proportions_in_both_themes_and_window_sizes(cx: &mut TestAppContext) {
    for (width, height, scale) in [
        (900., 500., 1.),
        (900., 500., 1.25),
        (1100., 800., 1.5),
        (1100., 800., 2.),
    ] {
        let (handle, view, _folder) = open_sized(cx, width, height);
        cx.simulate_window_scale_factor_change(handle, scale);
        cx.update_window(handle, |_, window, cx| {
            for dark in [false, true] {
                for (locale, language) in [("en", "english"), ("zh-CN", "chinese")] {
                    view.update(cx, |view, cx| {
                        view.controller.runtime.config.options.language =
                            crate::config::Language::from_name(language).unwrap();
                        view.ui.page = 0;
                        view.ui.dark = dark;
                        view.ui.locale = locale.into();
                        view.ui.elevated = true;
                        view.effects(window, cx);
                        cx.notify();
                    });
                    window.render_frame(cx);
                    let bar = window.find("title-bar").bounds();
                    let nav = window.find("navigation").bounds();
                    let header = window.find("page-header").bounds();
                    let status = window.find("status-card").bounds();
                    let shortcuts = window.find("shortcuts-card").bounds();
                    assert_eq!(nav.size.width, px(176.));
                    assert_eq!(nav.size.height, px(height) - bar.size.height);
                    assert_eq!(
                        header.origin,
                        gpui_kit::point(nav.right() + px(28.), bar.bottom() + px(28.))
                    );
                    assert_eq!(status.size.height, px(180.));
                    assert_eq!(status.origin.x, header.origin.x);
                    assert_eq!(status.right(), px(width - 28.));
                    assert_eq!(shortcuts.top(), status.bottom() + px(16.));
                    assert_eq!(shortcuts.bottom(), px(height - 28.));
                    assert!(shortcuts.size.height > px(100.));
                    let manage = window.find("manage-bindings").bounds();
                    let scroll = window.find("overview-shortcuts").bounds();
                    assert!(manage.is_contained_within(&shortcuts));
                    assert_eq!(scroll.left(), shortcuts.left() + px(16.));
                    assert_eq!(scroll.right(), shortcuts.right() - px(16.));
                    assert_eq!(scroll.bottom(), shortcuts.bottom() - px(16.));
                    assert!(scroll.top() >= manage.bottom() + px(16.));
                    assert_eq!(
                        window.find(("nav", 3usize)).bounds().bottom(),
                        px(height - 12.)
                    );
                    assert_eq!(window.find("listen").bounds().size.height, px(32.));
                    for action in ["listen", "test-receiver"] {
                        let bounds = window.find(action).bounds();
                        assert!((bounds.bottom() - (status.bottom() - px(16.))).abs() <= px(1. / scale),
                            "{locale}, dark={dark}, scale={scale}: {action} must align to the card bottom: {bounds:?}, {status:?}");
                    }
                }
            }
        })
        .unwrap();
    }
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
