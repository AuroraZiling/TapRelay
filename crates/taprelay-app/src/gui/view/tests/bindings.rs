use super::support::{open, settle, wait_until};
use crate::gui::action::Action;
use crate::gui::view::Command;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};

#[gpui_kit::test]
fn toggling_a_binding_persists_without_blocking_unrelated_controls(cx: &mut TestAppContext) {
    let (handle, view, folder) = open(cx);
    settle(cx, handle, &view);
    cx.update_window(handle, |_, _, cx| {
        cx.set_reduce_motion(true);
        view.update(cx, |view, cx| {
            view._pump = gpui_kit::Task::ready(());
            view.ui.page = 1;
            cx.notify();
        });
    })
    .unwrap();
    for enabled in [false, true] {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("binding-enabled-media.play-pause", cx);
            assert!(view.read(cx).controller.runtime.busy());
            assert!(!view.read(cx).ui.groups.busy);
            assert!(
                view.read(cx)
                    .ui
                    .binding
                    .rows
                    .iter()
                    .find(|row| row.id == "media.play-pause")
                    .unwrap()
                    .pending
            );
            assert_ne!(
                window.find("binding-enabled-media.previous").disabled(),
                Some(true)
            );
            assert_ne!(window.find("custom-mappings").disabled(), Some(true));
            let scope = view.read(cx).scope_pickers
                [&taprelay_core::function::FunctionId::MediaPrevious]
                .state
                .entity_id();
            assert_ne!(
                window.find(("multi-combo-box", scope)).disabled(),
                Some(true)
            );
        })
        .unwrap();
        settle(cx, handle, &view);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_ne!(
                window.find("binding-enabled-media.play-pause").disabled(),
                Some(true)
            );
            assert_eq!(
                view.read(cx).controller.runtime.config.functions
                    [&taprelay_core::function::FunctionId::MediaPlayPause]
                    .enabled,
                enabled
            );
            view.update(cx, |view, _| view.controller.tick(&mut view.ui));
        })
        .unwrap();
        let saved = crate::config::Config::load(&folder.path().join("config.json")).unwrap();
        assert_eq!(
            saved.functions[&taprelay_core::function::FunctionId::MediaPlayPause].enabled,
            enabled
        );
    }
}

#[gpui_kit::test]
fn completing_an_unchanged_binding_toggle_still_clears_its_pending_presentation(
    cx: &mut TestAppContext,
) {
    let (handle, view, _folder) = open(cx);
    settle(cx, handle, &view);
    let pending = cx
        .update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                view._pump = gpui_kit::Task::ready(());
                view.dispatch(
                    Command::Binding(
                        crate::runtime::commands::BindingCommand::SetFunctionEnabled {
                            id: taprelay_core::function::FunctionId::MediaPlayPause,
                            enabled: true,
                        },
                    ),
                    window,
                    cx,
                );
                assert!(view.ui.binding.rows.iter().any(|row| row.pending));
                view.ui.binding.clone()
            })
        })
        .unwrap();
    settle(cx, handle, &view);
    cx.update(|cx| {
        let binding = &view.read(cx).ui.binding;
        assert_ne!(binding, &pending);
        assert!(binding.rows.iter().all(|row| !row.pending));
    });
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
fn shortcut_button_records_cancels_and_removes_without_cross_triggering(cx: &mut TestAppContext) {
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
        assert!(
            window
                .find("binding-media.play-pause-0")
                .label()
                .unwrap()
                .contains("F8")
        );
        view.update(cx, |view, _| view.controller.tick(&mut view.ui));
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
    for trigger in ["binding-media.play-pause-0", "binding-add-media.play-pause"] {
        cx.update_window(handle, |_, window, cx| window.click(trigger, cx))
            .unwrap();
        settle(cx, handle, &view);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(view.read(cx).controller.recording());
            let cancel = window.find("capture-cancel-media.play-pause");
            assert_ne!(cancel.disabled(), Some(true));
            window.click("capture-cancel-media.play-pause", cx);
        })
        .unwrap();
        settle(cx, handle, &view);
        cx.update(|cx| {
            assert!(!view.read(cx).controller.recording());
            assert_eq!(
                view.read(cx).controller.runtime.config.functions,
                saved.functions
            );
        });
    }
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("remove-media.play-pause-0", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!view.read(cx).controller.recording());
        assert!(window.find("binding-add-media.play-pause").visible());
        assert!(
            view.read(cx).controller.runtime.config.functions
                [&taprelay_core::function::FunctionId::MediaPlayPause]
                .shortcuts
                .is_empty()
        );
        view.update(cx, |view, _| view.controller.tick(&mut view.ui));
    })
    .unwrap();
    let saved = crate::config::Config::load(&folder.path().join("config.json")).unwrap();
    assert!(
        saved.functions[&taprelay_core::function::FunctionId::MediaPlayPause]
            .shortcuts
            .is_empty()
    );
}
