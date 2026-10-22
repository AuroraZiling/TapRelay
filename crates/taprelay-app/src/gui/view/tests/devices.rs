use super::support::{
    open_with_transport,
    receiver::{Call, Receiver},
    settle, wait_until,
};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};

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
        assert!(
            window
                .find("device-tablet")
                .label()
                .unwrap()
                .contains("Tablet")
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
        assert!(
            window
                .find("device-tablet")
                .label()
                .unwrap()
                .contains("Tablet")
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
