use crate::gui::view::{AppView, Command};
use gpui_kit::component::{StyledExt, dialog::Dialog};
use gpui_kit::{prelude::*, *};

impl AppView {
    pub(in crate::gui::view) fn group_delete_dialog(
        &self,
        dialog: Dialog,
        cancel: Command,
        cx: &Context<Self>,
    ) -> Dialog {
        let busy = self.ui.groups.saving;
        let footer = div()
            .h_flex()
            .justify_end()
            .gap_2()
            .child(self.button("dialog-cancel", self.tr("common.cancel"), cancel, busy, cx))
            .child(self.button(
                "dialog-delete",
                self.tr(if busy {
                    "common.saving"
                } else {
                    "groups.delete_confirm"
                }),
                Command::Group("delete", String::new()),
                busy,
                cx,
            ));
        dialog
            .title(format!(
                "{} — {}",
                self.tr("groups.delete"),
                self.ui.groups.selected_name
            ))
            .child(self.ui.groups.deletion_warning.clone())
            .child(self.error(&self.ui.groups.form_error, cx))
            .footer(footer)
    }
}
