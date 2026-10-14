use super::{AppView, Command};
use crate::{
    action::Action,
    config::Config,
    gui::{Controller, Shell},
    runtime::Runtime,
    runtime_worker::RuntimeHandle,
};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{
    AnyWindowHandle, AppContext, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions, px,
    size,
};
use std::time::{Duration, Instant};

fn open(cx: &mut TestAppContext) -> (AnyWindowHandle, Entity<AppView>, tempfile::TempDir) {
    open_sized(cx, 900., 500.)
}

fn open_sized(
    cx: &mut TestAppContext,
    width: f32,
    height: f32,
) -> (AnyWindowHandle, Entity<AppView>, tempfile::TempDir) {
    cx.update(gpui_kit::init);
    let folder = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.options.language = crate::config::Language::from_name("english").unwrap();
    config.options.notifications = false;
    config.wizard.dismissed = true;
    config
        .functions
        .get_mut(&taprelay_core::function::FunctionId::MediaPlayPause)
        .unwrap()
        .enabled = true;
    let runtime = RuntimeHandle::spawn(
        config,
        |config| {
            let mut runtime = Runtime::new(config);
            runtime.state.adapter = true;
            runtime.state.peripheral = true;
            runtime.state.service = true;
            runtime.state.broadcasting = true;
            runtime.headless_tick();
            runtime
        },
        Runtime::headless_tick,
        64,
    )
    .unwrap();
    let mut controller = Controller::new(
        runtime,
        folder.path().join("config.json"),
        None,
        Shell::Headless,
        Default::default(),
        None,
    )
    .unwrap();
    controller.passed = true;
    controller.hidden = false;
    controller.startup_pending = false;
    let (handle, view) = cx.update(|cx| {
        gpui_kit::open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                    Default::default(),
                    size(px(width), px(height)),
                ))),
                ..Default::default()
            },
            cx,
            |window, cx| {
                cx.new(|cx| {
                    let mut view = AppView::new(controller, Default::default(), window, cx);
                    view.ui.mode = 2;
                    view
                })
            },
        )
        .unwrap()
    });
    (handle, view, folder)
}

#[gpui_kit::test]
fn settings_switch_between_catppuccin_themes_and_follow_system(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        let theme = gpui_kit::component::Theme::global(cx);
        assert_eq!(
            theme.theme_name().as_ref(),
            if view.read(cx).ui.dark {
                "Catppuccin Macchiato"
            } else {
                "Catppuccin Latte"
            }
        );
        window.render_frame(cx);
        window.click(("nav", 3usize), cx);
        for (choice, system_dark, name, background, primary) in [
            (2, false, "Catppuccin Macchiato", 0x1e2030, 0x8aadf4),
            (1, true, "Catppuccin Latte", 0xe5e9ef, 0x7287fd),
            (0, true, "Catppuccin Macchiato", 0x1e2030, 0x8aadf4),
            (0, false, "Catppuccin Latte", 0xe5e9ef, 0x7287fd),
        ] {
            view.update(cx, |view, _| {
                view.controller.system_theme = system_dark;
                view.controller.last_system_poll = Instant::now();
            });
            window.click(("theme", choice as usize), cx);
            let theme = gpui_kit::component::Theme::global(cx);
            assert_eq!(view.read(cx).ui.theme_choice, choice);
            assert_eq!(theme.theme_name().as_ref(), name);
            assert_eq!(theme.background, gpui_kit::rgb(background).into());
            assert_eq!(theme.primary, gpui_kit::rgb(primary).into());
            assert_eq!(theme.tokens.background, theme.background.into());
            assert_eq!(theme.button_primary, theme.primary);
            let color = theme.group_box.to_rgb();
            assert_eq!(
                view.read(cx).ui.overlay_background,
                [
                    (color.r * 255.) as u8,
                    (color.g * 255.) as u8,
                    (color.b * 255.) as u8,
                ]
            );
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn sidebar_toggle_keeps_its_position_when_collapsed(cx: &mut TestAppContext) {
    let (handle, _view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        cx.set_reduce_motion(true);
        window.render_frame(cx);
        let expanded = window.find("collapse").bounds();
        window.click("collapse", cx);
        assert_eq!(window.find("collapse").bounds(), expanded);
        let collapsed_item = window.find("0-0").bounds();
        assert_eq!(expanded.center().x, collapsed_item.center().x);
        window.click("collapse", cx);
        assert_eq!(window.find("collapse").bounds(), expanded);
    })
    .unwrap();
}

#[gpui_kit::test]
fn sidebar_animates_and_reverses_without_moving_the_toggle(cx: &mut TestAppContext) {
    let (handle, _view, _folder) = open(cx);
    cx.update(|cx| cx.set_reduce_motion(false));
    let measure = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            (
                window.find("navigation").bounds().size.width,
                window.find("collapse").bounds(),
            )
        })
        .unwrap()
    };
    let toggle = |cx: &mut TestAppContext| {
        cx.update_window(handle, |_, window, cx| window.click("collapse", cx))
            .unwrap();
    };
    let expanded = measure(cx);
    toggle(cx);
    assert_eq!(measure(cx), expanded);
    cx.executor().advance_clock(Duration::from_millis(80));
    let collapsing = measure(cx);
    assert!(collapsing.0 < expanded.0);
    assert_eq!(collapsing.1, expanded.1);
    toggle(cx);
    assert_eq!(measure(cx), collapsing);
    cx.executor().advance_clock(Duration::from_millis(250));
    assert_eq!(measure(cx), expanded);
    toggle(cx);
    cx.executor().advance_clock(Duration::from_millis(250));
    let collapsed = measure(cx);
    assert!(collapsed.0 < collapsing.0);
    assert_eq!(collapsed.1, expanded.1);
    toggle(cx);
    assert_eq!(measure(cx), collapsed);
    cx.executor().advance_clock(Duration::from_millis(80));
    let expanding = measure(cx);
    assert!(expanding.0 > collapsed.0 && expanding.0 < expanded.0);
    assert_eq!(expanding.1, expanded.1);
    cx.executor().advance_clock(Duration::from_millis(250));
    assert_eq!(measure(cx), expanded);
}

#[gpui_kit::test]
fn sidebar_toggles_and_navigates_in_both_locales(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        cx.set_reduce_motion(true);
        for language in ["english", "chinese"] {
            view.update(cx, |view, cx| {
                view.controller.runtime.config.options.language =
                    crate::config::Language::from_name(language).unwrap();
                view.controller.sync(&mut view.ui);
                cx.notify();
            });
            window.render_frame(cx);
            assert!(!view.read(cx).sidebar_collapsed);
            assert_eq!(window.find("collapse").label(), Some("Collapse sidebar"));
            let expanded_width = window.find("navigation").bounds().size.width;
            for collapsed in [true, false] {
                window.click("collapse", cx);
                assert_eq!(view.read(cx).sidebar_collapsed, collapsed);
                assert_eq!(
                    window.find("collapse").label(),
                    Some(if collapsed {
                        "Expand sidebar"
                    } else {
                        "Collapse sidebar"
                    })
                );
                let width = window.find("navigation").bounds().size.width;
                if collapsed {
                    assert!(width < expanded_width);
                } else {
                    assert_eq!(width, expanded_width);
                }
                for (item, page) in [("0-0", 0), ("0-1", 1), ("0-2", 4), ("0-3", 2)] {
                    window.click(item, cx);
                    assert_eq!(view.read(cx).ui.page, page);
                    assert_eq!(view.read(cx).sidebar_collapsed, collapsed);
                }
                window.click(("nav", 3usize), cx);
                assert_eq!(view.read(cx).ui.page, 3);
                assert!(window.find("page-header").visible());
            }
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn preserves_overview_proportions_in_both_themes_and_window_sizes(cx: &mut TestAppContext) {
    for (width, height) in [(900., 500.), (1100., 800.)] {
        let (handle, view, _folder) = open_sized(cx, width, height);
        cx.update_window(handle, |_, window, cx| {
            for dark in [false, true] {
                for (locale, language) in [("en", "english"), ("zh-CN", "chinese")] {
                    view.update(cx, |view, cx| {
                        view.controller.runtime.config.options.language =
                            crate::config::Language::from_name(language).unwrap();
                        view.ui.page = 0;
                        view.ui.dark = dark;
                        view.ui.locale = locale.into();
                        view.ui.elevated = true;
                        view.effects(window, cx);
                        cx.notify();
                    });
                    window.render_frame(cx);
                    let nav = window.find("navigation").bounds();
                    let header = window.find("page-header").bounds();
                    let status = window.find("status-card").bounds();
                    let shortcuts = window.find("shortcuts-card").bounds();
                    assert_eq!(nav.size.width, px(192.));
                    assert_eq!(nav.size.height, px(height));
                    assert_eq!(
                        header.origin,
                        gpui_kit::point(nav.right() + px(28.), px(28.))
                    );
                    assert_eq!(status.size.height, px(180.));
                    assert_eq!(status.origin.x, header.origin.x);
                    assert_eq!(status.right(), px(width - 28.));
                    assert_eq!(shortcuts.top(), status.bottom() + px(16.));
                    assert_eq!(shortcuts.bottom(), px(height - 28.));
                    assert!(shortcuts.size.height > px(100.));
                    let manage = window.find("manage-bindings").bounds();
                    let scroll = window
                        .find((gpui_kit::ElementId::from("overview-shortcuts"), "content"))
                        .bounds();
                    assert!(manage.is_contained_within(&shortcuts));
                    assert_eq!(scroll.left(), shortcuts.left() + px(16.));
                    assert_eq!(scroll.right(), shortcuts.right() - px(16.));
                    assert_eq!(scroll.bottom(), shortcuts.bottom() - px(16.));
                    assert!(scroll.top() >= manage.bottom() + px(16.));
                    assert_eq!(
                        window.find(("nav", 3usize)).bounds().bottom(),
                        px(height - 12.)
                    );
                    assert_eq!(window.find("listen").bounds().size.height, px(32.));
                }
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn overview_group_box_scrolls_long_shortcuts_without_moving_actions(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.ui.elevated = true;
            view.ui.bindings = (0..60)
                .map(|index| super::super::presentation::BindingRow {
                    text: format!("Ctrl + Shift + Shortcut {index}"),
                    function_name: format!("Action {index}"),
                    enabled: true,
                    ..Default::default()
                })
                .collect();
            cx.notify();
        });
        window.render_frame(cx);
        let first = "summary-Action 0-Ctrl + Shift + Shortcut 0";
        let last = "summary-Action 59-Ctrl + Shift + Shortcut 59";
        let card = window.find("shortcuts-card").bounds();
        let action = window.find("manage-bindings").bounds();
        assert!(window.find(first).visible());
        assert!(!window.find(last).visible());
        window.scroll(
            first,
            gpui_kit::ScrollDelta::Pixels(gpui_kit::point(px(0.), px(-5000.))),
            cx,
        );
        assert!(!window.find(first).visible());
        assert!(window.find(last).visible());
        assert!(window.find(last).bounds().is_contained_within(&card));
        assert_eq!(window.find("shortcuts-card").bounds(), card);
        assert_eq!(window.find("manage-bindings").bounds(), action);
        window.click("manage-bindings", cx);
        assert_eq!(view.read(cx).ui.page, 1);
    })
    .unwrap();
}

#[gpui_kit::test]
fn alerts_keep_actions_inside_with_long_messages_and_both_themes(cx: &mut TestAppContext) {
    for (width, height) in [(900., 500.), (1100., 800.)] {
        let (handle, view, _folder) = open_sized(cx, width, height);
        cx.update_window(handle, |_, window, cx| {
            for dark in [false, true] {
                for (locale, language) in [("en", "english"), ("zh-CN", "chinese")] {
                    view.update(cx, |view, cx| {
                        view.controller.runtime.config.options.language =
                            crate::config::Language::from_name(language).unwrap();
                        view.ui.page = 0;
                        view.ui.dark = dark;
                        view.ui.locale = locale.into();
                        view.ui.elevated = false;
                        view.ui.problem = format!("{} ", view.tr("permissions.warning")).repeat(4);
                        view.ui.problem_retry_allowed = true;
                        view.effects(window, cx);
                        cx.notify();
                    });
                    window.render_frame(cx);
                    for (alert_id, action_id) in [
                        ("admin-alert", "elevate"),
                        ("overview-alert", "overview-retry"),
                    ] {
                        let alert = window.find(alert_id).bounds();
                        let action = window.within(alert_id).find(action_id).bounds();
                        assert!(action.size.width > px(0.));
                        assert!(action.left() > alert.left());
                        assert!(action.right() < alert.right());
                        assert!(action.top() > alert.top());
                        assert!(action.bottom() < alert.bottom());
                        assert!(alert.right() <= px(width - 28.));
                    }
                    view.update(cx, |view, cx| {
                        view.ui.problem_retry_allowed = false;
                        cx.notify();
                    });
                    window.render_frame(cx);
                    assert!(window.try_find("overview-retry").is_none());
                    assert!(window.find("overview-alert").visible());
                }
            }
        })
        .unwrap();
    }
}

#[gpui_kit::test]
fn alert_retry_respects_disabled_state_and_accepts_pointer_and_keyboard(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        for (busy, keyboard) in [(true, false), (false, false), (false, true)] {
            let started = Instant::now() - Duration::from_secs(3);
            view.update(cx, |view, cx| {
                view.controller.started = started;
                view.ui.mode = 2;
                view.ui.page = 0;
                view.ui.problem = "Connection failed".into();
                view.ui.problem_busy = busy;
                view.ui.problem_retry_allowed = true;
                view.focus.focus(window, cx);
                cx.notify();
            });
            window.render_frame(cx);
            if keyboard {
                for _ in 0..16 {
                    if window.find("overview-retry").focused() == Some(true) {
                        break;
                    }
                    window.press("tab", cx);
                }
                assert_eq!(window.find("overview-retry").focused(), Some(true));
                window.press("enter", cx);
            } else {
                window.click("overview-retry", cx);
            }
            assert_eq!(view.read(cx).controller.started == started, busy);
        }
    })
    .unwrap();
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
fn environment_retry_waits_for_startup_checks(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        for busy in [true, false] {
            let started = Instant::now() - Duration::from_secs(1);
            view.update(cx, |view, cx| {
                view.controller.started = started;
                view.ui.mode = 0;
                view.ui.busy = busy;
                view.ui.problem_busy = false;
                view.ui.problem_retry_allowed = true;
                cx.notify();
            });
            window.render_frame(cx);
            window.click("retry-checks", cx);
            assert_eq!(view.read(cx).controller.started == started, busy);
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn receivers_use_group_box_insets_with_actions_in_the_header(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.ui.page = 2;
            view.ui.paired_devices = vec![super::super::presentation::DeviceRow {
                id: "tablet".into(),
                name: "Tablet with a long name 平板电脑".repeat(4),
                detail: "Ready".into(),
                device_kind: "tablet".into(),
                status: "ready".into(),
                action_label: "Disconnect".into(),
                enabled: true,
                ..Default::default()
            }];
            cx.notify();
        });
        window.render_frame(cx);
        let row = window.find("receiver-row-tablet").bounds();
        let action = window.find("device-tablet").bounds();
        assert_eq!(row.size.height, px(64.));
        assert_eq!(action.top(), row.top() + px(16.));
        assert_eq!(action.bottom(), row.bottom() - px(16.));
        assert_eq!(action.right(), row.right() - px(16.));
        assert_eq!(action.size.width, px(132.));
        assert_eq!(window.find("device-tablet").label(), Some("Start session"));
        assert!(row.contains(&action.center()));
        assert!(row.right() <= px(872.));
        let header = window.find("page-header").bounds();
        assert!(header.contains(&window.find("receiver-refresh").bounds().center()));
        assert!(header.contains(&window.find("receiver-settings").bounds().center()));
    })
    .unwrap();
}

fn settle(cx: &mut TestAppContext, handle: AnyWindowHandle, view: &Entity<AppView>) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        let busy = cx
            .update_window(handle, |_, window, cx| {
                view.update(cx, |view, cx| {
                    view.controller.runtime.poll();
                    view.controller.sync(&mut view.ui);
                    view.effects(window, cx);
                    cx.notify();
                    view.controller.runtime.busy()
                })
            })
            .unwrap();
        if !busy {
            break;
        }
        assert!(Instant::now() < deadline, "worker did not settle");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[gpui_kit::test]
fn edits_mapping_text_and_keeps_or_discards_the_draft(cx: &mut TestAppContext) {
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

#[gpui_kit::test]
fn invalid_group_name_stays_in_editor_and_save_persists(cx: &mut TestAppContext) {
    let (handle, view, folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.dispatch(Command::App(Action::Navigate, "4".into()), window, cx);
        });
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            // Edit a supplied group so this test never enumerates native windows.
            view.controller
                .runtime
                .config
                .foreground_app_rules
                .groups
                .insert(
                    "games".into(),
                    taprelay_core::foreground_app::ForegroundAppGroup {
                        name: "Games".into(),
                        foreground_apps: Default::default(),
                    },
                );
            view.dispatch(Command::Group("edit", "games".into()), window, cx);
        });
        window.render_frame(cx);
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
    let saved = Config::load(&folder.path().join("config.json")).unwrap();
    assert_eq!(saved.foreground_app_rules.groups["games"].name, "Work");
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.dispatch(Command::Group("edit", "games".into()), window, cx)
        });
        window.render_frame(cx);
        window.press("escape", cx);
        assert!(!view.read(cx).ui.groups.editor_open);
    })
    .unwrap();
}

#[gpui_kit::test]
fn renders_every_page_in_both_languages_and_the_minimum_window(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        for language in ["english", "chinese"] {
            for page in 0..5 {
                view.update(cx, |view, cx| {
                    view.dispatch(Command::App(Action::Language, language.into()), window, cx);
                    view.dispatch(Command::App(Action::Navigate, page.to_string()), window, cx);
                });
                window.render_frame(cx);
            }
        }
        for step in [0, 2, 3] {
            view.update(cx, |view, cx| {
                view.ui.mode = 1;
                view.ui.wizard_page = step;
                cx.notify();
            });
            window.render_frame(cx);
            assert!(window.find("wizard-next").visible());
        }
        for mode in [0, 3] {
            view.update(cx, |view, cx| {
                view.ui.mode = mode;
                cx.notify();
            });
            window.render_frame(cx);
            assert!(window.find("startup-quit").visible());
        }
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
fn homepage_ripple_tracks_inputs_and_clears_when_page_is_hidden(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.ui.page = 0;
            view.effects(window, cx);
            view.ui.input_count += 1;
            view.effects(window, cx);
            assert_eq!(view.ripples.frames(Instant::now()).len(), 1);
            cx.notify();
        });
        window.render_frame(cx);
        view.update(cx, |view, cx| {
            view.ui.page = 1;
            view.effects(window, cx);
            assert!(view.ripples.frames(Instant::now()).is_empty());
            view.ui.input_count += 10;
            view.ui.page = 0;
            view.effects(window, cx);
            assert!(view.ripples.frames(Instant::now()).is_empty());
            cx.set_reduce_motion(true);
            view.ui.input_count += 1;
            view.effects(window, cx);
            assert!(view.ripples.frames(Instant::now()).is_empty());
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn output_recording_accepts_tab_and_escape_without_moving_focus(cx: &mut TestAppContext) {
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
        ("escape", "key/41"),
    ] {
        cx.update_window(handle, |_, window, cx| {
            view.update(cx, |view, cx| {
                view.dispatch(
                    Command::Mapping("capture-output", String::new()),
                    window,
                    cx,
                )
            });
        })
        .unwrap();
        settle(cx, handle, &view);
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.press(key, cx);
            assert!(view.read(cx).focus.is_focused(window));
        })
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let recording = cx
                .update_window(handle, |_, window, cx| {
                    view.update(cx, |view, cx| {
                        view.controller.runtime.tick();
                        view.controller.sync(&mut view.ui);
                        view.effects(window, cx);
                        view.controller.recording()
                    })
                })
                .unwrap();
            if !recording {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "capture did not finish for {key}"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        cx.update(|cx| assert_eq!(view.read(cx).ui.mapping.selected_output.value, output));
    }
}
