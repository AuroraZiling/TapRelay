use super::support::{open, settle};
use crate::action::Action;
use crate::config::Config;
use crate::gui::view::Command;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};

#[gpui_kit::test]
fn invalid_group_name_stays_in_editor_and_save_persists(cx: &mut TestAppContext) {
    let (handle, view, folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.dispatch(Command::App(Action::Navigate, "4".into()), window, cx);
        });
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            // Edit a supplied group so this test never enumerates native windows.
            view.controller
                .runtime
                .config
                .foreground_app_rules
                .groups
                .insert(
                    "games".into(),
                    taprelay_core::foreground_app::ForegroundAppGroup {
                        name: "Games".into(),
                        foreground_apps: Default::default(),
                    },
                );
            view.dispatch(Command::Group("edit", "games".into()), window, cx);
        });
        window.render_frame(cx);
        window.click("group-name", cx);
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        assert!(view.read(cx).ui.groups.editor_open);
        assert!(!view.read(cx).ui.groups.form_error.is_empty());
        window.render_frame(cx);
        assert!(window.find("group-form-error").visible());
        window.click("group-name", cx);
        window.input("Work", cx);
        window.click("group-save", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update(|cx| assert!(!view.read(cx).ui.groups.editor_open));
    let saved = Config::load(&folder.path().join("config.json")).unwrap();
    assert_eq!(saved.foreground_app_rules.groups["games"].name, "Work");
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.dispatch(Command::Group("edit", "games".into()), window, cx)
        });
        window.render_frame(cx);
        window.press("escape", cx);
        assert!(!view.read(cx).ui.groups.editor_open);
    })
    .unwrap();
}
