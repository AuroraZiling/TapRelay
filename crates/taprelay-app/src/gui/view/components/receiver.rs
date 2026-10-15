use crate::gui::action::Action;
use crate::gui::view::{AppView, Command};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Icon, StyledExt,
    button::{Button, ButtonVariants},
};
use gpui_kit::{prelude::*, *};

impl AppView {
    pub(in crate::gui::view) fn bluetooth_status(&self, cx: &App) -> Div {
        div()
            .h_flex()
            .gap_1()
            .h_6()
            .px_2()
            .text_sm()
            .rounded_md()
            .border_1()
            .border_color(if self.ui.bluetooth_available {
                cx.theme().success
            } else {
                cx.theme().danger
            })
            .child(
                Icon::new(if self.ui.bluetooth_available {
                    IconName::Bluetooth
                } else {
                    IconName::BluetoothOff
                })
                .size(rems(0.875)),
            )
            .child(self.text(self.ui.adapter_label.clone()))
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
