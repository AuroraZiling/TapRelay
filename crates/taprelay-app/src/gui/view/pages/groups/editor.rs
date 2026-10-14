use crate::gui::view::{AppView, Command};
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{
    Icon, StyledExt,
    group_box::{GroupBox, GroupBoxVariants},
    input::Input,
};
use gpui_kit::{prelude::*, *};

impl AppView {
    pub(super) fn group_editor(&self, cx: &Context<Self>) -> AnyElement {
        use gpui_kit::component::tab::{Tab, TabBar};
        let group = &self.ui.groups;
        let busy = group.busy || group.saving;
        let tabs = TabBar::new("group-tabs")
            .underline()
            .w_full()
            .last_empty_space(div())
            .selected_index(usize::from(group.choosing_running))
            .child(
                Tab::new()
                    .label(format!(
                        "{} ({})",
                        self.tr("groups.selected"),
                        group.foreground_apps.len()
                    ))
                    .flex_1()
                    .disabled(busy),
            )
            .child(
                Tab::new()
                    .label(self.tr("groups.running"))
                    .flex_1()
                    .disabled(busy),
            )
            .on_click(cx.listener(|this, index: &usize, window, cx| {
                this.dispatch(
                    Command::Group(
                        if *index == 0 {
                            "show-selected"
                        } else {
                            "running"
                        },
                        String::new(),
                    ),
                    window,
                    cx,
                )
            }));
        let mut body = div()
            .v_flex()
            .flex_1()
            .min_h_0()
            .gap_4()
            .child(
                div().h_flex().gap_4().child(self.tr("groups.name")).child(
                    div().flex_1().min_w_0().child(
                        Input::new(&self.group_name)
                            .id("group-name")
                            .aria_label(self.tr("groups.name"))
                            .disabled(busy),
                    ),
                ),
            )
            .child(tabs);
        if group.choosing_running {
            body = body.child(
                div()
                    .h_flex()
                    .gap_2()
                    .child(self.caption(self.tr("groups.running_help"), cx).flex_1())
                    .child(self.icon_button(
                        "group-refresh",
                        IconName::RefreshCw,
                        self.tr("groups.refresh"),
                        Command::Group("running", String::new()),
                        busy,
                        cx,
                    ))
                    .child(self.button(
                        "group-browse",
                        self.tr("groups.browse"),
                        Command::Group("browse", String::new()),
                        busy,
                        cx,
                    )),
            );
        }
        let apps = if group.choosing_running {
            &group.running
        } else {
            &group.foreground_apps
        };
        let mut rows = div().v_flex().gap_2();
        if apps.is_empty() {
            rows = rows.child(self.caption(
                self.tr(if group.choosing_running {
                    "groups.running_empty"
                } else {
                    "groups.no_apps"
                }),
                cx,
            ));
        }
        for app in apps {
            let mut icons = self.controller.foreground_app_icons.borrow_mut();
            icons.request(&app.path);
            let icon = if let Some(image) = icons.image(&app.path) {
                img(image).size_8().into_any_element()
            } else {
                Icon::new(IconName::AppWindow).size_7().into_any_element()
            };
            let mut details = div()
                .v_flex()
                .flex_1()
                .min_w_0()
                .gap_1()
                .child(
                    div()
                        .text_ellipsis()
                        .child(icons.name(&app.path).unwrap_or(&app.name).to_string()),
                )
                .child(self.caption(app.path.clone(), cx).text_ellipsis());
            if icons.missing(&app.path) {
                details = details.child(self.error(&self.tr("groups.missing"), cx));
            }
            rows = rows.child(
                GroupBox::new()
                    .id(SharedString::from(format!("group-app-row-{}", app.path)))
                    .fill()
                    .content_style(StyleRefinement::default().h_flex().gap_3())
                    .child(div().size_8().flex_shrink_0().child(icon))
                    .child(details)
                    .child(self.icon_button(
                        SharedString::from(format!("group-app-{}", app.path)),
                        if group.choosing_running {
                            IconName::Plus
                        } else {
                            IconName::X
                        },
                        self.tr(if group.choosing_running {
                            "common.add"
                        } else {
                            "common.remove"
                        }),
                        Command::Group(
                            if group.choosing_running {
                                "add"
                            } else {
                                "remove"
                            },
                            app.path.clone(),
                        ),
                        busy,
                        cx,
                    )),
            );
        }
        body.child(self.scroller("group-editor", rows))
            .child(
                div()
                    .h_flex()
                    .justify_end()
                    .gap_3()
                    .flex_shrink_0()
                    .child(div().flex_1().min_w_0().when(
                        !group.form_error.is_empty(),
                        |feedback| {
                            feedback.child(
                                self.error(&group.form_error, cx)
                                    .id("group-form-error")
                                    .test_support(),
                            )
                        },
                    ))
                    .child(self.button(
                        "group-save",
                        self.tr(if group.saving {
                            "common.saving"
                        } else {
                            "groups.save"
                        }),
                        Command::Group("save", String::new()),
                        busy,
                        cx,
                    )),
            )
            .into_any_element()
    }
}
