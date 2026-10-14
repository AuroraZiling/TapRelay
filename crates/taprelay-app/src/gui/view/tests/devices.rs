use super::support::open;
use gpui_kit::test::TestWindowExt;
use gpui_kit::{AppContext, TestAppContext, px};

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
        assert_eq!(row.size.height, px(64.));
        assert_eq!(action.top(), row.top() + px(16.));
        assert_eq!(action.bottom(), row.bottom() - px(16.));
        assert_eq!(action.right(), row.right() - px(16.));
        assert_eq!(action.size.width, px(132.));
        assert_eq!(window.find("device-tablet").label(), Some("Start session"));
        assert!(row.contains(&action.center()));
        assert!(row.right() <= px(872.));
        let header = window.find("page-header").bounds();
        assert!(header.contains(&window.find("receiver-refresh").bounds().center()));
        assert!(header.contains(&window.find("receiver-settings").bounds().center()));
    })
    .unwrap();
}
