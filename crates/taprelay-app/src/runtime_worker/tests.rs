use super::*;
use taprelay_core::{function::FunctionId, input::InputCode};

fn until(handle: &mut RuntimeHandle, condition: impl Fn(&RuntimeHandle) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        handle.poll();
        if condition(handle) {
            return;
        }
        assert!(Instant::now() < deadline, "worker did not settle");
        thread::sleep(Duration::from_millis(1));
    }
}

fn idle(handle: &mut RuntimeHandle) {
    until(handle, |handle| !handle.busy());
}

fn block(handle: &mut RuntimeHandle) -> mpsc::Sender<()> {
    let (entered, started) = mpsc::channel();
    let (release, wait) = mpsc::channel();
    handle
        .update(move |_| {
            entered.send(()).unwrap();
            wait.recv_timeout(Duration::from_secs(3)).unwrap();
            Ok(())
        })
        .unwrap();
    started.recv_timeout(Duration::from_secs(3)).unwrap();
    release
}

#[test]
fn updates_preserve_preferences_and_all_unread_results_until_consumed() {
    let mut handle = RuntimeHandle::new(Config::default()).unwrap();
    handle.config.options.theme = crate::config::Theme::Dark;
    handle.config.window.width = 1100.;
    handle.config.wizard.dismissed = true;
    let preferences = serde_json::to_value(&handle.config).unwrap();
    handle
        .update(|runtime| {
            runtime
                .functions
                .get_mut(&FunctionId::MediaMute)
                .unwrap()
                .enabled = true;
            runtime.remembered_device = Some(Device::from_target(&Default::default()));
            runtime.error = Some("first notification".into());
            runtime.bindings_changed();
            anyhow::bail!("command failed")
        })
        .unwrap();
    idle(&mut handle);
    handle
        .update(|runtime| {
            runtime.error = Some("second notification".into());
            Ok(())
        })
        .unwrap();
    idle(&mut handle);
    let updated = serde_json::to_value(&handle.config).unwrap();
    for field in ["options", "window", "wizard"] {
        assert_eq!(updated[field], preferences[field]);
    }
    assert!(handle.config.functions[&FunctionId::MediaMute].enabled);
    assert!(handle.config.remembered_device.is_some());
    for expected in [
        "first notification",
        "command failed",
        "second notification",
    ] {
        assert_eq!(handle.take_notice(), Some(Notice::Error(expected.into())));
    }
    assert!(handle.take_notice().is_none());
    assert!(handle.take_bindings_changed());
    assert!(!handle.take_bindings_changed());
    handle.tick();
    until(&mut handle, |handle| handle.pending.is_empty());
    assert!(handle.take_notice().is_none());
    assert!(!handle.take_bindings_changed());
}

#[test]
fn navigation_cancellation_only_shows_progress_when_it_takes_time() {
    let mut handle = RuntimeHandle::new(Config::default()).unwrap();
    for _ in 0..3 {
        handle
            .apply_binding_command(BindingCommand::CancelCapture)
            .unwrap();
        assert!(handle.busy());
        assert!(
            !handle.progress_pending(),
            "a navigation cancellation must not immediately flash a progress toast"
        );
        handle
            .pending
            .values_mut()
            .for_each(|pending| pending.since -= Duration::from_millis(500));
        assert!(
            handle.progress_pending(),
            "a delayed operation needs feedback"
        );
        idle(&mut handle);
        assert!(!handle.progress_pending());
    }
}

#[test]
fn slow_worker_does_not_block_polling_or_accept_duplicate_edits() {
    let mut handle = RuntimeHandle::new(Config::default()).unwrap();
    let release = block(&mut handle);
    let started = Instant::now();
    for _ in 0..1000 {
        handle.tick();
    }
    assert!(started.elapsed() < Duration::from_millis(100));
    assert_eq!(handle.pending.len(), 2);
    assert!(
        handle
            .update(|_| panic!("duplicate executed"))
            .unwrap_err()
            .downcast_ref::<HandoffError>()
            .is_some_and(|error| matches!(error, HandoffError::Busy))
    );
    handle
        .pending
        .values_mut()
        .for_each(|pending| pending.since -= Duration::from_secs(6));
    assert!(handle.slow());
    assert!(handle.busy());
    release.send(()).unwrap();
    until(&mut handle, |handle| handle.pending.is_empty());
    assert!(!handle.slow());
    assert!(handle.take_notice().is_none());
}

#[test]
fn delayed_binding_edit_is_applied_once_without_overwriting_new_preferences() {
    let mut handle = RuntimeHandle::new(Config::default()).unwrap();
    let (release, wait) = mpsc::channel();
    handle
        .update(move |runtime| {
            wait.recv_timeout(Duration::from_secs(3)).unwrap();
            runtime.apply_binding_command(BindingCommand::SetFunctionEnabled {
                id: FunctionId::MediaMute,
                enabled: true,
            })
        })
        .unwrap();
    handle.config.options.theme = crate::config::Theme::Dark;
    handle.config.window.width = 1200.;
    release.send(()).unwrap();
    idle(&mut handle);
    assert!(handle.config.functions[&FunctionId::MediaMute].enabled);
    assert_eq!(handle.bindings_revision, 1);
    assert_eq!(handle.config.options.theme, crate::config::Theme::Dark);
    assert_eq!(handle.config.window.width, 1200.);
    assert!(handle.take_bindings_changed());
}

#[test]
fn cancellation_and_shutdown_wait_in_order_and_return_final_config() {
    let mut handle = RuntimeHandle::new(Config::default()).unwrap();
    let (release, wait) = mpsc::channel();
    handle
        .update(move |runtime| {
            wait.recv_timeout(Duration::from_secs(3)).unwrap();
            runtime.apply_binding_command(BindingCommand::SetFunctionEnabled {
                id: FunctionId::MediaMute,
                enabled: true,
            })
        })
        .unwrap();
    handle
        .apply_binding_command(BindingCommand::CancelCapture)
        .unwrap();
    handle
        .apply_binding_command(BindingCommand::CancelCapture)
        .unwrap();
    assert_eq!(handle.pending.len(), 2);
    let started = Instant::now();
    handle.shutdown().unwrap();
    assert!(started.elapsed() < Duration::from_millis(16));
    assert!(!handle.stopped());
    assert!(handle.refresh().is_err());
    release.send(()).unwrap();
    until(&mut handle, RuntimeHandle::stopped);
    assert!(handle.config.functions[&FunctionId::MediaMute].enabled);
    assert!(handle.take_bindings_changed());
    assert!(handle.pending.is_empty());
    assert!(handle.capture().is_none());
    assert!(!handle.state.ready);
}

#[test]
fn command_failure_is_delivered_once_and_does_not_discard_following_results() {
    let mut handle = RuntimeHandle::new(Config::default()).unwrap();
    handle
        .update(|runtime| {
            runtime.error = Some("same failure".into());
            anyhow::bail!("same failure")
        })
        .unwrap();
    idle(&mut handle);
    assert_eq!(
        handle.take_notice(),
        Some(Notice::Error("same failure".into()))
    );
    assert!(handle.take_notice().is_none());
    handle
        .update(|runtime| {
            runtime.matched = 42;
            Ok(())
        })
        .unwrap();
    idle(&mut handle);
    assert_eq!(handle.matched, 42);
    assert!(handle.take_notice().is_none());
}

#[test]
fn test_delivery_results_are_not_overwritten_by_later_snapshots() {
    let mut handle = RuntimeHandle::new(Config::default()).unwrap();
    for state in [
        TestStatus::Succeeded,
        TestStatus::Failed("delivery failed".into()),
    ] {
        handle
            .update(move |runtime| {
                runtime.test = state;
                Ok(())
            })
            .unwrap();
        idle(&mut handle);
    }
    assert_eq!(
        handle.take_notice(),
        Some(Notice::Test(TestStatus::Succeeded))
    );
    assert_eq!(
        handle.take_notice(),
        Some(Notice::Test(TestStatus::Failed("delivery failed".into())))
    );
    assert!(handle.take_notice().is_none());
}

#[test]
fn worker_panic_retires_queued_operations_and_exposes_one_terminal_error() {
    let mut handle = RuntimeHandle::new(Config::default()).unwrap();
    let (release, wait) = mpsc::channel();
    handle
        .update(move |runtime| {
            wait.recv_timeout(Duration::from_secs(3)).unwrap();
            runtime.test = TestStatus::Pending(7);
            panic!("scripted worker failure")
        })
        .unwrap();
    handle.shutdown().unwrap();
    release.send(()).unwrap();
    until(&mut handle, RuntimeHandle::stopped);
    assert!(handle.pending.is_empty());
    assert!(!handle.busy());
    assert_eq!(handle.test, TestStatus::Idle);
    assert!(matches!(
        handle.take_notice(),
        Some(Notice::Test(TestStatus::Failed(_)))
    ));
    assert!(
        matches!(handle.take_notice(), Some(Notice::Error(message)) if message.contains("panicked"))
    );
    handle.tick();
    assert!(handle.take_notice().is_none());
    assert!(handle.refresh().is_err());
}

#[test]
fn queue_rejection_keeps_unsent_input_and_does_not_execute_the_edit() {
    let mut handle =
        RuntimeHandle::spawn(Config::default(), Runtime::new, Runtime::tick, 1).unwrap();
    let release = block(&mut handle);
    handle.tick();
    let event = InputEvent {
        code: InputCode::Key(0x78),
        down: true,
        captured: Instant::now(),
    };
    handle.window_key(event);
    handle.consume_ui_input();
    let error = handle
        .apply_binding_command(BindingCommand::CancelCapture)
        .unwrap_err();
    assert!(matches!(
        error.downcast_ref::<HandoffError>(),
        Some(HandoffError::QueueFull)
    ));
    assert_eq!(handle.window_keys.len(), 1);
    assert!(handle.ui_input.is_some());
    assert_eq!(handle.pending.len(), 2);
    release.send(()).unwrap();
    until(&mut handle, |handle| handle.pending.is_empty());
    handle
        .apply_binding_command(BindingCommand::CancelCapture)
        .unwrap();
    idle(&mut handle);
    assert!(handle.window_keys.is_empty());
}

#[test]
fn phase3_nonblocking_latency_measurement() {
    let mut handle = RuntimeHandle::new(Config::default()).unwrap();
    for delay in [0, 100, 500] {
        let mut submission = Vec::new();
        let mut polling = Vec::new();
        for _ in 0..10 {
            let started = Instant::now();
            handle
                .update(move |_| {
                    thread::sleep(Duration::from_millis(delay));
                    Ok(())
                })
                .unwrap();
            submission.push(started.elapsed().as_micros());
            while handle.busy() {
                let started = Instant::now();
                handle.tick();
                polling.push(started.elapsed().as_micros());
                thread::sleep(Duration::from_millis(1));
            }
            until(&mut handle, |handle| handle.pending.is_empty());
        }
        submission.sort_unstable();
        polling.sort_unstable();
        println!(
            "async delay_ms={delay} submit_n={} submit_p50_us={} submit_p90_us={} submit_max_us={} poll_n={} poll_p50_us={} poll_p90_us={} poll_max_us={}",
            submission.len(),
            submission[5],
            submission[8],
            submission[9],
            polling.len(),
            polling[polling.len() / 2],
            polling[polling.len() * 9 / 10],
            polling.last().unwrap()
        );
        assert!(*submission.last().unwrap() < 16_000);
        assert!(*polling.last().unwrap() < 16_000);
    }
}
