use crate::gui::view::{AppView, Command};
use gpui_kit::component::{StyledExt, dialog::Dialog};
use gpui_kit::{prelude::*, *};

impl AppView {
    pub(in crate::gui::view) fn mapping_leave_dialog(
        &self,
        dialog: Dialog,
        cancel: Command,
        cx: &Context<Self>,
    ) -> Dialog {
        let busy = self.ui.mapping.saving;
        let footer = div()
            .h_flex()
            .justify_end()
            .gap_2()
            .child(self.button(
                "dialog-cancel",
                self.tr("common.keep_editing"),
                cancel,
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
            .child(self.button(
                "dialog-save",
                self.tr("mappings.save"),
                Command::Mapping("save-leave", String::new()),
                busy,
                cx,
            ));
        dialog
            .title(self.tr("common.save_changes"))
            .child(self.tr("mappings.unsaved_help"))
            .child(self.error(&self.ui.mapping.error, cx))
            .footer(footer)
    }
}
