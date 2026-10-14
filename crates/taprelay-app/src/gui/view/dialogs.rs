use super::{AppView, Command};
use gpui_kit::component::WindowExt;
use gpui_kit::{prelude::*, *};

impl AppView {
    pub(super) fn update_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = if self.ui.mapping.leave_open {
            1
        } else if self.ui.groups.confirming_delete {
            2
        } else {
            0
        };
        if next == self.dialog {
            return;
        }
        if self.dialog != 0 {
            window.close_dialog(cx);
        }
        self.dialog = next;
        if next == 0 {
            return;
        }
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, cx| {
            let Some(entity) = weak.upgrade() else {
                return dialog;
            };
            entity.update(cx, |this, cx| {
                let (busy, cancel) = if next == 1 {
                    (
                        this.ui.mapping.saving,
                        Command::Mapping("stay", String::new()),
                    )
                } else {
                    (
                        this.ui.groups.saving,
                        Command::Group("cancel", String::new()),
                    )
                };
                let dialog = if next == 1 {
                    this.mapping_leave_dialog(dialog, cancel.clone(), cx)
                } else {
                    this.group_delete_dialog(dialog, cancel.clone(), cx)
                };
                let weak = cx.entity().downgrade();
                dialog
                    .overlay_closable(false)
                    .close_button(!busy)
                    .on_cancel(move |_, _, cx| {
                        if busy {
                            return false;
                        }
                        let _ = weak.update(cx, |this, cx| {
                            match &cancel {
                                Command::Mapping(command, _) => {
                                    let _ =
                                        this.controller.mapping_command(&mut this.ui, command, "");
                                }
                                Command::Group(command, _) => {
                                    let _ =
                                        this.controller.group_command(&mut this.ui, command, "");
                                }
                                _ => {}
                            }
                            this.dialog = 0;
                            cx.notify();
                        });
                        true
                    })
            })
        });
    }
}
