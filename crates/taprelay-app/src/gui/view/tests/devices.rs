use super::support::{
    open, open_with_transport,
    receiver::{Call, Receiver},
    settle, wait_until,
};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, px};

#[gpui_kit::test]
fn receiver_loading_status_preserves_text_with_and_without_reduced_motion(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, _| view._pump = gpui_kit::Task::ready(()));
        for reduced_motion in [false, true] {
            cx.set_reduce_motion(reduced_motion);
            for scanning in [true, false] {
                view.update(cx, |view, cx| {
                    view.ui.page = 2;
                    view.ui.scanning = scanning;
                    view.ui.discovery_status = view.tr(if scanning {
                        "receiver.scanning"
                    } else {
                        "receiver.help"
                    });
                    cx.notify();
                });
                window.render_frame(cx);
                assert_eq!(view.read(cx).ui.scanning, scanning);
                assert_eq!(
                    window.find("page-description").label(),
                    Some(view.read(cx).ui.discovery_status.as_str())
                );
                assert!(window.find("page-description").visible());
                assert_eq!(
                    window.find("page-description").role(),
                    Some(gpui_kit::Role::Status)
                );
            }
        }
    })
    .unwrap();
}

#[gpui_kit::test]
fn receivers_use_group_box_insets_with_actions_in_the_header(cx: &mut TestAppContext) {
    let (handle, view, _folder) = open(cx);
    cx.update_window(handle, |_, window, cx| {
        view.update(cx, |view, cx| {
            view.ui.page = 2;
            view.ui.paired_devices = vec![crate::gui::presentation::DeviceRow {
                id: "tablet".into(),
                name: "Tablet with a long name 平板电脑".repeat(4),
                detail: "Ready".into(),
                device_kind: "tablet".into(),
                status: "ready".into(),
                action_label: "Disconnect".into(),
                enabled: true,
                ..Default::default()
            }];
            cx.notify();
        });
        window.render_frame(cx);
        let row = window.find("receiver-row-tablet").bounds();
        let action = window.find("device-tablet").bounds();
        assert!(row.size.height >= px(64.) && row.size.height <= px(88.));
        assert!((action.center().y - row.center().y).abs() <= px(0.5));
        assert_eq!(action.right(), row.right() - px(17.));
        assert_eq!(action.size.width, px(132.));
        assert_eq!(
            window.find("device-tablet").label(),
            Some(format!("Start session: {}", view.read(cx).ui.paired_devices[0].name).as_str())
        );
        assert!(row.contains(&action.center()));
        assert!(row.right() <= px(872.));
        let header = window.find("page-header").bounds();
        assert!(header.contains(&window.find("receiver-refresh").bounds().center()));
        assert!(header.contains(&window.find("receiver-settings").bounds().center()));
    })
    .unwrap();
}

#[gpui_kit::test]
fn device_controls_route_identity_reject_unavailable_and_disconnect(cx: &mut TestAppContext) {
    let receiver = Receiver::new();
    {
        let mut state = receiver.state.lock().unwrap();
        state.targets.push(taprelay_core::state::Target {
            id: "offline".into(),
            name: "Offline".into(),
            pairing: taprelay_core::state::Knowledge::Yes,
            availability: taprelay_core::devices::Availability::Unavailable,
            ..Default::default()
        });
    }
    let (handle, view, _folder) =
        open_with_transport(cx, 900., 500., Some(Box::new(receiver.clone())));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("0-3", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("device-offline", cx);
        assert!(receiver.calls.lock().unwrap().is_empty());
        assert!(view.read(cx).controller.runtime.state.selected.is_none());
        window.click("receiver-refresh", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.click("receiver-settings", cx);
        assert!(matches!(
            view.read(cx).controller.desktop,
            crate::gui::controller::Shell::Headless {
                bluetooth_settings_opened: true,
                ..
            }
        ));
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("device-tablet", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    assert_eq!(
        *receiver.calls.lock().unwrap(),
        [Call::Refresh, Call::Select("tablet".into())]
    );
    receiver.connected();
    wait_until(cx, handle, &view, |view| view.ui.ready);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(
            window.find("device-tablet").label(),
            Some("End session: Tablet")
        );
        assert_eq!(
            view.read(cx).controller.runtime.state.selected.as_deref(),
            Some("tablet")
        );
        window.click("device-tablet", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert!(!view.read(cx).ui.ready);
        assert_eq!(
            window.find("device-tablet").label(),
            Some("Start session: Tablet")
        );
    })
    .unwrap();
    settle(cx, handle, &view);
    assert_eq!(
        *receiver.calls.lock().unwrap(),
        [
            Call::Refresh,
            Call::Select("tablet".into()),
            Call::Disconnect,
        ]
    );
}
