use crate::gui::view::components::ShortcutButton;
use crate::gui::view::{AppView, Command};
use crate::runtime::commands::{BindingCommand, CaptureTarget};
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Selectable, StyledExt,
    group_box::{GroupBox, GroupBoxVariants},
    switch::Switch,
    tag::Tag,
};
use gpui_kit::{prelude::*, *};
use taprelay_core::function::FunctionId;

impl AppView {
    pub(in crate::gui::view) fn bindings(&self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let wide = window.viewport_size().width >= px(1100.);
        let scope_width = rems(if wide { 16. } else { 10. });
        let mut page = div().v_flex().flex_1().min_h_0().gap_6();
        if !self.ui.groups.error.is_empty() {
            page = page.child(self.error(&self.ui.groups.error, cx));
        }
        let header = div()
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
            );
        let mut rows = div()
            .grid()
            .grid_cols(1)
            .grid_rows(self.ui.binding.rows.len() as u16)
            .gap_2();
        let busy = self.controller.recording() || self.ui.groups.busy;
        for row in &self.ui.binding.rows {
            let Some(id) = FunctionId::from_stable_id(&row.id) else {
                continue;
            };
            let busy = busy || row.pending;
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
                let mut chip = ShortcutButton::new(
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
                    chip = chip.clear_button(self.plain_icon_button(
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
                let mut chip = ShortcutButton::new(
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
                    chip = chip.clear_button(self.plain_icon_button(
                        SharedString::from(format!("capture-cancel-{}", row.id)),
                        IconName::X,
                        self.tr("capture.cancel_shortcut"),
                        Command::Binding(BindingCommand::CancelCapture),
                        false,
                        cx,
                    ));
                }
                shortcuts = shortcuts.child(chip);
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
                shortcut_column = shortcut_column.child(
                    self.error(&self.ui.binding.capture.error, cx)
                        .id("binding-capture-error")
                        .text_xs()
                        .line_height(relative(1.25))
                        .test_support(),
                );
            }
            let mut gestures = div()
                .id(SharedString::from(format!("binding-function-{}", row.id)))
                .v_flex()
                .gap_1()
                .w(rems(15.))
                .flex_shrink_0()
                .when(!row.enabled, |column| column.opacity(0.5));
            for (index, gesture) in row.gestures.iter().enumerate() {
                gestures = gestures.child(
                    div()
                        .id(("gesture", index))
                        .h_flex()
                        .min_h_6()
                        .flex_shrink_0()
                        .gap_2()
                        .child(
                            self.caption(gesture.gesture.clone(), cx)
                                .w_10()
                                .text_right()
                                .flex_shrink_0(),
                        )
                        .child(
                            Icon::new(IconName::ArrowRight)
                                .size_4()
                                .flex_shrink_0()
                                .text_color(cx.theme().muted_foreground),
                        )
                        .child(
                            div()
                                .id(("gesture-badge", index))
                                .min_w_0()
                                .max_w_full()
                                .child(
                                    Tag::new()
                                        .min_w_0()
                                        .max_w_full()
                                        .child(self.text(gesture.action.clone()).min_w_0()),
                                )
                                .test_support(),
                        )
                        .test_support(),
                );
            }
            rows = rows.child(
                div()
                    .id(SharedString::from(format!("binding-row-{}", row.id)))
                    .h_full()
                    .child(
                        GroupBox::new()
                            .id(SharedString::from(format!("binding-{}", row.id)))
                            .fill()
                            .h_full()
                            .grid()
                            .grid_cols(1)
                            .grid_rows(1)
                            .content_style(
                                self.card_style(cx)
                                    .h_flex()
                                    .flex_1()
                                    .min_w_0()
                                    .min_h_16()
                                    .py_2(),
                            )
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
        page.child(self.scroller("bindings", div().v_flex().gap_2().child(header).child(rows)))
            .into_any_element()
    }

    fn scope_picker(
        &self,
        id: FunctionId,
        row: &crate::gui::presentation::FunctionBindingRow,
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
                busy || self.controller.runtime.saving_function_scope(id),
            ))
            .test_support()
            .into_any_element()
    }
}
