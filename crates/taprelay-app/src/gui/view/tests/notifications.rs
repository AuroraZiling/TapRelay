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
