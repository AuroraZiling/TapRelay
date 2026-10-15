use crate::gui::view::{AppView, Command};
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{
    Icon, Sizable, StyledExt,
    button::ButtonVariants,
    group_box::{GroupBox, GroupBoxVariants},
    input::Input,
};
use gpui_kit::{prelude::*, *};

impl AppView {
    pub(in crate::gui::view) fn browse_group_app(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.ui.groups.editor_open || self.ui.groups.busy || self.controller.recording() {
            return;
        }
        let editor = self.ui.groups.focus_request;
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(self.tr("groups.browse").into()),
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = paths
                .await
                .map_err(anyhow::Error::from)
                .and_then(|paths| paths);
            let _ = this.update_in(cx, |this, window, cx| {
                if !this.ui.groups.editor_open || this.ui.groups.focus_request != editor {
                    return;
                }
                match result {
                    Ok(Some(paths)) => {
                        if let Some(path) = paths.into_iter().next() {
                            this.dispatch(
                                Command::Group("picked", path.to_string_lossy().into_owned()),
                                window,
                                cx,
                            );
                        }
                    }
                    Ok(None) => {}
                    Err(error) => {
                        this.ui.groups.form_error = error.to_string();
                        cx.notify();
                    }
                }
            });
        })
        .detach();
    }

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
                div()
                    .h_flex()
                    .gap_4()
                    .child(self.text(self.tr("groups.name")))
                    .child(
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
                    .child(
                        self.icon_button(
                            "group-refresh",
                            IconName::RefreshCw,
                            self.tr("groups.refresh"),
                            Command::Group("running", String::new()),
                            busy,
                            cx,
                        )
                        .ghost(),
                    )
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
        let mut rows = div().v_flex().gap_1();
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
                img(image).size_6().into_any_element()
            } else {
                Icon::new(IconName::AppWindow).size_6().into_any_element()
            };
            let mut details = div()
                .v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                    self.text(icons.name(&app.path).unwrap_or(&app.name).to_string())
                        .text_sm()
                        .text_ellipsis(),
                )
                .child(self.caption(app.path.clone(), cx).text_xs().text_ellipsis());
            if icons.missing(&app.path) {
                details = details.child(self.error(&self.tr("groups.missing"), cx).text_xs());
            }
            rows = rows.child(
                GroupBox::new()
                    .id(SharedString::from(format!("group-app-row-{}", app.path)))
                    .fill()
                    .content_style(self.card_style(cx).h_flex().gap_3().px_3().py_2())
                    .child(div().size_6().flex_shrink_0().child(icon))
                    .child(details)
                    .child(
                        self.icon_button(
                            SharedString::from(format!("group-app-{}", app.path)),
                            if group.choosing_running {
                                IconName::Plus
                            } else {
                                IconName::X
                            },
                            self.tr_object(
                                if group.choosing_running {
                                    "a11y.add_app"
                                } else {
                                    "a11y.remove_app"
                                },
                                icons.name(&app.path).unwrap_or(&app.name),
                            ),
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
                        )
                        .ghost()
                        .small(),
                    )
                    .map(|row| {
                        self.reveal_focus(
                            SharedString::from(format!("app-focus-{}", app.path)),
                            row,
                        )
                    }),
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
                    .child(
                        self.button(
                            "group-save",
                            self.tr(if group.saving {
                                "common.saving"
                            } else {
                                "groups.save"
                            }),
                            Command::Group("save", String::new()),
                            busy,
                            cx,
                        )
                        .primary(),
                    ),
            )
            .into_any_element()
    }
}
