use crate::action::Action;
use crate::gui::view::{AppView, Command};
use gpui_kit::assets::IconName;
use gpui_kit::component::{Disableable, button::Button};
use gpui_kit::{prelude::*, *};

impl AppView {
    pub(in crate::gui::view) fn button(
        &self,
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        command: Command,
        disabled: bool,
        cx: &Context<Self>,
    ) -> Button {
        Button::new(id).label(label).disabled(disabled).on_click(
            cx.listener(move |this, _, window, cx| this.dispatch(command.clone(), window, cx)),
        )
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
        Button::new(id)
            .icon(icon)
            .accessibility_label(label.clone())
            .tooltip(label)
            .disabled(disabled)
            .on_click(
                cx.listener(move |this, _, window, cx| this.dispatch(command.clone(), window, cx)),
            )
    }
}
