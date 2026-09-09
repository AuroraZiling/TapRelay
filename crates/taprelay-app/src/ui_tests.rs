//! Headless view validation only: no native window, permissions, input hooks or Bluetooth.
use crate::*;
use slint::{ComponentHandle, ModelRc, VecModel};
#[test]
fn render_all_views_without_hardware() {
    slint::platform::set_platform(Box::new(i_slint_backend_testing::TestingBackend::new(
        i_slint_backend_testing::TestingBackendOptions {
            renderer_name: Some("software".into()),
            mock_time: true,
            threading: false,
        },
    )))
    .unwrap();
    let ui = AppWindow::new().unwrap();
    ui.set_device_name("Living room tablet".into());
    ui.set_device_selected(true);
    ui.set_stage("Connected; waiting for HID subscription".into());
    ui.set_stage_detail(
        "Time in this stage: 23 s · Retry or check system Bluetooth settings.".into(),
    );
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
    ui.set_binding_summary("F8 · LCtrl + mouse.side1".into());
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
    ui.set_log_text("22:00:00 INFO  HID service initialized\n22:00:01 INFO  Waiting for receiver subscription\n22:00:24 WARN  Receiver has not subscribed yet".into());
    ui.set_log_lines(ModelRc::new(VecModel::from(
        ui.get_log_text()
            .lines()
            .map(slint::SharedString::from)
            .collect::<Vec<_>>(),
    )));
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
        ui.set_nearby_devices(ModelRc::new(VecModel::from(
            rows.iter()
                .filter(|r| !r.paired)
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
    let position = slint::LogicalPosition::new(30., 214.);
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
    let _ = ui.window().take_snapshot().unwrap();
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
    click(114., 650.);
    assert!(
        ui.get_logs_paused(),
        "Log pause must propagate to the window"
    );
    assert!(actions.borrow().contains(&("logs".into(), 0)));
    click(400., 96.);
    ui.window().dispatch_event(WindowEvent::KeyPressed {
        text: "needle".into(),
    });
    assert_eq!(
        ui.get_log_search(),
        "needle",
        "Search input must propagate to the window"
    );
    ui.set_page(0);
    let _ = ui.window().take_snapshot().unwrap();
    ui.set_page(3);
    let _ = ui.window().take_snapshot().unwrap();
    assert!(ui.get_logs_paused());
    assert_eq!(
        ui.get_log_search(),
        "needle",
        "Navigation must preserve the log search"
    );
    // Exercise the bounded log model with wrapping and Unicode at full capacity.
    let rows: Vec<slint::SharedString> = (0..1000)
        .map(|i| {
            format!(
                "12:00:00 INFO 日志 {i}: {}",
                "wrapped message ".repeat(i % 7 + 1)
            )
            .into()
        })
        .collect();
    ui.set_logs_paused(false);
    ui.set_log_lines(ModelRc::new(VecModel::from(rows)));
    ui.set_log_text("log model revision changed".into());
    let shot = ui.window().take_snapshot().unwrap();
    assert_eq!((shot.width(), shot.height()), (1000, 700));
}
