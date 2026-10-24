use super::support::open;
use gpui_kit::component::WindowExt;
use gpui_kit::test::TestAppContextExt;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};
use std::time::Duration;

#[gpui_kit::test]
async fn feedback_notifications_deduplicate_dismiss_and_auto_hide(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view._pump = gpui_kit::Task::ready(());
            view.controller.say("Test sent".into());
            view.effects(window, cx);
        });
    })
    .unwrap();
    cx.wait_for(handle, Duration::from_secs(1), |window, _| {
        window.try_find("notification").is_some()
    })
    .await;
    let first = cx
        .update_window(handle, |_, window, cx| {
            assert_eq!(window.notifications(cx).len(), 1);
            assert_eq!(
                window.find("notification-message").role(),
                Some(gpui_kit::Role::Status)
            );
            assert_eq!(
                window.find("notification-message").label(),
                Some("Test sent")
            );
            let first = window.notifications(cx)[0].clone();
            view.update(cx, |view, cx| view.effects(window, cx));
            first
        })
        .unwrap();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(window.notifications(cx)[0], first);
        view.update(cx, |view, cx| {
            view.controller.say("Test sent".into());
            view.effects(window, cx);
        });
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(window.notifications(cx).len(), 1);
        assert_ne!(window.notifications(cx)[0], first);
        window.render_frame(cx);
    })
    .unwrap();
    // GPUI's notification entrance animation uses wall-clock time.
    std::thread::sleep(Duration::from_millis(410));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.hover("notification", cx);
        let notification = window.find("notification");
        assert_eq!(
            gpui_kit::base::test_support::snapshots(window)
                .iter()
                .filter(|element| {
                    element.path().starts_with(notification.path())
                        && element.role() == Some(gpui_kit::Role::Button)
                })
                .count(),
            1,
            "the notification must have exactly one close button, including on hover"
        );
        assert!(window.within("notification").find("close").visible());
        window.within("notification").click("close", cx);
    })
    .unwrap();
    cx.wait_for(handle, Duration::from_secs(1), |window, cx| {
        window.notifications(cx).is_empty()
    })
    .await;
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| view.effects(window, cx));
        assert!(window.notifications(cx).is_empty());
        window.hover("page-header", cx);
        view.update(cx, |view, cx| {
            view.controller.say("Test sent".into());
            view.effects(window, cx);
        });
    })
    .unwrap();
    cx.wait_for(handle, Duration::from_secs(1), |window, cx| {
        window.notifications(cx).len() == 1
    })
    .await;
    cx.wait_for(handle, Duration::from_secs(10), |window, cx| {
        window.notifications(cx).is_empty()
    })
    .await;
}

#[test]
fn notification_content_exposes_a_polite_live_region_in_both_languages() {
    use gpui_kit::{Element, Role, accesskit};
    for message in ["Connected to Tablet", "已连接到平板"] {
        let element = super::super::window::notification_message(message);
        let mut node = accesskit::Node::new(element.a11y_role().unwrap());
        element.write_a11y_info(&mut node);
        assert_eq!(node.role(), Role::Status);
        assert_eq!(node.label(), Some(message));
        assert_eq!(node.value(), Some(message));
        assert_eq!(node.live(), Some(accesskit::Live::Polite));
    }
}

#[gpui_kit::test]
fn passthrough_feedback_does_not_emit_window_or_system_notifications(cx: &mut TestAppContext) {
    use crate::runtime::passthrough::{Notice, Status};

    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, _, cx| {
        view.update(cx, |view, _| {
            view._pump = gpui_kit::Task::ready(());
            // Keep this test headless; disabling the capsule must not fall back
            // to either notification channel.
            view.controller.runtime.config.options.passthrough_overlay = false;
            view.controller.hidden = true;
            view.controller.runtime.config.options.notifications = true;
            let crate::gui::controller::Shell::Headless { foreground, .. } =
                &mut view.controller.desktop
            else {
                unreachable!()
            };
            *foreground = false;
            for status in [Status::Controlling, Status::Stopped, Status::Disconnected] {
                view.controller.passthrough_feedback(
                    Some(Notice {
                        status,
                        device: "Tablet".into(),
                    }),
                    &view.ui,
                );
                assert!(view.controller.pending_notification.is_none());
            }
            let crate::gui::controller::Shell::Headless { notifications, .. } =
                &view.controller.desktop
            else {
                unreachable!()
            };
            assert!(notifications.borrow().is_empty());
        });
    })
    .unwrap();
}

#[gpui_kit::test]
fn foreground_errors_keep_window_feedback_without_consuming_system_notification_cooldown(
    cx: &mut TestAppContext,
) {
    use crate::gui::controller::Shell;

    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view._pump = gpui_kit::Task::ready(());
            view.controller.runtime.config.options.notifications = true;
            view.controller.error("Foreground failure");
            assert_eq!(
                view.controller.pending_notification.as_deref(),
                Some(view.tr("error.operation").as_str())
            );
            let Shell::Headless {
                foreground,
                notifications,
                ..
            } = &mut view.controller.desktop
            else {
                unreachable!()
            };
            assert!(notifications.borrow().is_empty());
            *foreground = false;
            // The visible window has lost focus. A disabled preference must
            // also leave the cooldown available for a subsequent notification.
            assert!(!view.controller.hidden);
            view.controller.runtime.config.options.notifications = false;
            view.controller.notify_error("Disabled");
            view.controller.runtime.config.options.notifications = true;
            view.controller.notify_error("Background failure");
            view.controller.notify_error("Repeated failure");
            let Shell::Headless { notifications, .. } = &view.controller.desktop else {
                unreachable!()
            };
            assert_eq!(
                *notifications.borrow(),
                vec![("TapRelay".into(), "Background failure".into())]
            );
            view.effects(window, cx);
        });
    })
    .unwrap();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(window.notifications(cx).len(), 1);
    })
    .unwrap();
}

#[gpui_kit::test]
async fn progress_notifications_persist_and_clear_without_removing_feedback(
    cx: &mut TestAppContext,
) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view._pump = gpui_kit::Task::ready(());
            view.ui.progress_message = "Sending…".into();
            view.effects(window, cx);
        });
    })
    .unwrap();
    for _ in 0..60 {
        cx.background_executor
            .advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
    }
    let progress = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            assert_eq!(window.notifications(cx).len(), 1);
            assert!(window.find("notification").visible());
            let progress = window.notifications(cx)[0].clone();
            view.update(cx, |view, cx| {
                view.effects(window, cx);
                view.controller.say("Test sent".into());
                view.effects(window, cx);
            });
            progress
        })
        .unwrap();
    cx.update_window(handle, |_, window, cx| {
        assert_eq!(window.notifications(cx).len(), 2);
        assert!(window.notifications(cx).contains(&progress));
        view.update(cx, |view, cx| {
            view.controller.sync(&mut view.ui);
            assert!(view.ui.progress_message.is_empty());
            view.effects(window, cx);
        });
    })
    .unwrap();
    cx.wait_for(handle, Duration::from_secs(1), |window, cx| {
        let notifications = window.notifications(cx);
        notifications.len() == 1 && !notifications.contains(&progress)
    })
    .await;
}
