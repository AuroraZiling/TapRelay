use super::{AppView, Command};
use crate::gui::action::Action;
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::base::motion::{Transition, transition};
#[cfg(not(windows))]
use gpui_kit::component::TitleBar;
use gpui_kit::component::{
    ActiveTheme, Icon, Selectable, StyledExt,
    button::{Button, ButtonVariants},
    sidebar::SidebarToggleButton,
};
use gpui_kit::{prelude::*, *};
use std::time::Duration;

const CHROME_SIZE_REMS: f32 = 2.75;
const SIDEBAR_PADDING_REMS: f32 = 0.375;

const TITLES: [&str; 5] = [
    "nav.overview",
    "nav.bindings",
    "nav.receiver",
    "nav.settings",
    "nav.groups",
];

impl AppView {
    pub(super) fn title_bar(&mut self, window: &Window, cx: &Context<Self>) -> AnyElement {
        let icon = self.app_icon.image(window.rem_size() * 1.5, window);
        let origin = f32::from(window.rem_size()) * (CHROME_SIZE_REMS - 1.5) * 0.5;
        // A fractional device-pixel origin softens the raster even at its native size.
        let offset = px((origin * window.scale_factor()).round() / window.scale_factor() - origin);
        let brand = div()
            .id("titlebar-brand")
            .h_flex()
            .flex_shrink_0()
            .text_sm()
            .when(self.ui.mode == 2, |brand| {
                brand.child(
                    div()
                        .w(rems(CHROME_SIZE_REMS))
                        .h(rems(CHROME_SIZE_REMS))
                        .flex_shrink_0()
                        .h_flex()
                        .justify_center()
                        // Native title-bar hit testing must not treat this control as a drag area.
                        .occlude()
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
            })
            .child(
                div()
                    .w(rems(CHROME_SIZE_REMS))
                    .flex_shrink_0()
                    .h_flex()
                    .justify_center()
                    .child(
                        img(icon)
                            .id("titlebar-icon")
                            .size_6()
                            .relative()
                            .left(offset)
                            .top(offset)
                            .test_support(),
                    ),
            )
            .child(self.text("TapRelay").id("titlebar-title").test_support())
            .test_support();

        #[cfg(not(windows))]
        return TitleBar::new()
            .h(rems(CHROME_SIZE_REMS))
            .pl_0()
            .bg(cx.theme().title_bar)
            .border_b_0()
            .child(brand)
            .into_any_element();

        #[cfg(windows)]
        {
            let control = |id, label, icon, area, close| {
                div()
                    .id(id)
                    .role(Role::Button)
                    .aria_label(label)
                    .w_12()
                    .h_full()
                    .flex_shrink_0()
                    .h_flex()
                    .justify_center()
                    .window_control_area(area)
                    .text_color(cx.theme().foreground)
                    .hover(|style| {
                        style
                            .bg(if close {
                                cx.theme().danger
                            } else {
                                cx.theme().secondary_hover
                            })
                            .text_color(if close {
                                cx.theme().danger_foreground
                            } else {
                                cx.theme().secondary_foreground
                            })
                    })
                    .active(|style| {
                        style
                            .bg(if close {
                                cx.theme().danger_active
                            } else {
                                cx.theme().secondary_active
                            })
                            .text_color(if close {
                                cx.theme().danger_foreground
                            } else {
                                cx.theme().secondary_foreground
                            })
                    })
                    .child(Icon::new(icon).size_3())
                    .test_support()
            };
            let supported = window.window_controls();
            div()
                .id("title-bar")
                .h_flex()
                .h(rems(CHROME_SIZE_REMS))
                .flex_shrink_0()
                .bg(cx.theme().title_bar)
                .child(
                    div()
                        .h_flex()
                        .flex_1()
                        .h_full()
                        .window_control_area(WindowControlArea::Drag)
                        .child(brand),
                )
                .child(
                    div()
                        .id("window-controls")
                        .h_flex()
                        .h_full()
                        .flex_shrink_0()
                        .when(supported.minimize, |controls| {
                            controls.child(control(
                                "minimize",
                                "Minimize",
                                IconName::WindowMinimize,
                                WindowControlArea::Min,
                                false,
                            ))
                        })
                        .when(supported.maximize, |controls| {
                            let (id, label, icon) = if window.is_maximized() {
                                ("restore", "Restore", IconName::WindowRestore)
                            } else {
                                ("maximize", "Maximize", IconName::WindowMaximize)
                            };
                            controls.child(control(id, label, icon, WindowControlArea::Max, false))
                        })
                        .child(control(
                            "close",
                            "Close",
                            IconName::WindowClose,
                            WindowControlArea::Close,
                            true,
                        ))
                        .test_support(),
                )
                .test_support()
                .into_any_element()
        }
    }

    pub(super) fn navigation(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let target_width = if self.sidebar_collapsed {
            window.rem_size() * CHROME_SIZE_REMS
        } else {
            window.rem_size() * 11.
        };
        let width = transition(
            "sidebar-width",
            target_width,
            Transition::new(Duration::from_millis(200)),
            window,
            cx,
        );
        let content_width = target_width - window.rem_size() * SIDEBAR_PADDING_REMS * 2.;
        let nav = |id: ElementId, page: usize, icon| {
            let label = self.tr(TITLES[page]);
            Button::new(id)
                .ghost()
                .child(Icon::new(icon).size_4())
                .accessibility_label(label.clone())
                .tooltip(label.clone())
                .h_8()
                .w_full()
                .px_1p5()
                .when(!self.sidebar_collapsed, |button| {
                    button
                        .px_2()
                        .child(div().flex_1().min_w_0().text_ellipsis().child(label))
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
            .px(rems(SIDEBAR_PADDING_REMS))
            .gap_3()
            .overflow_hidden()
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().sidebar_foreground)
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
