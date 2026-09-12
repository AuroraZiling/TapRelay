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
    ui.set_function_cards(ModelRc::new(VecModel::from(vec![
        FunctionCard {
            title: "Media".into(),
            items: ModelRc::new(VecModel::from(vec![
                FunctionItem {
                    id: "media.play-pause".into(),
                    label: "Play / Pause".into(),
                    enabled: false,
                    activation: "Press".into(),
                },
                FunctionItem {
                    id: "media.previous".into(),
                    label: "Previous".into(),
                    enabled: false,
                    activation: "Press".into(),
                },
                FunctionItem {
                    id: "media.next".into(),
                    label: "Next".into(),
                    enabled: false,
                    activation: "Press".into(),
                },
                FunctionItem {
                    id: "media.mute".into(),
                    label: "Mute / Unmute".into(),
                    enabled: false,
                    activation: "Press".into(),
                },
                FunctionItem {
                    id: "media.rewind".into(),
                    label: "Rewind".into(),
                    enabled: false,
                    activation: "Hold".into(),
                },
                FunctionItem {
                    id: "media.fast-forward".into(),
                    label: "Fast Forward".into(),
                    enabled: false,
                    activation: "Hold".into(),
                },
            ])),
        },
        FunctionCard {
            title: "Virtual".into(),
            items: ModelRc::new(VecModel::from(vec![FunctionItem {
                id: "virtual.passthrough".into(),
                label: "Toggle Passthrough".into(),
                enabled: false,
                activation: "Press".into(),
            }])),
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
    ui.set_log_lines(ModelRc::new(VecModel::from(vec![
        LogLine {
            text: "22:00:00 INFO  HID service initialized".into(),
            level: LogLevel::Info,
        },
        LogLine {
            text: "22:00:01 DEBUG Receiver discovery started".into(),
            level: LogLevel::Debug,
        },
        LogLine {
            text: "22:00:24 WARN  Receiver has not subscribed yet".into(),
            level: LogLevel::Warning,
        },
        LogLine {
            text: "22:00:31 ERROR Advertising stopped".into(),
            level: LogLevel::Error,
        },
    ])));
    ui.set_data_directory("D:\\Apps\\TapRelay".into());
    ui.set_app_version("0.1.0".into());
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
                6
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
                if mode == 2 && page == 1 && dark {
                    let pixel = |x: usize, y: usize| shot.as_slice()[y * shot.width() as usize + x];
                    assert_ne!(
                        pixel(100, 150),
                        pixel(80, 150),
                        "Function category panels must be visible below the introduction"
                    );
                    assert_ne!(
                        pixel(550, 150),
                        pixel(80, 150),
                        "The second category must occupy the right column"
                    );
                }
                if mode == 2 && page == 5 && dark {
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
        activation: "Press".into(),
        enabled: true,
        shortcuts: ModelRc::new(VecModel::from(vec![shortcut])),
    };
    ui.set_active_bindings(ModelRc::new(VecModel::from(vec![row.clone()])));
    ui.set_disabled_bindings(ModelRc::new(VecModel::default()));
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
        ui.set_active_bindings(ModelRc::new(VecModel::from(vec![preview_row])));
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
    ui.set_active_bindings(ModelRc::new(VecModel::from(vec![empty_row])));
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
        actions.borrow().contains(&("navigate".into(), 5)),
        "Sidebar tooltip must not consume navigation clicks"
    );
    // Page-local edits and callbacks must cross the shell interface after extraction.
    ui.set_page(4);
    ui.global::<Theme>().set_mode(ThemeMode::Dark);
    let log_header = ui.window().take_snapshot().unwrap();
    let info_dot_visible = (882..=886).any(|x| {
        (39..=42).any(|y| {
            let pixel = log_header.as_slice()[y * log_header.width() as usize + x];
            pixel.r > 180 && pixel.g > 180 && pixel.b > 180
        })
    });
    assert!(
        info_dot_visible,
        "The Info icon must render a visible dot above its vertical stem"
    );
    let click = |x, y| {
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
    click(170., 45.);
    assert!(
        actions.borrow().contains(&("clear-logs".into(), 0)),
        "The log header must expose a clear action beside its title"
    );
    click(848., 45.);
    assert!(
        ui.get_log_debug(),
        "The debug severity toggle must propagate to the window"
    );
    assert!(actions.borrow().contains(&("logs".into(), 0)));
    ui.set_page(0);
    let _ = ui.window().take_snapshot().unwrap();
    ui.set_page(4);
    let _ = ui.window().take_snapshot().unwrap();
    assert!(
        ui.get_log_debug(),
        "Navigation must preserve the severity toggles"
    );
    // Exercise the bounded log model with wrapping and Unicode at full capacity.
    let logs = std::rc::Rc::new(VecModel::from(log_rows(1000, 0)));
    ui.set_log_lines(logs.clone().into());
    // Log updates reach the layout through instantiation and change handlers,
    // so each step is drained (mock time plus a snapshot) before it is asserted.
    let settle = || {
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(16));
        let _ = ui.window().take_snapshot().unwrap();
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(16));
    };
    settle();
    let shot = ui.window().take_snapshot().unwrap();
    assert_eq!((shot.width(), shot.height()), (1000, 700));
    // The page opens at the newest lines and follows the tail from there.
    let tail = ui.get_log_viewport_y();
    assert!(
        tail < -1000.,
        "Opening the log page must start at the newest lines: {tail}"
    );
    for row in log_rows(200, 1000) {
        logs.push(row);
    }
    settle();
    let followed = ui.get_log_viewport_y();
    assert!(
        followed < tail,
        "A reader parked at the tail must follow new lines: {followed} !< {tail}"
    );
    // A reader who scrolled up keeps their position through later updates.
    ui.set_log_viewport_y(-40.);
    settle();
    assert_eq!(ui.get_log_viewport_y(), -40.);
    for row in log_rows(200, 1200) {
        logs.push(row);
    }
    settle();
    assert_eq!(
        ui.get_log_viewport_y(),
        -40.,
        "Log updates must not move a reader who scrolled up"
    );
    // Shortening the log must not leave the viewport out of range.
    ui.set_log_lines(ModelRc::new(VecModel::<LogLine>::default()));
    settle();
    assert_eq!(ui.get_log_viewport_y(), 0.);

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
}

/// Wrapped, Unicode and severity-cycled log rows as the controller publishes them.
fn log_rows(count: usize, offset: usize) -> Vec<LogLine> {
    (0..count)
        .map(|i| {
            let index = offset + i;
            LogLine {
                text: format!(
                    "12:00:00 INFO 日志 {index}: {}",
                    "wrapped message ".repeat(index % 7 + 1)
                )
                .into(),
                level: [
                    LogLevel::Debug,
                    LogLevel::Info,
                    LogLevel::Warning,
                    LogLevel::Error,
                ][index % 4],
            }
        })
        .collect()
}

// Use the real catalog and deliberately long shortcuts, rather than English
// placeholders in every locale. This exercises the actual nested page layouts.
fn preview_function_layouts(ui: &AppWindow, out: &std::path::Path) {
    use taprelay_core::function::{Activation, CategoryId, FUNCTION_CATALOG};
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
        let cards = [CategoryId::Media, CategoryId::Virtual].map(|category| {
            let definitions: Vec<_> = FUNCTION_CATALOG
                .iter()
                .filter(|d| d.category == category)
                .collect();
            FunctionCard {
                title: i18n::text(zh, definitions[0].category_key).into(),
                items: ModelRc::new(VecModel::from(
                    definitions
                        .iter()
                        .enumerate()
                        .map(|(i, d)| FunctionItem {
                            id: d.id.stable_id().into(),
                            label: i18n::text(zh, d.name_key).into(),
                            enabled: i % 2 == 0,
                            activation: i18n::text(
                                zh,
                                if d.activation == Activation::Hold {
                                    "bindings.activation.hold"
                                } else {
                                    "bindings.activation.press"
                                },
                            )
                            .into(),
                        })
                        .collect::<Vec<_>>(),
                )),
            }
        });
        ui.set_function_cards(ModelRc::new(VecModel::from(cards.to_vec())));
        let rows: Vec<_> = FUNCTION_CATALOG
            .iter()
            .enumerate()
            .map(|(i, d)| FunctionBindingRow {
                id: d.id.stable_id().into(),
                label: i18n::text(zh, d.name_key).into(),
                activation: i18n::text(
                    zh,
                    if d.activation == Activation::Hold {
                        "bindings.activation.hold"
                    } else {
                        "bindings.activation.press"
                    },
                )
                .into(),
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
        ui.set_active_bindings(ModelRc::new(VecModel::from(vec![
            rows[0].clone(),
            rows[1].clone(),
            rows[4].clone(),
            rows[5].clone(),
        ])));
        ui.set_disabled_bindings(ModelRc::new(VecModel::from(vec![
            rows[3].clone(),
            rows[6].clone(),
        ])));
        for mode in [2, 1] {
            ui.set_mode(mode);
            for page in [1, 2] {
                ui.set_page(page);
                ui.set_wizard_page(page - 1);
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
    click(400., 188.);
    click(383., 260.);
    assert!(
        actions.borrow().is_empty(),
        "Neither a saved second shortcut nor an add-second button should be interactive"
    );
    use slint::Model;
    assert_eq!(
        ui.get_active_bindings()
            .row_data(0)
            .unwrap()
            .shortcuts
            .row_count(),
        2,
        "Presenting one shortcut must not truncate the multi-shortcut model"
    );
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::PointerMoved {
            position: slint::LogicalPosition::new(343., 188.),
        });
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(250));
    slint::platform::update_timers_and_animations();
    let hovered = ui.window().take_snapshot().unwrap();
    assert!(
        (178..198).any(|y| (333..353).any(|x| {
            let p = hovered.as_slice()[y * hovered.width() as usize + x];
            i16::from(p.r) > i16::from(p.g) + 20
        })),
        "The delete icon must show its destructive hover feedback"
    );
    click(343., 188.);
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
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::PointerMoved {
            position: slint::LogicalPosition::new(1045., 522.),
        });
    i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(250));
    slint::platform::update_timers_and_animations();
    let unbind_hover = ui.window().take_snapshot().unwrap();
    let tint = unbind_hover.as_slice()[510 * unbind_hover.width() as usize + 1070];
    assert!(
        tint.r > tint.g,
        "The right-aligned unbind button must have a red-tinted hover background"
    );
    let mut bytes = format!(
        "P6\n{} {}\n255\n",
        unbind_hover.width(),
        unbind_hover.height()
    )
    .into_bytes();
    for pixel in unbind_hover.as_slice() {
        bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
    }
    std::fs::write(out.join("bindings-unbind-hover.ppm"), bytes).unwrap();
    click(1045., 522.);
    assert!(
        actions
            .borrow()
            .contains(&("unbind-function".into(), 0, "media.mute".into())),
        "Disabled rows must keep their whole-row unbind button usable"
    );
    ui.set_page(1);
    let _ = ui.window().take_snapshot().unwrap();
    click(140., 190.);
    assert!(
        actions
            .borrow()
            .contains(&("toggle-function".into(), 0, "media.play-pause".into())),
        "A category capsule must toggle its stable function id"
    );
    ui.set_active_bindings(ModelRc::new(VecModel::default()));
    ui.set_disabled_bindings(ModelRc::new(VecModel::default()));
    ui.set_page(2);
    let _ = ui.window().take_snapshot().unwrap();
    click(1020., 158.);
    assert!(
        actions
            .borrow()
            .contains(&("navigate".into(), 1, String::new())),
        "The empty-state action must be clickable inside its panel"
    );
}
