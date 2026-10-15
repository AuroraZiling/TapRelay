use super::support::{open, settle, wait_until};
use crate::action::Action;
use crate::config::Config;
use crate::gui::view::Command;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, px};

#[gpui_kit::test]
fn mapping_editor_has_equal_columns_and_keeps_save_outside_scrolling(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.dispatch(Command::App(Action::Navigate, "1".into()), window, cx)
        });
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("custom-mappings", cx);
        window.click("mapping-create", cx);
        for mode in [0usize, 1] {
            window.click(("entry-mode", mode), cx);
            let left = window.find("mapping-input-column").bounds();
            let right = window.find("mapping-output-column").bounds();
            let save = window.find("mapping-save").bounds();
            assert_eq!(left.size.width, right.size.width);
            assert_eq!(left.top(), right.top());
            assert_eq!(left.right() + px(24.), right.left());
            assert_eq!(left.left(), window.find("page-header").bounds().left());
            assert!(right.right() <= px(872.));
            assert_eq!(save.bottom(), px(472.));
            assert!(
                window
                    .find("page-header")
                    .bounds()
                    .contains(&window.find("mapping-back").bounds().center())
            );
            window.scroll(
                ("entry-mode", mode),
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-300.))),
                cx,
            );
            assert_eq!(window.find("mapping-save").bounds(), save);
            window.scroll(
                if mode == 0 {
                    "mapping-record-output"
                } else {
                    "mapping-output-choice"
                },
                gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(300.))),
                cx,
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn edits_mapping_text_and_keeps_or_discards_the_draft(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        // Keep dialog hit targets stationary during consecutive simulated clicks.
        cx.set_reduce_motion(true);
        view.update(cx, |view, cx| {
            view.dispatch(Command::App(Action::Navigate, "1".into()), window, cx)
        });
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("custom-mappings", cx);
        window.click("mapping-create", cx);
        window.click("mapping-name", cx);
        window.input("Playback 中文", cx);
        assert_eq!(window.find("mapping-name").value(), Some("Playback 中文"));
        window.click(("entry-mode", 1usize), cx);
        window.scroll(
            ("entry-mode", 1usize),
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-200.))),
            cx,
        );
        window.click("mapping-output-choice", cx);
        window.press("escape", cx);
        assert!(view.read(cx).ui.mapping.editor);
        assert!(!view.read(cx).ui.mapping.leave_open);
        window.scroll(
            "mapping-output-choice",
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(200.))),
            cx,
        );
        window.click("mapping-name", cx);
        window.press("escape", cx);
        assert!(window.find("dialog-discard").visible());
        window.click("dialog-cancel", cx);
        assert_eq!(window.find("mapping-name").value(), Some("Playback 中文"));
        window.click("mapping-back", cx);
        window.click("dialog-discard", cx);
        assert!(window.find("mapping-create").visible());
        assert!(!view.read(cx).ui.mapping.editor);
        assert!(
            view.read(cx)
                .controller
                .runtime
                .config
                .custom_mappings
                .is_empty()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn mapping_save_failure_keeps_draft_then_retry_delete_and_undo_persist(cx: &mut TestAppContext) {
    let (handle, view, folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.dispatch(Command::App(Action::Navigate, "1".into()), window, cx)
        });
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("custom-mappings", cx);
        window.click("mapping-create", cx);
        window.click("mapping-name", cx);
        window.input("Save retry 中文", cx);
        view.update(cx, |view, cx| {
            view.controller
                .mapping_draft
                .as_mut()
                .unwrap()
                .mapping
                .shortcuts
                .push(taprelay_core::function::Shortcut::keyboard(
                    Default::default(),
                    0x41,
                ));
            view.dispatch(
                Command::Mapping("select-output", "key/5".into()),
                window,
                cx,
            );
            view.controller.path = folder.path().join("missing/config.json");
        });
        window.click("mapping-save", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        let state = view.read(cx);
        assert!(state.ui.mapping.editor);
        assert!(!state.ui.mapping.saving);
        assert!(!state.ui.mapping.error.is_empty());
        assert_eq!(state.ui.mapping.name, "Save retry 中文");
        assert_eq!(
            state
                .controller
                .mapping_draft
                .as_ref()
                .unwrap()
                .mapping
                .shortcuts,
            [taprelay_core::function::Shortcut::keyboard(
                Default::default(),
                0x41
            )]
        );
        assert_eq!(state.ui.mapping.selected_output.value, "key/5");
        assert!(state.controller.runtime.config.custom_mappings.is_empty());
        window.render_frame(cx);
        assert!(window.find("mapping-form-error").visible());
        view.update(cx, |view, _| {
            view.controller.path = folder.path().join("config.json")
        });
        window.click("mapping-save", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    let saved = Config::load(&folder.path().join("config.json")).unwrap();
    assert_eq!(saved.custom_mappings.len(), 1);
    assert_eq!(saved.custom_mappings[0].name, "Save retry 中文");
    cx.update_window(handle, |_, window, cx| {
        assert!(!view.read(cx).ui.mapping.editor);
        window.render_frame(cx);
        window.click(
            gpui_kit::SharedString::from(format!("mapping-delete-{}", saved.custom_mappings[0].id)),
            cx,
        );
    })
    .unwrap();
    settle(cx, handle, &view);
    assert!(
        Config::load(&folder.path().join("config.json"))
            .unwrap()
            .custom_mappings
            .is_empty()
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("mapping-undo", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    assert_eq!(
        Config::load(&folder.path().join("config.json"))
            .unwrap()
            .custom_mappings,
        saved.custom_mappings
    );
}

#[gpui_kit::test]
fn mapping_conflict_identifies_existing_function_without_saving(cx: &mut TestAppContext) {
    let (handle, view, folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.dispatch(Command::App(Action::Navigate, "1".into()), window, cx)
        });
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("custom-mappings", cx);
        window.click("mapping-create", cx);
        let shortcut = taprelay_core::function::Shortcut::keyboard(Default::default(), 0x41);
        view.update(cx, |view, cx| {
            view.controller
                .runtime
                .config
                .functions
                .get_mut(&taprelay_core::function::FunctionId::MediaPlayPause)
                .unwrap()
                .shortcuts
                .push(shortcut.clone());
            view.controller
                .mapping_draft
                .as_mut()
                .unwrap()
                .mapping
                .shortcuts
                .push(shortcut);
            view.dispatch(
                Command::Mapping("select-output", "key/5".into()),
                window,
                cx,
            );
        });
        window.click("mapping-save", cx);
        assert!(view.read(cx).ui.mapping.editor);
        assert_eq!(
            view.read(cx).ui.mapping.conflict_target,
            "function/media.play-pause"
        );
        assert!(
            view.read(cx)
                .controller
                .runtime
                .config
                .custom_mappings
                .is_empty()
        );
        assert!(!folder.path().join("config.json").exists());
        assert!(!view.read(cx).ui.mapping.error.is_empty());
    })
    .unwrap();
}

#[gpui_kit::test]
fn output_recording_accepts_tab_and_escape_cancels_without_changing_output(
    cx: &mut TestAppContext,
) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.dispatch(Command::App(Action::Navigate, "1".into()), window, cx)
        });
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("custom-mappings", cx);
        window.click("mapping-create", cx);
    })
    .unwrap();
    for (key, output) in [
        ("tab", "key/43"),
        ("shift-tab", "key/43"),
        ("escape", "key/43"),
    ] {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("mapping-record-output", cx);
        })
        .unwrap();
        settle(cx, handle, &view);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.press(key, cx);
            assert!(view.read(cx).focus.is_focused(window));
        })
        .unwrap();
        wait_until(cx, handle, &view, |view| !view.controller.recording());
        cx.update(|cx| assert_eq!(view.read(cx).ui.mapping.selected_output.value, output));
    }
}

#[gpui_kit::test]
fn records_input_selects_a_filtered_output_and_saves_through_controls(cx: &mut TestAppContext) {
    let (handle, view, folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("0-1", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("custom-mappings", cx);
        window.click("mapping-create", cx);
        window.click("mapping-name", cx);
        window.input("Recorded mapping 中文", cx);
        window.click("mapping-add-input", cx);
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
            window.find(("mapping-input", 0usize)).label(),
            Some("Record shortcut for F8")
        );
        window.click(("entry-mode", 1usize), cx);
        window.scroll(
            ("entry-mode", 1usize),
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-250.))),
            cx,
        );
        window.click("mapping-output-filter", cx);
        window.input("F9", cx);
        assert_eq!(window.find("mapping-output-filter").value(), Some("F9"));
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(view.read(cx).ui.mapping.output_options.len(), 1);
        window.click("mapping-output-choice", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(window.within("popup-menu").find(0usize).label(), Some("F9"));
        window.within("popup-menu").click(0usize, cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("mapping-output-choice").label(),
            Some("Output key: F9")
        );
        window.click("mapping-save", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(window.find("mapping-create").visible());
        assert!(!view.read(cx).ui.mapping.editor);
    })
    .unwrap();
    let saved = Config::load(&folder.path().join("config.json")).unwrap();
    assert_eq!(saved.custom_mappings.len(), 1);
    assert_eq!(saved.custom_mappings[0].name, "Recorded mapping 中文");
    assert_eq!(
        saved.custom_mappings[0].shortcuts,
        [taprelay_core::function::Shortcut::keyboard(
            Default::default(),
            0x77
        )]
    );
    assert_eq!(
        saved.custom_mappings[0].output,
        taprelay_core::mapping::MappingOutput::Keyboard {
            usage: 0x42,
            modifiers: 0
        }
    );
}
