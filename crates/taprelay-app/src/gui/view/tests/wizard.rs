use super::support::{
    open_with_transport,
    receiver::{Call, Receiver},
    settle, wait_until,
};
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, ScrollDelta, TestAppContext, point, px};

#[gpui_kit::test]
fn wizard_requires_a_ready_receiver_and_persists_completion(cx: &mut TestAppContext) {
    let receiver = Receiver::new();
    let (handle, view, folder) =
        open_with_transport(cx, 900., 1800., Some(Box::new(receiver.clone())));
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click(("nav", 3usize), cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.scroll(
            "settings",
            ScrollDelta::Pixels(point(px(0.), px(-2000.))),
            cx,
        );
        window.click("wizard", cx);
        assert!(window.find("binding-enabled-media.play-pause").visible());
        assert_eq!(view.read(cx).ui.wizard_page, 0);
        window.click("wizard-back", cx);
        assert_eq!(view.read(cx).ui.wizard_page, 0);
        window.click("wizard-next", cx);
        assert!(window.find("receiver-refresh").visible());
        assert_eq!(view.read(cx).ui.wizard_page, 2);
        window.click("wizard-next", cx);
        assert_eq!(view.read(cx).ui.wizard_page, 2);
        window.click("wizard-back", cx);
        assert_eq!(view.read(cx).ui.wizard_page, 0);
        window.click("wizard-next", cx);
        window.click("device-tablet", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("wizard-next", cx);
        assert_eq!(view.read(cx).ui.wizard_page, 2);
    })
    .unwrap();
    receiver.connected();
    wait_until(cx, handle, &view, |view| view.ui.receiver_next_allowed);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("wizard-next", cx);
        assert_eq!(view.read(cx).ui.wizard_page, 3);
        assert!(window.find("test-receiver").visible());
        assert_eq!(window.find("wizard-next").label(), Some("Finish"));
        window.click("test-receiver", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    assert_eq!(
        *receiver.calls.lock().unwrap(),
        [Call::Select("tablet".into()), Call::Send]
    );
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        window.click("wizard-next", cx);
    })
    .unwrap();
    settle(cx, handle, &view);
    cx.update_window(handle, |_, window, cx| {
        window.render_frame(cx);
        assert_eq!(view.read(cx).ui.mode, 2);
        assert!(window.find("status-card").visible());
        assert!(window.try_find("wizard-next").is_none());
    })
    .unwrap();
    let saved = crate::config::Config::load(&folder.path().join("config.json")).unwrap();
    assert!(saved.wizard.dismissed);
    assert_eq!(saved.wizard.page, 3);
}
