use super::{
    Action, BindingCommand, CaptureTarget, Controller, DesktopEvent, FunctionId, Presentation,
    WindowCommand, config, desktop,
};
use gpui_kit::assets::IconName;
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{
    ActiveTheme, Disableable, Icon, Selectable, Sizable, StyledExt, Theme, ThemeMode, WindowExt,
    button::{Button, ButtonVariants},
    checkbox::Checkbox,
    group_box::{GroupBox, GroupBoxVariants},
    input::{Input, InputEvent, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    scroll::ScrollableElement,
    switch::Switch,
};
use gpui_kit::{prelude::*, *};
use std::time::{Duration, Instant};

mod editors;
mod pages;
mod shell;

#[derive(Clone)]
enum Command {
    App(Action, String),
    Binding(BindingCommand),
    Group(&'static str, String),
    Mapping(&'static str, String),
    Scope(FunctionId, String),
}

pub(super) struct AppView {
    controller: Controller,
    ui: Presentation,
    focus: FocusHandle,
    capture_focus: Option<FocusHandle>,
    group_name: Entity<InputState>,
    mapping_name: Entity<InputState>,
    output_filter: Entity<InputState>,
    ripples: super::ripple::Ripples,
    dialog: u8,
    group_focus_request: i32,
    mapping_focus_request: i32,
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
        let mut subscriptions = vec![
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
            group_name,
            mapping_name,
            output_filter,
            ripples: Default::default(),
            dialog: 0,
            group_focus_request: 0,
            mapping_focus_request: 0,
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
            Command::Scope(id, group) => {
                let mut rules = self.controller.runtime.config.foreground_app_rules.clone();
                rules.toggle_scope(*id, group);
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

    fn save_geometry(&mut self, window: &Window) {
        if self.controller.hidden || !window.is_visible() {
            return;
        }
        self.ui.geometry.maximized = window.is_maximized();
        if !window.is_maximized() {
            let WindowBounds::Windowed(bounds) = window.window_bounds() else {
                return;
            };
            if bounds.size.width >= px(900.) && bounds.size.height >= px(500.) {
                self.ui.geometry.width = bounds.size.width.into();
                self.ui.geometry.height = bounds.size.height.into();
                let scale = window.scale_factor();
                self.ui.geometry.x = Some((f32::from(bounds.origin.x) * scale).round() as i32);
                self.ui.geometry.y = Some((f32::from(bounds.origin.y) * scale).round() as i32);
            }
        }
        if self.controller.runtime.config.window != self.ui.geometry {
            self.controller.save_geometry(&mut self.ui);
        }
    }

    fn effects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = if self.ui.dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        };
        if Theme::global(cx).mode != mode {
            Theme::change(mode, Some(window), cx);
        }
        gpui_kit::component::set_locale(&self.ui.locale);
        let bytes = |c: Hsla| {
            let c = c.to_rgb();
            [(c.r * 255.) as u8, (c.g * 255.) as u8, (c.b * 255.) as u8]
        };
        self.ui.overlay_background = bytes(cx.theme().group_box);
        self.ui.overlay_foreground = bytes(cx.theme().foreground);
        for (input, value) in [
            (&self.group_name, &self.ui.groups.selected_name),
            (&self.mapping_name, &self.ui.mapping.name),
            (&self.output_filter, &self.ui.mapping.output_filter),
        ] {
            if input.read(cx).value().as_ref() != value.as_str() {
                input.update(cx, |input, cx| input.set_value(value.clone(), window, cx));
            }
        }
        for (input, key) in [
            (&self.group_name, "groups.name_example"),
            (&self.mapping_name, "mappings.name_example"),
        ] {
            let placeholder = self.tr(key);
            if input.read(cx).presentation().placeholder().as_ref() != placeholder.as_str() {
                input.update(cx, |input, cx| {
                    input.set_placeholder(placeholder, window, cx)
                });
            }
        }
        if self.ui.groups.focus_request != self.group_focus_request {
            self.group_focus_request = self.ui.groups.focus_request;
            if self.ui.groups.editor_open {
                self.group_name
                    .update(cx, |input, cx| input.focus(window, cx));
            }
        }
        if self.ui.mapping.focus_request != self.mapping_focus_request {
            self.mapping_focus_request = self.ui.mapping.focus_request;
            if self.ui.mapping.editor {
                self.mapping_name
                    .update(cx, |input, cx| input.focus(window, cx));
            }
        }
        if self.controller.recording() && self.capture_focus.is_none() {
            self.capture_focus = window.focused(cx);
            self.focus.focus(window, cx);
        } else if !self.controller.recording()
            && let Some(focus) = self.capture_focus.take()
        {
            focus.focus(window, cx);
        }
        self.update_dialog(window, cx);
        if let Some(command) = self.ui.window_command.take() {
            match command {
                WindowCommand::Quit => {
                    self.controller.save_geometry(&mut self.ui);
                    self.controller.flush(true);
                    cx.quit();
                }
                WindowCommand::Show | WindowCommand::Hide => {
                    #[cfg(windows)]
                    if let Some(handle) = native_handle(window) {
                        desktop::set_window_visible(handle, command == WindowCommand::Show);
                    }
                    if command == WindowCommand::Show {
                        window.activate_window();
                    }
                }
            }
        }
        let color = self.signal(cx);
        self.ripples.update(
            self.ui.input_count,
            [color.h, color.s, color.l, color.a],
            Instant::now(),
            !self.controller.hidden
                && window.is_visible()
                && !cx.reduce_motion()
                && ((self.ui.mode == 2 && self.ui.page == 0)
                    || (self.ui.mode == 1 && self.ui.wizard_page == 3)),
        );
    }

    fn key(&mut self, key: &str, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.controller.recording() {
            if key == "escape" && down && self.dialog == 0 {
                if self.ui.groups.editor_open && !self.ui.groups.saving {
                    self.dispatch(Command::Group("cancel", String::new()), window, cx);
                    cx.stop_propagation();
                } else if self.ui.mapping.editor && !self.ui.mapping.saving {
                    self.dispatch(Command::Mapping("back", String::new()), window, cx);
                    cx.stop_propagation();
                }
            }
            return;
        }
        if desktop::ui_input_is_injected() {
            return;
        }
        if key == "escape" && down && self.ui.mapping.capture_slot != 2 {
            self.dispatch(Command::Binding(BindingCommand::CancelCapture), window, cx);
        } else if let Some(code) = crate::capture_key::virtual_key(key) {
            self.controller
                .runtime
                .window_key(taprelay_core::input::InputEvent {
                    code: taprelay_core::input::InputCode::Key(code),
                    down,
                    captured: Instant::now(),
                });
        }
        cx.stop_propagation();
    }

    fn button(
        &self,
        id: impl Into<ElementId>,
        label: impl Into<SharedString>,
        command: Command,
        disabled: bool,
        cx: &Context<Self>,
    ) -> Button {
        Button::new(id).label(label).disabled(disabled).on_click(
            cx.listener(move |this, _, window, cx| this.dispatch(command.clone(), window, cx)),
        )
    }

    fn app_button(
        &self,
        id: &'static str,
        key: &str,
        action: Action,
        disabled: bool,
        cx: &Context<Self>,
    ) -> Button {
        self.button(
            id,
            self.tr(key),
            Command::App(action, String::new()),
            disabled,
            cx,
        )
    }

    fn signal(&self, cx: &App) -> Hsla {
        match self.ui.state_color {
            0 => cx.theme().success,
            1 => cx.theme().muted_foreground,
            2 => cx.theme().warning,
            _ => cx.theme().danger,
        }
    }

    fn caption(&self, text: impl Into<SharedString>, cx: &App) -> Div {
        div()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(text.into())
    }
    fn error(&self, text: &str, cx: &App) -> Div {
        div().text_color(cx.theme().danger).child(text.to_owned())
    }
    fn section(&self, key: &'static str) -> GroupBox {
        GroupBox::new().id(key).fill().title(self.tr(key))
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
            .p_7();
        if self.ui.mode == 2 {
            content = content.child(self.page_header(cx));
        }
        let body = match self.ui.mode {
            0 | 3 => self.environment(cx),
            1 => self.wizard(window, cx),
            _ => match self.ui.page {
                0 => self.overview(window, cx),
                1 if self.ui.mapping.open => self.mappings(cx),
                1 => self.bindings(cx),
                2 => self.devices(cx),
                3 => self.settings(cx),
                _ => self.groups(cx),
            },
        };
        content = content.child(body);
        let mut root = div()
            .id("taprelay")
            .track_focus(&self.focus)
            .size_full()
            .h_flex()
            .relative()
            .items_stretch()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if !this.controller.recording() {
                    this.key(&event.keystroke.key, true, window, cx);
                }
            }))
            .on_key_up(cx.listener(|this, event: &KeyUpEvent, window, cx| {
                this.key(&event.keystroke.key, false, window, cx)
            }));
        if self.ui.mode == 2 {
            root = root.child(self.navigation(cx));
        }
        root.child(content).when(!self.ui.toast.is_empty(), |root| {
            root.child(
                div()
                    .absolute()
                    .right_5()
                    .bottom_5()
                    .max_w(rems(27.5))
                    .p_4()
                    .rounded_lg()
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().group_box)
                    .child(self.ui.toast.clone()),
            )
        })
    }
}

#[cfg(windows)]
fn native_handle(window: &Window) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
