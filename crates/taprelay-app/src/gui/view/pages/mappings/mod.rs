use crate::gui::view::{AppView, Command};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    Disableable, StyledExt,
    group_box::{GroupBox, GroupBoxVariants},
    switch::Switch,
};
use gpui_kit::{prelude::*, *};

mod dialog;
mod editor;

impl AppView {
    pub(in crate::gui::view) fn mappings(&self, cx: &Context<Self>) -> AnyElement {
        let mapping = &self.ui.mapping;
        if mapping.editor {
            return self.mapping_editor(cx);
        }
        let mut body = div().v_flex().flex_1().min_h_0().gap_6().child(
            div()
                .h_flex()
                .gap_3()
                .child(self.caption(self.tr("mappings.help"), cx).flex_1())
                .when(mapping.can_undo, |row| {
                    row.child(self.button(
                        "mapping-undo",
                        self.tr("common.undo"),
                        Command::Mapping("undo", String::new()),
                        mapping.busy,
                        cx,
                    ))
                }),
        );
        if !mapping.error.is_empty() {
            body = body.child(self.error(&mapping.error, cx));
        }
        if mapping.rows.is_empty() {
            return body
                .child(
                    div()
                        .v_flex()
                        .flex_1()
                        .justify_center()
                        .items_center()
                        .child(self.caption(self.tr("mappings.empty"), cx)),
                )
                .into_any_element();
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
                    self.caption(self.tr("common.description"), cx)
                        .flex_grow(2.)
                        .flex_basis(px(0.))
                        .min_w_0(),
                )
                .child(
                    self.caption(self.tr("common.mapping"), cx)
                        .flex_grow(3.)
                        .flex_basis(px(0.))
                        .min_w_0(),
                )
                .child(
                    self.caption(self.tr("common.effective_scope"), cx)
                        .w(rems(7.75))
                        .flex_shrink_0(),
                )
                .child(
                    self.caption(self.tr("common.actions"), cx)
                        .w_20()
                        .flex_shrink_0(),
                ),
        );
        for row in &mapping.rows {
            let id = row.id.clone();
            rows = rows.child(
                GroupBox::new()
                    .id(SharedString::from(format!("mapping-{}", row.id)))
                    .fill()
                    .content_style(StyleRefinement::default().h_flex().min_h_16())
                    .child(
                        div().w_9().flex_shrink_0().child(
                            Switch::new(SharedString::from(format!("mapping-toggle-{id}")))
                                .accessibility_label(row.name.clone())
                                .checked(row.enabled)
                                .disabled(mapping.busy)
                                .on_change(cx.listener(move |this, _, window, cx| {
                                    this.dispatch(
                                        Command::Mapping("toggle", id.clone()),
                                        window,
                                        cx,
                                    )
                                })),
                        ),
                    )
                    .child(
                        div()
                            .flex_grow(2.)
                            .flex_basis(px(0.))
                            .min_w_0()
                            .text_ellipsis()
                            .child(row.name.clone()),
                    )
                    .child(
                        self.caption(format!("{} → {}", row.input, row.output), cx)
                            .flex_grow(3.)
                            .flex_basis(px(0.))
                            .min_w_0(),
                    )
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "mapping-scope-label-{}",
                                row.id
                            )))
                            .w(rems(7.75))
                            .flex_shrink_0()
                            .text_ellipsis()
                            .tooltip({
                                let text = row.scope.clone();
                                move |window, cx| {
                                    gpui_kit::component::tooltip::Tooltip::new(text.clone())
                                        .build(window, cx)
                                }
                            })
                            .child(row.scope.clone()),
                    )
                    .child(
                        div()
                            .h_flex()
                            .w_20()
                            .gap_2()
                            .flex_shrink_0()
                            .child(self.icon_button(
                                SharedString::from(format!("mapping-edit-{}", row.id)),
                                IconName::Pencil,
                                self.tr("common.edit"),
                                Command::Mapping("edit", row.id.clone()),
                                mapping.busy,
                                cx,
                            ))
                            .child(self.icon_button(
                                SharedString::from(format!("mapping-delete-{}", row.id)),
                                IconName::Trash,
                                self.tr("common.delete"),
                                Command::Mapping("delete", row.id.clone()),
                                mapping.busy,
                                cx,
                            )),
                    ),
            );
        }
        body.child(self.scroller("mappings", rows))
            .into_any_element()
    }
}
