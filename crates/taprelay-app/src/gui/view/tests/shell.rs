use super::support::{open, open_with_config, settle};
use crate::gui::action::Action;
use crate::gui::view::Command;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, px};
use std::time::Duration;

#[gpui_kit::test]
fn title_bar_remains_above_every_screen_in_both_locales_and_themes(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    for scale in [1., 1.25, 1.5, 2.] {
        cx.simulate_window_scale_factor_change(handle, scale);
        cx.update_window(handle, |_, window, cx| {
            for language in ["en", "zh-CN"] {
                for dark in [false, true] {
                    for mode in [0, 1, 2, 3] {
                        view.update(cx, |view, cx| {
                            view._pump = gpui_kit::Task::ready(());
                            view.controller.runtime.config.options.language =
                                crate::config::Language::from_name(language).unwrap();
                            view.controller.passed = matches!(mode, 1 | 2);
                            view.controller.fatal = (mode == 3).then(|| "Startup failed".into());
                            view.controller.sync(&mut view.ui);
                            view.ui.mode = mode;
                            view.ui.dark = dark;
                            view.effects(window, cx);
                            cx.notify();
                        });
                        window.render_frame(cx);
                        let bar = window.find("title-bar").bounds();
                        let workspace = window.find("workspace").bounds();
                        let brand = window.find("titlebar-brand").bounds();
                        let controls = window.find("window-controls").bounds();
                        assert_eq!(bar.size.height, px(44.));
                        assert_eq!(bar.size.width, px(900.));
                        assert_eq!(workspace.top(), bar.bottom());
                        assert_eq!(workspace.bottom(), px(500.));
                        assert!(
                            brand.left() >= bar.left()
                                && brand.top() >= bar.top()
                                && brand.right() <= bar.right()
                                && brand.bottom() <= bar.bottom(),
                            "scale={scale} mode={mode}: brand={brand:?}, bar={bar:?}"
                        );
                        assert!(window.try_find("titlebar-status").is_none());
                        assert_eq!(controls.right(), bar.right());
                        assert!(controls.top() >= bar.top());
                        assert!(controls.bottom() <= bar.bottom());
                        assert!(controls.left() >= brand.right() + px(100.));
                        for control in ["minimize", "maximize", "close"] {
                            assert!(window.within("title-bar").find(control).visible());
                            #[cfg(windows)]
                            assert_eq!(
                                window.within("title-bar").find(control).bounds().size.width,
                                px(48.)
                            );
                        }
                        assert_eq!(window.try_find("navigation").is_some(), mode == 2);
                        assert_eq!(window.try_find("page-header").is_some(), mode == 2);
                        assert_eq!(window.try_find("collapse").is_some(), mode == 2);
                        let icon = window.find("titlebar-icon").bounds();
                        let title = window.find("titlebar-title").bounds();
                        assert!(title.left() >= icon.right() + px(8.));
                        assert!(title.left() <= icon.right() + px(12.));
                        if mode == 2 {
                            let toggle = window.within("title-bar").find("collapse").bounds();
                            assert!((toggle.center().x - px(22.)).abs() <= px(1. / scale));
                            assert!((toggle.center().y - px(22.)).abs() <= px(1. / scale));
                            assert!(icon.left() > toggle.right());
                            assert_eq!(window.find("navigation").bounds().top(), bar.bottom());
                            assert_eq!(window.find("0-0").bounds().top(), bar.bottom() + px(12.));
                            assert_eq!(
                                window.find("receiver-status-indicator").label(),
                                Some(view.read(cx).ui.stage.as_str()),
                            );
                        }
                    }
                }
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn window_close_preserves_unsaved_mapping_confirmation_and_close_to_tray(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view._pump = gpui_kit::Task::ready(());
            view.dispatch(Command::App(Action::Navigate, "1".into()), window, cx);
        });
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("custom-mappings", cx);
        window.click("mapping-create", cx);
        window.click("mapping-name", cx);
        window.input("Unsaved mapping", cx);
    })
    .unwrap();
    let mut visual = gpui_kit::VisualTestContext::from_window(handle, cx);
    assert!(!visual.simulate_close());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("dialog-cancel").visible());
        assert!(!view.read(cx).controller.hidden);
        window.click("dialog-cancel", cx);
        assert_eq!(window.find("mapping-name").value(), Some("Unsaved mapping"));
    })
    .unwrap();
    assert!(!visual.simulate_close());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("dialog-discard", cx);
        assert!(view.read(cx).controller.hidden);
        assert!(!view.read(cx).controller.runtime.stopped());
        assert!(view.read(cx).controller.mapping_draft.is_none());
    })
    .unwrap();
}

#[gpui_kit::test]
fn sidebar_state_is_saved_and_restored_when_the_view_is_recreated(cx: &mut TestAppContext) {
    let (mut handle, mut view, mut folder) = open(cx);
    for collapsed in [true, false] {
        cx.update_window(handle, |_, window, cx| {
            cx.set_reduce_motion(true);
            window.render_frame(cx);
            window.click("collapse", cx);
            assert_eq!(view.read(cx).sidebar_collapsed, collapsed);
        })
        .unwrap();
        let saved = crate::config::Config::load(&folder.path().join("config.json")).unwrap();
        assert_eq!(saved.options.sidebar_collapsed, collapsed);
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, _| view._pump = gpui_kit::Task::ready(()));
            window.remove_window();
        })
        .unwrap();
        (handle, view, folder) = open_with_config(cx, 900., 500., saved, None);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(view.read(cx).sidebar_collapsed, collapsed);
            assert_eq!(
                window.find("navigation").bounds().size.width,
                px(if collapsed { 44. } else { 176. })
            );
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn sidebar_toggle_keeps_its_position_when_collapsed(cx: &mut TestAppContext) {
    let (handle, _view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        cx.set_reduce_motion(true);
        window.render_frame(cx);
        let expanded = window.find("collapse").bounds();
        let icon = window.find("titlebar-icon").bounds();
        let title = window.find("titlebar-title").bounds();
        window.click("collapse", cx);
        assert_eq!(window.find("collapse").bounds(), expanded);
        let sidebar = window.find("navigation").bounds();
        assert_eq!(
            sidebar.size.width,
            window.find("title-bar").bounds().size.height
        );
        assert_eq!(expanded.center().x, sidebar.center().x);
        let collapsed_item = window.find("0-0").bounds();
        assert_eq!(expanded.center().x, collapsed_item.center().x);
        assert_eq!(collapsed_item.size.width, collapsed_item.size.height);
        assert_eq!(window.find("titlebar-icon").bounds(), icon);
        assert_eq!(window.find("titlebar-title").bounds(), title);
        window.click("collapse", cx);
        assert_eq!(window.find("collapse").bounds(), expanded);
        assert_eq!(window.find("titlebar-icon").bounds(), icon);
        assert_eq!(window.find("titlebar-title").bounds(), title);
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
            assert_eq!(window.find("titlebar-title").bounds().left(), px(88.));
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
        for language in ["en", "zh-CN"] {
            view.update(cx, |view, cx| {
                view.controller.runtime.config.options.language =
                    crate::config::Language::from_name(language).unwrap();
                view.controller.sync(&mut view.ui);
                cx.notify();
            });
            window.render_frame(cx);
            assert!(!view.read(cx).sidebar_collapsed);
            assert_eq!(
                window.find("collapse").label(),
                Some(view.read(cx).tr("nav.collapse").as_str())
            );
            let expanded_width = window.find("navigation").bounds().size.width;
            for collapsed in [true, false] {
                window.click("collapse", cx);
                assert_eq!(view.read(cx).sidebar_collapsed, collapsed);
                assert_eq!(
                    window.find("collapse").label(),
                    Some(
                        view.read(cx)
                            .tr(if collapsed {
                                "nav.expand"
                            } else {
                                "nav.collapse"
                            })
                            .as_str()
                    )
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
