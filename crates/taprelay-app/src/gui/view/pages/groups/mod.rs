use crate::gui::view::{AppView, Command};
use gpui_kit::assets::IconName;
use gpui_kit::component::{
    StyledExt,
    group_box::{GroupBox, GroupBoxVariants},
};
use gpui_kit::{prelude::*, *};

mod dialog;
mod editor;

impl AppView {
    pub(in crate::gui::view) fn groups(&self, cx: &Context<Self>) -> AnyElement {
        let group = &self.ui.groups;
        if group.editor_open {
            return self.group_editor(cx);
        }
        let mut rows = div().v_flex().gap_2();
        for row in &group.groups {
            rows = rows.child(
                GroupBox::new()
                    .id(SharedString::from(format!("group-{}", row.id)))
                    .fill()
                    .content_style(StyleRefinement::default().h_flex())
                    .child(
                        div()
                            .v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap(rems(0.3125))
                            .child(div().text_ellipsis().child(row.name.clone()))
                            .child(
                                self.caption(
                                    if row.summary.is_empty() {
                                        self.tr("groups.list_empty")
                                    } else {
                                        row.summary.clone()
                                    },
                                    cx,
                                )
                                .text_ellipsis(),
                            ),
                    )
                    .child(self.caption(row.count_label.clone(), cx))
                    .child(self.icon_button(
                        SharedString::from(format!("group-edit-{}", row.id)),
                        IconName::Pencil,
                        self.tr("common.edit"),
                        Command::Group("edit", row.id.clone()),
                        group.busy,
                        cx,
                    ))
                    .child(self.icon_button(
                        SharedString::from(format!("group-delete-{}", row.id)),
                        IconName::Trash,
                        self.tr("common.delete"),
                        Command::Group("request-delete", row.id.clone()),
                        group.busy,
                        cx,
                    )),
            );
        }
        div()
            .v_flex()
            .flex_1()
            .min_h_0()
            .gap_4()
            .child(self.caption(self.tr("groups.help"), cx))
            .when(!group.error.is_empty(), |body| {
                body.child(self.error(&group.error, cx))
            })
            .child(self.scroller("groups", rows))
            .into_any_element()
    }
}
