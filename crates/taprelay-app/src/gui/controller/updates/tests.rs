use super::*;
use crate::{
    config::Config,
    gui::{
        action::Action,
        controller::startup_tests::{controller, ready},
    },
};

#[test]
fn checks_after_startup_delay_and_again_after_24_hours_without_downloading() {
    let (mut app, _folder) = controller(ready());
    let now = app.started;
    let due = now + STARTUP_DELAY;
    app.updates.offline.as_mut().unwrap().latest = Some(Target::for_test("2.0.0", false));
    app.poll_updates(due - Duration::from_millis(1));
    assert_eq!(app.updates.status, UpdateStatus::Idle);
    app.poll_updates(due);
    assert!(app.updates.dialog);
    assert_eq!(app.updates.status, UpdateStatus::Available);
    assert!(!app.updates.requested_notes && app.updates.download.is_none());
    app.passed = true;
    let mut ui = Presentation::default();
    app.action(&mut ui, Action::RemindUpdateLater, "").unwrap();
    assert!(!app.updates.dialog);
    assert_eq!(app.runtime.config.options.skipped_update_version, None);
    app.poll_updates(due + INTERVAL - Duration::from_millis(1));
    assert!(!app.updates.dialog);
    app.poll_updates(due + INTERVAL);
    assert!(app.updates.dialog);
    assert!(app.updates.download.is_none());
}

#[test]
fn disabled_automatic_checks_do_not_run_but_manual_check_can_open_the_exact_portable_zip() {
    let (mut app, _folder) = controller(ready());
    app.passed = true;
    let mut ui = Presentation::default();
    app.action(&mut ui, Action::SetAutoCheckUpdates, "0")
        .unwrap();
    assert!(!Config::load(&app.path).unwrap().options.auto_check_updates);
    let due = app.started + INTERVAL;
    let target = Target::for_test("2.3.4", true);
    let expected = target.portable_url().unwrap();
    app.updates.offline.as_mut().unwrap().latest = Some(target);
    app.poll_updates(due);
    assert_eq!(app.updates.status, UpdateStatus::Idle);
    app.action(&mut ui, Action::CheckUpdates, "").unwrap();
    app.poll_updates(due);
    assert!(app.updates.dialog);
    assert!(app.updates.offline.as_ref().unwrap().opened.is_empty());
    app.action(&mut ui, Action::AcceptUpdate, "").unwrap();
    assert_eq!(app.updates.offline.as_ref().unwrap().opened, [expected]);
    assert!(!app.updates.dialog && app.updates.download.is_none() && app.exit.is_none());
}

#[test]
fn skipped_tag_survives_reload_but_does_not_hide_a_manual_check_or_newer_version() {
    let (mut app, _folder) = controller(ready());
    app.passed = true;
    let mut ui = Presentation::default();
    let due = app.started + STARTUP_DELAY;
    app.updates.offline.as_mut().unwrap().latest = Some(Target::for_test("2.0.0", false));
    app.poll_updates(due);
    app.action(&mut ui, Action::SkipUpdate, "").unwrap();
    let saved = Config::load(&app.path).unwrap();
    assert_eq!(
        saved.options.skipped_update_version.as_deref(),
        Some("v2.0.0")
    );
    app.poll_updates(due + INTERVAL);
    assert!(!app.updates.dialog);
    app.action(&mut ui, Action::CheckUpdates, "").unwrap();
    app.poll_updates(due + INTERVAL);
    assert!(app.updates.dialog);
    app.action(&mut ui, Action::RemindUpdateLater, "").unwrap();
    app.updates.offline.as_mut().unwrap().latest = Some(Target::for_test("2.0.1", false));
    app.poll_updates(due + INTERVAL * 2);
    assert!(app.updates.dialog);
    assert_eq!(app.updates.target.as_ref().unwrap().tag(), "v2.0.1");
}

#[test]
fn disabling_auto_check_ignores_an_in_flight_result_and_manual_checks_can_join_one() {
    let (mut app, _folder) = controller(ready());
    let now = app.started;
    let state = &mut app.updates;
    state.offline.as_mut().unwrap().latest = Some(Target::for_test("2.0.0", true));
    state.check(false, now);
    state.auto_changed(false, now);
    state.discovery(now, false, None);
    assert_eq!(state.status, UpdateStatus::Idle);
    state.check(false, now);
    state.check(true, now);
    state.auto_changed(false, now);
    state.discovery(now, false, Some("v2.0.0"));
    assert!(state.dialog);
}
