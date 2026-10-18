use super::support::{open, open_sized, settle, wait_until};
use crate::config::Config;
use crate::gui::action::Action;
use crate::gui::view::Command;
use gpui_kit::base::test_support::snapshots;
use gpui_kit::component::ActiveTheme;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, px};

#[gpui_kit::test]
fn mapping_leave_dialog_is_centered_and_keeps_validation_on_the_editor(cx: &mut TestAppContext) {
    for (height, language) in [(480., "en"), (800., "zh-CN")] {
        let (handle, view, _folder) = open_sized(cx, 900., height);
        settle(cx, handle, &view);
        cx.update_window(handle, |_, window, cx| {
            cx.set_reduce_motion(true);
            view.update(cx, |view, cx| {
                view._pump = gpui_kit::Task::ready(());
                view.controller.runtime.config.options.language =
                    crate::config::Language::from_name(language).unwrap();
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
            window.input("Unsaved", cx);
            window.click("mapping-save", cx);
            assert!(!view.read(cx).ui.mapping.error.is_empty());
            window.click("mapping-back", cx);
            window.render_frame(cx);
            let dialog = window.find("mapping-leave-dialog").bounds();
            assert_eq!(
                f32::from(dialog.center().y).round(),
                f32::from(window.viewport_size().height / 2.).round()
            );
            assert!(!snapshots(window).iter().any(|element| {
                element.path().contains(&"mapping-leave-dialog".into())
                    && element.role() == Some(gpui_kit::Role::Alert)
            }));
            window.click("dialog-save", cx);
            assert!(window.try_find("mapping-leave-dialog").is_none());
            assert!(view.read(cx).ui.mapping.editor);
            assert!(window.find("mapping-form-error").visible());
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn output_controls_keep_modifiers_on_the_right_and_center_the_preview(cx: &mut TestAppContext) {
    for width in [900., 1280.] {
        let (handle, view, _folder) = open_sized(cx, width, 800.);
        settle(cx, handle, &view);
        cx.update_window(handle, |_, window, cx| {
            cx.set_reduce_motion(true);
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
            for language in ["en", "zh-CN"] {
                view.update(cx, |view, cx| {
                    view.controller.runtime.config.options.language =
                        crate::config::Language::from_name(language).unwrap();
                    view.controller.sync(&mut view.ui);
                    view.effects(window, cx);
                    cx.notify();
                });
                for mode in [0usize, 1] {
                    window.render_frame(cx);
                    window.click(("entry-mode", mode), cx);
                    let column = window.find("mapping-output-column").bounds();
                    let preview = window.find("mapping-output-preview").bounds();
                    let text = window.find("mapping-output-preview-text").bounds();
                    assert_eq!(
                        f32::from(preview.center().y).round(),
                        f32::from(text.center().y).round()
                    );
                    let control = window
                        .find(if mode == 0 {
                            "mapping-record-output"
                        } else {
                            "mapping-output-choice"
                        })
                        .bounds();
                    assert!(control.size.width >= px(64.));
                    assert!(control.size.width <= px(192.));
                    assert_eq!(control.left(), column.left());
                    let mut right = control.right();
                    for index in 0..4usize {
                        let modifier = window.find(("output-modifier", index)).bounds();
                        assert!(modifier.size.height >= px(16.));
                        assert!(
                            modifier.left() >= right + px(8.),
                            "{language}, width={width}: controls overlap"
                        );
                        assert_eq!(
                            f32::from(modifier.center().y).round(),
                            f32::from(control.center().y).round()
                        );
                        assert!(modifier.right() <= column.right());
                        right = modifier.right();
                    }
                    assert!(window.try_find("mapping-output-filter").is_none());
                    if mode == 1 {
                        let help = window.find("mapping-output-help").bounds();
                        let controls = window.find("mapping-output-controls").bounds();
                        assert!(help.top() > controls.bottom());
                        assert_eq!(help.left(), controls.left());
                    } else {
                        assert!(window.try_find("mapping-output-help").is_none());
                    }
                    let save = window.find("mapping-save");
                    assert_eq!(save.label(), Some(view.read(cx).tr("common.save").as_str()));
                    let scale = window.scale_factor();
                    assert!(window.painted_quads().iter().any(|quad| {
                        quad.background == cx.theme().tokens.button_primary.into()
                            && (quad.bounds.left().as_f32()
                                - f32::from(save.bounds().left()) * scale)
                                .abs()
                                < 1.
                            && (quad.bounds.top().as_f32() - f32::from(save.bounds().top()) * scale)
                                .abs()
                                < 1.
                    }));
                }
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn mapping_shortcut_capsules_keep_cancel_inside_and_preserve_inputs(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
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
        window.click("mapping-add-input", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| window.press("f8", cx))
        .unwrap();
    wait_until(cx, handle, &view, |view| !view.controller.recording());
    let shortcut = taprelay_core::function::Shortcut::keyboard(Default::default(), 0x77);
    for slot in [0usize, 1] {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            if slot == 0 {
                let button = window.find(("mapping-input", 0usize)).bounds();
                let clear = window.find(("mapping-remove-input", 0usize)).bounds();
                assert_eq!(button.size.height, px(32.));
                assert!(clear.left() > button.center().x);
                assert!(button.contains(&clear.center()));
                assert!(clear.right() < button.right());
                window.click(("mapping-input", 0usize), cx);
            } else {
                window.click("mapping-add-input", cx);
            }
        })
        .unwrap();
        settle(cx, handle, &view);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert!(view.read(cx).controller.recording());
            let button = if slot == 0 {
                window.find(("mapping-input", 0usize)).bounds()
            } else {
                window.find("mapping-add-input").bounds()
            };
            let cancel = window.find("mapping-cancel-capture");
            assert_ne!(cancel.disabled(), Some(true));
            let bounds = cancel.bounds();
            assert!(bounds.left() > button.center().x);
            assert!(bounds.right() < button.right());
            assert!(bounds.top() >= button.top() && bounds.bottom() <= button.bottom());
            window.click("mapping-cancel-capture", cx);
        })
        .unwrap();
        settle(cx, handle, &view);
        cx.update(|cx| {
            assert!(!view.read(cx).controller.recording());
            assert_eq!(
                view.read(cx)
                    .controller
                    .mapping_draft
                    .as_ref()
                    .unwrap()
                    .mapping
                    .shortcuts
                    .as_slice(),
                std::slice::from_ref(&shortcut)
            );
        });
    }
    cx.update_window(handle, |_, window, cx| {
        window.click(("mapping-remove-input", 0usize), cx);
        assert!(view.read(cx).ui.mapping.inputs.is_empty());
        assert!(!view.read(cx).controller.recording());
    })
    .unwrap();
}

#[gpui_kit::test]
fn searchable_output_choice_hides_mouse_modifiers_and_restores_keyboard_modifiers(
    cx: &mut TestAppContext,
) {
    let (handle, view, _folder) = open(cx);
    settle(cx, handle, &view);
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
        window.click(("entry-mode", 1usize), cx);
    })
    .unwrap();
    for value in ["key/66", "mouse/4", "key/67"] {
        cx.update_window(handle, |_, window, cx| {
            let label = view
                .read(cx)
                .ui
                .mapping
                .output_options
                .iter()
                .find(|option| option.value == value)
                .unwrap()
                .label
                .clone();
            window.click("mapping-output-choice", cx);
            window.press("secondary-a", cx);
            window.input(&label, cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.press("down", cx);
            window.press("enter", cx);
        })
        .unwrap();
        cx.run_until_parked();
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(view.read(cx).ui.mapping.selected_output.value, value);
            assert_eq!(
                view.read(cx)
                    .output_choice
                    .read(cx)
                    .selected_value()
                    .map(String::as_str),
                Some(value)
            );
            if value.starts_with("mouse/") {
                assert!(
                    !window
                        .find("mapping-output-preview-text")
                        .label()
                        .unwrap()
                        .contains("Ctrl")
                );
                for mode in [0usize, 1] {
                    window.click(("entry-mode", mode), cx);
                    for index in 0..4usize {
                        assert!(window.try_find(("output-modifier", index)).is_none());
                    }
                }
            } else {
                for index in 0..4usize {
                    assert!(window.find(("output-modifier", index)).visible());
                }
                if value == "key/66" {
                    window.click(("output-modifier", 0usize), cx);
                }
                assert!(view.read(cx).ui.mapping.ctrl);
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn mapping_name_focus_ring_is_not_clipped_by_the_editor_scroll_region(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        cx.set_reduce_motion(true);
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
    })
    .unwrap();
    for scale in [1., 1.5, 2.] {
        cx.simulate_window_scale_factor_change(handle, scale);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.click("mapping-name", cx);
            let input = window.find("mapping-name").bounds();
            assert_eq!(input.left(), window.find("page-header").bounds().left());
            let ring = window
                .painted_quads()
                .into_iter()
                .find(|quad| {
                    quad.bounds.left().as_f32() < f32::from(input.left()) * scale
                        && quad.bounds.right().as_f32() > f32::from(input.right()) * scale
                        && quad.bounds.top().as_f32() < f32::from(input.top()) * scale
                        && quad.bounds.bottom().as_f32() > f32::from(input.bottom()) * scale
                        && quad.border_widths.left.as_f32() > scale
                })
                .expect("focused description input must paint its outer focus ring");
            assert!(
                ring.content_mask.bounds.left() <= ring.bounds.left(),
                "scale={scale}: left focus ring is clipped: {ring:?}"
            );
            assert!(
                ring.content_mask.bounds.right() >= ring.bounds.right(),
                "scale={scale}: right focus ring is clipped: {ring:?}"
            );
        })
        .unwrap();
    }
}

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
fn mapping_save_failure_keeps_draft_then_retry_and_delete_persist(cx: &mut TestAppContext) {
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
        assert!(window.try_find("mapping-undo").is_none());
    })
    .unwrap();
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
        assert!(window.try_find("mapping-output-filter").is_none());
        window.click("mapping-output-choice", cx);
        window.input("F9", cx);
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        window.press("down", cx);
        window.press("enter", cx);
    })
    .unwrap();
    cx.run_until_parked();
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
