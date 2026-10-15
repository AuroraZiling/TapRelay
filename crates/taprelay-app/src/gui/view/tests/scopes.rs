use super::support::{open_sized, settle};
use crate::gui::view::{AppView, Command};
use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::searchable_list::SearchableListItem;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AnyWindowHandle, AppContext, Entity, Role, Task, TestAppContext, px};
use taprelay_core::{foreground_app::ForegroundAppGroup, function::FunctionId};

const FUNCTION: FunctionId = FunctionId::MediaPlayPause;

fn open_scopes(
    cx: &mut TestAppContext,
    width: f32,
) -> (AnyWindowHandle, Entity<AppView>, tempfile::TempDir) {
    let (handle, view, folder) = open_sized(cx, width, 500.);
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        cx.set_reduce_motion(true);
        view.update(cx, |view, cx| {
            view._pump = Task::ready(());
            let mut rules = view.controller.runtime.config.foreground_app_rules.clone();
            for (id, name) in [
                ("games", "Games"),
                ("video", "Video editing applications 视频编辑应用"),
                ("work", "Work"),
            ] {
                rules.groups.insert(
                    id.into(),
                    ForegroundAppGroup {
                        name: name.into(),
                        foreground_apps: vec![format!(r"C:\Apps\{name}.exe")],
                    },
                );
            }
            view.controller
                .save_foreground_app_rules(&mut view.ui, rules)
                .unwrap();
            view.dispatch(
                Command::App(crate::gui::action::Action::Navigate, "1".into()),
                window,
                cx,
            );
        });
    })
    .unwrap();
    settle(cx, handle, &view);
    (handle, view, folder)
}

fn choose(cx: &mut TestAppContext, handle: AnyWindowHandle, view: &Entity<AppView>, steps: usize) {
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        for _ in 0..steps {
            window.press("down", cx);
        }
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
    settle(cx, handle, view);
}

#[gpui_kit::test]
fn scope_combobox_multiselects_without_search_and_restores_all_apps(cx: &mut TestAppContext) {
    let (handle, view, folder) = open_scopes(cx, 900.);
    let state = cx.update(|cx| view.read(cx).scope_pickers[&FUNCTION].state.clone());
    let id = ("multi-combo-box", state.entity_id());
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find(id).role(), Some(Role::ComboBox));
        window.click("scope-media.play-pause", cx);
        assert_eq!(window.find(id).expanded(), Some(true));
        assert!(
            snapshots(window)
                .iter()
                .all(|element| { !element.visible() || element.role() != Some(Role::TextInput) })
        );
    })
    .unwrap();
    for groups in [vec!["games"], vec!["games", "work"]] {
        choose(cx, handle, &view, 2);
        cx.update_window(handle, |_, window, cx| {
            assert_eq!(state.read(cx).selected_values(), groups);
            assert!(view.read(cx).ui.groups.error.is_empty());
            view.update(cx, |view, cx| view.effects(window, cx));
            assert_eq!(state.read(cx).selected_values(), groups);
        })
        .unwrap();
        let saved = crate::config::Config::load(&folder.path().join("config.json")).unwrap();
        assert_eq!(
            saved.foreground_app_rules.assignments[&FUNCTION],
            groups.into_iter().map(String::from).collect()
        );
    }
    choose(cx, handle, &view, 1);
    cx.update(|cx| {
        assert_eq!(state.read(cx).selected_values(), [""]);
        assert!(
            !view
                .read(cx)
                .controller
                .runtime
                .config
                .foreground_app_rules
                .assignments
                .contains_key(&FUNCTION)
        );
    });
    choose(cx, handle, &view, 1);
    choose(cx, handle, &view, 0);
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(state.read(cx).selected_values(), [""]);
        window.press("escape", cx);
    })
    .unwrap();
    cx.run_until_parked();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.find(id).expanded(), Some(false));
        assert_eq!(window.find(id).focused(), Some(true));
    })
    .unwrap();
    let saved = crate::config::Config::load(&folder.path().join("config.json")).unwrap();
    assert!(
        !saved
            .foreground_app_rules
            .assignments
            .contains_key(&FUNCTION)
    );
}

#[gpui_kit::test]
fn scope_combobox_reverts_to_saved_groups_after_a_write_failure(cx: &mut TestAppContext) {
    let (handle, view, folder) = open_scopes(cx, 900.);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.dispatch(
                Command::Scope(FUNCTION, ["games".into()].into()),
                window,
                cx,
            )
        });
    })
    .unwrap();
    settle(cx, handle, &view);
    let state = cx.update(|cx| view.read(cx).scope_pickers[&FUNCTION].state.clone());
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, _| {
            view.controller.path = folder.path().to_owned()
        });
        window.render_frame(cx);
        window.click("scope-media.play-pause", cx);
    })
    .unwrap();
    choose(cx, handle, &view, 4);
    cx.update(|cx| {
        assert_eq!(state.read(cx).selected_values(), ["games"]);
        view.update(cx, |view, _| {
            assert_eq!(
                view.controller.runtime.take_foreground_app_rules_saved(),
                Some(false)
            );
        });
    });
    let saved = crate::config::Config::load(&folder.path().join("config.json")).unwrap();
    assert_eq!(
        saved.foreground_app_rules.assignments[&FUNCTION],
        ["games".into()].into()
    );
}

#[gpui_kit::test]
fn scope_tags_collapse_without_overflowing_and_keep_the_full_selection(cx: &mut TestAppContext) {
    for (width, visible, scope_width) in [(900., 1usize, 160.), (1280., 2usize, 256.)] {
        let (handle, view, _folder) = open_scopes(cx, width);
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.dispatch(
                    Command::Scope(
                        FUNCTION,
                        ["games".into(), "video".into(), "work".into()].into(),
                    ),
                    window,
                    cx,
                );
            });
        })
        .unwrap();
        settle(cx, handle, &view);
        for language in ["english", "chinese"] {
            cx.update_window(handle, |_, window, cx| {
                view.update(cx, |view, cx| {
                    view.controller.runtime.config.options.language =
                        crate::config::Language::from_name(language).unwrap();
                    view.controller.sync(&mut view.ui);
                    view.effects(window, cx);
                    cx.notify();
                });
                window.render_frame(cx);
                let bounds = window.find("scope-media.play-pause").bounds();
                assert_eq!(bounds.size.width, px(scope_width));
                let mut scope = window.within("scope-media.play-pause");
                for index in 0..visible {
                    let tag = scope.find(("scope-tag", index));
                    assert!(tag.visible());
                    assert!(tag.bounds().is_contained_within(&bounds));
                }
                assert!(scope.try_find(("scope-tag", visible)).is_none());
                let more = scope.find("scope-more");
                assert!(more.bounds().is_contained_within(&bounds));
                let expected = if language == "english" {
                    format!("+{} more", 3 - visible)
                } else {
                    format!("+{} 项", 3 - visible)
                };
                assert_eq!(more.label(), Some(expected.as_str()));
                let state = &view.read(cx).scope_pickers[&FUNCTION].state;
                assert_eq!(state.read(cx).selected_values().len(), 3);
                scope.click("scope-more", cx);
                window.press("escape", cx);
            })
            .unwrap();
            cx.run_until_parked();
        }
    }
}

#[gpui_kit::test]
fn scope_combobox_refreshes_labels_without_recreating_state(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open_scopes(cx, 900.);
    cx.update_window(handle, |_, window, cx| {
        let state = view.read(cx).scope_pickers[&FUNCTION].state.clone();
        view.update(cx, |view, cx| {
            let rules = &mut view.controller.runtime.config.foreground_app_rules;
            rules.groups.get_mut("work").unwrap().name = "Work renamed".into();
            rules.assignments.insert(FUNCTION, ["work".into()].into());
            view.controller.sync(&mut view.ui);
            view.effects(window, cx);
        });
        assert_eq!(
            state.entity_id(),
            view.read(cx).scope_pickers[&FUNCTION].state.entity_id()
        );
        assert_eq!(state.read(cx).selected_values(), ["work"]);
        assert_eq!(state.read(cx).selection()[0].1.title(), "Work renamed");
        view.update(cx, |view, cx| {
            view.controller
                .runtime
                .config
                .foreground_app_rules
                .assignments
                .remove(&FUNCTION);
            view.controller
                .runtime
                .config
                .foreground_app_rules
                .groups
                .remove("work");
            view.controller.runtime.config.options.language =
                crate::config::Language::from_name("chinese").unwrap();
            view.controller.sync(&mut view.ui);
            view.effects(window, cx);
        });
        assert_eq!(state.read(cx).selection()[0].1.title(), "所有应用");
        assert!(!view.read(cx).controller.runtime.busy());
    })
    .unwrap();
}
