use super::*;

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
            .icon(Icon::new(icon).size_4())
            .accessibility_label(label.clone())
            .tooltip(label)
            .size_9()
            .disabled(disabled)
            .on_click(
                cx.listener(move |this, _, window, cx| this.dispatch(command.clone(), window, cx)),
            )
    }

    pub(super) fn heading(&self, text: String) -> Div {
        div()
            .text_2xl()
            .line_height(rems(1.875))
            .font_medium()
            .child(text)
    }

    pub(super) fn navigation(&self, cx: &Context<Self>) -> AnyElement {
        let nav = |page: usize, icon| {
            self.icon_button(
                ("nav", page),
                icon,
                self.tr(TITLES[page]),
                Command::App(Action::Navigate, page.to_string()),
                false,
                cx,
            )
            .toggled(self.ui.page == page as i32)
            .map(|button| {
                if self.ui.page == page as i32 {
                    button.primary()
                } else {
                    button.ghost()
                }
            })
        };
        div()
            .id("navigation")
            .w(rems(3.75))
            .h_full()
            .flex_shrink_0()
            .v_flex()
            .p_3()
            .gap(rems(0.625))
            .bg(cx.theme().sidebar)
            .child(nav(0, IconName::LayoutDashboard))
            .child(nav(1, IconName::Keyboard))
            .child(nav(4, IconName::Folder))
            .child(nav(2, IconName::Bluetooth))
            .child(div().flex_1())
            .child(nav(3, IconName::Settings))
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
                header = header.child(
                    self.button(
                        "custom-mappings",
                        self.tr("mappings.title"),
                        Command::Mapping("open", String::new()),
                        self.ui.groups.busy,
                        cx,
                    )
                    .h_8(),
                );
            }
            1 => {
                header = header.child(
                    self.button(
                        if self.ui.mapping.editor {
                            "mapping-back"
                        } else {
                            "mappings-back"
                        },
                        self.tr("common.back"),
                        Command::Mapping("back", String::new()),
                        self.ui.mapping.saving,
                        cx,
                    )
                    .h_8()
                    .ghost(),
                );
                if !self.ui.mapping.editor {
                    header = header.child(
                        self.button(
                            "mapping-create",
                            self.tr("mappings.create"),
                            Command::Mapping("create", String::new()),
                            self.ui.mapping.busy,
                            cx,
                        )
                        .h_8()
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
                    .h_8()
                    .ghost()
                } else {
                    self.button(
                        "group-create",
                        self.tr("common.new"),
                        Command::Group("create", String::new()),
                        self.ui.groups.busy,
                        cx,
                    )
                    .h_8()
                    .primary()
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
            .child(div().text_xs().child(self.ui.adapter_label.clone()))
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
            )
            .ghost(),
            self.icon_button(
                "receiver-settings",
                IconName::Settings,
                self.tr("receiver.open.settings"),
                Command::App(Action::BluetoothSettings, String::new()),
                false,
                cx,
            )
            .ghost(),
        ]
    }
}
