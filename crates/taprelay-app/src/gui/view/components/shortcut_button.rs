use gpui_kit::component::{
    Sizable, StyledExt,
    button::{Button, ButtonVariants},
};
use gpui_kit::{prelude::*, *};

#[derive(IntoElement)]
pub(in crate::gui::view) struct ShortcutButton {
    button: Button,
    clear_button: Option<Button>,
}

impl ShortcutButton {
    pub(in crate::gui::view) fn new(button: Button) -> Self {
        Self {
            button,
            clear_button: None,
        }
    }

    pub(in crate::gui::view) fn clear_button(mut self, button: Button) -> Self {
        self.clear_button = Some(button);
        self
    }
}

impl RenderOnce for ShortcutButton {
    fn render(self, _: &mut Window, _: &mut App) -> impl IntoElement {
        div()
            .relative()
            .h_flex()
            .max_w_full()
            .child(
                self.button
                    .max_w_full()
                    .when(self.clear_button.is_some(), |button| button.pr_7()),
            )
            .when_some(self.clear_button, |this, button| {
                this.child(
                    div()
                        .absolute()
                        .block_mouse_except_scroll()
                        .right_0p5()
                        .top_0()
                        .h_full()
                        .h_flex()
                        .child(button.ghost().xsmall()),
                )
            })
    }
}
