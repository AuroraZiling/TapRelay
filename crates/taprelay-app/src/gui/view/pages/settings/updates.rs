use crate::gui::{
    action::Action,
    presentation::updates::{UpdateOffer, UpdateStatus},
    typography,
    view::AppView,
};
use crate::updates::NoteKind;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{
    Disableable, Sizable, StyledExt, button::ButtonVariants, dialog::Dialog, progress::Progress,
    scroll::ScrollableElement,
};
use gpui_kit::{prelude::*, *};

impl AppView {
    fn update_status_text(&self) -> String {
        let key = match self.ui.update_status {
            UpdateStatus::Idle => "updates.title",
            UpdateStatus::Checking => "updates.checking",
            UpdateStatus::Current => "updates.current",
            UpdateStatus::Available => "updates.available",
            UpdateStatus::CheckFailed => "updates.check_failed",
            UpdateStatus::DownloadFailed => "updates.download_failed",
            UpdateStatus::ApplyFailed => "updates.apply_failed",
            UpdateStatus::Unsupported => "updates.unsupported",
            UpdateStatus::Applying => "updates.applying",
            UpdateStatus::Downloading(value) => {
                return self.tr_object("updates.downloading", &value.to_string());
            }
        };
        if self.ui.update_status == UpdateStatus::Available
            && let Some(offer) = &self.ui.update_offer
        {
            self.tr_object(key, &offer.tag)
        } else {
            self.tr(key)
        }
    }

    pub(super) fn update_settings(&self, cx: &Context<Self>) -> impl IntoElement {
        let status = self.ui.update_status;
        let unsupported = status == UpdateStatus::Unsupported;
        div()
            .v_flex()
            .gap_3()
            .child(
                self.setting_row(
                    "about-updates",
                    div().child(
                        self.text(self.update_status_text())
                            .id("update-status")
                            .role(Role::Status)
                            .test_support(),
                    ),
                    self.app_button(
                        "check-updates",
                        "updates.check",
                        Action::CheckUpdates,
                        unsupported || status == UpdateStatus::Checking || status.committing(),
                        cx,
                    ),
                ),
            )
            .child(
                self.setting_row(
                    "auto-check-updates",
                    self.setting_text("updates.auto_check", "updates.frequency", cx),
                    self.setting_switch(
                        "auto-check-updates",
                        "updates.auto_check",
                        self.ui.auto_check_updates,
                        Action::SetAutoCheckUpdates,
                        cx,
                    )
                    .disabled(unsupported),
                ),
            )
    }

    fn release_notes(&self, offer: &UpdateOffer, window: &Window, cx: &Context<Self>) -> Div {
        if !offer.show_notes {
            return div().child(self.app_button(
                "show-update-notes",
                "updates.show_notes",
                Action::LoadUpdateNotes,
                false,
                cx,
            ));
        }
        let notes = offer
            .notes
            .as_ref()
            .map(|notes| {
                if self.ui.locale.starts_with("zh") {
                    &notes.zh
                } else {
                    &notes.en
                }
            })
            .filter(|notes| !notes.blocks.is_empty());
        let mut body = div().v_flex().min_w_0().gap_2();
        if let Some(notes) = notes {
            if notes.fallback {
                body = body.child(self.caption(self.tr("updates.notes_original"), cx));
            }
            let blocks = div()
                .v_flex()
                .gap_2()
                .children(notes.blocks.iter().enumerate().map(|(ix, block)| {
                    let label = self
                        .text(block.text.clone())
                        .id(("release-note", ix))
                        .test_support()
                        .min_w_0();
                    match block.kind {
                        NoteKind::Paragraph => label.into_any_element(),
                        NoteKind::Bullet => div()
                            .h_flex()
                            .items_start()
                            .gap_2()
                            .child("•")
                            .child(label.flex_1())
                            .into_any_element(),
                        NoteKind::Added
                        | NoteKind::Improved
                        | NoteKind::Fixed
                        | NoteKind::Heading => label
                            .role(Role::Heading)
                            .aria_level(3)
                            .font_semibold()
                            .when(ix > 0, |label| label.mt_1())
                            .into_any_element(),
                    }
                }));
            body = body.child(
                div()
                    .id("release-notes")
                    .max_h(window.viewport_size().height * 0.43)
                    .child(blocks.pr_3())
                    .overflow_y_scrollbar(),
            );
        } else if offer.notes_loading {
            body = body.child(
                self.caption(self.tr("updates.notes_loading"), cx)
                    .id("release-notes-loading")
                    .role(Role::Status)
                    .test_support(),
            );
        } else {
            body = body.child(
                self.app_button(
                    "update-release-link",
                    "updates.release_link",
                    Action::UpdateReleaseNotes,
                    false,
                    cx,
                )
                .link()
                .role(Role::Link)
                .cursor_pointer()
                .self_start(),
            );
        }
        body
    }

    pub(in crate::gui::view) fn release_dialog(
        &self,
        dialog: Dialog,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Dialog {
        let Some(offer) = &self.ui.update_offer else {
            return dialog;
        };
        let status = self.ui.update_status;
        let mut body = self.release_notes(offer, window, cx);
        if let UpdateStatus::Downloading(value) = status {
            body = body.child(
                div()
                    .mt_2()
                    .v_flex()
                    .gap_1()
                    .child(
                        self.text(self.update_status_text())
                            .id("update-progress-label")
                            .role(Role::Status)
                            .test_support(),
                    )
                    .child(
                        Progress::new("update-progress")
                            .small()
                            .value(f32::from(value))
                            .accessibility_label(self.tr("updates.download_progress")),
                    ),
            );
        } else if status == UpdateStatus::Applying {
            body = body.child(self.text(self.update_status_text()).role(Role::Status));
        } else if matches!(
            status,
            UpdateStatus::ApplyFailed | UpdateStatus::DownloadFailed
        ) {
            body = body.child(self.error(&self.update_status_text(), cx));
        }
        let mut footer = div().h_flex().gap_2().justify_between().flex_wrap();
        if !status.committing() {
            let accept = if offer.portable {
                "updates.portable"
            } else {
                "updates.install"
            };
            footer = footer
                .child(
                    self.app_button("update-skip", "updates.skip", Action::SkipUpdate, false, cx)
                        .ghost(),
                )
                .child(
                    div()
                        .h_flex()
                        .gap_2()
                        .justify_end()
                        .flex_wrap()
                        .child(self.app_button(
                            "update-later",
                            "updates.later",
                            Action::RemindUpdateLater,
                            false,
                            cx,
                        ))
                        .child(
                            self.app_button(
                                "update-accept",
                                accept,
                                Action::AcceptUpdate,
                                false,
                                cx,
                            )
                            .primary(),
                        ),
                );
        }
        let title = self.tr_object("updates.dialog_title", &offer.tag);
        let content = self.dialog_content(
            "release-dialog",
            title,
            body.text_size(typography::BODY_SMALL),
            footer,
        );
        let weak = cx.entity().downgrade();
        self.centered_dialog(dialog, 35., content, window, cx)
            .overlay_closable(false)
            .close_button(false)
            .on_ok(|_, _, _| false)
            .on_cancel(move |_, _, cx| {
                weak.update(cx, |this, cx| {
                    if this.ui.update_status.committing() {
                        return false;
                    }
                    let _ = this
                        .controller
                        .action(&mut this.ui, Action::RemindUpdateLater, "");
                    this.dialog = 0;
                    cx.notify();
                    true
                })
                .unwrap_or(true)
            })
    }
}
