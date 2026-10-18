use super::support::{open, open_sized, settle};
use crate::config::Config;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, px};

#[gpui_kit::test]
fn group_delete_dialog_stays_centered_with_wrapped_text_and_errors(cx: &mut TestAppContext) {
    for (height, language) in [(480., "en"), (800., "zh-CN")] {
        let (handle, view, _folder) = open_sized(cx, 900., height);
        settle(cx, handle, &view);
        cx.update_window(handle, |_, window, cx| {
            cx.set_reduce_motion(true);
            view.update(cx, |view, cx| {
                view._pump = gpui_kit::Task::ready(());
                view.controller.runtime.config.options.language =
                    crate::config::Language::from_name(language).unwrap();
                view.controller.sync(&mut view.ui);
                view.ui.page = 4;
                view.ui.groups.confirming_delete = true;
                view.ui.groups.selected_name = "Editors 应用组".repeat(3);
                view.ui.groups.deletion_warning =
                    "Playback 播放 / Pause 暂停 / Speed 倍速".repeat(3);
                view.effects(window, cx);
                cx.notify();
            });
            for error in ["", "Could not save configuration. 无法保存配置。"] {
                view.update(cx, |view, cx| {
                    view.ui.groups.form_error = error.into();
                    cx.notify();
                });
                window.render_frame(cx);
                let dialog = window.find("group-delete-dialog").bounds();
                assert!((dialog.center().y - window.viewport_size().height / 2.).abs() <= px(0.5));
                assert!(dialog.top() > px(0.));
                assert!(dialog.bottom() < window.viewport_size().height);
            }
            window.press("escape", cx);
            assert!(window.try_find("group-delete-dialog").is_none());
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn browsing_an_executable_is_async_and_handles_cancel_invalid_and_selected_paths(
    cx: &mut TestAppContext,
) {
    let (handle, view, folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("0-2", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| window.click("group-create", cx))
        .unwrap();
    settle(cx, handle, &view);
    for path in [
        None,
        Some(r"C:\Apps\Notes.txt"),
        Some(r"C:\Apps\Editor.exe"),
    ] {
        cx.update_window(handle, |_, window, cx| window.click("group-browse", cx))
            .unwrap();
        assert!(cx.did_prompt_for_paths());
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.find("group-name").visible());
        })
        .unwrap();
        cx.simulate_path_prompt_response(|options| {
            assert!(options.files);
            assert!(!options.directories && !options.multiple);
            path.map(|path| vec![path.into()])
        });
        cx.run_until_parked();
        settle(cx, handle, &view);
        cx.update(|cx| {
            let groups = &view.read(cx).ui.groups;
            assert!(groups.editor_open);
            match path {
                None => {
                    assert!(groups.foreground_apps.is_empty());
                    assert!(groups.form_error.is_empty());
                }
                Some(path) if path.ends_with(".txt") => {
                    assert!(groups.foreground_apps.is_empty());
                    assert!(!groups.form_error.is_empty());
                }
                Some(path) => {
                    assert_eq!(groups.foreground_apps.len(), 1);
                    assert_eq!(groups.foreground_apps[0].path, path);
                    assert!(!groups.choosing_running);
                    assert!(groups.form_error.is_empty());
                }
            }
        });
    }
    cx.update_window(handle, |_, window, cx| window.click("group-save", cx))
        .unwrap();
    settle(cx, handle, &view);
    let saved = Config::load(&folder.path().join("config.json")).unwrap();
    let group = saved.foreground_app_rules.groups.values().next().unwrap();
    assert_eq!(group.foreground_apps, [r"C:\Apps\Editor.exe"]);
}

#[gpui_kit::test]
fn a_late_file_picker_result_does_not_change_another_group_editor(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("0-2", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.click("group-create", cx);
        window.click("group-browse", cx);
        window.click("group-cancel", cx);
        window.click("group-create", cx);
    })
    .unwrap();
    assert!(cx.did_prompt_for_paths());
    cx.simulate_path_prompt_response(|_| Some(vec![r"C:\Apps\OldEditor.exe".into()]));
    cx.run_until_parked();
    settle(cx, handle, &view);
    cx.update(|cx| {
        assert!(view.read(cx).ui.groups.editor_open);
        assert!(view.read(cx).ui.groups.foreground_apps.is_empty());
    });
}

#[gpui_kit::test]
fn invalid_group_name_stays_in_editor_and_save_persists(cx: &mut TestAppContext) {
    let (handle, view, folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("0-2", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("group-create", cx);
        window.click("group-name", cx);
        window.press("ctrl-a", cx);
        window.input("Games", cx);
        window.click("group-save", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    let saved = Config::load(&folder.path().join("config.json")).unwrap();
    assert_eq!(saved.foreground_app_rules.groups.len(), 1);
    let id = saved.foreground_app_rules.groups.keys().next().unwrap();
    let edit = gpui_kit::SharedString::from(format!("group-edit-{id}"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(edit.clone(), cx);
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
    let updated = Config::load(&folder.path().join("config.json")).unwrap();
    assert_eq!(updated.foreground_app_rules.groups[id].name, "Work");
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(edit, cx);
        window.press("escape", cx);
        assert!(!view.read(cx).ui.groups.editor_open);
    })
    .unwrap();
}

#[gpui_kit::test]
fn creates_selects_removes_and_deletes_an_application_group_through_controls(
    cx: &mut TestAppContext,
) {
    let (handle, view, folder) = open(cx);
    let app = r"C:\Apps\Editor.exe";
    cx.update_window(handle, |_, window, cx| {
        cx.set_reduce_motion(true);
        view.update(cx, |view, _| {
            let crate::gui::controller::Shell::Headless { running_apps, .. } =
                &mut view.controller.desktop
            else {
                unreachable!()
            };
            running_apps.push(app.into());
        });
        window.render_frame(cx);
        window.click("0-2", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("group-create", cx);
        window.click("group-name", cx);
        window.press("ctrl-a", cx);
        window.input("Editors 中文", cx);
        let add = gpui_kit::SharedString::from(format!("group-app-{app}"));
        assert!(window.find(add.clone()).visible());
        window.click(add.clone(), cx);
        assert!(window.try_find(add.clone()).is_none());
        window.within("group-tabs").click(0usize, cx);
        assert!(window.find(add.clone()).visible());
        window.click(add.clone(), cx);
        assert!(window.try_find(add.clone()).is_none());
        assert!(view.read(cx).ui.groups.foreground_apps.is_empty());
        window.within("group-tabs").click(1usize, cx);
        window.click(add, cx);
        window.click("group-save", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    let saved = Config::load(&folder.path().join("config.json")).unwrap();
    assert_eq!(saved.foreground_app_rules.groups.len(), 1);
    let (id, group) = saved.foreground_app_rules.groups.iter().next().unwrap();
    assert_eq!(group.name, "Editors 中文");
    assert_eq!(group.foreground_apps, [app]);
    let delete = gpui_kit::SharedString::from(format!("group-delete-{id}"));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!view.read(cx).ui.groups.editor_open);
        assert!(window.find(delete.clone()).visible());
        window.click(delete.clone(), cx);
        window.within("dialog").click("dialog-cancel", cx);
        assert!(window.find(delete.clone()).visible());
        assert_eq!(
            view.read(cx)
                .controller
                .runtime
                .config
                .foreground_app_rules
                .groups
                .len(),
            1
        );
        window.click(delete.clone(), cx);
        window.within("dialog").click("dialog-delete", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.try_find(delete).is_none());
        assert!(window.find("group-create").visible());
        assert!(!view.read(cx).ui.groups.confirming_delete);
    })
    .unwrap();
    assert!(
        Config::load(&folder.path().join("config.json"))
            .unwrap()
            .foreground_app_rules
            .groups
            .is_empty()
    );
}
