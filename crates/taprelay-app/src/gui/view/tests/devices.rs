use super::support::{
    open_with_transport,
    receiver::{Call, Receiver},
    settle, wait_until,
};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext};

#[gpui_kit::test]
fn list_scrollbars_stay_aligned_with_the_viewport_while_content_scrolls(cx: &mut TestAppContext) {
    use crate::gui::presentation::DeviceRow;
    use gpui_kit::{ScrollDelta, Task, VisualTestContext, point, px};

    let (handle, view, _folder) = super::support::open_sized(cx, 900., 500.);
    settle(cx, handle, &view);
    let mut cx = VisualTestContext::from_window(handle, cx);
    cx.update(|_, cx| {
        cx.set_reduce_motion(true);
        view.update(cx, |view, _| {
            view._pump = Task::ready(());
            view.ui.paired_devices = (0..20)
                .map(|index| DeviceRow {
                    id: index.to_string(),
                    name: format!("Tablet {index}"),
                    ..Default::default()
                })
                .collect();
        });
    });
    for (page, region) in [(2, "devices"), (3, "settings")] {
        let viewport = cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.ui.page = page;
                cx.notify();
            });
            window.render_frame(cx);
            window.find(region).bounds()
        });
        for delta in [-120., -10000., 10000.] {
            cx.update(|window, cx| {
                window.scroll(region, ScrollDelta::Pixels(point(px(0.), px(delta))), cx);
                assert_eq!(window.find(region).bounds(), viewport);
                assert!(window.find("page-header").bounds().bottom() <= viewport.top());
                let offset = view.read(cx).content_scroll.offset().y;
                assert!(if delta < 0. {
                    offset < px(0.)
                } else {
                    offset == px(0.)
                });
            });
            assert_eq!(
                cx.debug_bounds("scrollbar-overlay"),
                Some(viewport),
                "{region}"
            );
        }
    }
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
