use super::support::open;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};
use std::time::Duration;

#[gpui_kit::test]
fn sidebar_toggle_keeps_its_position_when_collapsed(cx: &mut TestAppContext) {
    let (handle, _view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        cx.set_reduce_motion(true);
        window.render_frame(cx);
        let expanded = window.find("collapse").bounds();
        window.click("collapse", cx);
        assert_eq!(window.find("collapse").bounds(), expanded);
        let collapsed_item = window.find("0-0").bounds();
        assert_eq!(expanded.center().x, collapsed_item.center().x);
        window.click("collapse", cx);
        assert_eq!(window.find("collapse").bounds(), expanded);
    })
    .unwrap();
}

#[gpui_kit::test]
fn sidebar_animates_and_reverses_without_moving_the_toggle(cx: &mut TestAppContext) {
    let (handle, _view, _folder) = open(cx);
    cx.update(|cx| cx.set_reduce_motion(false));
    let measure = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            (
                window.find("navigation").bounds().size.width,
                window.find("collapse").bounds(),
            )
        })
        .unwrap()
    };
    let toggle = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| window.click("collapse", cx))
            .unwrap();
    };
    let expanded = measure(cx);
    toggle(cx);
    assert_eq!(measure(cx), expanded);
    cx.executor().advance_clock(Duration::from_millis(80));
    let collapsing = measure(cx);
    assert!(collapsing.0 < expanded.0);
    assert_eq!(collapsing.1, expanded.1);
    toggle(cx);
    assert_eq!(measure(cx), collapsing);
    cx.executor().advance_clock(Duration::from_millis(250));
    assert_eq!(measure(cx), expanded);
    toggle(cx);
    cx.executor().advance_clock(Duration::from_millis(250));
    let collapsed = measure(cx);
    assert!(collapsed.0 < collapsing.0);
    assert_eq!(collapsed.1, expanded.1);
    toggle(cx);
    assert_eq!(measure(cx), collapsed);
    cx.executor().advance_clock(Duration::from_millis(80));
    let expanding = measure(cx);
    assert!(expanding.0 > collapsed.0 && expanding.0 < expanded.0);
    assert_eq!(expanding.1, expanded.1);
    cx.executor().advance_clock(Duration::from_millis(250));
    assert_eq!(measure(cx), expanded);
}

#[gpui_kit::test]
fn sidebar_toggles_and_navigates_in_both_locales(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        cx.set_reduce_motion(true);
        for language in ["english", "chinese"] {
            view.update(cx, |view, cx| {
                view.controller.runtime.config.options.language =
                    crate::config::Language::from_name(language).unwrap();
                view.controller.sync(&mut view.ui);
                cx.notify();
            });
            window.render_frame(cx);
            assert!(!view.read(cx).sidebar_collapsed);
            assert_eq!(window.find("collapse").label(), Some("Collapse sidebar"));
            let expanded_width = window.find("navigation").bounds().size.width;
            for collapsed in [true, false] {
                window.click("collapse", cx);
                assert_eq!(view.read(cx).sidebar_collapsed, collapsed);
                assert_eq!(
                    window.find("collapse").label(),
                    Some(if collapsed {
                        "Expand sidebar"
                    } else {
                        "Collapse sidebar"
                    })
                );
                let width = window.find("navigation").bounds().size.width;
                if collapsed {
                    assert!(width < expanded_width);
                } else {
                    assert_eq!(width, expanded_width);
                }
                for (item, page) in [("0-0", 0), ("0-1", 1), ("0-2", 4), ("0-3", 2)] {
                    window.click(item, cx);
                    assert_eq!(view.read(cx).ui.page, page);
                    assert_eq!(view.read(cx).sidebar_collapsed, collapsed);
                }
                window.click(("nav", 3usize), cx);
                assert_eq!(view.read(cx).ui.page, 3);
                assert!(window.find("page-header").visible());
            }
        }
    })
    .unwrap();
}
