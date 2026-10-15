use crate::gui::view::{AppView, Command};
use crate::gui::{BindingCommand, CaptureTarget, FunctionId};
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Selectable, StyledExt,
    group_box::{GroupBox, GroupBoxVariants},
    switch::Switch,
};
use gpui_kit::{prelude::*, *};

impl AppView {
    pub(in crate::gui::view) fn bindings(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let wide = window.viewport_size().width >= px(1100.);
        let scope_width = rems(if wide { 16. } else { 10. });
        let mut page = div()
            .v_flex()
            .flex_1()
            .min_h_0()
            .gap_6()
            .child(self.caption(self.tr("bindings.help"), cx));
        if !self.ui.groups.error.is_empty() {
            page = page.child(self.error(&self.ui.groups.error, cx));
        }
        let mut rows = div().v_flex().gap_2().child(
            div()
                .h_flex()
                .h_7()
                .px_4()
                .gap_4()
                .child(
                    self.caption(self.tr("common.enabled"), cx)
                        .w_9()
                        .flex_shrink_0(),
                )
                .child(
                    self.caption(self.tr("common.shortcut"), cx)
                        .flex_1()
                        .min_w_0(),
                )
                .child(
                    self.caption(self.tr("common.function"), cx)
                        .w(rems(15.))
                        .flex_shrink_0(),
                )
                .child(
                    self.caption(self.tr("common.effective_scope"), cx)
                        .w(scope_width)
                        .flex_shrink_0(),
                ),
        );
        let busy = self.controller.recording() || self.ui.groups.busy;
        for row in &self.ui.binding.rows {
            let Some(id) = FunctionId::from_stable_id(&row.id) else {
                continue;
            };
            let capturing = self.ui.binding.capture.function_id == row.id;
            let mut shortcuts = div().h_flex().gap_2().flex_wrap();
            for shortcut in &row.shortcuts {
                let target = CaptureTarget {
                    id,
                    slot: shortcut.slot as usize,
                };
                let recording = capturing && self.ui.binding.capture.slot == shortcut.slot;
                let label = if recording {
                    self.ui.binding.capture.text.clone()
                } else {
                    shortcut.text.clone()
                };
                let mut chip = div()
                    .h_flex()
                    .max_w_full()
                    .rounded_md()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().group_box)
                    .child(
                        self.button(
                            SharedString::from(format!("binding-{}-{}", row.id, shortcut.slot)),
                            label,
                            Command::Binding(BindingCommand::BeginCapture(target)),
                            !row.enabled || busy,
                            cx,
                        )
                        .accessibility_label(self.tr_object(
                            "a11y.record_shortcut",
                            &format!("{}: {}", row.label, shortcut.text),
                        ))
                        .selected(recording),
                    );
                if row.enabled {
                    chip = chip.child(self.icon_button(
                        SharedString::from(if recording {
                            format!("capture-cancel-{}", row.id)
                        } else {
                            format!("remove-{}-{}", row.id, shortcut.slot)
                        }),
                        IconName::X,
                        if recording {
                            self.tr("capture.cancel_shortcut")
                        } else {
                            self.tr_object(
                                "a11y.remove_shortcut",
                                &format!("{}: {}", row.label, shortcut.text),
                            )
                        },
                        Command::Binding(if recording {
                            BindingCommand::CancelCapture
                        } else {
                            BindingCommand::DeleteShortcut(target)
                        }),
                        busy && !recording,
                        cx,
                    ));
                }
                shortcuts = shortcuts.child(chip);
            }
            if row.enabled && row.shortcuts.len() < 2 {
                let recording =
                    capturing && self.ui.binding.capture.slot == row.shortcuts.len() as i32;
                shortcuts = shortcuts.child(
                    self.button(
                        SharedString::from(format!("binding-add-{}", row.id)),
                        if recording {
                            self.ui.binding.capture.text.clone()
                        } else {
                            self.tr("shortcuts.unset")
                        },
                        Command::Binding(BindingCommand::BeginCapture(CaptureTarget {
                            id,
                            slot: row.shortcuts.len(),
                        })),
                        busy,
                        cx,
                    )
                    .icon(IconName::Plus)
                    .accessibility_label(self.tr_object("a11y.add_shortcut", &row.label))
                    .selected(recording),
                );
                if recording {
                    shortcuts = shortcuts.child(self.icon_button(
                        SharedString::from(format!("capture-cancel-{}", row.id)),
                        IconName::X,
                        self.tr("capture.cancel_shortcut"),
                        Command::Binding(BindingCommand::CancelCapture),
                        false,
                        cx,
                    ));
                }
            } else if !row.enabled && row.shortcuts.is_empty() {
                shortcuts =
                    shortcuts.child(self.caption(self.tr("shortcuts.unset"), cx).opacity(0.5));
            }
            let mut shortcut_column = div()
                .id(SharedString::from(format!("binding-shortcuts-{}", row.id)))
                .v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(shortcuts);
            if capturing && !self.ui.binding.capture.error.is_empty() {
                shortcut_column =
                    shortcut_column.child(self.error(&self.ui.binding.capture.error, cx));
            }
            let mut gestures = div()
                .id(SharedString::from(format!("binding-function-{}", row.id)))
                .v_flex()
                .w(rems(15.))
                .flex_shrink_0()
                .when(!row.enabled, |column| column.opacity(0.5));
            for (index, gesture) in row.gestures.iter().enumerate() {
                gestures = gestures.child(
                    div()
                        .id(("gesture", index))
                        .h_flex()
                        .h_6()
                        .gap_2()
                        .child(self.caption(gesture.gesture.clone(), cx).w_10())
                        .child(
                            Icon::new(IconName::ArrowRight)
                                .size_4()
                                .text_color(cx.theme().muted_foreground),
                        )
                        .child(
                            div()
                                .px_2()
                                .py(rems(0.125))
                                .rounded_md()
                                .bg(cx.theme().secondary)
                                .child(self.text(gesture.action.clone())),
                        ),
                );
            }
            rows = rows.child(
                div()
                    .id(SharedString::from(format!("binding-row-{}", row.id)))
                    .child(
                        GroupBox::new()
                            .id(SharedString::from(format!("binding-{}", row.id)))
                            .fill()
                            .content_style(StyleRefinement::default().h_flex().min_h_16())
                            .child(
                                div().w_9().flex_shrink_0().child(
                                    Switch::new(SharedString::from(format!(
                                        "binding-enabled-{}",
                                        row.id
                                    )))
                                    .accessibility_label(row.label.clone())
                                    .checked(row.enabled)
                                    .disabled(busy)
                                    .on_change(cx.listener(
                                        move |this, enabled: &bool, window, cx| {
                                            this.dispatch(
                                                Command::Binding(
                                                    BindingCommand::SetFunctionEnabled {
                                                        id,
                                                        enabled: *enabled,
                                                    },
                                                ),
                                                window,
                                                cx,
                                            );
                                        },
                                    )),
                                ),
                            )
                            .child(shortcut_column.test_support())
                            .child(gestures.test_support())
                            .child(
                                div()
                                    .id(SharedString::from(format!("binding-scope-{}", row.id)))
                                    .w(scope_width)
                                    .flex_shrink_0()
                                    .child(self.scope_picker(
                                        id,
                                        row,
                                        busy,
                                        if wide { 2 } else { 1 },
                                        cx,
                                    ))
                                    .test_support(),
                            ),
                    )
                    .test_support()
                    .map(|row_view| {
                        self.reveal_focus(
                            SharedString::from(format!("binding-focus-{}", row.id)),
                            row_view,
                        )
                    }),
            );
        }
        page.child(self.scroller("bindings", rows))
            .into_any_element()
    }

    fn scope_picker(
        &self,
        id: FunctionId,
        row: &crate::FunctionBindingRow,
        busy: bool,
        visible_limit: usize,
        cx: &Context<Self>,
    ) -> AnyElement {
        if !row.scope_editable {
            return self
                .caption(self.tr("common.all_apps"), cx)
                .into_any_element();
        }
        let picker = &self.scope_pickers[&id];
        div()
            .id(SharedString::from(format!("scope-{}", row.id)))
            .role(Role::Group)
            .aria_label(
                self.tr_object("a11y.scope", &format!("{}: {}", row.label, row.scope_label)),
            )
            .w_full()
            .child(picker.element(
                visible_limit,
                self.ui.locale.clone(),
                self.tr("common.all_apps"),
                busy,
            ))
            .test_support()
            .into_any_element()
    }
}
