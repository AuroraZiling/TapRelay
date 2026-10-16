use super::support::{open, open_sized, settle};
use crate::{
    config::{Config, Geometry, Theme},
    gui::{action::Action, view::Command},
};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, Task, TestAppContext};

#[gpui_kit::test]
fn window_geometry_stays_in_memory_until_close(cx: &mut TestAppContext) {
    let (handle, view, folder) = open_sized(cx, 1200., 700.);
    let path = folder.path().join("config.json");
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view._pump = Task::ready(());
            view.update_geometry(window);
            view.controller.tick(&mut view.ui);
            assert!(!path.exists());
            assert_eq!(view.ui.geometry.width, 1200.);
            assert_eq!(view.ui.geometry.height, 700.);
            view.dispatch(Command::App(Action::Theme, "dark".into()), window, cx);
            let saved = Config::load(&path).unwrap();
            assert_eq!(saved.options.theme, Theme::Dark);
            assert_eq!(saved.window, Geometry::default());
            view.dispatch(Command::App(Action::Close, String::new()), window, cx);
            let saved = Config::load(&path).unwrap();
            assert_eq!(saved.window, view.ui.geometry);
            assert!(saved.options.close_hint_seen);
            assert!(view.controller.hidden);
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn failed_preference_save_is_reported_and_retried_on_the_next_change(cx: &mut TestAppContext) {
    let (handle, view, folder) = open(cx);
    let parent = folder.path().join("unavailable");
    let path = parent.join("config.json");
    cx.update_window(handle, |_, _, cx| {
        view.update(cx, |view, _| {
            view._pump = Task::ready(());
            view.controller.path = path.clone();
            view.controller
                .action(&mut view.ui, Action::Theme, "dark")
                .unwrap();
            assert!(view.controller.pending_notification.take().is_some());
            assert!(!path.exists());
            std::fs::create_dir(&parent).unwrap();
            view.controller.tick(&mut view.ui);
            assert!(!path.exists());
            assert!(view.controller.pending_notification.is_none());
            view.controller
                .action(&mut view.ui, Action::SetStartHidden, "1")
                .unwrap();
            let saved = Config::load(&path).unwrap();
            assert_eq!(saved.options.theme, Theme::Dark);
            assert!(saved.options.start_hidden);
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn preference_changed_during_a_group_save_preserves_both_changes(cx: &mut TestAppContext) {
    let (handle, view, folder) = open(cx);
    let path = folder.path().join("config.json");
    let mut rules = taprelay_core::foreground_app::ForegroundAppRules::default();
    rules.groups.insert(
        "games".into(),
        taprelay_core::foreground_app::ForegroundAppGroup {
            name: "Games".into(),
            foreground_apps: vec![r"C:\Games\game.exe".into()],
        },
    );
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, _| view._pump = Task::ready(()));
        window.render_frame(cx);
        view.update(cx, |view, _| {
            view.controller
                .runtime
                .set_foreground_app_rules(rules.clone(), &path)
                .unwrap();
        });
        window.click("collapse", cx);
        assert!(view.read(cx).controller.save_pending);
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update(|cx| {
        view.update(cx, |view, _| view.controller.tick(&mut view.ui));
    });
    let saved = Config::load(&path).unwrap();
    assert!(saved.options.sidebar_collapsed);
    assert_eq!(saved.foreground_app_rules, rules);
}
