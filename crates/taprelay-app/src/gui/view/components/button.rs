use crate::gui::action::Action;
use crate::gui::view::{AppView, Command};
use crate::runtime::commands::BindingCommand;
use gpui_kit::assets::IconName;
use gpui_kit::component::{Disableable, button::Button};
use gpui_kit::{prelude::*, *};

impl AppView {
    pub(in crate::gui::view) fn button_text(&self, text: impl Into<SharedString>) -> Div {
        // GPUI Kit 0.7.3 overrides the root font size for built-in button labels.
        div()
            .min_w_0()
            .whitespace_nowrap()
            .text_ellipsis()
            .text_size(rems(0.8125))
            .child(text.into())
    }

    pub(in crate::gui::view) fn button(
        &self,
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        command: Command,
        disabled: bool,
        cx: &Context<Self>,
    ) -> Button {
        let label = label.into();
        self.command_button(id, command, disabled, cx)
            .accessibility_label(label.clone())
            .child(self.button_text(label))
    }

    pub(in crate::gui::view) fn app_button(
        &self,
        id: &'static str,
        key: &str,
        action: Action,
        disabled: bool,
        cx: &Context<Self>,
    ) -> Button {
        self.button(
            id,
            self.tr(key),
            Command::App(action, String::new()),
            disabled,
            cx,
        )
    }

    pub(in crate::gui::view) fn icon_button(
        &self,
        id: impl Into<ElementId>,
        icon: IconName,
        label: String,
        command: Command,
        disabled: bool,
        cx: &Context<Self>,
    ) -> Button {
        self.plain_icon_button(id, icon, label.clone(), command, disabled, cx)
            .tooltip(label)
    }

    pub(in crate::gui::view) fn plain_icon_button(
        &self,
        id: impl Into<ElementId>,
        icon: IconName,
        label: String,
        command: Command,
        disabled: bool,
        cx: &Context<Self>,
    ) -> Button {
        self.command_button(id, command, disabled, cx)
            .icon(icon)
            .accessibility_label(label)
    }

    fn command_button(
        &self,
        id: impl Into<ElementId>,
        command: Command,
        disabled: bool,
        cx: &Context<Self>,
    ) -> Button {
        let cancel_capture = matches!(
            command,
            Command::Binding(BindingCommand::CancelCapture) | Command::Mapping("cancel-capture", _)
        );
        Button::new(id)
            .disabled(disabled)
            .when(cancel_capture && !disabled, |button| {
                // Native mouse capture commits on release, before a button's click callback.
                button.on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        this.dispatch(Command::Binding(BindingCommand::CancelCapture), window, cx);
                        cx.stop_propagation();
                    }),
                )
            })
            .on_click(
                cx.listener(move |this, _, window, cx| this.dispatch(command.clone(), window, cx)),
            )
    }
}
