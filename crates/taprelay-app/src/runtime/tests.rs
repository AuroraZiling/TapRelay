use super::test_input::FakeInput;
use super::*;

use crate::config::Config;
use taprelay_core::{
    function::{FunctionConfig, FunctionId, ModifierSet, Shortcut},
    input::MouseButton,
};

struct FakeTransport;
impl Transport for FakeTransport {
    fn snapshot(&mut self) -> Option<Snapshot> {
        None
    }
    fn refresh(&self) -> Result<()> {
        Ok(())
    }
    fn restart(&self) -> Result<u64> {
        Ok(1)
    }
    fn select(&self, _: String) -> Result<u64> {
        Ok(1)
    }
    fn invalidate(&self) {}
    fn send(&self, _: QueuedCommand) -> Result<oneshot::Receiver<Result<(), BackendError>>> {
        let (tx, rx) = oneshot::channel();
        tx.send(Ok(())).unwrap();
        Ok(rx)
    }
}

#[test]
fn mapping_profile_is_revoked_even_after_the_input_source_is_gone() {
    struct LinkedTransport(taprelay_core::passthrough::InputLink);
    impl Transport for LinkedTransport {
        fn input_link(&self) -> Option<taprelay_core::passthrough::InputLink> {
            Some(self.0.clone())
        }
        fn snapshot(&mut self) -> Option<Snapshot> {
            None
        }
        fn refresh(&self) -> Result<()> {
            Ok(())
        }
        fn restart(&self) -> Result<u64> {
            Ok(1)
        }
        fn select(&self, _: String) -> Result<u64> {
            Ok(1)
        }
        fn invalidate(&self) {
            self.0.end();
        }
        fn send(&self, _: QueuedCommand) -> Result<oneshot::Receiver<Result<(), BackendError>>> {
            unreachable!()
        }
    }
    let (link, _receiver) = taprelay_core::passthrough::InputLink::channel(std::sync::Arc::new(
        std::sync::atomic::AtomicU64::new(1),
    ));
    let mut runtime = Runtime::new(Config::default());
    runtime.transport = Some(Box::new(LinkedTransport(link.clone())));
    runtime
        .custom_mappings
        .push(taprelay_core::mapping::CustomMapping {
            id: 1,
            name: "Speed".into(),
            enabled: true,
            shortcuts: vec![Shortcut::mouse(ModifierSet::empty(), MouseButton::Side1)],
            output: taprelay_core::mapping::MappingOutput::Keyboard {
                usage: 0x4f,
                modifiers: 0,
            },
            groups: Default::default(),
        });
    runtime.listening = true;
    runtime.input = Some(Box::new(FakeInput(None)));
    runtime.configure_input();
    assert!(link.profile_requested() && !link.capture_requested());
    runtime.listening = false;
    runtime.input.take();
    runtime.configure_input();
    assert_eq!(link.mapping_epoch(), 0);
    assert!(!link.profile_requested());
}

struct SnapshotTransport(Option<Snapshot>);
impl Transport for SnapshotTransport {
    fn snapshot(&mut self) -> Option<Snapshot> {
        self.0.take()
    }
    fn refresh(&self) -> Result<()> {
        Ok(())
    }
    fn restart(&self) -> Result<u64> {
        Ok(1)
    }
    fn select(&self, _: String) -> Result<u64> {
        Ok(1)
    }
    fn invalidate(&self) {}
    fn send(&self, _: QueuedCommand) -> Result<oneshot::Receiver<Result<(), BackendError>>> {
        unreachable!()
    }
}

struct StoppedTransport(Option<Snapshot>);
impl Transport for StoppedTransport {
    fn snapshot(&mut self) -> Option<Snapshot> {
        self.0.take()
    }
    fn is_finished(&self) -> bool {
        true
    }
    fn refresh(&self) -> Result<()> {
        Ok(())
    }
    fn restart(&self) -> Result<u64> {
        unreachable!()
    }
    fn select(&self, _: String) -> Result<u64> {
        unreachable!()
    }
    fn invalidate(&self) {}
    fn send(&self, _: QueuedCommand) -> Result<oneshot::Receiver<Result<(), BackendError>>> {
        unreachable!()
    }
}

fn restoring_runtime() -> Runtime {
    Runtime::new(Config {
        remembered_device: Some(crate::config::Device {
            id: "remembered".into(),
            name: "Tablet".into(),
            identity: vec!["container:tablet".into()],
            aliases: vec![],
        }),
        ..Config::default()
    })
}

#[test]
fn startup_creation_failure_is_visible_without_a_remembered_device_and_retry_clears_it() {
    let mut runtime = Runtime::new(Config::default());
    assert!(
        runtime
            .start_bluetooth_with(|remembered| {
                assert!(remembered.is_none());
                anyhow::bail!("Bluetooth unavailable")
            })
            .is_err()
    );
    assert_eq!(
        runtime.state.last_error.as_deref(),
        Some("Bluetooth unavailable")
    );
    runtime
        .start_bluetooth_with(|_| Ok(Box::new(FakeTransport)))
        .unwrap();
    assert_eq!(runtime.state.last_error, None);
}

#[test]
fn startup_retries_creation_and_stopped_worker_at_five_second_intervals() {
    let mut runtime = restoring_runtime();
    assert!(
        runtime
            .start_bluetooth_with(|_| anyhow::bail!("not ready"))
            .is_err()
    );
    let due = runtime.startup_retry_at.unwrap();
    runtime.retry_startup_with(due - std::time::Duration::from_millis(1), |_| {
        panic!("too early")
    });
    runtime.retry_startup_with(due, |remembered| {
        assert_eq!(remembered.unwrap().id, "remembered");
        anyhow::bail!("still not ready")
    });
    runtime.retry_startup_with(due + std::time::Duration::from_secs(4), |_| {
        panic!("too early")
    });
    runtime.retry_startup_with(due + std::time::Duration::from_secs(5), |remembered| {
        assert!(remembered.is_some());
        Ok(Box::new(StoppedTransport(None)))
    });
    runtime.retry_startup_with(due + std::time::Duration::from_secs(10), |remembered| {
        assert!(remembered.is_some());
        Ok(Box::new(FakeTransport))
    });
    runtime.retry_startup_with(due + std::time::Duration::from_secs(100), |_| {
        panic!("worker is running")
    });
}

#[test]
fn stopped_workers_stale_ready_snapshot_does_not_complete_restore() {
    let mut runtime = restoring_runtime();
    let target = runtime.startup_restore.clone().unwrap();
    runtime.transport = Some(Box::new(StoppedTransport(Some(Snapshot {
        selected: Some(target.id.clone()),
        targets: vec![target],
        ready: true,
        ..Default::default()
    }))));
    runtime.tick();
    assert!(!runtime.state.ready);
    assert!(runtime.startup_restore.is_some());
}

#[test]
fn manual_actions_cancel_restore_and_reject_late_automatic_selection() {
    for action in ["select", "disconnect", "pair"] {
        let mut runtime = restoring_runtime();
        let target = runtime.startup_restore.clone().unwrap();
        runtime.transport = Some(Box::new(SnapshotTransport(Some(Snapshot {
            selected: Some(target.id.clone()),
            targets: vec![target],
            ready: true,
            generation: 2,
            ..Default::default()
        }))));
        let _ = match action {
            "select" => runtime.choose("other".into()),
            "disconnect" => runtime.disconnect(),
            _ => runtime.pair("other".into()),
        };
        runtime.tick();
        assert!(runtime.startup_restore.is_none());
        assert!(!runtime.state.ready);
        assert_ne!(runtime.state.selected.as_deref(), Some("remembered"));
        runtime.retry_startup_with(Instant::now() + std::time::Duration::from_secs(600), |_| {
            panic!("cancelled")
        });
    }
}

#[test]
fn startup_restore_survives_transport_creation_failure() {
    let mut runtime = Runtime::new(Config {
        remembered_device: Some(crate::config::Device {
            id: "remembered".into(),
            name: "Tablet".into(),
            identity: vec!["container:tablet".into()],
            aliases: vec![],
        }),
        ..Config::default()
    });
    assert!(
        runtime
            .start_bluetooth_with(|_| anyhow::bail!("not ready"))
            .is_err()
    );
    runtime
        .start_bluetooth_with(|remembered| {
            assert_eq!(remembered.unwrap().id, "remembered");
            Ok(Box::new(FakeTransport))
        })
        .unwrap();
}

#[test]
fn startup_keeps_remembered_device_until_ready() {
    let mut config = Config::default();
    config.remembered_device = Some(crate::config::Device {
        id: "remembered".into(),
        name: "Tablet".into(),
        identity: vec!["container:tablet".into()],
        aliases: vec![],
    });
    let mut runtime = Runtime::new(config);
    let mut offered = None;
    runtime
        .start_bluetooth_with(|remembered| {
            offered = remembered;
            Ok(Box::new(FakeTransport))
        })
        .unwrap();
    assert_eq!(offered.unwrap().id, "remembered");
    runtime.start_bluetooth().unwrap();
    assert!(runtime.startup_restore.is_some());
}

#[test]
fn ready_session_updates_memory_and_explicit_disconnect_clears_it() {
    let target = taprelay_core::state::Target {
        id: "gatt-endpoint".into(),
        name: "Tablet".into(),
        identity: vec!["container:tablet".into()],
        aliases: vec!["classic-endpoint".into()],
        ..Default::default()
    };
    let mut runtime = Runtime::new(Config::default());
    runtime.state.selected = Some(target.id.clone());
    runtime.transport = Some(Box::new(SnapshotTransport(Some(Snapshot {
        generation: 1,
        selected: Some(target.id.clone()),
        targets: vec![target],
        ready: true,
        ..Default::default()
    }))));
    runtime.tick();
    let remembered = runtime.remembered_device.as_ref().unwrap();
    assert_eq!(remembered.id, "gatt-endpoint");
    assert_eq!(remembered.identity, ["container:tablet"]);
    assert!(runtime.disconnect().is_err());
    assert!(runtime.remembered_device.is_none());
}

#[test]
fn startup_snapshot_is_accepted_only_for_the_remembered_physical_device() {
    let remembered = crate::config::Device {
        id: "classic-endpoint".into(),
        name: "Tablet".into(),
        identity: vec!["container:tablet".into()],
        aliases: vec![],
    };
    let restored = taprelay_core::state::Target {
        id: "gatt-endpoint".into(),
        name: "Renamed tablet".into(),
        identity: remembered.identity.clone(),
        ..Default::default()
    };
    let config = Config {
        remembered_device: Some(remembered),
        ..Default::default()
    };
    let mut runtime = Runtime::new(config);
    runtime.transport = Some(Box::new(SnapshotTransport(Some(Snapshot {
        generation: 1,
        selected: Some(restored.id.clone()),
        targets: vec![restored],
        ready: true,
        ..Default::default()
    }))));
    runtime.tick();
    assert!(runtime.state.ready);
    assert_eq!(
        runtime.remembered_device.as_ref().unwrap().id,
        "gatt-endpoint"
    );

    let unrelated = taprelay_core::state::Target {
        id: "other".into(),
        name: "Tablet".into(),
        identity: vec!["container:other".into()],
        ..Default::default()
    };
    runtime.state = Snapshot::default();
    runtime.transport = Some(Box::new(SnapshotTransport(Some(Snapshot {
        generation: 2,
        selected: Some(unrelated.id.clone()),
        targets: vec![unrelated],
        ready: true,
        ..Default::default()
    }))));
    runtime.tick();
    assert!(!runtime.state.ready);
    assert_eq!(
        runtime.remembered_device.as_ref().unwrap().id,
        "gatt-endpoint"
    );
}

#[test]
fn explicit_test_needs_no_bindings_or_listener() {
    let mut runtime = Runtime::new(Config::default());
    runtime.transport = Some(Box::new(FakeTransport));
    runtime.state.ready = true;
    runtime.state.selected = Some("receiver".into());
    runtime.send().unwrap();
    runtime.tick();
    assert_eq!(runtime.delivered, 1);
    assert!(!runtime.listening);
}

#[test]
fn toggling_function_preserves_saved_shortcuts() {
    let mut runtime = Runtime::new(Config::default());
    let id = FunctionId::MediaNext;
    let shortcuts = vec![Shortcut::mouse(ModifierSet::empty(), MouseButton::Side1)];
    runtime.functions.insert(
        id,
        FunctionConfig {
            enabled: true,
            shortcuts: shortcuts.clone(),
        },
    );
    runtime.set_function_enabled(id, false).unwrap();
    assert!(!runtime.functions[&id].enabled);
    assert_eq!(runtime.functions[&id].shortcuts, shortcuts);
    runtime.set_function_enabled(id, true).unwrap();
    assert!(runtime.functions[&id].enabled);
    assert_eq!(runtime.functions[&id].shortcuts, shortcuts);
}

#[test]
fn disabled_function_shortcut_can_be_saved_but_is_not_recognized() {
    let mut runtime = Runtime::new(Config::default());
    runtime.functions.insert(
        FunctionId::MediaNext,
        FunctionConfig {
            enabled: false,
            shortcuts: vec![Shortcut::mouse(ModifierSet::empty(), MouseButton::Side1)],
        },
    );
    runtime.bindings_changed();
    runtime.input = Some(Box::new(FakeInput(None)));
    runtime.listening = true;
    runtime
        .sender
        .try_send(RoutedInput::Edge {
            event: InputEvent {
                code: InputCode::Mouse(MouseButton::Side1),
                down: true,
                captured: Instant::now(),
            },
            result: RouteResult::default(),
        })
        .unwrap();
    runtime.tick();
    assert_eq!(runtime.matched, 0);
}
fn app_runtime() -> Runtime {
    let mut config = Config::default();
    config.functions.insert(
        FunctionId::AppToggleListening,
        FunctionConfig {
            enabled: true,
            shortcuts: vec![Shortcut::keyboard(ModifierSet::empty(), 0x78)],
        },
    );
    let mut runtime = Runtime::new(config);
    runtime.input = Some(Box::new(FakeInput(None)));
    runtime
}

#[test]
fn expired_and_timed_out_receipts_do_not_stop_listening_or_emit_test_failure() {
    for error in [BackendError::Stale, BackendError::TimedOut] {
        let mut runtime = app_runtime();
        runtime.input_initialized = true;
        runtime.listening = true;
        runtime.test = TestStatus::Pending(1);
        let (reply, result) = oneshot::channel();
        runtime.receipts.push(Receipt {
            created: Instant::now(),
            test: Some(1),
            action: MediaCommand::PlayPause,
            result,
        });
        reply.send(Err(error)).unwrap();
        runtime.tick();
        assert!(runtime.error.is_none());
        assert!(runtime.listening);
        assert_eq!(runtime.test, TestStatus::Idle);
        assert!(runtime.receipts.is_empty());
    }
}

fn function_output(action: FunctionAction, down: bool, created: Instant) -> RouteResult {
    RouteResult {
        outputs: vec![RoutedOutput::Function {
            binding: taprelay_core::binding::BindingKey {
                function: FunctionId::AppToggleListening,
                slot: 0,
            },
            action,
            down,
            created,
            token: 1,
            revision: 0,
        }],
        ..Default::default()
    }
}

#[test]
fn ui_input_cutoff_preserves_media_releases_but_suppresses_queued_presses() {
    for action in [MediaCommand::Rewind, MediaCommand::FastForward] {
        let mut runtime = Runtime::new(Config::default());
        runtime.input_initialized = true;
        runtime.input = Some(Box::new(FakeInput(None)));
        runtime.transport = Some(Box::new(FakeTransport));
        runtime.listening = true;
        runtime.state.ready = true;
        runtime.state.selected = Some("receiver".into());
        let action = FunctionAction::Media(action);
        runtime.apply_router_outputs(function_output(action, true, Instant::now()));
        runtime.tick_with_capture_context(true, false);
        assert_eq!(runtime.delivered, 1);

        for down in [false, true] {
            let event = InputEvent {
                code: InputCode::Key(0x77),
                down,
                captured: Instant::now(),
            };
            runtime
                .sender
                .try_send(RoutedInput::Edge {
                    event,
                    result: function_output(action, down, event.captured),
                })
                .unwrap();
            runtime.consume_ui_input();
            runtime.tick_with_capture_context(true, false);
            assert_eq!(
                runtime.delivered, 2,
                "only the held command's release should pass"
            );
        }
        assert!(runtime.error.is_none());

        let event = InputEvent {
            code: InputCode::Key(0x77),
            down: true,
            captured: Instant::now(),
        };
        runtime
            .sender
            .try_send(RoutedInput::Edge {
                event,
                result: function_output(action, true, event.captured),
            })
            .unwrap();
        runtime.tick_with_capture_context(true, false);
        assert_eq!(runtime.delivered, 3, "new input should still pass");
    }
}

#[test]
fn app_shortcut_keeps_input_alive_at_startup_and_across_listener_changes() {
    let mut runtime = app_runtime();
    runtime.tick();
    assert!(runtime.input_initialized);
    assert!(runtime.state.input);
    assert!(!runtime.listening);
    let action = FunctionAction::App(AppCommand::ToggleListening);
    for expected in [true, false, true] {
        runtime.apply_router_outputs(function_output(action, true, Instant::now()));
        assert_eq!(runtime.listening, expected);
        assert!(runtime.input.is_some());
        runtime.apply_router_outputs(function_output(action, false, Instant::now()));
        assert_eq!(runtime.listening, expected);
    }
    assert!(runtime.receipts.is_empty());
    assert!(runtime.error.is_none());
}

#[test]
fn passthrough_binding_keeps_capture_available_with_listening_paused() {
    let mut runtime = app_runtime();
    let binding = runtime
        .functions
        .get_mut(&FunctionId::AppToggleListening)
        .unwrap();
    let passthrough = binding.clone();
    binding.enabled = false;
    runtime
        .functions
        .insert(FunctionId::AppTogglePassthrough, passthrough);
    runtime.tick();
    assert!(!runtime.listening);
    assert!(runtime.state.input);
    runtime.finish_recording();
    assert!(runtime.input.is_some());
    runtime
        .set_function_enabled(FunctionId::AppTogglePassthrough, false)
        .unwrap();
    assert!(runtime.input.is_none());
}

#[test]
fn removing_or_disabling_last_app_binding_releases_idle_input() {
    let mut runtime = app_runtime();
    runtime
        .set_function_enabled(FunctionId::AppToggleListening, false)
        .unwrap();
    assert!(runtime.input.is_none());
    let mut runtime = app_runtime();
    runtime
        .remove_shortcut(FunctionId::AppToggleListening, 0)
        .unwrap();
    assert!(runtime.input.is_none());
    let mut runtime = app_runtime();
    runtime
        .apply_binding_command(BindingCommand::BeginCapture(CaptureTarget {
            id: FunctionId::AppToggleListening,
            slot: 0,
        }))
        .unwrap();
    runtime.finish_recording();
    assert!(runtime.input.is_some());
}

#[test]
fn paused_or_stale_media_presses_do_not_reach_transport() {
    let mut runtime = app_runtime();
    runtime.transport = Some(Box::new(FakeTransport));
    runtime.state.ready = true;
    runtime.state.selected = Some("receiver".into());
    let action = FunctionAction::Media(MediaCommand::PlayPause);
    runtime.apply_router_outputs(function_output(action, true, Instant::now()));
    assert!(runtime.receipts.is_empty());
    let stale = Instant::now();
    runtime.set_listening(false).unwrap();
    runtime.set_listening(true).unwrap();
    runtime.apply_router_outputs(function_output(action, true, stale));
    assert!(runtime.receipts.is_empty());
    runtime.apply_router_outputs(function_output(action, true, Instant::now()));
    assert_eq!(runtime.receipts.len(), 1);
}

#[test]
fn failed_idle_app_hook_is_reported_without_automatic_restart() {
    let mut runtime = app_runtime();
    runtime.input_initialized = true;
    runtime.input = Some(Box::new(FakeInput(Some("hook failed"))));
    runtime.tick();
    assert_eq!(runtime.error.as_deref(), Some("hook failed"));
    assert!(runtime.input.is_none());
    runtime.tick();
    assert!(runtime.input.is_none());
    assert!(!runtime.listening);
}
fn capture_runtime() -> Runtime {
    let mut runtime = Runtime::new(Config::default());
    runtime.input = Some(Box::new(FakeInput(None)));
    runtime.input_initialized = true;
    runtime.listening = true;
    for (id, key) in [
        (FunctionId::MediaPlayPause, 0x77),
        (FunctionId::MediaNext, 0x78),
    ] {
        runtime.functions.insert(
            id,
            FunctionConfig {
                enabled: true,
                shortcuts: vec![Shortcut::keyboard(ModifierSet::empty(), key)],
            },
        );
    }
    runtime
}

#[test]
fn custom_capture_returns_a_draft_result_without_changing_live_bindings() {
    let mut runtime = capture_runtime();
    let functions = runtime.functions.clone();
    for output in [false, true] {
        let mut draft = capture_mapping_draft();
        draft.output = taprelay_core::mapping::MappingOutput::Keyboard {
            usage: 0,
            modifiers: 0,
        };
        runtime
            .apply_binding_command(BindingCommand::BeginCustomCapture {
                output,
                slot: 0,
                draft,
            })
            .unwrap();
        runtime.tick_with_capture_context(true, false);
        queue_capture_keys(&mut runtime, &[(0x27, true), (0x27, false)], false);
        runtime.tick_with_capture_context(true, false);
        assert!(runtime.capture().is_none());
        let (_, recorded_output, _, shortcut) = runtime.custom_capture_result.as_ref().unwrap();
        assert_eq!(*recorded_output, output);
        assert_eq!(*shortcut, Shortcut::keyboard(ModifierSet::empty(), 0x27));
        assert_eq!(runtime.functions, functions);
        assert!(runtime.custom_mappings.is_empty());
        assert_eq!(runtime.bindings_revision, 0);
        assert_eq!(runtime.matched, 0);
    }
}

fn capture_mapping_draft() -> taprelay_core::mapping::CustomMapping {
    taprelay_core::mapping::CustomMapping {
        id: 25,
        name: "Speed".into(),
        enabled: true,
        shortcuts: Vec::new(),
        output: taprelay_core::mapping::MappingOutput::Keyboard {
            usage: 0x4f,
            modifiers: 0,
        },
        groups: Default::default(),
    }
}

#[test]
fn custom_capture_checks_duplicates_on_press_and_allows_retry_without_changing_the_draft() {
    let mut runtime = capture_runtime();
    let mut draft = capture_mapping_draft();
    draft
        .shortcuts
        .push(Shortcut::keyboard(ModifierSet::empty(), 0x79));
    // The persisted slot may be replaced, while the unsaved other slot stays reserved.
    let mut persisted = draft.clone();
    persisted.shortcuts[0] = Shortcut::keyboard(ModifierSet::empty(), 0x7a);
    runtime.custom_mappings.push(persisted);
    let original = runtime.custom_mappings.clone();
    runtime
        .apply_binding_command(BindingCommand::BeginCustomCapture {
            output: false,
            slot: 1,
            draft,
        })
        .unwrap();
    runtime.tick_with_capture_context(true, false);
    queue_capture_keys(&mut runtime, &[(0x79, true)], false);
    runtime.tick_with_capture_context(true, false);
    assert_eq!(
        runtime.capture().unwrap().error,
        Some(CaptureError::DuplicateMapping("Speed".into()))
    );
    assert!(runtime.custom_capture_result.is_none());
    queue_capture_keys(&mut runtime, &[(0x79, false)], false);
    runtime.tick_with_capture_context(true, false);
    assert!(runtime.recording());
    queue_capture_keys(&mut runtime, &[(0x78, true)], false);
    runtime.tick_with_capture_context(true, false);
    assert_eq!(
        runtime.capture().unwrap().error,
        Some(CaptureError::Duplicate(FunctionId::MediaNext))
    );
    queue_capture_keys(&mut runtime, &[(0x78, false), (0x27, true)], false);
    runtime.tick_with_capture_context(true, false);
    assert!(runtime.capture().unwrap().error.is_none());
    queue_capture_keys(&mut runtime, &[(0x27, false)], false);
    runtime.tick_with_capture_context(true, false);
    assert!(runtime.capture().is_none());
    assert_eq!(
        runtime.custom_capture_result.as_ref().unwrap().3,
        Shortcut::keyboard(ModifierSet::empty(), 0x27)
    );
    assert_eq!(runtime.custom_mappings, original);
}

#[test]
fn receiver_recording_accepts_mouse_buttons_and_escape_cancels() {
    let mut runtime = capture_runtime();
    runtime
        .apply_binding_command(BindingCommand::BeginCustomCapture {
            output: true,
            slot: 2,
            draft: capture_mapping_draft(),
        })
        .unwrap();
    runtime.tick_with_capture_context(true, false);
    for down in [true, false] {
        runtime.window_keys.push(InputEvent {
            code: InputCode::Mouse(MouseButton::Left),
            down,
            captured: Instant::now(),
        });
    }
    runtime.tick_with_capture_context(true, false);
    assert!(runtime.capture().is_none());
    assert_eq!(
        runtime.custom_capture_result.as_ref().unwrap().3,
        Shortcut::mouse(ModifierSet::empty(), MouseButton::Left)
    );
    runtime
        .apply_binding_command(BindingCommand::BeginCustomCapture {
            output: true,
            slot: 2,
            draft: capture_mapping_draft(),
        })
        .unwrap();
    runtime.tick_with_capture_context(true, false);
    queue_capture_keys(&mut runtime, &[(0x1b, true), (0x1b, false)], false);
    runtime.tick_with_capture_context(true, false);
    assert!(runtime.capture().is_none());
    assert_eq!(
        runtime.custom_capture_result.as_ref().unwrap().3,
        Shortcut::mouse(ModifierSet::empty(), MouseButton::Left)
    );
}

#[test]
fn receiver_recording_ignores_modifiers_in_preview_and_finishes_before_they_are_released() {
    let mut runtime = capture_runtime();
    for modifier in [
        0x10, 0x11, 0x12, 0x5b, 0x5c, 0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5,
    ] {
        runtime
            .apply_binding_command(BindingCommand::BeginCustomCapture {
                output: true,
                slot: 2,
                draft: capture_mapping_draft(),
            })
            .unwrap();
        runtime.tick_with_capture_context(true, false);
        queue_capture_keys(&mut runtime, &[(modifier, true)], false);
        runtime.tick_with_capture_context(true, false);
        let capture = runtime.capture().unwrap();
        assert!(capture.preview.is_empty());
        assert!(capture.error.is_none());
        queue_capture_keys(&mut runtime, &[(0x41, true)], false);
        runtime.tick_with_capture_context(true, false);
        assert_eq!(
            runtime.capture().unwrap().preview,
            vec![InputCode::Key(0x41)]
        );
        queue_capture_keys(&mut runtime, &[(0x41, false)], false);
        runtime.tick_with_capture_context(true, false);
        assert!(runtime.capture().is_none());
        assert_eq!(
            runtime.custom_capture_result.as_ref().unwrap().3,
            Shortcut::keyboard(ModifierSet::empty(), 0x41)
        );
        queue_capture_keys(&mut runtime, &[(modifier, false)], false);
        runtime.tick_with_capture_context(true, false);
    }
}

#[test]
fn builtin_capture_reports_conflict_with_a_disabled_custom_mapping() {
    let mut runtime = capture_runtime();
    runtime
        .custom_mappings
        .push(taprelay_core::mapping::CustomMapping {
            id: 1,
            name: "Speed".into(),
            enabled: false,
            shortcuts: vec![Shortcut::keyboard(ModifierSet::empty(), 0x79)],
            output: taprelay_core::mapping::MappingOutput::Keyboard {
                usage: 0x4f,
                modifiers: 0,
            },
            groups: Default::default(),
        });
    let original = runtime.functions.clone();
    begin_capture(&mut runtime, FunctionId::MediaPlayPause, 0);
    runtime.tick_with_capture_context(true, false);
    queue_capture_keys(&mut runtime, &[(0x79, true), (0x79, false)], false);
    runtime.tick_with_capture_context(true, false);
    assert_eq!(
        runtime.capture().unwrap().error,
        Some(CaptureError::DuplicateMapping("Speed".into()))
    );
    assert_eq!(runtime.functions, original);
}

fn begin_capture(runtime: &mut Runtime, id: FunctionId, slot: usize) {
    runtime
        .apply_binding_command(BindingCommand::BeginCapture(CaptureTarget { id, slot }))
        .unwrap();
}

fn queue_capture_keys(runtime: &mut Runtime, keys: &[(u8, bool)], native: bool) {
    for &(key, down) in keys {
        let event = InputEvent {
            code: InputCode::Key(key),
            down,
            captured: Instant::now(),
        };
        if native {
            runtime
                .sender
                .try_send(RoutedInput::Edge {
                    event,
                    result: function_output(
                        FunctionAction::App(AppCommand::ToggleListening),
                        down,
                        event.captured,
                    ),
                })
                .unwrap();
        } else {
            runtime.window_keys.push(event);
        }
    }
}

#[test]
fn capture_waits_for_old_input_then_commits_the_target_for_both_sources() {
    for (native, listening) in [(false, false), (false, true), (true, false), (true, true)] {
        let mut runtime = capture_runtime();
        runtime.listening = listening;
        let id = FunctionId::MediaPlayPause;
        begin_capture(&mut runtime, id, 1);
        let original = runtime.functions.clone();
        queue_capture_keys(&mut runtime, &[(0x11, true), (0x77, true)], native);
        runtime.tick_with_capture_context(true, true);
        assert_eq!(
            runtime.capture().unwrap().phase,
            CapturePhase::WaitingForRelease
        );
        assert_eq!(runtime.functions, original);
        queue_capture_keys(&mut runtime, &[(0x77, false), (0x11, false)], native);
        runtime.tick_with_capture_context(true, false);
        assert_eq!(runtime.capture().unwrap().phase, CapturePhase::Recording);
        queue_capture_keys(&mut runtime, &[(0x79, true), (0x79, false)], native);
        runtime.tick_with_capture_context(true, false);
        assert!(runtime.capture().is_none());
        assert_eq!(
            runtime.functions[&id].shortcuts,
            vec![
                original[&id].shortcuts[0].clone(),
                Shortcut::keyboard(ModifierSet::empty(), 0x79)
            ]
        );
        assert_eq!(runtime.bindings_revision, 1);
        assert_eq!(runtime.listening, listening);
        assert_eq!(runtime.matched, 0);
        let saved = Config {
            functions: runtime.functions.clone(),
            ..Default::default()
        };
        let restored: Config =
            serde_json::from_str(&serde_json::to_string(&saved).unwrap()).unwrap();
        assert_eq!(restored.functions, runtime.functions);
    }
}

#[test]
fn capture_rejects_standalone_primary_mouse_buttons_and_allows_modified_retry() {
    for button in [MouseButton::Left, MouseButton::Right] {
        let mut runtime = capture_runtime();
        let id = FunctionId::MediaPlayPause;
        begin_capture(&mut runtime, id, 0);
        runtime.tick_with_capture_context(true, false);
        let original = runtime.functions.clone();
        for modified in [false, true] {
            if modified {
                queue_capture_keys(&mut runtime, &[(0x11, true)], false);
            }
            for down in [true, false] {
                runtime.window_keys.push(InputEvent {
                    code: InputCode::Mouse(button),
                    down,
                    captured: Instant::now(),
                });
            }
            if modified {
                queue_capture_keys(&mut runtime, &[(0x11, false)], false);
            }
            runtime.tick_with_capture_context(true, false);
            if !modified {
                assert_eq!(
                    runtime.capture().unwrap().error,
                    Some(CaptureError::Invalid)
                );
                assert_eq!(runtime.functions, original);
                assert_eq!(runtime.bindings_revision, 0);
            }
        }
        assert!(runtime.capture().is_none());
        assert_eq!(
            runtime.functions[&id].shortcuts,
            vec![Shortcut::mouse(ModifierSet::from_keys([0x11]), button)]
        );
    }
}

#[test]
fn capture_invalid_and_duplicate_attempts_preserve_bindings_and_allow_retry() {
    let mut runtime = capture_runtime();
    let id = FunctionId::MediaPlayPause;
    begin_capture(&mut runtime, id, 0);
    runtime.tick_with_capture_context(true, false);
    let original = runtime.functions.clone();
    for (keys, expected) in [
        (vec![(0x11, true), (0x11, false)], CaptureError::Invalid),
        (
            vec![(0x41, true), (0x42, true), (0x42, false), (0x41, false)],
            CaptureError::Invalid,
        ),
        (
            vec![(0x78, true), (0x78, false)],
            CaptureError::Duplicate(FunctionId::MediaNext),
        ),
    ] {
        queue_capture_keys(&mut runtime, &keys, false);
        runtime.tick_with_capture_context(true, false);
        assert_eq!(runtime.capture().unwrap().error, Some(expected));
        assert_eq!(runtime.functions, original);
        assert_eq!(runtime.bindings_revision, 0);
    }
    queue_capture_keys(&mut runtime, &[(0x79, true), (0x79, false)], false);
    runtime.tick_with_capture_context(true, false);
    assert!(runtime.capture().is_none());
    assert_eq!(
        runtime.functions[&id].shortcuts,
        vec![Shortcut::keyboard(ModifierSet::empty(), 0x79)]
    );
    assert_eq!(runtime.bindings_revision, 1);
}

#[test]
fn capture_reuses_shortcuts_in_disjoint_groups_but_rejects_overlap_edits() {
    use taprelay_core::foreground_app::ForegroundAppGroup;
    let mut runtime = capture_runtime();
    for (id, name, path) in [
        (FunctionId::MediaPlayPause, "games", r"C:\game.exe"),
        (FunctionId::MediaNext, "work", r"C:\work.exe"),
    ] {
        runtime.foreground_app_rules.groups.insert(
            name.into(),
            ForegroundAppGroup {
                name: name.into(),
                foreground_apps: vec![path.into()],
            },
        );
        runtime
            .foreground_app_rules
            .assignments
            .insert(id, [name.into()].into());
    }
    begin_capture(&mut runtime, FunctionId::MediaPlayPause, 0);
    runtime.tick_with_capture_context(true, false);
    runtime.submit_capture(Shortcut::keyboard(ModifierSet::empty(), 0x78));
    assert!(runtime.capture().is_none());
    assert_eq!(
        runtime.functions[&FunctionId::MediaPlayPause].shortcuts,
        runtime.functions[&FunctionId::MediaNext].shortcuts
    );
    let original = runtime.foreground_app_rules.clone();
    let revision = runtime.bindings_revision;
    let mut overlap = original.clone();
    overlap
        .groups
        .get_mut("work")
        .unwrap()
        .foreground_apps
        .push(r"c:\GAME.exe".into());
    assert!(runtime.set_foreground_app_rules(overlap).is_err());
    let mut deleted = original.clone();
    deleted.remove_group("games");
    assert!(runtime.set_foreground_app_rules(deleted).is_err());
    assert_eq!(runtime.foreground_app_rules, original);
    assert_eq!(runtime.bindings_revision, revision);
}

#[test]
fn capture_cancellation_rejects_pending_and_late_input_without_polluting_next_session() {
    for cancel in [0, 1, 2] {
        let mut runtime = capture_runtime();
        let id = FunctionId::MediaPlayPause;
        begin_capture(&mut runtime, id, 0);
        runtime.tick_with_capture_context(true, false);
        let original = runtime.functions.clone();
        if cancel == 0 {
            queue_capture_keys(&mut runtime, &[(0x1b, true)], true);
        }
        queue_capture_keys(&mut runtime, &[(0x79, true), (0x79, false)], true);
        if cancel == 1 {
            runtime
                .apply_binding_command(BindingCommand::CancelCapture)
                .unwrap();
        }
        runtime.tick_with_capture_context(cancel != 2, false);
        assert!(runtime.capture().is_none());
        assert_eq!(runtime.functions, original);
        let stale = InputEvent {
            code: InputCode::Key(0x7a),
            down: true,
            captured: Instant::now(),
        };
        begin_capture(&mut runtime, id, 0);
        runtime.tick_with_capture_context(true, false);
        runtime.window_keys.push(stale);
        queue_capture_keys(&mut runtime, &[(0x79, true), (0x79, false)], false);
        runtime.tick_with_capture_context(true, false);
        assert_eq!(
            runtime.functions[&id].shortcuts,
            vec![Shortcut::keyboard(ModifierSet::empty(), 0x79)]
        );
        assert_eq!(runtime.bindings_revision, 1);
        assert_eq!(runtime.matched, 0);
    }
}

#[test]
fn capture_target_changes_and_failed_edits_have_one_authoritative_state() {
    let mut runtime = capture_runtime();
    let id = FunctionId::MediaPlayPause;
    begin_capture(&mut runtime, id, 0);
    runtime.tick_with_capture_context(true, false);
    queue_capture_keys(&mut runtime, &[(0x79, true)], false);
    runtime.tick_with_capture_context(true, false);
    begin_capture(&mut runtime, FunctionId::MediaNext, 0);
    let target = CaptureTarget {
        id: FunctionId::MediaNext,
        slot: 0,
    };
    assert_eq!(
        runtime.capture().unwrap().kind,
        CaptureKind::Function(target)
    );
    assert!(
        runtime
            .apply_binding_command(BindingCommand::BeginCapture(CaptureTarget { id, slot: 2 }))
            .is_err()
    );
    assert_eq!(
        runtime.capture().unwrap().kind,
        CaptureKind::Function(target)
    );
    queue_capture_keys(&mut runtime, &[(0x79, false)], false);
    runtime.tick_with_capture_context(true, false);
    queue_capture_keys(&mut runtime, &[(0x7a, true), (0x7a, false)], false);
    runtime.tick_with_capture_context(true, false);
    assert_eq!(
        runtime.functions[&id].shortcuts,
        vec![Shortcut::keyboard(ModifierSet::empty(), 0x77)]
    );
    assert_eq!(
        runtime.functions[&target.id].shortcuts,
        vec![Shortcut::keyboard(ModifierSet::empty(), 0x7a)]
    );
    begin_capture(&mut runtime, id, 0);
    assert!(
        runtime
            .apply_binding_command(BindingCommand::DeleteShortcut(CaptureTarget {
                id,
                slot: 2
            }))
            .is_err()
    );
    assert!(runtime.capture().is_none());
    begin_capture(&mut runtime, id, 0);
    runtime
        .apply_binding_command(BindingCommand::SetFunctionEnabled { id, enabled: false })
        .unwrap();
    assert!(runtime.capture().is_none());
    assert!(
        runtime
            .apply_binding_command(BindingCommand::BeginCapture(CaptureTarget { id, slot: 0 }))
            .is_err()
    );
    assert!(runtime.capture().is_none());
    begin_capture(&mut runtime, target.id, 0);
    runtime
        .apply_binding_command(BindingCommand::DeleteShortcut(target))
        .unwrap();
    assert!(runtime.capture().is_none());
    assert!(runtime.functions[&target.id].shortcuts.is_empty());
}

#[test]
fn interrupted_capture_waits_for_held_keys_before_accepting_a_new_gesture() {
    let mut runtime = capture_runtime();
    begin_capture(&mut runtime, FunctionId::MediaPlayPause, 0);
    runtime.tick_with_capture_context(true, false);
    let original = runtime.functions.clone();
    queue_capture_keys(&mut runtime, &[(0x79, true)], false);
    runtime.tick_with_capture_context(true, true);
    runtime.last_transport_ready = true;
    runtime.tick_with_capture_context(true, true);
    assert_eq!(
        runtime.capture().unwrap().phase,
        CapturePhase::WaitingForRelease
    );
    assert!(runtime.capture().unwrap().preview.is_empty());
    queue_capture_keys(&mut runtime, &[(0x79, false)], false);
    runtime.tick_with_capture_context(true, false);
    assert_eq!(runtime.functions, original);
    assert_eq!(runtime.capture().unwrap().phase, CapturePhase::Recording);
    queue_capture_keys(&mut runtime, &[(0x7a, true), (0x7a, false)], false);
    runtime.tick_with_capture_context(true, false);
    assert_eq!(runtime.bindings_revision, 1);
    assert!(runtime.capture().is_none());
}

#[test]
fn capture_listener_failure_discards_candidate_and_ends_session() {
    let mut runtime = capture_runtime();
    begin_capture(&mut runtime, FunctionId::MediaPlayPause, 0);
    runtime.tick_with_capture_context(true, false);
    let original = runtime.functions.clone();
    queue_capture_keys(&mut runtime, &[(0x79, true), (0x79, false)], false);
    runtime.input = Some(Box::new(FakeInput(Some("capture hook failed"))));
    runtime.tick_with_capture_context(true, false);
    assert!(runtime.capture().is_none());
    assert_eq!(runtime.error.as_deref(), Some("capture hook failed"));
    assert_eq!(runtime.functions, original);
    assert!(runtime.input.is_none());
    assert!(!runtime.listening);
}

#[test]
fn capture_deduplicates_window_and_hook_edges_and_does_not_trigger_old_binding() {
    let mut runtime = capture_runtime();
    let id = FunctionId::MediaPlayPause;
    begin_capture(&mut runtime, id, 0);
    runtime.tick_with_capture_context(true, false);
    for down in [true, true, false] {
        queue_capture_keys(&mut runtime, &[(0x77, down)], true);
        queue_capture_keys(&mut runtime, &[(0x77, down)], false);
        runtime.tick_with_capture_context(true, false);
    }
    assert!(runtime.capture().is_none());
    assert_eq!(runtime.bindings_revision, 1);
    assert_eq!(runtime.matched, 0);
    assert!(runtime.listening);
    assert_eq!(
        runtime.functions[&id].shortcuts,
        vec![Shortcut::keyboard(ModifierSet::empty(), 0x77)]
    );
}
#[test]
fn handoff_drains_accepted_hook_input_before_a_queued_listener_edit() {
    use std::sync::{Arc, Mutex, mpsc as sync};
    struct GatedInput {
        gate: Mutex<Option<sync::Receiver<()>>>,
        entered: sync::Sender<()>,
        listening: Arc<Mutex<Vec<bool>>>,
    }
    impl InputSource for GatedInput {
        fn is_finished(&self) -> bool {
            false
        }
        fn configure(
            &self,
            _: &FunctionConfigs,
            _: &[taprelay_core::mapping::CustomMapping],
            _: &taprelay_core::foreground_app::ForegroundAppRules,
            listening: bool,
            _: bool,
            _: u64,
        ) -> RouteResult {
            self.listening.lock().unwrap().push(listening);
            if let Some(gate) = self.gate.lock().unwrap().take() {
                self.entered.send(()).unwrap();
                gate.recv_timeout(std::time::Duration::from_secs(3))
                    .unwrap();
            }
            RouteResult::default()
        }
        fn terminate(&self, reason: RouterReason) -> RouteResult {
            FakeInput(None).terminate(reason)
        }
        fn replay(&self, input: taprelay_core::input_router::PhysicalInput) -> Result<()> {
            FakeInput(None).replay(input)
        }
    }
    let (release, gate) = sync::channel();
    let (entered, blocked) = sync::channel();
    let (sender, input) = sync::channel();
    let observed = Arc::new(Mutex::new(Vec::new()));
    let listening = observed.clone();
    let mut handle = crate::runtime::worker::RuntimeHandle::spawn(
        Config::default(),
        move |_| {
            let mut runtime = app_runtime();
            runtime.input_initialized = true;
            runtime.input = Some(Box::new(GatedInput {
                gate: Mutex::new(Some(gate)),
                entered,
                listening,
            }));
            sender.send(runtime.sender.clone()).unwrap();
            runtime
        },
        Runtime::tick,
        64,
    )
    .unwrap();
    blocked
        .recv_timeout(std::time::Duration::from_secs(3))
        .unwrap();
    let created = Instant::now();
    input
        .recv()
        .unwrap()
        .try_send(RoutedInput::Edge {
            event: InputEvent {
                code: InputCode::Key(0x78),
                down: true,
                captured: created,
            },
            result: function_output(
                FunctionAction::App(AppCommand::ToggleListening),
                true,
                created,
            ),
        })
        .unwrap();
    handle.set_listening(false).unwrap();
    release.send(()).unwrap();
    let deadline = Instant::now() + std::time::Duration::from_secs(3);
    while handle.busy() {
        handle.poll();
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let states = observed.lock().unwrap();
    assert!(states.contains(&true), "the accepted shortcut must execute");
    assert_eq!(
        states.last(),
        Some(&false),
        "the later edit must take effect last"
    );
    assert!(!handle.listening);
}
