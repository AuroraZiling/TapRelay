use crate::action::Action;
use crate::gui::view::{AppView, Command};
use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme, Icon, StyledExt, button::Button};
use gpui_kit::{prelude::*, *};

impl AppView {
    pub(in crate::gui::view) fn bluetooth_status(&self, cx: &App) -> Div {
        div()
            .h_flex()
            .gap(rems(0.375))
            .h_8()
            .px_2()
            .rounded_md()
            .border_2()
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
                .size_4(),
            )
            .child(div().child(self.ui.adapter_label.clone()))
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
            ),
            self.icon_button(
                "receiver-settings",
                IconName::Settings,
                self.tr("receiver.open.settings"),
                Command::App(Action::BluetoothSettings, String::new()),
                false,
                cx,
            ),
        ]
    }
}
