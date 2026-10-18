use crate::gui::view::{AppView, Command};
use gpui_kit::assets::IconName;
use gpui_kit::component::{Icon, StyledExt, button::ButtonVariants, dialog::Dialog};
use gpui_kit::{prelude::*, *};

impl AppView {
    pub(in crate::gui::view) fn mapping_leave_dialog(
        &self,
        dialog: Dialog,
        cancel: Command,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Dialog {
        let busy = self.ui.mapping.saving;
        let footer = || {
            div()
                .h_flex()
                .justify_end()
                .gap_2()
                .child(self.button(
                    "dialog-cancel",
                    self.tr("common.keep_editing"),
                    cancel.clone(),
                    busy,
                    cx,
                ))
                .child(self.button(
                    "dialog-discard",
                    self.tr("common.discard_changes"),
                    Command::Mapping("discard", String::new()),
                    busy,
                    cx,
                ))
                .child(
                    self.button(
                        "dialog-save",
                        self.tr("common.save"),
                        Command::Mapping("save-leave", String::new()),
                        busy,
                        cx,
                    )
                    .primary()
                    .icon(
                        Icon::new(IconName::Save)
                            .transform(Transformation::scale(size(0.875, 0.875))),
                    ),
                )
        };
        let content = || {
            self.dialog_content(
                "mapping-leave-dialog",
                self.tr("common.save_changes"),
                self.text(self.tr("mappings.unsaved_help")),
                footer(),
            )
        };
        self.centered_dialog(dialog, content(), content(), window, cx)
    }
}
