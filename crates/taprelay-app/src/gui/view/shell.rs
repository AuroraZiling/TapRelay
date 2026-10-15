use super::{AppView, Command};
use crate::action::Action;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::base::motion::{Transition, transition};
use gpui_kit::component::{
    ActiveTheme, Icon, Selectable, StyledExt, TitleBar,
    button::{Button, ButtonVariants},
    sidebar::SidebarToggleButton,
};
use gpui_kit::{prelude::*, *};
use std::time::Duration;

const TITLES: [&str; 5] = [
    "nav.overview",
    "nav.bindings",
    "nav.receiver",
    "nav.settings",
    "nav.groups",
];

impl AppView {
    pub(super) fn title_bar(&self, cx: &App) -> TitleBar {
        TitleBar::new()
            .h_10()
            .pl(rems(1.125))
            .bg(cx.theme().title_bar)
            .border_b_0()
            .child(
                div()
                    .id("titlebar-brand")
                    .h_flex()
                    .gap_2()
                    .flex_shrink_0()
                    .text_sm()
                    .child(img(self.app_icon.clone()).size_4())
                    .child(self.text("TapRelay"))
                    .test_support(),
            )
    }

    pub(super) fn navigation(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let target_width = if self.sidebar_collapsed {
            window.rem_size() * 3.25
        } else {
            window.rem_size() * 11.75
        };
        let width = transition(
            "sidebar-width",
            target_width,
            Transition::new(Duration::from_millis(200)),
            window,
            cx,
        );
        let content_width = target_width - window.rem_size() * 1.25;
        let nav = |id: ElementId, page: usize, icon| {
            let label = self.tr(TITLES[page]);
            Button::new(id)
                .ghost()
                .icon(icon)
                .accessibility_label(label.clone())
                .tooltip(label.clone())
                .h_9()
                .w_full()
                .when(!self.sidebar_collapsed, |button| {
                    button.label(label).justify_start()
                })
                .selected(self.ui.page == page as i32)
                .toggled(self.ui.page == page as i32)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.dispatch(Command::App(Action::Navigate, page.to_string()), window, cx)
                }))
        };
        let menu = div()
            .v_flex()
            .gap_1()
            .child(nav("0-0".into(), 0, IconName::LayoutDashboard))
            .child(nav("0-1".into(), 1, IconName::Keyboard))
            .child(nav("0-2".into(), 4, IconName::Folder))
            .child(nav("0-3".into(), 2, IconName::Bluetooth))
            .w(content_width);
        let settings = nav(("nav", 3usize).into(), 3, IconName::Settings);
        div()
            .id("navigation")
            .w(width)
            .h_full()
            .flex_shrink_0()
            .v_flex()
            .py_3()
            .px_2p5()
            .gap_3()
            .overflow_hidden()
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
            .child(
                div()
                    .w_8()
                    .h_9()
                    .flex_shrink_0()
                    .h_flex()
                    .justify_center()
                    .child(
                        SidebarToggleButton::new()
                            .collapsed(self.sidebar_collapsed)
                            .accessibility_label(self.tr(if self.sidebar_collapsed {
                                "nav.expand"
                            } else {
                                "nav.collapse"
                            }))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.sidebar_collapsed = !this.sidebar_collapsed;
                                cx.notify();
                            })),
                    ),
            )
            .child(
                div()
                    .id("navigation-menu")
                    .w(content_width)
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(menu),
            )
            .child(div().w(content_width).child(settings))
            .test_support()
            .into_any_element()
    }

    pub(super) fn page_header(&self, cx: &Context<Self>) -> AnyElement {
        let mut header = div()
            .id("page-header")
            .h_flex()
            .gap_3()
            .flex_shrink_0()
            .child(self.heading(self.tr(TITLES[self.ui.page as usize])));
        let mut crumbs = Vec::new();
        if self.ui.page == 1 && self.ui.mapping.open {
            crumbs.push(self.tr("mappings.title"));
            if self.ui.mapping.editor {
                crumbs.push(self.tr(if self.ui.mapping.new_mapping {
                    "common.new"
                } else {
                    "common.edit"
                }));
            }
        }
        if self.ui.page == 4 && self.ui.groups.editor_open {
            crumbs.push(self.tr(if self.ui.groups.selected_id.is_empty() {
                "common.new"
            } else {
                "common.edit"
            }));
        }
        for (index, crumb) in crumbs.into_iter().enumerate() {
            header = header
                .child(
                    Icon::new(IconName::ChevronRight)
                        .size_4()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(self.heading(crumb).id(("breadcrumb", index)));
        }
        if self.ui.page == 2 {
            header = header.child(self.bluetooth_status(cx));
        }
        header = header.child(div().flex_1());
        match self.ui.page {
            1 if !self.ui.mapping.open => {
                header = header.child(
                    self.button(
                        "custom-mappings",
                        self.tr("mappings.title"),
                        Command::Mapping("open", String::new()),
                        self.ui.groups.busy,
                        cx,
                    )
                    .primary(),
                );
            }
            1 => {
                header = header.child(self.button(
                    if self.ui.mapping.editor {
                        "mapping-back"
                    } else {
                        "mappings-back"
                    },
                    self.tr("common.back"),
                    Command::Mapping("back", String::new()),
                    self.ui.mapping.saving,
                    cx,
                ));
                if !self.ui.mapping.editor {
                    header = header.child(
                        self.button(
                            "mapping-create",
                            self.tr("mappings.create"),
                            Command::Mapping("create", String::new()),
                            self.ui.mapping.busy,
                            cx,
                        )
                        .primary(),
                    );
                }
            }
            2 => header = header.children(self.receiver_actions(cx)),
            4 => {
                header = header.child(if self.ui.groups.editor_open {
                    self.button(
                        "group-cancel",
                        self.tr("common.back"),
                        Command::Group("cancel", String::new()),
                        self.ui.groups.saving,
                        cx,
                    )
                    .secondary()
                } else {
                    self.button(
                        "group-create",
                        self.tr("common.new"),
                        Command::Group("create", String::new()),
                        self.ui.groups.busy,
                        cx,
                    )
                    .primary()
                });
            }
            _ => {}
        }
        header.test_support().into_any_element()
    }
}
