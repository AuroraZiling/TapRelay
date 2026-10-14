use super::*;
use gpui_kit::base::motion::{Transition, transition};
use gpui_kit::component::Collapsible;
use gpui_kit::component::sidebar::{
    SidebarItem, SidebarMenu, SidebarMenuItem, SidebarToggleButton,
};

const TITLES: [&str; 5] = [
    "nav.overview",
    "nav.bindings",
    "nav.receiver",
    "nav.settings",
    "nav.groups",
];

impl AppView {
    pub(super) fn icon_button(
        &self,
        id: impl Into<ElementId>,
        icon: IconName,
        label: String,
        command: Command,
        disabled: bool,
        cx: &Context<Self>,
    ) -> Button {
        Button::new(id)
            .icon(icon)
            .accessibility_label(label.clone())
            .tooltip(label)
            .disabled(disabled)
            .on_click(
                cx.listener(move |this, _, window, cx| this.dispatch(command.clone(), window, cx)),
            )
    }

    pub(super) fn heading(&self, text: String) -> Div {
        div().text_xl().child(text)
    }

    pub(super) fn navigation(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let target_width = if self.sidebar_collapsed {
            window.rem_size() * 3.5 + px(1.)
        } else {
            window.rem_size() * 12.
        };
        let width = transition(
            "sidebar-width",
            target_width,
            Transition::new(Duration::from_millis(200)),
            window,
            cx,
        );
        let content_width = target_width - window.rem_size() * 1.5 - px(1.);
        let nav = |page: usize, icon| {
            SidebarMenuItem::new(self.tr(TITLES[page]))
                .icon(Icon::new(icon).size_4())
                .min_h_9()
                .active(self.ui.page == page as i32)
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.dispatch(Command::App(Action::Navigate, page.to_string()), window, cx)
                }))
        };
        let menu = SidebarMenu::new()
            .child(nav(0, IconName::LayoutDashboard))
            .child(nav(1, IconName::Keyboard))
            .child(nav(4, IconName::Folder))
            .child(nav(2, IconName::Bluetooth))
            .w(content_width)
            .collapsed(self.sidebar_collapsed);
        let settings = nav(3, IconName::Settings)
            .collapsed(self.sidebar_collapsed)
            .render(("nav", 3usize), window, cx)
            .into_any_element();
        div()
            .id("navigation")
            .w(width)
            .h_full()
            .flex_shrink_0()
            .v_flex()
            .p_3()
            .gap_3()
            .overflow_hidden()
            .border_r_1()
            .border_color(cx.theme().sidebar_border)
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
                            .accessibility_label(if self.sidebar_collapsed {
                                "Expand sidebar"
                            } else {
                                "Collapse sidebar"
                            })
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
                    .child(menu.render(0usize, window, cx).into_any_element()),
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
        for crumb in crumbs {
            header = header
                .child(
                    Icon::new(IconName::ChevronRight)
                        .size_4()
                        .text_color(cx.theme().muted_foreground),
                )
                .child(self.heading(crumb));
        }
        if self.ui.page == 2 {
            header = header.child(self.bluetooth_status(cx));
        }
        header = header.child(div().flex_1());
        match self.ui.page {
            1 if !self.ui.mapping.open => {
                header = header.child(self.button(
                    "custom-mappings",
                    self.tr("mappings.title"),
                    Command::Mapping("open", String::new()),
                    self.ui.groups.busy,
                    cx,
                ));
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
                    header = header.child(self.button(
                        "mapping-create",
                        self.tr("mappings.create"),
                        Command::Mapping("create", String::new()),
                        self.ui.mapping.busy,
                        cx,
                    ));
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
                } else {
                    self.button(
                        "group-create",
                        self.tr("common.new"),
                        Command::Group("create", String::new()),
                        self.ui.groups.busy,
                        cx,
                    )
                });
            }
            _ => {}
        }
        header.test_support().into_any_element()
    }

    pub(super) fn bluetooth_status(&self, cx: &App) -> Div {
        div()
            .h_flex()
            .gap(rems(0.375))
            .h_8()
            .px_2()
            .rounded_md()
            .border_2()
            .border_color(if self.ui.bluetooth_available {
                cx.theme().success
            } else {
                cx.theme().danger
            })
            .child(
                Icon::new(if self.ui.bluetooth_available {
                    IconName::Bluetooth
                } else {
                    IconName::BluetoothOff
                })
                .size_4(),
            )
            .child(div().child(self.ui.adapter_label.clone()))
    }

    pub(super) fn receiver_actions(&self, cx: &Context<Self>) -> [Button; 2] {
        [
            self.icon_button(
                "receiver-refresh",
                IconName::RefreshCw,
                self.tr("receiver.refresh"),
                Command::App(Action::Refresh, String::new()),
                self.ui.scanning,
                cx,
            ),
            self.icon_button(
                "receiver-settings",
                IconName::Settings,
                self.tr("receiver.open.settings"),
                Command::App(Action::BluetoothSettings, String::new()),
                false,
                cx,
            ),
        ]
    }
}
