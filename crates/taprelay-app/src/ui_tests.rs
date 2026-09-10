//! Headless view validation only: no native window, permissions, input hooks or Bluetooth.
use crate::*;
use slint::{ComponentHandle, Model, ModelRc, VecModel, platform::WindowAdapter};

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
                3
            } else {
                5
            } {
                ui.set_page(page);
                ui.set_wizard_page(page.min(2));
                ui.set_problem(
                    if !dark && ((mode == 1 && page == 1) || (mode == 2 && page == 2)) {
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
    ui.set_wizard_page(2);
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
    ui.set_wizard_page(1);
    ui.set_capture_index(-1);
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
    // Preview empty, pending-new and existing-row capture layouts at minimum width.
    ui.set_mode(1);
    ui.set_wizard_page(0);
    ui.set_capture_text("请按下快捷键，松开完成".into());
    for (name, capture) in [("edit", 0), ("new", -2), ("empty", -1)] {
        if name == "empty" {
            ui.set_bindings(ModelRc::new(VecModel::<BindingRow>::default()));
        }
        ui.set_capture_index(capture);
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(250));
        let shot = ui.window().take_snapshot().unwrap();
        let mut bytes = format!("P6\n{} {}\n255\n", shot.width(), shot.height()).into_bytes();
        for pixel in shot.as_slice() {
            bytes.extend_from_slice(&[pixel.r, pixel.g, pixel.b]);
        }
        std::fs::write(out.join(format!("bindings-{name}.ppm")), bytes).unwrap();
    }
    // New rows must take keyboard focus immediately, including the Escape shortcut.
    let capture_actions = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = capture_actions.clone();
    ui.on_action(move |name, index, text| {
        observed
            .borrow_mut()
            .push((name.to_string(), index, text.to_string()))
    });
    ui.set_capture_index(-2);
    let _ = ui.window().take_snapshot().unwrap();
    // A capture-control press must cancel before raw mouse input can be previewed.
    let cancel_position = slint::LogicalPosition::new(195., 158.);
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::PointerMoved {
            position: cancel_position,
        });
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::PointerPressed {
            position: cancel_position,
            button: slint::platform::PointerEventButton::Left,
        });
    assert!(
        capture_actions
            .borrow()
            .iter()
            .any(|(name, _, _)| name == "cancel-capture"),
        "Cancel must be dispatched on mouse down, before a held left button becomes a capture preview: {:?}",
        capture_actions.borrow()
    );
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::PointerReleased {
            position: cancel_position,
            button: slint::platform::PointerEventButton::Left,
        });
    assert_eq!(
        capture_actions
            .borrow()
            .iter()
            .filter(|(name, _, _)| name == "cancel-capture")
            .count(),
        1
    );
    capture_actions.borrow_mut().clear();
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
        "A newly inserted recording row must receive Escape without an extra click"
    );
    ui.set_capture_index(-1);
    // Cover the chip padding, label-to-X gap and X with one hover surface.
    ui.set_bindings(ModelRc::new(VecModel::from(vec![BindingRow {
        text: "F8".into(),
        keys: ModelRc::new(VecModel::from(vec!["F8".into()])),
        enabled: true,
    }])));
    let mut hover_background = None;
    for x in [29., 40., 55., 70., 81.] {
        ui.window()
            .dispatch_event(slint::platform::WindowEvent::PointerMoved {
                position: slint::LogicalPosition::new(x, 158.),
            });
        i_slint_backend_testing::mock_elapsed_time(std::time::Duration::from_millis(250));
        let shot = ui.window().take_snapshot().unwrap();
        let pixel = shot.as_slice()[148 * shot.width() as usize + 40];
        let rgb = (pixel.r, pixel.g, pixel.b);
        assert_eq!(
            *hover_background.get_or_insert(rgb),
            rgb,
            "Hover must not disappear in chip padding or gaps at x={x}"
        );
    }
    // Mutate the model during the press, as the real controller does. Release
    // must not activate the add button that moves into the deleted chip's slot.
    let delete_actions = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let observed = delete_actions.clone();
    let weak = ui.as_weak();
    ui.on_action(move |name, _, _| {
        observed.borrow_mut().push(name.to_string());
        if name == "delete" {
            weak.upgrade()
                .unwrap()
                .set_bindings(ModelRc::new(VecModel::<BindingRow>::default()));
        }
    });
    let position = slint::LogicalPosition::new(70., 158.);
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::PointerMoved { position });
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::PointerPressed {
            position,
            button: slint::platform::PointerEventButton::Left,
        });
    assert_eq!(delete_actions.borrow().as_slice(), &["delete"]);
    assert_eq!(ui.get_bindings().row_count(), 0);
    let _ = ui.window().take_snapshot().unwrap();
    ui.window()
        .dispatch_event(slint::platform::WindowEvent::PointerReleased {
            position,
            button: slint::platform::PointerEventButton::Left,
        });
    assert_eq!(delete_actions.borrow().as_slice(), &["delete"]);
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
    // Page-local edits and callbacks must cross the shell interface after extraction.
    ui.set_page(3);
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
    ui.set_page(3);
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
