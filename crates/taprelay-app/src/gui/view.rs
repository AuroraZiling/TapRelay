use super::{
    Action, BindingCommand, Controller, DesktopEvent, FunctionId, Presentation, config, desktop,
    presentation::ChoiceOption,
};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{
    ActiveTheme, StyledExt,
    combobox::{ComboboxEvent, ComboboxState},
    input::{InputEvent, InputState},
};
use gpui_kit::{prelude::*, *};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

mod components;
mod dialogs;
mod pages;
mod shell;
mod window;

#[derive(Clone)]
enum Command {
    App(Action, String),
    Binding(BindingCommand),
    Group(&'static str, String),
    Mapping(&'static str, String),
    Scope(FunctionId, BTreeSet<String>),
}

pub(super) struct AppView {
    controller: Controller,
    ui: Presentation,
    focus: FocusHandle,
    capture_focus: Option<FocusHandle>,
    sidebar_collapsed: bool,
    content_scroll: ScrollHandle,
    scroll_region: std::cell::Cell<&'static str>,
    app_icon: components::AppIcon,
    group_name: Entity<InputState>,
    mapping_name: Entity<InputState>,
    output_filter: Entity<InputState>,
    report_rate: Entity<ComboboxState<Vec<ChoiceOption>>>,
    report_rate_auto_label: String,
    scope_pickers: BTreeMap<FunctionId, components::ScopePicker>,
    ripples: super::ripple::Ripples,
    dialog: u8,
    group_focus_request: i32,
    mapping_focus_request: i32,
    last_progress_message: String,
    last_tick: Instant,
    _subscriptions: Vec<Subscription>,
    _pump: Task<()>,
}

impl AppView {
    pub(super) fn new(
        mut controller: Controller,
        geometry: config::Geometry,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        super::theme::init(cx);
        let group_name = cx.new(|cx| InputState::new(window, cx));
        let mapping_name = cx.new(|cx| InputState::new(window, cx));
        let output_filter = cx.new(|cx| InputState::new(window, cx));
        let report_rate =
            cx.new(|cx| ComboboxState::new(Vec::<ChoiceOption>::new(), Vec::new(), window, cx));
        let mut subscriptions = vec![
            cx.subscribe_in(&report_rate, window, |this, _, event, window, cx| {
                if let ComboboxEvent::Change(values) = event
                    && let Some(value) = values.first()
                {
                    this.dispatch(
                        Command::App(Action::SetPassthroughMouseReportRate, value.clone()),
                        window,
                        cx,
                    );
                }
            }),
            cx.subscribe_in(
                &group_name,
                window,
                |this, input, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => {
                        this.ui.groups.selected_name = input.read(cx).value().to_string()
                    }
                    InputEvent::PressEnter { .. }
                        if this.ui.groups.editor_open
                            && !this.ui.groups.saving
                            && this.dialog == 0 =>
                    {
                        this.dispatch(Command::Group("save", String::new()), window, cx);
                    }
                    _ => {}
                },
            ),
            cx.subscribe(&mapping_name, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.ui.mapping.name = input.read(cx).value().to_string();
                }
            }),
            cx.subscribe_in(
                &output_filter,
                window,
                |this, input, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        this.dispatch(
                            Command::Mapping("filter-output", input.read(cx).value().to_string()),
                            window,
                            cx,
                        );
                    }
                },
            ),
        ];
        let weak = cx.entity().downgrade();
        let handle = window.window_handle();
        subscriptions.push(cx.intercept_keystrokes(move |event, window, cx| {
            if window.window_handle() == handle {
                let _ = weak.update(cx, |this, cx| {
                    if this.controller.recording() {
                        this.key(&event.keystroke.key, true, window, cx);
                    }
                });
            }
        }));
        let weak = cx.entity().downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            let _ = weak.update(cx, |this, cx| {
                this.dispatch(Command::App(Action::Close, String::new()), window, cx)
            });
            false
        });
        subscriptions
            .push(cx.observe_window_bounds(window, |this, window, _| this.save_geometry(window)));
        let pump = cx.spawn_in(window, async move |this, cx| {
            loop {
                let delay = this.update_in(cx, |this, window, cx| {
                    #[cfg(windows)]
                    this.controller.overlay.tick();
                    if this.last_tick.elapsed() >= Duration::from_millis(50) {
                        this.pull_inputs(cx);
                        let before = this.ui.clone();
                        for event in desktop::events() {
                            let action = match event {
                                DesktopEvent::Show => Action::Show,
                                DesktopEvent::Toggle => Action::Listen,
                                DesktopEvent::Quit => Action::Quit,
                                DesktopEvent::Resume => Action::Resume,
                            };
                            if let Err(error) = this.controller.action(&mut this.ui, action, "") {
                                this.controller.command_error(&error);
                            }
                        }
                        this.controller.tick(&mut this.ui);
                        this.last_tick = Instant::now();
                        this.effects(window, cx);
                        if this.ui != before {
                            cx.notify();
                        }
                    }
                    #[cfg(windows)]
                    if this.controller.overlay.is_visible() {
                        return Duration::from_millis(16);
                    }
                    Duration::from_millis(50)
                });
                let Ok(delay) = delay else { break };
                cx.background_executor().timer(delay).await;
            }
        });
        let mut ui = Presentation {
            geometry,
            ..Default::default()
        };
        ui.binding.capture.slot = -1;
        ui.mapping.capture_slot = -1;
        controller.sync(&mut ui);
        let mut this = Self {
            controller,
            ui,
            focus: cx.focus_handle(),
            capture_focus: None,
            sidebar_collapsed: false,
            content_scroll: ScrollHandle::new(),
            scroll_region: std::cell::Cell::new(""),
            app_icon: components::AppIcon::default(),
            group_name,
            mapping_name,
            output_filter,
            report_rate,
            report_rate_auto_label: String::new(),
            scope_pickers: BTreeMap::new(),
            ripples: Default::default(),
            dialog: 0,
            group_focus_request: 0,
            mapping_focus_request: 0,
            last_progress_message: String::new(),
            last_tick: Instant::now() - Duration::from_millis(50),
            _subscriptions: subscriptions,
            _pump: pump,
        };
        this.focus.focus(window, cx);
        this.effects(window, cx);
        this
    }

    fn tr(&self, key: &str) -> String {
        self.controller.tr(key)
    }

    fn tr_object(&self, key: &str, object: &str) -> String {
        rust_i18n::t!(key, locale = self.ui.locale.as_str(), object = object).into_owned()
    }

    fn dispatch(&mut self, command: Command, window: &mut Window, cx: &mut Context<Self>) {
        self.pull_inputs(cx);
        self.controller.runtime.poll();
        self.save_geometry(window);
        self.controller.runtime.consume_ui_input();
        let result = match &command {
            Command::App(action, value) => self.controller.action(&mut self.ui, *action, value),
            Command::Binding(command) => self
                .controller
                .binding_action(&mut self.ui, command.clone()),
            Command::Group(command, value) => {
                self.controller.group_command(&mut self.ui, command, value)
            }
            Command::Mapping(command, value) => {
                self.controller
                    .mapping_command(&mut self.ui, command, value)
            }
            Command::Scope(id, groups) => {
                let mut rules = self.controller.runtime.config.foreground_app_rules.clone();
                if groups.is_empty() {
                    rules.assignments.remove(id);
                } else {
                    rules.assignments.insert(*id, groups.clone());
                }
                self.controller
                    .save_foreground_app_rules(&mut self.ui, rules)
            }
        };
        if let Err(error) = result {
            match command {
                Command::Group(..)
                    if self.ui.groups.editor_open || self.ui.groups.confirming_delete =>
                {
                    self.ui.groups.form_error = error.to_string()
                }
                Command::Group(..) | Command::Scope(..) => self.ui.groups.error = error.to_string(),
                Command::Mapping(..) => self.ui.mapping.error = error.to_string(),
                _ => self.controller.command_error(&error),
            }
        }
        self.controller.sync(&mut self.ui);
        self.effects(window, cx);
        cx.notify();
    }

    fn pull_inputs(&mut self, cx: &App) {
        if self.ui.groups.editor_open {
            self.ui.groups.selected_name = self.group_name.read(cx).value().to_string();
        }
        if self.ui.mapping.editor {
            self.ui.mapping.name = self.mapping_name.read(cx).value().to_string();
        }
    }
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut content = div()
            .v_flex()
            .h_full()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .gap(rems(1.125))
            .p_7()
            .bg(cx.theme().background)
            .rounded_tl(cx.theme().radius_tokens().xl);
        if self.ui.mode == 2 {
            content = content.child(self.page_header(cx));
        }
        let body = match self.ui.mode {
            0 | 3 => self.environment(cx),
            1 => self.wizard(window, cx),
            _ => match self.ui.page {
                0 => self.overview(window, cx),
                1 if self.ui.mapping.open => self.mappings(cx),
                1 => self.bindings(window, cx),
                2 => self.devices(cx),
                3 => self.settings(cx),
                _ => self.groups(cx),
            },
        };
        content = content.child(body);
        let root = div()
            .id("taprelay")
            .track_focus(&self.focus)
            .size_full()
            .v_flex()
            .relative()
            .items_stretch()
            .bg(cx.theme().sidebar)
            .text_color(cx.theme().foreground)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if !this.controller.recording() {
                    this.key(&event.keystroke.key, true, window, cx);
                }
            }))
            .on_key_up(cx.listener(|this, event: &KeyUpEvent, window, cx| {
                this.key(&event.keystroke.key, false, window, cx)
            }));
        let mut workspace = div()
            .id("workspace")
            .h_flex()
            .items_stretch()
            .flex_1()
            .min_h_0()
            .test_support();
        if self.ui.mode == 2 {
            workspace = workspace.child(self.navigation(window, cx));
        }
        root.child(self.title_bar(window, cx))
            .child(workspace.child(content))
    }
}

#[cfg(test)]
mod tests;
