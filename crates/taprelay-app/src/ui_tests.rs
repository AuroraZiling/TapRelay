//! Headless view validation only: no native window, permissions, input hooks or Bluetooth.
use crate::*;
use i_slint_backend_testing::{ElementHandle, ElementRoot};
use slint::{ComponentHandle, Model, ModelRc, VecModel, platform::WindowAdapter};

fn set_function_bindings(ui: &AppWindow, rows: Vec<FunctionBindingRow>) {
    ui.global::<BindingUi>()
        .set_rows(ModelRc::new(VecModel::from(rows)));
}

fn set_binding_capture(ui: &AppWindow, function_id: &str, slot: i32, text: &str, error: &str) {
    ui.global::<BindingUi>().set_capture(BindingCaptureState {
        function_id: function_id.into(),
        slot,
        text: text.into(),
        error: error.into(),
    });
}

fn binding_element(ui: &AppWindow, accessible_id: &str) -> ElementHandle {
    let accessible_id = accessible_id.to_owned();
    let query_id = accessible_id.clone();
    ui.root_element()
        .query_descendants()
        .match_predicate(move |element| {
            element
                .accessible_id()
                .is_some_and(|candidate| candidate.as_str() == query_id)
        })
        .find_first()
        .unwrap_or_else(|| panic!("missing accessible binding element: {accessible_id}"))
}

#[derive(Debug, PartialEq, Eq)]
enum BindingUiAction {
    Begin(String, i32),
    Delete(String, i32),
    Toggle(String, bool),
}

struct SoftwareTestPlatform {
    window: std::rc::Rc<slint::platform::software_renderer::MinimalSoftwareWindow>,
}

impl slint::platform::Platform for SoftwareTestPlatform {
    fn create_window_adapter(
        &self,
    ) -> Result<std::rc::Rc<dyn slint::platform::WindowAdapter>, slint::PlatformError> {
        Ok(self.window.clone())
    }

    fn duration_since_start(&self) -> std::time::Duration {
        std::time::Duration::from_millis(i_slint_backend_testing::get_mocked_time())
    }
}

#[test]
fn render_all_views_without_hardware() {
    let render_window = slint::platform::software_renderer::MinimalSoftwareWindow::new(
        slint::platform::software_renderer::RepaintBufferType::ReusedBuffer,
    );
    slint::platform::set_platform(Box::new(SoftwareTestPlatform {
        window: render_window.clone(),
    }))
    .unwrap();
    let ui = AppWindow::new().unwrap();
    render_pages(&ui);
    check_binding_capture(&ui);
    check_navigation_and_settings(&ui);
    check_surface_redraw(&ui, &render_window);
    check_binding_layouts_and_actions(&ui);
    check_about_links(&ui);
}

fn check_about_links(ui: &AppWindow) {
    use slint::platform::{PointerEventButton, WindowEvent};
    ui.set_mode(2);
    ui.set_page(3);
    let actions = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = actions.clone();
    ui.on_action(move |name, value| {
        observed
            .borrow_mut()
            .push((name.to_string(), value.to_string()));
    });
    for locale in ["en", "zh-cn"] {
        i18n::apply(ui, locale);
        for dark in [false, true] {
            ui.global::<Theme>().set_mode(if dark {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            });
            for (width, height) in [(900., 500.), (1120., 700.)] {
                ui.window().set_size(slint::LogicalSize::new(width, height));
                let _ = ui.window().take_snapshot().unwrap();
                ui.window().dispatch_event(WindowEvent::PointerScrolled {
                    position: slint::LogicalPosition::new(500., 350.),
                    delta_x: 0.,
                    delta_y: -3000.,
                });
                i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(250));
                slint::platform::update_timers_and_animations();
                let snapshot = ui.window().take_snapshot().unwrap();
                if let Some(dir) = std::env::var_os("TAPRELAY_UI_SNAPSHOT_DIR") {
                    let dir = std::path::PathBuf::from(dir);
                    std::fs::create_dir_all(&dir).unwrap();
                    std::fs::write(
                        dir.join(format!(
                            "about-{locale}-{dark}-{}x{}.rgba",
                            snapshot.width(),
                            snapshot.height()
                        )),
                        snapshot.as_bytes(),
                    )
                    .unwrap();
                }
                let license = binding_element(ui, "settings-license");
                let notices = binding_element(ui, "settings-third-party-notices");
                let repository = binding_element(ui, "settings-repository");
                assert!(
                    license.absolute_position().x + license.size().width
                        <= notices.absolute_position().x
                );
                assert_eq!(license.absolute_position().y, notices.absolute_position().y);
                assert!(
                    license.absolute_position().y + license.size().height
                        < repository.absolute_position().y
                );
                for (command, label_key) in [
                    ("repository", i18n::keys::SETTINGS_REPOSITORY),
                    ("license", i18n::keys::SETTINGS_LICENSE),
                    (
                        "third-party-notices",
                        i18n::keys::SETTINGS_THIRD_PARTY_NOTICES,
                    ),
                ] {
                    let button = binding_element(ui, &format!("settings-{command}"));
                    assert_eq!(
                        button.accessible_label().as_deref(),
                        Some(i18n::text(locale, label_key).as_str())
                    );
                    let origin = button.absolute_position();
                    let size = button.size();
                    assert!(origin.x >= 0. && origin.x + size.width <= width);
                    assert!(origin.y >= 0. && origin.y + size.height <= height);
                    actions.borrow_mut().clear();
                    let position = slint::LogicalPosition::new(
                        origin.x + size.width / 2.,
                        origin.y + size.height / 2.,
                    );
                    ui.window()
                        .dispatch_event(WindowEvent::PointerMoved { position });
                    ui.window().dispatch_event(WindowEvent::PointerPressed {
                        position,
                        button: PointerEventButton::Left,
                    });
                    ui.window().dispatch_event(WindowEvent::PointerReleased {
                        position,
                        button: PointerEventButton::Left,
                    });
                    button.invoke_accessible_default_action();
                    assert_eq!(*actions.borrow(), vec![(command.into(), String::new()); 2]);
                    assert!(crate::action::Action::try_from(command).is_ok());
                }
            }
        }
    }
}

fn render_pages(ui: &AppWindow) {
    ui.set_device_name("Living room tablet".into());
    ui.set_device_selected(true);
    ui.set_stage("Connected; waiting for HID subscription".into());
    ui.set_bindings(ModelRc::new(VecModel::from(vec![
        BindingRow {
            text: "F8".into(),
            function_name: "Play / pause".into(),
            keys: ModelRc::new(VecModel::from(vec!["F8".into()])),
            enabled: true,
        },
        BindingRow {
            text: "LCtrl + ]".into(),
            function_name: "Next track".into(),
            keys: ModelRc::new(VecModel::from(vec!["LCtrl".into(), "]".into()])),
            enabled: true,
        },
    ])));
    ui.set_paired_devices(ModelRc::new(VecModel::from(vec![DeviceRow {
        name: "Living room tablet".into(),
        detail: "Connected; preparing media controls".into(),
        selected: true,
        paired: true,
        action: "device".into(),
        action_label: "Connect".into(),
        ..Default::default()
    }])));
    ui.set_checks(ModelRc::new(VecModel::from(vec![
        CheckRow {
            title: "Bluetooth available".into(),
            state: 2,
        },
        CheckRow {
            title: "BLE peripheral support".into(),
            state: 2,
        },
        CheckRow {
            title: "HID service".into(),
            state: 1,
        },
        CheckRow {
            title: "Advertising".into(),
            state: 0,
        },
    ])));
    ui.set_language_options(i18n::language_options("en"));
    ui.set_data_directory("D:\\Apps\\TapRelay".into());
    ui.set_app_version(version::VERSION.into());
    for (locale, dark, width, height) in [("en", true, 1000., 700.), ("zh-cn", false, 800., 560.)] {
        i18n::apply(ui, locale);
        let global = ui.global::<I18n>();
        assert_eq!(global.get_locale(), locale);
        assert_eq!(
            global.get_text().nav_overview,
            i18n::text(locale, i18n::keys::NAV_OVERVIEW)
        );
        ui.global::<Theme>().set_mode(if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        });
        ui.set_bluetooth_available(dark);
        ui.window().set_size(slint::LogicalSize::new(width, height));
        for mode in 0..3 {
            ui.set_mode(mode);
            for page in 0..if mode == 0 { 1 } else { 4 } {
                ui.set_page(page);
                ui.set_wizard_page(page.min(3));
                ui.set_problem(
                    if !dark && ((mode == 1 && page == 2) || (mode == 2 && page == 2)) {
                        "Bluetooth service unavailable".into()
                    } else {
                        "".into()
                    },
                );
                ui.show().unwrap();
                i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(100));
                let _ = ui.window().take_snapshot().unwrap();
            }
        }
    }
}

fn check_binding_capture(ui: &AppWindow) {
    ui.window().set_size(slint::LogicalSize::new(900., 500.));
    ui.set_mode(1);
    ui.set_wizard_page(1);
    let shortcut = ShortcutItem {
        text: "F8".into(),
        keys: ModelRc::new(VecModel::from(vec!["F8".into()])),
        slot: 0,
        enabled: true,
    };
    let row = FunctionBindingRow {
        id: "media.play-pause".into(),
        label: "Play / Pause".into(),
        gestures: ModelRc::new(VecModel::from(vec![GestureAction {
            gesture: "Tap".into(),
            action: "Play / Pause".into(),
        }])),
        enabled: true,
        shortcuts: ModelRc::new(VecModel::from(vec![shortcut])),
    };
    set_function_bindings(ui, vec![row.clone()]);
    set_binding_capture(
        ui,
        "media.play-pause",
        0,
        "请按下快捷键，松开完成",
        "无效组合",
    );
    let _ = ui.window().take_snapshot().unwrap();
    let error = binding_element(ui, "binding-error-media.play-pause");
    assert_eq!(
        error.accessible_live_region(),
        Some(i_slint_backend_testing::AccessibleLiveness::Polite)
    );
    assert_eq!(error.accessible_label().as_deref(), Some("无效组合"));
    let captured_keys = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = captured_keys.clone();
    ui.global::<BindingUi>()
        .on_key_input(move |text, down| observed.borrow_mut().push((text.to_string(), down)));
    let cancelled = std::rc::Rc::new(std::cell::Cell::new(false));
    let observed = cancelled.clone();
    ui.global::<BindingUi>()
        .on_cancel_capture(move || observed.set(true));
    let mut empty_row = row.clone();
    empty_row.shortcuts = ModelRc::new(VecModel::default());
    set_function_bindings(ui, vec![empty_row]);
    set_binding_capture(ui, "media.play-pause", 0, "请按下快捷键，松开完成", "");
    let _ = ui.window().take_snapshot().unwrap();
    for key in [
        slint::platform::Key::F8.into(),
        slint::SharedString::from("k"),
    ] {
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyPressed { text: key.clone() });
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::KeyReleased { text: key });
    }
    let recorded: Vec<_> = captured_keys
        .borrow()
        .iter()
        .map(|(text, down)| (crate::capture_key::virtual_key(text), *down))
        .collect();
    assert_eq!(
        recorded,
        vec![
            (Some(0x77), true),
            (Some(0x77), false),
            (Some(0x4b), true),
            (Some(0x4b), false)
        ]
    );
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed {
            text: slint::platform::Key::Escape.into(),
        });
    assert!(
        cancelled.get(),
        "An empty slot must receive Escape without an extra click"
    );
    set_binding_capture(ui, "", -1, "", "");
}

fn check_navigation_and_settings(ui: &AppWindow) {
    // Exercise real pointer routing through the tooltip wrapper, not just callback invocation.
    i18n::apply(ui, "en");
    ui.set_mode(2);
    ui.set_page(0);
    ui.window().set_size(slint::LogicalSize::new(1000., 700.));
    let _ = ui.window().take_snapshot().unwrap();
    let actions = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let a = actions.clone();
    ui.on_action(move |name, value| a.borrow_mut().push((name.to_string(), value.to_string())));
    use slint::platform::{PointerEventButton, WindowEvent};
    let position = slint::LogicalPosition::new(30., 670.);
    ui.window().dispatch_event(WindowEvent::PointerMoved {
        position: slint::LogicalPosition::new(70., 500.),
    });
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    assert!(
        actions.borrow().contains(&("navigate".into(), "3".into())),
        "Sidebar tooltip must not consume navigation clicks"
    );
    ui.set_page(3);
    let _ = ui.window().take_snapshot().unwrap();

    // The remaining log UI opens the on-disk directory from Settings.
    ui.window().dispatch_event(WindowEvent::PointerScrolled {
        position: slint::LogicalPosition::new(500., 500.),
        delta_x: 0.,
        delta_y: -2000.,
    });
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(100));
    let _ = ui.window().take_snapshot().unwrap();
    let position = slint::LogicalPosition::new(260., 654.);
    ui.window()
        .dispatch_event(WindowEvent::PointerMoved { position });
    ui.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Left,
    });
    ui.window().dispatch_event(WindowEvent::PointerReleased {
        position,
        button: PointerEventButton::Left,
    });
    assert!(
        actions
            .borrow()
            .contains(&("logs-folder".into(), "".into()))
    );
    let _ = ui.window().take_snapshot().unwrap();

    // Every settings switch owns a named command, so one control can never be
    // wired to another's setting the way the shared "setting" id allowed.
    // Clipped rows leave the accessibility tree, so grow the viewport first.
    ui.window().set_size(slint::LogicalSize::new(1000., 1100.));
    let _ = ui.window().take_snapshot().unwrap();
    for (accessible_id, command) in [
        ("setting-autostart", "set-autostart"),
        ("setting-start-hidden", "set-start-hidden"),
        ("setting-auto-listen", "set-auto-listen"),
        ("setting-close-to-tray", "set-close-to-tray"),
        ("setting-always-admin", "set-always-admin"),
        (
            "setting-connection-wait-warning",
            "set-connection-wait-warning",
        ),
        ("setting-notifications", "set-notifications"),
    ] {
        actions.borrow_mut().clear();
        let switch = binding_element(ui, accessible_id);
        switch.invoke_accessible_default_action();
        switch.invoke_accessible_default_action();
        let emitted = actions.borrow().clone();
        assert_eq!(
            emitted.len(),
            2,
            "{accessible_id} must send one command per toggle"
        );
        assert!(
            emitted.iter().all(|(name, _)| name == command),
            "{accessible_id} must send {command}, got {emitted:?}"
        );
        assert!(
            crate::action::Action::try_from(command).is_ok(),
            "{command} must also be a known action id in src/action.rs"
        );
        assert_eq!(
            emitted[0].1,
            if emitted[1].1 == "1" { "0" } else { "1" },
            "{command} must carry the switch's new state"
        );
    }
    ui.window().set_size(slint::LogicalSize::new(1000., 700.));
    let _ = ui.window().take_snapshot().unwrap();
}

fn check_surface_redraw(
    ui: &AppWindow,
    render_window: &slint::platform::software_renderer::MinimalSoftwareWindow,
) {
    // Seed the retained renderer cache and confirm unchanged UI has no damage.
    use slint::platform::software_renderer::PremultipliedRgbaColor;
    let mut frame = vec![PremultipliedRgbaColor::default(); 1000 * 700];
    render_window.window().request_redraw();
    let mut first_region = None;
    assert!(render_window.draw_if_needed(|renderer| {
        first_region = Some(renderer.render(frame.as_mut_slice(), 1000));
    }));
    assert_eq!(
        first_region.unwrap().bounding_box_size(),
        slint::PhysicalSize::new(1000, 700)
    );

    render_window.window().request_redraw();
    let mut unchanged_region = None;
    assert!(render_window.draw_if_needed(|renderer| {
        unchanged_region = Some(renderer.render(frame.as_mut_slice(), 1000));
    }));
    assert_eq!(
        unchanged_region.unwrap().bounding_box_size(),
        slint::PhysicalSize::default()
    );

    #[cfg(windows)]
    {
        let expected = frame.clone();
        // Model a lost surface, without minimizing or changing any UI property.
        // Exercise the same invalidation used before native RedrawRequested.
        for _ in 0..3 {
            frame.fill(PremultipliedRgbaColor::default());
            crate::window_rendering::invalidate_surface(ui.window());
            render_window.window().request_redraw();
            assert!(render_window.draw_if_needed(|renderer| {
                let region = renderer.render(frame.as_mut_slice(), 1000);
                assert_eq!(
                    region.bounding_box_origin(),
                    slint::PhysicalPosition::default()
                );
                assert_eq!(
                    region.bounding_box_size(),
                    slint::PhysicalSize::new(1000, 700)
                );
            }));
            assert!(
                frame
                    .iter()
                    .zip(&expected)
                    .all(|(a, b)| (a.red, a.green, a.blue, a.alpha)
                        == (b.red, b.green, b.blue, b.alpha)),
                "expose must recover every pixel without UI changes"
            );
            assert!(
                !render_window.draw_if_needed(|_| panic!("expose must not create a redraw loop"))
            );
        }
    }
}

fn check_binding_layouts_and_actions(ui: &AppWindow) {
    use taprelay_core::function::FUNCTION_CATALOG;
    set_binding_capture(ui, "", -1, "", "");
    ui.set_problem("".into());
    for (locale, dark, width, height) in [
        ("zh-cn", true, 900., 500.),
        ("en", false, 900., 500.),
        ("zh-cn", false, 1120., 700.),
        ("en", true, 1120., 700.),
    ] {
        i18n::apply(ui, locale);
        ui.global::<Theme>().set_mode(if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        });
        ui.window().set_size(slint::LogicalSize::new(width, height));
        let rows: Vec<_> = FUNCTION_CATALOG
            .iter()
            .enumerate()
            .map(|(i, d)| FunctionBindingRow {
                id: d.id.stable_id().into(),
                label: i18n::text(locale, d.name_key).into(),
                gestures: ModelRc::new(VecModel::from({
                    let mut gestures = vec![GestureAction {
                        gesture: i18n::text(locale, "bindings.gesture.press").into(),
                        action: i18n::text(locale, d.tap_action.name_key()).into(),
                    }];
                    if let Some(hold) = d.hold_action {
                        gestures.push(GestureAction {
                            gesture: i18n::text(locale, "bindings.gesture.hold").into(),
                            action: i18n::text(locale, hold.name_key()).into(),
                        });
                    }
                    gestures
                })),
                enabled: true,
                shortcuts: ModelRc::new(VecModel::from(
                    (0..(2 - i % 3))
                        .map(|slot| ShortcutItem {
                            text: if slot == 0 {
                                "Ctrl + Shift + Alt + Win + PageDown"
                            } else {
                                "Ctrl + Shift + Alt + Mouse Button 5"
                            }
                            .into(),
                            slot: slot as i32,
                            enabled: true,
                            ..Default::default()
                        })
                        .collect::<Vec<_>>(),
                )),
            })
            .collect();
        let mut rows = rows;
        rows[3].enabled = false;
        rows[2].enabled = false;
        set_function_bindings(ui, rows);
        ui.set_mode(2);
        ui.set_page(1);
        let _ = ui.window().take_snapshot().unwrap();
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(250));
        slint::platform::update_timers_and_animations();
        let _ = ui.window().take_snapshot().unwrap();
        let function_column = binding_element(ui, "binding-function-media.play-pause");
        for slot in 0..2 {
            let shortcut = binding_element(ui, &format!("binding-slot-media.play-pause-{slot}"));
            assert!(
                shortcut.absolute_position().x + shortcut.size().width
                    <= function_column.absolute_position().x,
                "Long shortcut labels must not invade the trigger column"
            );
        }
    }
    let actions = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = actions.clone();
    ui.global::<BindingUi>()
        .on_begin_capture(move |function, slot| {
            observed
                .borrow_mut()
                .push(BindingUiAction::Begin(function.to_string(), slot));
        });
    let observed = actions.clone();
    ui.global::<BindingUi>()
        .on_delete_shortcut(move |function, slot| {
            observed
                .borrow_mut()
                .push(BindingUiAction::Delete(function.to_string(), slot));
        });
    let observed = actions.clone();
    ui.global::<BindingUi>()
        .on_set_function_enabled(move |function, enabled| {
            observed
                .borrow_mut()
                .push(BindingUiAction::Toggle(function.to_string(), enabled));
        });
    ui.set_mode(2);
    ui.set_page(1);
    let _ = ui.window().take_snapshot().unwrap();
    let disabled_second = binding_element(ui, "binding-disabled-slot-media.mute-1");
    let function_column = binding_element(ui, "binding-function-media.mute");
    assert!(
        disabled_second.absolute_position().x + disabled_second.size().width
            <= function_column.absolute_position().x,
        "Disabled shortcut chips must stay inside the shortcut column"
    );
    binding_element(ui, "binding-slot-media.play-pause-1").invoke_accessible_default_action();
    binding_element(ui, "binding-add-media.previous").invoke_accessible_default_action();
    assert_eq!(
        *actions.borrow(),
        vec![
            BindingUiAction::Begin("media.play-pause".into(), 1),
            BindingUiAction::Begin("media.previous".into(), 1),
        ],
        "Both an existing second shortcut and the add-second control must be interactive"
    );
    assert_eq!(
        ui.global::<BindingUi>()
            .get_rows()
            .row_data(0)
            .unwrap()
            .shortcuts
            .row_count(),
        2,
        "Presenting one shortcut must not truncate the multi-shortcut model"
    );
    actions.borrow_mut().clear();
    binding_element(ui, "binding-delete-media.play-pause-0").invoke_accessible_default_action();
    assert!(
        actions
            .borrow()
            .contains(&BindingUiAction::Delete("media.play-pause".into(), 0)),
        "The shortcut delete button must expose its default accessibility action"
    );
    assert!(
        !actions
            .borrow()
            .iter()
            .any(|action| matches!(action, BindingUiAction::Begin(_, _))),
        "Deleting a shortcut must not start recording"
    );
    actions.borrow_mut().clear();
    let enabled_switch = binding_element(ui, "binding-enabled-media.play-pause");
    assert_eq!(
        enabled_switch.accessible_role(),
        Some(i_slint_backend_testing::AccessibleRole::Switch)
    );
    enabled_switch.invoke_accessible_default_action();
    binding_element(ui, "binding-enabled-media.next").invoke_accessible_default_action();
    assert_eq!(
        *actions.borrow(),
        vec![
            BindingUiAction::Toggle("media.play-pause".into(), false),
            BindingUiAction::Toggle("media.next".into(), true),
        ],
        "Switches must disable active functions and enable disabled unbound functions"
    );
    assert_eq!(
        ui.global::<BindingUi>().get_rows().row_count(),
        FUNCTION_CATALOG.len()
    );
    assert_eq!(
        ui.global::<BindingUi>()
            .get_rows()
            .row_data(0)
            .unwrap()
            .shortcuts
            .row_count(),
        2
    );
    set_binding_capture(ui, "media.play-pause", 0, "Recording", "");
    let _ = ui.window().take_snapshot().unwrap();
    actions.borrow_mut().clear();
    let disabled_switch = binding_element(ui, "binding-enabled-media.next");
    assert_eq!(disabled_switch.accessible_enabled(), Some(false));
    disabled_switch.invoke_accessible_default_action();
    assert!(
        actions.borrow().is_empty(),
        "Recording must lock function switches"
    );
    set_binding_capture(ui, "", -1, "", "");
    ui.window().set_size(slint::LogicalSize::new(1120., 900.));
    let _ = ui.window().take_snapshot().unwrap();
    let app = ui.global::<BindingUi>().get_rows().row_data(4).unwrap();
    assert_eq!(app.id.as_str(), "app.toggle-listening");
    actions.borrow_mut().clear();
    binding_element(ui, "binding-enabled-app.toggle-listening").invoke_accessible_default_action();
    binding_element(ui, "binding-slot-app.toggle-listening-0").invoke_accessible_default_action();
    assert_eq!(
        *actions.borrow(),
        vec![
            BindingUiAction::Toggle("app.toggle-listening".into(), false),
            BindingUiAction::Begin("app.toggle-listening".into(), 0),
        ]
    );
}
