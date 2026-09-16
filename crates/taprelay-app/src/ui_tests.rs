//! Headless view validation only: no native window, permissions, input hooks or Bluetooth.
use crate::*;
use slint::{ComponentHandle, ModelRc, VecModel, platform::WindowAdapter};

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
    ui.set_device_name("Living room tablet".into());
    ui.set_device_selected(true);
    ui.set_stage("Connected; waiting for HID subscription".into());
    ui.set_bindings(ModelRc::new(VecModel::from(vec![
        BindingRow {
            text: "F8".into(),
            keys: ModelRc::new(VecModel::from(vec!["F8".into()])),
            enabled: true,
        },
        BindingRow {
            text: "LCtrl + ]".into(),
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
    ui.set_data_directory("D:\\Apps\\TapRelay".into());
    ui.set_app_version(version::VERSION.into());
    let out = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/gui-previews");
    std::fs::create_dir_all(&out).unwrap();
    for (suffix, zh, dark, width, height) in [
        ("en-dark", false, true, 1000., 700.),
        ("zh-light", true, false, 800., 560.),
    ] {
        i18n::apply(&ui, zh);
        ui.global::<Theme>().set_mode(if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        });
        ui.set_bluetooth_available(dark);
        ui.window().set_size(slint::LogicalSize::new(width, height));
        for mode in 0..3 {
            ui.set_mode(mode);
            for page in 0..if mode == 0 {
                1
            } else if mode == 1 {
                4
            } else {
                5
            } {
                ui.set_page(page);
                ui.set_wizard_page(page.min(3));
                ui.set_problem(
                    if !dark && ((mode == 1 && page == 2) || (mode == 2 && page == 3)) {
                        "Bluetooth service unavailable".into()
                    } else {
                        "".into()
                    },
                );
                ui.show().unwrap();
                i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(100));
                let shot = ui.window().take_snapshot().unwrap();
                assert_eq!(shot.width(), width as u32);
                assert_eq!(shot.height(), height as u32);
                if mode == 2 && page == 4 && dark {
                    let width = shot.width() as usize;
                    let background = shot.as_slice()[300 * width + 70];
                    let mut longest = 0;
                    for y in 400..shot.height() as usize {
                        let mut run = 0;
                        for x in 60..width.saturating_sub(20) {
                            if shot.as_slice()[y * width + x] != background {
                                run += 1;
                                longest = longest.max(run);
                            } else {
                                run = 0;
                            }
                        }
                    }
                    assert!(
                        longest < 500,
                        "The administrator restart button must size to its content, not span {longest}px"
                    );
                }
                let mut bytes =
                    format!("P6\n{} {}\n255\n", shot.width(), shot.height()).into_bytes();
                for pixel in shot.as_slice() {
                    bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
                }
                std::fs::write(out.join(format!("{mode}-{page}-{suffix}.ppm")), bytes).unwrap();
            }
        }
    }
    // Tap waves must change pixels beyond the counter, then disappear completely.
    ui.set_mode(1);
    ui.set_wizard_page(3);
    ui.set_input_count(0);
    ui.window().set_size(slint::LogicalSize::new(900., 500.));
    let idle = ui.window().take_snapshot().unwrap();
    ui.set_input_count(1);
    slint::platform::update_timers_and_animations();
    let _ = ui.window().take_snapshot().unwrap();
    for _ in 0..10 {
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(16));
        slint::platform::update_timers_and_animations();
    }
    let wave = ui.window().take_snapshot().unwrap();
    let mut bytes = format!("P6\n{} {}\n255\n", wave.width(), wave.height()).into_bytes();
    for pixel in wave.as_slice() {
        bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
    }
    std::fs::write(out.join("wizard-tap-wave.ppm"), bytes).unwrap();
    let changed = idle
        .as_slice()
        .iter()
        .zip(wave.as_slice())
        .filter(|(a, b)| a != b)
        .count();
    assert!(
        changed > 300,
        "Tap must animate beyond the counter: {changed}"
    );
    ui.set_input_count(2);
    let _ = ui.window().take_snapshot().unwrap();
    for _ in 0..60 {
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(16));
        slint::platform::update_timers_and_animations();
    }
    ui.set_input_count(0);
    let reset = ui.window().take_snapshot().unwrap();
    assert_eq!(
        idle.as_slice(),
        reset.as_slice(),
        "Waves must settle; counter reset must not emit a wave"
    );
    // Exercise the shared receiver page with real typed view projections.
    use taprelay_core::{
        devices::*,
        state::{Knowledge, Snapshot, Target},
    };
    ui.set_mode(1);
    ui.set_wizard_page(2);
    for (name, adapter, discovery, connection) in [
        (
            "off",
            AdapterState::Disabled,
            DiscoveryState::Idle,
            Connection::Disconnected,
        ),
        (
            "unavailable",
            AdapterState::Unavailable,
            DiscoveryState::Idle,
            Connection::Disconnected,
        ),
        (
            "scanning",
            AdapterState::Available,
            DiscoveryState::Scanning,
            Connection::Disconnected,
        ),
        (
            "empty",
            AdapterState::Available,
            DiscoveryState::ResultsAvailable,
            Connection::Disconnected,
        ),
        (
            "paired",
            AdapterState::Available,
            DiscoveryState::ResultsAvailable,
            Connection::Disconnected,
        ),
        (
            "connecting",
            AdapterState::Available,
            DiscoveryState::ResultsAvailable,
            Connection::Connecting,
        ),
        (
            "connected",
            AdapterState::Available,
            DiscoveryState::ResultsAvailable,
            Connection::Connected,
        ),
        (
            "failed",
            AdapterState::Available,
            DiscoveryState::Failed,
            Connection::Failed,
        ),
    ] {
        let mut state = Snapshot {
            adapter_state: adapter,
            discovery,
            selected: Some("tablet".into()),
            ..Default::default()
        };
        if !matches!(name, "off" | "unavailable" | "scanning" | "empty") {
            state.targets.push(Target {
                id: "tablet".into(),
                name: "Artemis’s iPad".into(),
                pairing: Knowledge::Yes,
                availability: Availability::Nearby,
                connection,
                ..Default::default()
            });
            state.targets.push(Target {
                id: "other".into(),
                name: "Another nearby device".into(),
                pairing: Knowledge::No,
                availability: Availability::Nearby,
                ..Default::default()
            });
            state.target_status = Some(state.targets[0].clone());
        }
        state.ready = connection == Connection::Connected;
        let rows: Vec<_> = state
            .targets
            .iter()
            .map(|t| receiver_view::row(t, &state, |key| i18n::text(false, key).into()))
            .collect();
        ui.set_paired_devices(ModelRc::new(VecModel::from(
            rows.iter()
                .filter(|r| r.paired)
                .cloned()
                .collect::<Vec<_>>(),
        )));
        ui.set_adapter_label(i18n::text(false, receiver_view::adapter_label_key(adapter)).into());
        ui.set_bluetooth_available(adapter == AdapterState::Available);
        ui.set_discovery_status(i18n::text(false, receiver_view::page_status_key(&state)).into());
        ui.set_scanning(discovery == DiscoveryState::Scanning);
        ui.set_receiver_next_allowed(receiver_next_allowed(&state));
        assert_eq!(ui.get_receiver_next_allowed(), name == "connected");
        i18n::apply(&ui, false);
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(100));
        let shot = ui.window().take_snapshot().unwrap();
        let mut bytes = format!("P6\n{} {}\n255\n", shot.width(), shot.height()).into_bytes();
        for pixel in shot.as_slice() {
            bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
        }
        std::fs::write(out.join(format!("receiver-{name}.ppm")), bytes).unwrap();
        if name == "connected" {
            let mut hovered = None;
            for x in [598., 600., 604., 608., 598.] {
                ui.window()
                    .dispatch_event(slint::platform::WindowEvent::PointerMoved {
                        position: slint::LogicalPosition::new(x, 191.),
                    });
                i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(1000));
                let current = ui.window().take_snapshot().unwrap();
                let pixels = current.as_slice().to_vec();
                assert!(pixels != shot.as_slice(), "Hover must display a tooltip");
                assert!(
                    pixels
                        .iter()
                        .zip(shot.as_slice())
                        .enumerate()
                        .any(|(i, (a, b))| {
                            let y = i / current.width() as usize;
                            !(163..219).contains(&y) && a != b
                        }),
                    "The tooltip bubble must be visible outside the row, not only the hover highlight"
                );
                assert!(
                    hovered.get_or_insert(pixels.clone()) == &pixels,
                    "Tooltip must remain stable while moving over the status icon"
                );
            }
            ui.window()
                .dispatch_event(slint::platform::WindowEvent::PointerMoved {
                    position: slint::LogicalPosition::new(400., 350.),
                });
            i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(1000));
            let _ = ui.window().take_snapshot().unwrap();
            i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(1000));
            let left = ui.window().take_snapshot().unwrap();
            assert!(
                left.as_slice()[..800 * 400] == shot.as_slice()[..800 * 400],
                "Leaving the icon must dismiss the tooltip and highlight"
            );
        }
    }
    // Preview the new function-grouped binding page at minimum width, and
    // verify that its empty-slot recorder receives keyboard focus.
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
    ui.set_function_bindings(ModelRc::new(VecModel::from(vec![row.clone()])));
    ui.set_capture_text("请按下快捷键，松开完成".into());
    for (name, function, slot) in [
        ("edit", "media.play-pause", 0),
        ("new", "media.play-pause", 0),
        ("error", "media.play-pause", 0),
        ("empty", "", -1),
    ] {
        let mut preview_row = row.clone();
        if name == "new" {
            preview_row.shortcuts = ModelRc::new(VecModel::default());
        }
        ui.set_function_bindings(ModelRc::new(VecModel::from(vec![preview_row])));
        ui.set_capture_function(function.into());
        ui.set_capture_slot(slot);
        ui.set_capture_error(if name == "error" { "无效组合" } else { "" }.into());
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(250));
        let shot = ui.window().take_snapshot().unwrap();
        if name == "error" {
            let stride = shot.width() as usize;
            let red = |x: usize, y: usize| {
                let p = shot.as_slice()[y * stride + x];
                i16::from(p.r) > i16::from(p.g) + 30
            };
            assert!(
                (190..220).any(|y| (230..530).any(|x| red(x, y))),
                "Validation must appear to the right of the recording chip"
            );
            assert!(
                !(238..270).any(|y| (28..870).any(|x| red(x, y))),
                "Validation must not appear below or increase the row height"
            );
            assert_eq!(
                shot.as_slice()[245 * stride + 40],
                shot.as_slice()[280 * stride + 40],
                "The row must end at the same height when validation fails"
            );
        }
        let mut bytes = format!("P6\n{} {}\n255\n", shot.width(), shot.height()).into_bytes();
        for pixel in shot.as_slice() {
            bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
        }
        std::fs::write(out.join(format!("bindings-{name}.ppm")), bytes).unwrap();
    }
    let capture_actions = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = capture_actions.clone();
    ui.on_action(move |name, index, text| {
        observed
            .borrow_mut()
            .push((name.to_string(), index, text.to_string()))
    });
    let mut empty_row = row.clone();
    empty_row.shortcuts = ModelRc::new(VecModel::default());
    ui.set_function_bindings(ModelRc::new(VecModel::from(vec![empty_row])));
    ui.set_capture_function("media.play-pause".into());
    ui.set_capture_slot(0);
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
    let recorded: Vec<_> = capture_actions
        .borrow()
        .iter()
        .filter(|(name, _, _)| name == "capture-key")
        .map(|(_, down, text)| (crate::capture_key::virtual_key(text), *down))
        .collect();
    assert_eq!(
        recorded,
        vec![
            (Some(0x77), 1),
            (Some(0x77), 0),
            (Some(0x4b), 1),
            (Some(0x4b), 0)
        ]
    );
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::KeyPressed {
            text: slint::platform::Key::Escape.into(),
        });
    assert!(
        capture_actions
            .borrow()
            .iter()
            .any(|(action, _, _)| action == "cancel-capture"),
        "An empty slot must receive Escape without an extra click"
    );
    // Exercise real pointer routing through the tooltip wrapper, not just callback invocation.
    i18n::apply(&ui, false);
    ui.set_mode(2);
    ui.set_page(0);
    ui.window().set_size(slint::LogicalSize::new(1000., 700.));
    let _ = ui.window().take_snapshot().unwrap();
    let actions = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let a = actions.clone();
    ui.on_action(move |name, index, _| a.borrow_mut().push((name.to_string(), index)));
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
        actions.borrow().contains(&("navigate".into(), 4)),
        "Sidebar tooltip must not consume navigation clicks"
    );
    ui.set_page(4);
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
    assert!(actions.borrow().contains(&("logs-folder".into(), 0)));
    let _ = ui.window().take_snapshot().unwrap();

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
    preview_function_layouts(&ui, &out);
    preview_fixed_overview(&ui, &out);
}

// Use the real catalog and deliberately long shortcuts, rather than English
// placeholders in every locale. This exercises the actual nested page layouts.
fn preview_function_layouts(ui: &AppWindow, out: &std::path::Path) {
    use taprelay_core::function::FUNCTION_CATALOG;
    ui.set_capture_function("".into());
    ui.set_capture_slot(-1);
    ui.set_capture_error("".into());
    ui.set_problem("".into());
    for (suffix, zh, dark, width, height) in [
        ("zh-dark-min", true, true, 900., 500.),
        ("en-light-min", false, false, 900., 500.),
        ("zh-light", true, false, 1120., 700.),
        ("en-dark", false, true, 1120., 700.),
    ] {
        i18n::apply(ui, zh);
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
                label: i18n::text(zh, d.name_key).into(),
                gestures: ModelRc::new(VecModel::from({
                    let mut gestures = vec![GestureAction {
                        gesture: i18n::text(zh, "bindings.gesture.press").into(),
                        action: i18n::text(zh, d.action.name_key()).into(),
                    }];
                    if let Some(hold) = d.hold_action {
                        gestures.push(GestureAction {
                            gesture: i18n::text(zh, "bindings.gesture.hold").into(),
                            action: i18n::text(zh, hold.name_key()).into(),
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
        ui.set_function_bindings(ModelRc::new(VecModel::from(rows)));
        for mode in [2, 1] {
            ui.set_mode(mode);
            {
                let page = 2;
                ui.set_page(page);
                ui.set_wizard_page(0);
                let _ = ui.window().take_snapshot().unwrap();
                i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(250));
                slint::platform::update_timers_and_animations();
                let shot = ui.window().take_snapshot().unwrap();
                assert_eq!((shot.width(), shot.height()), (width as u32, height as u32));
                if mode == 2 && page == 2 {
                    // The shortcut must leave the trigger column clear.
                    let stride = shot.width() as usize;
                    let gap_x = width as usize - 368;
                    let background = shot.as_slice()[165 * stride + gap_x];
                    assert!(
                        (176..201).all(|y| (gap_x..gap_x + 5)
                            .all(|x| shot.as_slice()[y * stride + x] == background)),
                        "Long shortcut labels must not invade the trigger column"
                    );
                }
                let mut bytes =
                    format!("P6\n{} {}\n255\n", shot.width(), shot.height()).into_bytes();
                for pixel in shot.as_slice() {
                    bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
                }
                std::fs::write(
                    out.join(format!("layout-{mode}-{page}-{suffix}.ppm")),
                    bytes,
                )
                .unwrap();
            }
        }
    }
    let actions = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = actions.clone();
    ui.on_action(move |name, slot, value| {
        observed
            .borrow_mut()
            .push((name.to_string(), slot, value.to_string()))
    });
    let click = |x, y| {
        use slint::platform::{PointerEventButton, WindowEvent};
        let position = slint::LogicalPosition::new(x, y);
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
    };
    ui.set_mode(2);
    ui.set_page(2);
    let _ = ui.window().take_snapshot().unwrap();
    click(452., 188.);
    click(435., 260.);
    assert!(
        actions.borrow().is_empty(),
        "Neither a saved second shortcut nor an add-second button should be interactive"
    );
    use slint::Model;
    assert_eq!(
        ui.get_function_bindings()
            .row_data(0)
            .unwrap()
            .shortcuts
            .row_count(),
        2,
        "Presenting one shortcut must not truncate the multi-shortcut model"
    );
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::PointerMoved {
            position: slint::LogicalPosition::new(395., 188.),
        });
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(250));
    slint::platform::update_timers_and_animations();
    let hovered = ui.window().take_snapshot().unwrap();
    assert!(
        (178..198).any(|y| (385..405).any(|x| {
            let p = hovered.as_slice()[y * hovered.width() as usize + x];
            i16::from(p.r) > i16::from(p.g) + 20
        })),
        "The delete icon must show its destructive hover feedback"
    );
    click(395., 188.);
    assert!(
        actions
            .borrow()
            .contains(&("delete".into(), 0, "media.play-pause".into())),
        "The shortcut close icon must receive pointer presses and delete its slot"
    );
    assert!(
        !actions
            .borrow()
            .iter()
            .any(|(name, _, _)| name == "capture"),
        "Deleting a shortcut must not start recording"
    );
    actions.borrow_mut().clear();
    click(122., 188.);
    click(122., 332.);
    assert_eq!(
        *actions.borrow(),
        vec![
            ("toggle-function".into(), 0, "media.play-pause".into()),
            ("toggle-function".into(), 1, "media.next".into()),
        ],
        "Switches must disable active functions and enable disabled unbound functions"
    );
    assert_eq!(
        ui.get_function_bindings().row_count(),
        FUNCTION_CATALOG.len()
    );
    assert_eq!(
        ui.get_function_bindings()
            .row_data(0)
            .unwrap()
            .shortcuts
            .row_count(),
        2
    );
    ui.set_capture_function("media.play-pause".into());
    ui.set_capture_slot(0);
    let _ = ui.window().take_snapshot().unwrap();
    actions.borrow_mut().clear();
    click(122., 332.);
    assert!(
        actions.borrow().is_empty(),
        "Recording must lock function switches"
    );
    ui.set_capture_function("".into());
}

// Content may be clipped, but cannot resize the two page-owned grid rows.
fn preview_fixed_overview(ui: &AppWindow, out: &std::path::Path) {
    ui.set_mode(2);
    ui.set_page(0);
    ui.set_elevated(false);
    ui.set_bindings(ModelRc::new(VecModel::from(
        (0..12)
            .map(|_| BindingRow {
                text: "Ctrl + Shift + Alt + Win + PageDown".into(),
                enabled: true,
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    )));
    for (name, zh, height) in [
        ("zh-min", true, 500.),
        ("en-min", false, 500.),
        ("zh-tall", true, 700.),
    ] {
        i18n::apply(ui, zh);
        ui.set_problem("Bluetooth service unavailable".into());
        ui.window().set_size(slint::LogicalSize::new(900., height));
        let _ = ui.window().take_snapshot().unwrap();
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(1000));
        slint::platform::update_timers_and_animations();
        let shot = ui.window().take_snapshot().unwrap();
        assert_eq!((shot.width(), shot.height()), (900, height as u32));
        let stride = shot.width() as usize;
        let background = shot.as_slice()[(height as usize - 1) * stride + 80];
        assert!(
            (height as usize - 24..height as usize)
                .all(|y| (80..900).all(|x| shot.as_slice()[y * stride + x] == background)),
            "Overview content must stay above the bottom window padding"
        );
        assert!(
            (200..height as usize - 28)
                .filter(|&y| shot.as_slice()[y * stride + 90] != background)
                .count()
                > 150,
            "Both grid cards must occupy the available page space"
        );
        let saved_bindings = ui.get_bindings();
        ui.set_bindings(ModelRc::new(VecModel::default()));
        let empty = ui.window().take_snapshot().unwrap();
        assert!(
            (140..height as usize - 28)
                .all(|y| shot.as_slice()[y * stride + 90] == empty.as_slice()[y * stride + 90]),
            "Changing binding content must not move either card boundary"
        );
        ui.set_bindings(saved_bindings);
        let mut bytes = format!("P6\n{} {}\n255\n", shot.width(), shot.height()).into_bytes();
        for pixel in shot.as_slice() {
            bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
        }
        std::fs::write(out.join(format!("overview-fixed-{name}.ppm")), bytes).unwrap();
    }
}
