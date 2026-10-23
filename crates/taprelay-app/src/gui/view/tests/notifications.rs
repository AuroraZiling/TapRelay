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
        window.hover("page-header", cx);
        assert!(window.find("notification-dismiss").visible());
        assert_eq!(
            window.find("notification-dismiss").label(),
            Some("Dismiss notification")
        );
        view.read(cx).focus.clone().focus(window, cx);
        for _ in 0..25 {
            if window.find("notification-dismiss").focused() == Some(true) {
                break;
            }
            window.press("tab", cx);
        }
        assert_eq!(window.find("notification-dismiss").focused(), Some(true));
        window.press("enter", cx);
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
fn passthrough_feedback_preserves_device_and_respects_system_notification_setting(
    cx: &mut TestAppContext,
) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, _, cx| {
        view.update(cx, |view, _| {
            view._pump = gpui_kit::Task::ready(());
            view.controller.hidden = true;
            for (language, expected) in [
                ("en", "Disconnected from Tablet"),
                ("zh-CN", "已断开与 Tablet 的连接"),
            ] {
                view.controller.runtime.config.options.language =
                    crate::config::Language::from_name(language).unwrap();
                for enabled in [true, false] {
                    view.controller.runtime.config.options.notifications = enabled;
                    view.controller
                        .passthrough_feedback(&crate::runtime::passthrough::Notice {
                            status: crate::runtime::passthrough::Status::Disconnected,
                            device: "Tablet".into(),
                        });
                    assert_eq!(
                        view.controller.pending_notification.as_deref(),
                        Some(expected)
                    );
                    let crate::gui::controller::Shell::Headless { notifications, .. } =
                        &view.controller.desktop
                    else {
                        unreachable!()
                    };
                    assert_eq!(
                        *notifications.borrow(),
                        if enabled {
                            vec![("TapRelay".into(), expected.into())]
                        } else {
                            Vec::new()
                        }
                    );
                    notifications.borrow_mut().clear();
                }
            }
        });
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
