use super::support::{open, open_with_config, settle};
use crate::gui::action::Action;
use crate::gui::view::Command;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};

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
        })
        .unwrap();
    }
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
