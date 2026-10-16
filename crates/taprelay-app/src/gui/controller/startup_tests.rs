use super::*;
use crate::config::Config;
use crate::runtime::Runtime;
use taprelay_core::state::Snapshot;

fn controller(state: Snapshot) -> (Controller, tempfile::TempDir) {
    let folder = tempfile::tempdir().unwrap();
    let mut config = Config::default();
    config.options.language = Language::from_name("en").unwrap();
    config.options.notifications = false;
    config.options.auto_listen = false;
    config.wizard.dismissed = true;
    let runtime = RuntimeHandle::spawn(
        config,
        move |config| {
            let mut runtime = Runtime::new(config);
            runtime.state = state;
            runtime
        },
        Runtime::headless_tick,
        64,
    )
    .unwrap();
    let controller = Controller::new(
        runtime,
        folder.path().join("config.json"),
        None,
        Shell::Headless {
            autostart: false,
            running_apps: Vec::new(),
            notifications: Default::default(),
            bluetooth_settings_opened: false,
        },
        Default::default(),
        None,
    )
    .unwrap();
    (controller, folder)
}

fn ready() -> Snapshot {
    Snapshot {
        adapter: true,
        peripheral: true,
        service: true,
        broadcasting: true,
        ..Default::default()
    }
}

#[test]
fn bluetooth_settings_open_without_transport_after_startup_failure() {
    let (mut controller, _folder) = controller(Snapshot {
        last_error: Some("Bluetooth unavailable".into()),
        ..Default::default()
    });
    let mut ui = Presentation::default();
    controller.tick(&mut ui);
    assert_eq!(controller.startup_state(), StartupState::Failed);
    controller
        .action(&mut ui, Action::BluetoothSettings, "")
        .unwrap();
    assert!(matches!(
        controller.desktop,
        Shell::Headless {
            bluetooth_settings_opened: true,
            ..
        }
    ));
}

#[test]
fn incomplete_checks_keep_startup_pending_without_a_presentation_delay() {
    for completed in 0..4 {
        let (mut controller, _folder) = controller(Snapshot {
            adapter: completed >= 1,
            peripheral: completed >= 2,
            service: completed >= 3,
            ..Default::default()
        });
        let mut ui = Presentation::default();
        for elapsed in [
            Duration::ZERO,
            Duration::from_secs(1),
            Duration::from_secs(29),
        ] {
            controller.started = Instant::now() - elapsed;
            controller.tick(&mut ui);
            assert_eq!(controller.startup_state(), StartupState::Checking);
            assert!(controller.startup_pending);
            assert!(controller.hidden);
            assert_eq!(ui.window_command, None);
            assert!(ui.busy);
        }
    }
}

#[test]
fn completed_checks_apply_start_hidden_only_after_the_wizard_was_dismissed() {
    for start_hidden in [false, true] {
        for dismissed in [false, true] {
            let (mut controller, _folder) = controller(ready());
            controller.runtime.config.options.start_hidden = start_hidden;
            controller.runtime.config.wizard.dismissed = dismissed;
            let mut ui = Presentation::default();
            controller.tick(&mut ui);
            let hidden = start_hidden && dismissed;
            assert_eq!(controller.startup_state(), StartupState::Ready);
            assert!(!controller.startup_pending);
            assert_eq!(controller.hidden, hidden);
            assert_eq!(ui.mode, if dismissed { 2 } else { 1 });
            assert_eq!(ui.window_command, (!hidden).then_some(WindowCommand::Show));

            ui.window_command = None;
            controller.tick(&mut ui);
            assert_eq!(ui.window_command, None);
        }
    }
}

#[test]
fn startup_errors_show_the_window_even_when_start_hidden_is_enabled() {
    for fatal in [false, true] {
        let (mut controller, _folder) = controller(Snapshot {
            last_error: (!fatal).then(String::new),
            ..ready()
        });
        controller.fatal = fatal.then(String::new);
        controller.runtime.config.options.start_hidden = true;
        let mut ui = Presentation::default();
        controller.tick(&mut ui);
        assert_eq!(controller.startup_state(), StartupState::Failed);
        assert!(!controller.passed);
        assert!(!controller.hidden);
        assert!(!controller.startup_pending);
        assert_eq!(ui.window_command, Some(WindowCommand::Show));
        assert!(!ui.problem.is_empty());
    }
}

#[test]
fn failure_takes_priority_over_previously_passed_checks() {
    let (mut controller, _folder) = controller(ready());
    controller.passed = true;
    controller.fatal = Some("Configuration failed".into());
    controller.runtime.config.options.start_hidden = true;
    let mut ui = Presentation::default();
    controller.tick(&mut ui);
    assert_eq!(controller.startup_state(), StartupState::Failed);
    assert_eq!(ui.window_command, Some(WindowCommand::Show));
    assert!(!controller.hidden);
}

#[test]
fn startup_timeout_is_a_failure_instead_of_a_presentation_delay() {
    let (mut controller, _folder) = controller(Snapshot::default());
    controller.started = Instant::now() - Duration::from_secs(31);
    controller.runtime.config.options.start_hidden = true;
    let mut ui = Presentation::default();
    controller.tick(&mut ui);
    assert_eq!(controller.startup_state(), StartupState::Failed);
    assert_eq!(ui.window_command, Some(WindowCommand::Show));
    assert!(!ui.busy);
    assert!(ui.checks.iter().any(|row| row.state == 3));
}

#[test]
fn stopped_worker_cannot_pass_startup_with_stale_success_flags() {
    let (mut controller, _folder) = controller(ready());
    controller.runtime.config.options.start_hidden = true;
    controller.runtime.shutdown().unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    while !controller.runtime.stopped() {
        controller.runtime.poll();
        assert!(Instant::now() < deadline, "runtime did not stop");
        std::thread::yield_now();
    }
    let mut ui = Presentation::default();
    controller.tick(&mut ui);
    assert_eq!(controller.startup_state(), StartupState::Failed);
    assert!(!controller.passed);
    assert_eq!(ui.window_command, Some(WindowCommand::Show));
}

#[test]
fn startup_completion_keeps_a_manually_opened_window_visible() {
    let (mut controller, _folder) = controller(ready());
    controller.runtime.config.options.start_hidden = true;
    let mut ui = Presentation::default();
    controller.action(&mut ui, Action::Show, "").unwrap();
    assert_eq!(ui.window_command.take(), Some(WindowCommand::Show));
    controller.tick(&mut ui);
    assert_eq!(controller.startup_state(), StartupState::Ready);
    assert!(!controller.hidden);
    assert_eq!(ui.window_command, None);
}

#[test]
fn recovery_keeps_a_window_opened_for_startup_failure_visible() {
    let (mut controller, _folder) = controller(ready());
    controller.runtime.config.options.start_hidden = true;
    controller.fatal = Some("Startup failed".into());
    let mut ui = Presentation::default();
    controller.tick(&mut ui);
    assert_eq!(ui.window_command.take(), Some(WindowCommand::Show));
    controller.fatal = None;
    controller.tick(&mut ui);
    assert_eq!(controller.startup_state(), StartupState::Ready);
    assert!(!controller.hidden);
    assert_eq!(ui.window_command, None);
}
