use crate::gui::action::Action;
use crate::gui::view::{AppView, Command};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Icon, StyledExt,
    button::{Button, ButtonVariants},
    shimmer::ShimmerText,
    tag::Tag,
};
use gpui_kit::{prelude::*, *};

impl AppView {
    pub(in crate::gui::view) fn bluetooth_status(&self) -> Tag {
        let tag = if self.ui.bluetooth_available {
            Tag::primary()
        } else {
            Tag::danger()
        };
        tag.child(
            div()
                .h_flex()
                .gap_1()
                .child(
                    Icon::new(if self.ui.bluetooth_available {
                        IconName::Bluetooth
                    } else {
                        IconName::BluetoothOff
                    })
                    .size_3(),
                )
                .child(self.text(self.ui.adapter_label.clone())),
        )
    }

    pub(in crate::gui::view) fn receiver_description(&self, cx: &App) -> Stateful<Div> {
        div()
            .id("receiver-discovery-status")
            .role(Role::Status)
            .aria_label(self.ui.discovery_status.clone())
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .map(|description| {
                if self.ui.scanning {
                    description.child(
                        ShimmerText::new(self.ui.discovery_status.clone())
                            .id("receiver-discovery-shimmer"),
                    )
                } else {
                    description.child(self.ui.discovery_status.clone())
                }
            })
    }

    pub(in crate::gui::view) fn receiver_actions(&self, cx: &Context<Self>) -> [Button; 2] {
        [
            self.icon_button(
                "receiver-refresh",
                IconName::RefreshCw,
                self.tr("receiver.refresh"),
                Command::App(Action::Refresh, String::new()),
                self.ui.scanning,
                cx,
            )
            .ghost(),
            self.icon_button(
                "receiver-settings",
                IconName::Settings,
                self.tr("receiver.open.settings"),
                Command::App(Action::BluetoothSettings, String::new()),
                false,
                cx,
            )
            .ghost(),
        ]
    }
}
