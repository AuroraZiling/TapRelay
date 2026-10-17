use crate::gui::view::{AppView, Command};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    ActiveTheme, Icon, StyledExt, button::ButtonVariants, dialog::Dialog, window_paddings,
};
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
        let padding = window_paddings(window);
        let viewport = window.viewport_size()
            - size(padding.left + padding.right, padding.top + padding.bottom);
        let width = px(448.).min(viewport.width - cx.theme().spacing_tokens().lg * 2.);
        let dialog_content = content();
        // GPUI Kit 0.7.3 only exposes a top offset, so measure the content before centering.
        let height = div()
            .w(width)
            .border_1()
            .p_4()
            .child(content())
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
            .close_button(false)
            .child(dialog_content)
    }
}
