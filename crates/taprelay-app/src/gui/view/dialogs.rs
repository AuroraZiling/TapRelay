use super::{AppView, Command};
use crate::gui::{presentation::updates::UpdateStatus, typography};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::WindowExt;
use gpui_kit::component::{ActiveTheme, StyledExt, dialog::Dialog, window_paddings};
use gpui_kit::{prelude::*, *};

impl AppView {
    pub(super) fn centered_dialog(
        &self,
        dialog: Dialog,
        width_rems: f32,
        content: AnyElement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Dialog {
        let padding = window_paddings(window);
        let viewport = window.viewport_size()
            - size(padding.left + padding.right, padding.top + padding.bottom);
        let width = (window.rem_size() * width_rems)
            .min(viewport.width - cx.theme().spacing_tokens().lg * 2.);
        dialog.centered().w(width).p_4().child(content)
    }

    pub(super) fn dialog_content(
        &self,
        id: &'static str,
        title: String,
        body: impl IntoElement,
        footer: impl IntoElement,
    ) -> AnyElement {
        // The styled Dialog does not connect its title to its accessible name.
        div()
            .id(id)
            .role(Role::Dialog)
            .aria_label(title.clone())
            .v_flex()
            .gap_4()
            .min_w_0()
            .child(
                self.text(title)
                    .role(Role::Heading)
                    .aria_level(2)
                    .text_size(typography::TITLE_SMALL)
                    .font_semibold(),
            )
            .child(body)
            .child(footer)
            .test_support()
            .into_any_element()
    }

    pub(super) fn update_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let next = if self.ui.mapping.leave_open {
            1
        } else if self.ui.groups.confirming_delete {
            2
        } else if self.ui.update_dialog_open
            && self.ui.update_offer.is_some()
            && !self.controller.hidden
            && (self.ui.mode == 2
                || self.ui.update_status.committing()
                || matches!(
                    self.ui.update_status,
                    UpdateStatus::DownloadFailed | UpdateStatus::ApplyFailed
                ))
            && !self.ui.mapping.editor
            && !self.ui.groups.editor_open
            && !self.controller.recording()
        {
            3
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
        if next == 3
            && self
                .ui
                .update_offer
                .as_ref()
                .is_some_and(|offer| offer.show_notes)
        {
            let _ = self.controller.action(
                &mut self.ui,
                crate::gui::action::Action::LoadUpdateNotes,
                "",
            );
        }
        let weak = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, window, cx| {
            let Some(entity) = weak.upgrade() else {
                return dialog;
            };
            entity.update(cx, |this, cx| {
                if next == 3 {
                    return this.release_dialog(dialog, window, cx);
                }
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
                    this.mapping_leave_dialog(dialog, cancel.clone(), window, cx)
                } else {
                    this.group_delete_dialog(dialog, cancel.clone(), window, cx)
                };
                let weak = cx.entity().downgrade();
                dialog
                    .overlay_closable(false)
                    .close_button(false)
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
