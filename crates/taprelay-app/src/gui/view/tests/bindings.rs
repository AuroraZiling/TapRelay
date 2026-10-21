use super::support::{open, open_sized, settle, wait_until};
use crate::gui::action::Action;
use crate::gui::view::Command;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, px};
use std::time::Duration;

#[gpui_kit::test]
fn invalid_shortcut_feedback_is_compact_in_both_languages(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        for language in ["en", "zh-CN"] {
            view.update(cx, |view, cx| {
                view._pump = gpui_kit::Task::ready(());
                view.controller.runtime.config.options.language =
                    crate::config::Language::from_name(language).unwrap();
                view.ui.page = 1;
                view.ui.binding.capture.function_id = "media.play-pause".into();
                view.ui.binding.capture.slot = 0;
                view.ui.binding.capture.error = view.tr("capture.invalid");
                cx.notify();
            });
            window.render_frame(cx);
            let error = window.find("binding-capture-error");
            assert_eq!(error.role(), Some(gpui_kit::Role::Alert));
            assert_eq!(
                error.label(),
                Some(view.read(cx).tr("capture.invalid").as_str())
            );
            assert!(error.visible());
            assert!(error.bounds().size.height <= px(18.));
            let column = window.find("binding-shortcuts-media.play-pause").bounds();
            assert!(error.bounds().left() >= column.left());
            assert!(error.bounds().right() <= column.right());
            assert!(error.bounds().top() >= column.top());
            assert!(error.bounds().bottom() <= column.bottom());
            assert!(
                error.bounds().top()
                    >= window
                        .find("binding-add-media.play-pause")
                        .bounds()
                        .bottom()
                        + px(4.)
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn toggling_a_binding_keeps_other_switches_visually_stable(cx: &mut TestAppContext) {
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
            let other = window.find("binding-enabled-media.previous").bounds();
            let colors = |window: &gpui_kit::Window| {
                let scale = window.scale_factor();
                window
                    .painted_quads()
                    .into_iter()
                    .filter(|quad| {
                        let center = quad.bounds.center();
                        other.contains(&gpui_kit::point(
                            px(center.x.as_f32() / scale),
                            px(center.y.as_f32() / scale),
                        ))
                    })
                    .map(|quad| quad.background)
                    .collect::<Vec<_>>()
            };
            let before = colors(window);
            assert!(!before.is_empty());
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
            assert_eq!(
                colors(window),
                before,
                "toggling one binding must not dim unrelated switches"
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
fn bindings_scrollbar_stays_visible_when_idle_and_moves_the_list(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view._pump = gpui_kit::Task::ready(());
            view.ui.page = 1;
            cx.notify();
        });
        window.render_frame(cx);
        window.hover("titlebar-title", cx);
    })
    .unwrap();
    for after_scroll in [false, true] {
        cx.executor().advance_clock(Duration::from_secs(5));
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let viewport = window.find("bindings").bounds();
            let scale = window.scale_factor();
            let thumb = window
                .painted_quads()
                .into_iter()
                .find(|quad| {
                    let bounds = quad.bounds;
                    !quad.background.is_transparent()
                        && bounds.left().as_f32() / scale >= f32::from(viewport.right()) - 12.
                        && bounds.right().as_f32() / scale <= f32::from(viewport.right())
                        && bounds.top().as_f32() / scale >= f32::from(viewport.top())
                        && bounds.bottom().as_f32() / scale <= f32::from(viewport.bottom())
                        && bounds.size.width.as_f32() / scale < 12.
                        && bounds.size.height.as_f32() / scale > 20.
                        && bounds.size.height.as_f32() / scale < f32::from(viewport.size.height)
                })
                .expect("the scrollbar thumb must be painted even while idle");
            if !after_scroll {
                let before = view.read(cx).content_scroll.offset();
                let position = gpui_kit::point(
                    px(thumb.bounds.center().x.as_f32() / scale),
                    viewport.bottom() - px(8.),
                );
                window.click_at("bindings", position - viewport.origin, cx);
                assert!(view.read(cx).content_scroll.offset().y < before.y);
                window.hover("titlebar-title", cx);
            }
        })
        .unwrap();
    }
}

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
            assert_eq!(scope.size.width, px(160.));
            assert_eq!(shortcuts.right() + px(16.), function.left());
            assert_eq!(function.right() + px(16.), scope.left());
            assert_eq!(scope.right() + px(17.), bounds.right());
            assert!(bounds.right() <= px(872.));
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn function_badges_fit_their_rows_without_overlapping(cx: &mut TestAppContext) {
    for width in [900., 1280.] {
        let (handle, view, _folder) = open_sized(cx, width, 500.);
        for scale in [1., 1.5, 2.] {
            cx.simulate_window_scale_factor_change(handle, scale);
            cx.update_window(handle, |_, window, cx| {
                for language in ["en", "zh-CN"] {
                    view.update(cx, |view, cx| {
                        view._pump = gpui_kit::Task::ready(());
                        view.controller.runtime.config.options.language =
                            crate::config::Language::from_name(language).unwrap();
                        view.controller.sync(&mut view.ui);
                        view.ui.page = 1;
                        view.effects(window, cx);
                        cx.notify();
                    });
                    window.render_frame(cx);
                    for row in &view.read(cx).ui.binding.rows {
                        let column = window.within(gpui_kit::SharedString::from(format!(
                            "binding-function-{}", row.id
                        )));
                        let mut previous_bottom = None;
                        for index in 0..row.gestures.len() {
                            let gesture = column.find(("gesture", index)).bounds();
                            let badge = column.find(("gesture-badge", index)).bounds();
                            assert!(badge.top() >= gesture.top() && badge.bottom() <= gesture.bottom(),
                                "{language}, scale={scale}, {}: badge {badge:?} exceeds row {gesture:?}", row.id);
                            assert!(badge.left() >= gesture.left() && badge.right() <= gesture.right());
                            if let Some(bottom) = previous_bottom {
                                assert!(
                                    badge.top() + px(1. / scale) >= bottom + px(4.),
                                    "{}: badges need a visible gap ({language}, scale={scale}): previous bottom {bottom:?}, badge {badge:?}, gesture {gesture:?}",
                                    row.id
                                );
                            }
                            previous_bottom = Some(badge.bottom());
                        }
                    }
                }
            }).unwrap();
        }
    }
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
        assert_eq!(
            window.find("binding-media.play-pause-0").label(),
            Some("Record shortcut for Play / Pause: F8")
        );
        assert_eq!(
            window
                .find("binding-media.play-pause-0")
                .bounds()
                .size
                .height,
            px(32.)
        );
        assert_eq!(
            window
                .find("binding-add-media.play-pause")
                .bounds()
                .size
                .height,
            px(32.)
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
            let button = window.find(trigger).bounds();
            let cancel = window.find("capture-cancel-media.play-pause");
            let bounds = cancel.bounds();
            assert_ne!(cancel.disabled(), Some(true));
            assert!(bounds.left() > button.center().x);
            assert!(bounds.right() < button.right());
            assert!(bounds.top() >= button.top() && bounds.bottom() <= button.bottom());
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
        let button = window.find("binding-media.play-pause-0").bounds();
        let clear = window.find("remove-media.play-pause-0").bounds();
        assert!(clear.left() > button.center().x);
        assert!(clear.right() < button.right());
        assert!(clear.top() >= button.top() && clear.bottom() <= button.bottom());
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
