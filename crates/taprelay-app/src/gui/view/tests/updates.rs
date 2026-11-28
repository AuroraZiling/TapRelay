use super::support::open_sized;
use crate::{
    config::Config,
    gui::presentation::updates::UpdateStatus,
    updates::{Notes, Target},
};
use gpui_kit::{AppContext, Role, Task, TestAppContext, test::TestWindowExt};

fn bilingual_notes() -> Notes {
    serde_json::from_value(serde_json::json!({
        "tag": "v2.3.4",
        "en": { "fallback": false, "blocks": [
            {"kind":"improved", "text":"Improvements"},
            {"kind":"bullet", "text":"Reconnect the last receiver more quickly."}
        ] },
        "zh": { "fallback": false, "blocks": [
            {"kind":"improved", "text":"改进"},
            {"kind":"bullet", "text":"更快地重新连接上次使用的接收设备。"}
        ] }
    }))
    .unwrap()
}

#[gpui_kit::test]
fn manual_checks_work_when_auto_is_off_and_a_skipped_portable_release_can_be_requested_again(
    cx: &mut TestAppContext,
) {
    let (handle, view, folder) = open_sized(cx, 900., 2500.);
    cx.update_window(handle, |_, window, cx| {
        cx.set_reduce_motion(true);
        view.update(cx, |v, _| v._pump = Task::ready(()));
        window.render_frame(cx);
        window.click(("nav", 3usize), cx);
        window.click("auto-check-updates", cx);
        assert_eq!(window.find("auto-check-updates").checked(), Some(false));
        window.click("check-updates", cx);
        view.update(cx, |v, cx| {
            v.controller.tick(&mut v.ui);
            v.effects(window, cx);
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(
            window.find("update-status").label(),
            Some("TapRelay is up to date.")
        );
        let saved = || {
            Config::load(&folder.path().join("config.json"))
                .unwrap()
                .options
        };
        assert!(!saved().auto_check_updates);

        window.click(("language", 2usize), cx);
        view.update(cx, |v, cx| {
            v.controller.offer_update_for_test(
                &mut v.ui,
                Target::for_test("2.3.4", true),
                Some(bilingual_notes()),
                true,
            );
            v.effects(window, cx);
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(
            window.find("release-dialog").label(),
            Some("更新到 TapRelay v2.3.4")
        );
        assert_eq!(
            window.find(("release-note", 1usize)).label(),
            Some("更快地重新连接上次使用的接收设备。")
        );
        assert_eq!(window.find("update-accept").label(), Some("下载便携版 ZIP"));
        window.within("dialog").click("update-skip", cx);
        assert!(!view.read(cx).ui.update_dialog_open);
        assert_eq!(saved().skipped_update_version.as_deref(), Some("v2.3.4"));

        window.click("check-updates", cx);
        view.update(cx, |v, cx| {
            v.controller.tick(&mut v.ui);
            v.effects(window, cx);
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(window.find("update-release-link").role(), Some(Role::Link));
        window.within("dialog").click("update-release-link", cx);
        assert!(view.read(cx).ui.update_dialog_open);
        window.within("dialog").click("update-accept", cx);
        assert!(!view.read(cx).ui.update_dialog_open);
        assert_ne!(view.read(cx).ui.update_status, UpdateStatus::Applying);
    })
    .unwrap();
}

#[gpui_kit::test]
fn installed_update_keeps_its_notes_during_download_and_allows_retry_after_failure(
    cx: &mut TestAppContext,
) {
    let (handle, view, _folder) = open_sized(cx, 840., 620.);
    cx.update_window(handle, |_, window, cx| {
        cx.set_reduce_motion(true);
        view.update(cx, |v, cx| {
            v._pump = Task::ready(());
            v.controller.offer_update_for_test(
                &mut v.ui,
                Target::for_test("2.3.4", false),
                Some(bilingual_notes()),
                false,
            );
            v.effects(window, cx);
            cx.notify();
        });
        window.render_frame(cx);
        assert!(!view.read(cx).ui.update_offer.as_ref().unwrap().show_notes);
        assert!(window.try_find(("release-note", 0usize)).is_none());
        window.within("dialog").click("show-update-notes", cx);
        assert!(view.read(cx).ui.update_offer.as_ref().unwrap().show_notes);
        assert_eq!(
            window.find(("release-note", 0usize)).label(),
            Some("Improvements")
        );
        view.update(cx, |v, cx| {
            v.controller.offer_update_for_test(
                &mut v.ui,
                Target::for_test("2.3.4", false),
                None,
                true,
            );
            v.controller.set_notes_loading_for_test(true);
            v.controller.sync(&mut v.ui);
            v.effects(window, cx);
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(
            window.find("release-notes-loading").role(),
            Some(gpui_kit::Role::Status)
        );
        assert!(window.try_find("update-release-link").is_none());
        view.update(cx, |v, cx| {
            v.controller.set_notes_loading_for_test(false);
            v.controller.offer_update_for_test(
                &mut v.ui,
                Target::for_test("2.3.4", false),
                Some(bilingual_notes()),
                true,
            );
            v.effects(window, cx);
            cx.notify();
        });
        window.render_frame(cx);
        assert_eq!(
            window.find("update-accept").label(),
            Some("Update and restart")
        );
        window.within("dialog").click("update-accept", cx);
        assert!(matches!(
            view.read(cx).ui.update_status,
            UpdateStatus::Downloading(_)
        ));
        assert_eq!(
            window.find("update-progress-label").role(),
            Some(Role::Status)
        );
        assert!(window.try_find("update-accept").is_none());
        window.press("escape", cx);
        assert!(view.read(cx).ui.update_dialog_open);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while view.read(cx).ui.update_status != UpdateStatus::DownloadFailed {
            assert!(std::time::Instant::now() < deadline);
            view.update(cx, |v, cx| {
                v.controller.tick(&mut v.ui);
                v.effects(window, cx);
                cx.notify();
            });
            std::thread::yield_now();
        }
        window.render_frame(cx);
        assert!(window.find("update-accept").visible());
        assert_eq!(
            window.find(("release-note", 0usize)).label(),
            Some("Improvements")
        );
        window.press("escape", cx);
        assert!(!view.read(cx).ui.update_dialog_open);
        assert!(
            view.read(cx)
                .controller
                .runtime
                .config
                .options
                .skipped_update_version
                .is_none()
        );
    })
    .unwrap();
}

#[gpui_kit::test]
fn confirmed_update_dialog_opens_and_retries_before_bluetooth_is_ready(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open_sized(cx, 840., 620.);
    cx.update_window(handle, |_, window, cx| {
        cx.set_reduce_motion(true);
        view.update(cx, |v, cx| {
            v._pump = Task::ready(());
            v.controller.passed = false;
            v.ui.mode = 0;
            v.controller.offer_update_for_test(
                &mut v.ui,
                Target::for_test("2.3.4", false),
                Some(bilingual_notes()),
                false,
            );
            v.effects(window, cx);
            cx.notify();
        });
        window.render_frame(cx);
        assert!(window.try_find("release-dialog").is_none());

        view.update(cx, |v, cx| {
            v.controller
                .action(&mut v.ui, crate::gui::action::Action::AcceptUpdate, "")
                .unwrap();
            v.effects(window, cx);
            cx.notify();
        });
        window.render_frame(cx);
        assert!(matches!(
            view.read(cx).ui.update_status,
            UpdateStatus::Downloading(_)
        ));
        assert_eq!(
            window.find("update-progress-label").role(),
            Some(Role::Status)
        );
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while view.read(cx).ui.update_status != UpdateStatus::DownloadFailed {
            assert!(std::time::Instant::now() < deadline);
            view.update(cx, |v, cx| {
                v.controller.poll_updates(std::time::Instant::now());
                v.controller.sync(&mut v.ui);
                v.effects(window, cx);
                cx.notify();
            });
            std::thread::yield_now();
        }
        window.render_frame(cx);
        assert!(!view.read(cx).controller.passed);
        assert_eq!(view.read(cx).ui.mode, 0);
        assert!(window.find("update-accept").visible());
        window.within("dialog").click("update-accept", cx);
        assert!(matches!(
            view.read(cx).ui.update_status,
            UpdateStatus::Downloading(_)
        ));
        while view.read(cx).ui.update_status != UpdateStatus::DownloadFailed {
            assert!(std::time::Instant::now() < deadline);
            view.update(cx, |v, cx| {
                v.controller.poll_updates(std::time::Instant::now());
                v.controller.sync(&mut v.ui);
                v.effects(window, cx);
                cx.notify();
            });
            std::thread::yield_now();
        }
        window.render_frame(cx);
        window.within("dialog").click("update-later", cx);
        assert!(!view.read(cx).ui.update_dialog_open);
    })
    .unwrap();
}
