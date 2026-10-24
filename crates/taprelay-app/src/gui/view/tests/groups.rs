use super::support::{open, open_with_config, settle};
use crate::config::Config;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};

#[gpui_kit::test]
fn deleting_a_group_used_by_a_function_requires_confirmation(cx: &mut TestAppContext) {
    check_used_group_delete(cx, false);
}

#[gpui_kit::test]
fn deleting_a_group_used_by_a_disabled_mapping_requires_confirmation(cx: &mut TestAppContext) {
    check_used_group_delete(cx, true);
}

fn check_used_group_delete(cx: &mut TestAppContext, mapping: bool) {
    use taprelay_core::{
        foreground_app::ForegroundAppGroup,
        function::{FunctionId, Shortcut},
        mapping::{CustomMapping, MappingOutput},
    };

    let mut config = Config::default();
    config.wizard.dismissed = true;
    config.foreground_app_rules.groups.insert(
        "work".into(),
        ForegroundAppGroup {
            name: "Work".into(),
            foreground_apps: vec![r"C:\Apps\Editor.exe".into()],
        },
    );
    if mapping {
        config.custom_mappings.push(CustomMapping {
            id: 1,
            name: "Speed".into(),
            enabled: false,
            shortcuts: vec![Shortcut::keyboard(Default::default(), 0x77)],
            output: MappingOutput::Keyboard {
                usage: 0x59,
                modifiers: 0,
            },
            groups: ["work".into()].into(),
        });
    } else {
        config
            .foreground_app_rules
            .assignments
            .insert(FunctionId::MediaPlayPause, ["work".into()].into());
    }
    let geometry = config.window.clone();
    let (handle, view, folder) =
        open_with_config(cx, geometry.width, geometry.height, config, None);
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        cx.set_reduce_motion(true);
        window.render_frame(cx);
        window.click("0-2", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("group-delete-work", cx);
        assert!(view.read(cx).ui.groups.confirming_delete);
        assert!(window.find("group-delete-dialog").visible());
        if mapping {
            assert!(view.read(cx).ui.groups.deletion_warning.contains("Speed"));
        }
        window.within("dialog").click("dialog-cancel", cx);
        assert!(
            view.read(cx)
                .controller
                .runtime
                .config
                .foreground_app_rules
                .groups
                .contains_key("work")
        );
        window.click("group-delete-work", cx);
        window.within("dialog").click("dialog-delete", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    let saved = Config::load(&folder.path().join("config.json")).unwrap();
    assert!(saved.foreground_app_rules.groups.is_empty());
    assert!(saved.foreground_app_rules.assignments.is_empty());
    assert!(
        saved
            .custom_mappings
            .iter()
            .all(|mapping| mapping.groups.is_empty())
    );
}

#[gpui_kit::test]
fn deleting_an_unused_group_keeps_it_on_save_failure_and_allows_retry(cx: &mut TestAppContext) {
    let (handle, view, folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("0-2", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.click("group-create", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.click("group-save", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    let saved_path = folder.path().join("config.json");
    let original = Config::load(&saved_path).unwrap().foreground_app_rules;
    let id = original.groups.keys().next().unwrap();
    let delete = gpui_kit::SharedString::from(format!("group-delete-{id}"));
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, _| {
            view.controller.path = folder.path().to_owned()
        });
        window.render_frame(cx);
        window.click(delete.clone(), cx);
        assert!(!view.read(cx).ui.groups.confirming_delete);
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find(delete.clone()).visible());
        assert!(!view.read(cx).ui.groups.error.is_empty());
        assert_eq!(
            view.read(cx).controller.runtime.config.foreground_app_rules,
            original
        );
        view.update(cx, |view, _| view.controller.path = saved_path.clone());
        window.click(delete, cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    assert!(
        Config::load(&saved_path)
            .unwrap()
            .foreground_app_rules
            .groups
            .is_empty()
    );
    cx.update(|cx| {
        assert!(!view.read(cx).ui.groups.confirming_delete);
        assert!(view.read(cx).ui.groups.error.is_empty());
    });
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
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-a"
            } else {
                "ctrl-a"
            },
            cx,
        );
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
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-a"
            } else {
                "ctrl-a"
            },
            cx,
        );
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
        window.press(
            if cfg!(target_os = "macos") {
                "cmd-a"
            } else {
                "ctrl-a"
            },
            cx,
        );
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
        assert!(!view.read(cx).ui.groups.confirming_delete);
        assert!(window.try_find("group-delete-dialog").is_none());
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
