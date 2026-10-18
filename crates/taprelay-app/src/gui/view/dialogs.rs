use super::{AppView, Command};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::WindowExt;
use gpui_kit::component::{ActiveTheme, StyledExt, dialog::Dialog, window_paddings};
use gpui_kit::{prelude::*, *};

impl AppView {
    pub(super) fn centered_dialog(
        &self,
        dialog: Dialog,
        content: AnyElement,
        measurement: AnyElement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Dialog {
        let padding = window_paddings(window);
        let viewport = window.viewport_size()
            - size(padding.left + padding.right, padding.top + padding.bottom);
        let width =
            (window.rem_size() * 28.).min(viewport.width - cx.theme().spacing_tokens().lg * 2.);
        // GPUI Kit 0.7.3 only exposes a top offset, so measure the content before centering.
        let height = div()
            .w(width)
            .border_1()
            .p_4()
            .child(measurement)
            .into_any_element()
            .layout_as_root(
                size(AvailableSpace::Definite(width), AvailableSpace::MinContent),
                window,
                &mut *cx,
            );
        dialog
            .w(width)
            .p_4()
            .margin_top((viewport.height - height.height) / 2.)
            .child(content)
    }

    pub(super) fn dialog_content(
        &self,
        id: &'static str,
        title: String,
        body: impl IntoElement,
        footer: impl IntoElement,
    ) -> AnyElement {
        // GPUI Kit 0.7.3 does not connect Dialog::title to the dialog's accessible name.
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
                    .text_lg()
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
        window.open_dialog(cx, move |dialog, window, cx| {
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
