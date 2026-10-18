use crate::gui::action::Action;
use crate::gui::view::{AppView, Command};
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{
    ActiveTheme, Icon, StyledExt,
    group_box::{GroupBox, GroupBoxVariants},
};
use gpui_kit::{prelude::*, *};

impl AppView {
    pub(in crate::gui::view) fn devices(&self, cx: &Context<Self>) -> AnyElement {
        let mut body = div().v_flex().gap_3();
        for row in &self.ui.paired_devices {
            let action = Action::try_from(row.action.as_str()).unwrap_or(Action::Device);
            let kind = match row.device_kind.as_str() {
                "tablet" => IconName::Tablet,
                "phone" => IconName::Smartphone,
                "computer" => IconName::Monitor,
                _ => IconName::Bluetooth,
            };
            let (status, color) = match row.status.as_str() {
                "ready" => (IconName::CircleCheck, cx.theme().success),
                "busy" => (IconName::Loader, cx.theme().muted_foreground),
                "failed" => (IconName::CircleAlert, cx.theme().danger),
                "linked" => (IconName::BluetoothConnected, cx.theme().info),
                "paired" => (IconName::Link2, cx.theme().muted_foreground),
                _ => (IconName::BluetoothOff, cx.theme().muted_foreground),
            };
            body = body.child(
                div()
                    .id(SharedString::from(format!("receiver-row-{}", row.id)))
                    .child(
                        GroupBox::new()
                            .id(SharedString::from(format!("receiver-{}", row.id)))
                            .fill()
                            .content_style(self.card_style(cx).h_flex().gap_3())
                            .child(Icon::new(kind).size_5().flex_shrink_0())
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .child(self.text(row.name.clone()).text_sm().text_ellipsis()),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!("receiver-status-{}", row.id)))
                                    .role(Role::Status)
                                    .aria_label(format!("{}: {}", row.name, row.detail))
                                    .h_flex()
                                    .justify_center()
                                    .size_8()
                                    .flex_shrink_0()
                                    .tooltip({
                                        let text = row.detail.clone();
                                        move |window, cx| {
                                            gpui_kit::component::tooltip::Tooltip::new(text.clone())
                                                .build(window, cx)
                                        }
                                    })
                                    .child(Icon::new(status).size(rems(1.125)).text_color(color)),
                            )
                            .child(
                                div()
                                    .id(SharedString::from(format!(
                                        "device-action-hint-{}",
                                        row.id
                                    )))
                                    .w(rems(8.25))
                                    .flex_shrink_0()
                                    .tooltip({
                                        let text = row.action_label.clone();
                                        move |window, cx| {
                                            gpui_kit::component::tooltip::Tooltip::new(text.clone())
                                                .build(window, cx)
                                        }
                                    })
                                    .child(
                                        self.button(
                                            SharedString::from(format!("device-{}", row.id)),
                                            if row.action.is_empty() {
                                                self.tr("receiver.connect")
                                            } else {
                                                row.action_label.clone()
                                            },
                                            Command::App(action, row.id.clone()),
                                            !row.enabled || row.action.is_empty(),
                                            cx,
                                        )
                                        .accessibility_label(format!(
                                            "{}: {}",
                                            if row.action.is_empty() {
                                                self.tr("receiver.connect")
                                            } else {
                                                row.action_label.clone()
                                            },
                                            row.name
                                        ))
                                        .w_full(),
                                    ),
                            ),
                    )
                    .test_support()
                    .map(|row_view| {
                        self.reveal_focus(
                            SharedString::from(format!("device-focus-{}", row.id)),
                            row_view,
                        )
                    }),
            );
        }
        if self.ui.paired_devices.is_empty() && !self.ui.scanning {
            body = body.child(self.caption(self.tr("receiver.empty"), cx));
        }
        self.scroller("devices", body)
    }
}
