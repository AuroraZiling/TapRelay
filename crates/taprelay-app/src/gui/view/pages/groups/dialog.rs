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
        let title = format!(
            "{} — {}",
            self.tr("groups.delete"),
            self.ui.groups.selected_name
        );
        dialog.close_button(false).child(
            self.dialog_content(
                "group-delete-dialog",
                title,
                div()
                    .v_flex()
                    .gap_3()
                    .child(self.text(self.ui.groups.deletion_warning.clone()))
                    .when(!self.ui.groups.form_error.is_empty(), |body| {
                        body.child(self.error(&self.ui.groups.form_error, cx))
                    }),
                footer,
            ),
        )
    }
}
