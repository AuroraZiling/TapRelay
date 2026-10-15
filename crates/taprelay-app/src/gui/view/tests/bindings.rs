use super::support::{open, settle, wait_until};
use crate::action::Action;
use crate::gui::view::Command;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, px};

#[gpui_kit::test]
fn bindings_keep_four_aligned_columns_and_a_header_action(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.dispatch(Command::App(Action::Navigate, "1".into()), window, cx)
        });
        window.render_frame(cx);
        let header = window.find("page-header").bounds();
        assert!(header.contains(&window.find("custom-mappings").bounds().center()));
        for row in &view.read(cx).ui.binding.rows {
            let find = |prefix| {
                window
                    .find(gpui_kit::SharedString::from(format!("{prefix}-{}", row.id)))
                    .bounds()
            };
            let bounds = find("binding-row");
            let shortcuts = find("binding-shortcuts");
            let function = find("binding-function");
            let scope = find("binding-scope");
            assert!(bounds.size.height >= px(64.));
            assert_eq!(function.size.width, px(240.));
            assert_eq!(scope.size.width, px(124.));
            assert_eq!(shortcuts.right() + px(16.), function.left());
            assert_eq!(function.right() + px(16.), scope.left());
            assert_eq!(scope.right() + px(16.), bounds.right());
            assert!(bounds.right() <= px(872.));
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn capture_escape_keeps_existing_shortcuts_and_restores_focus(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.dispatch(Command::App(Action::Navigate, "1".into()), window, cx)
        });
    })
    .unwrap();
    settle(cx, handle, &view);
    let (id, before) = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let row = view
                .read(cx)
                .ui
                .binding
                .rows
                .iter()
                .find(|row| row.enabled && row.shortcuts.len() < 2)
                .unwrap()
                .clone();
            let id = gpui_kit::SharedString::from(format!("binding-add-{}", row.id));
            let before = view.read(cx).controller.runtime.config.functions.clone();
            for _ in 0..32 {
                if window.find(id.clone()).focused() == Some(true) {
                    break;
                }
                window.press("tab", cx);
            }
            assert_eq!(window.find(id.clone()).focused(), Some(true));
            window.click(id.clone(), cx);
            (id, before)
        })
        .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(view.read(cx).controller.recording());
        window.click(id.clone(), cx);
        assert!(view.read(cx).controller.recording());
        window.press("escape", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!view.read(cx).controller.recording());
        assert_eq!(view.read(cx).controller.runtime.config.functions, before);
        assert!(view.read(cx).capture_focus.is_none());
        assert_eq!(window.find(id).focused(), Some(true));
    })
    .unwrap();
}

#[gpui_kit::test]
fn recording_a_shortcut_updates_the_row_and_saved_binding(cx: &mut TestAppContext) {
    let (handle, view, folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("0-1", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("binding-add-media.play-pause", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        assert!(view.read(cx).controller.recording());
        window.press("f8", cx);
    })
    .unwrap();
    wait_until(cx, handle, &view, |view| !view.controller.recording());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("binding-media.play-pause-0").label(),
            Some("F8")
        );
        view.update(cx, |view, _| view.controller.flush(true));
    })
    .unwrap();
    let saved = crate::config::Config::load(&folder.path().join("config.json")).unwrap();
    assert_eq!(
        saved.functions[&taprelay_core::function::FunctionId::MediaPlayPause].shortcuts,
        [taprelay_core::function::Shortcut::keyboard(
            Default::default(),
            0x77
        )]
    );
}
