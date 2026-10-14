use super::support::{open, settle};
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
    cx.update(|cx| {
        assert!(!view.read(cx).controller.recording());
        assert_eq!(view.read(cx).controller.runtime.config.functions, before);
        assert!(view.read(cx).capture_focus.is_none());
    });
}
