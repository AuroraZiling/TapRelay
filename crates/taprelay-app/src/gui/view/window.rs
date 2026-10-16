use super::components::LiveElement;
use super::{AppView, Command};
use crate::{
    config::Geometry, gui::presentation::WindowCommand, platform::desktop,
    runtime::commands::BindingCommand,
};
use gpui_kit::base::TestSupportExt;
use gpui_kit::component::{ActiveTheme, Theme, ThemeMode, WindowExt, notification::Notification};
use gpui_kit::component::{
    StyledExt,
    button::{Button, ButtonVariants},
};
use gpui_kit::{prelude::*, *};
use std::time::Instant;

struct FeedbackNotification;
struct ProgressNotification;

fn notification(message: String, close_label: String) -> Notification {
    Notification::new().content(move |_, _, cx| {
        div()
            .h_flex()
            .items_start()
            .gap_3()
            .child(
                div()
                    .id("notification-message")
                    .role(Role::Status)
                    .aria_label(message.clone())
                    .flex_1()
                    .min_w_0()
                    .text_sm()
                    .child(message.clone())
                    .live(accesskit::Live::Polite)
                    .test_support(),
            )
            .child(
                Button::new("notification-dismiss")
                    .ghost()
                    .icon(gpui_kit::assets::IconName::Close)
                    .accessibility_label(close_label.clone())
                    .on_click(cx.listener(|this, _, window, cx| this.dismiss(window, cx))),
            )
            .into_any_element()
    })
}

impl AppView {
    pub(super) fn update_geometry(&mut self, window: &Window) {
        if self.controller.hidden || !window.is_visible() {
            return;
        }
        self.ui.geometry.maximized = window.is_maximized();
        if !window.is_maximized() {
            let WindowBounds::Windowed(bounds) = window.window_bounds() else {
                return;
            };
            if bounds.size.width >= px(Geometry::MIN_WIDTH)
                && bounds.size.height >= px(Geometry::MIN_HEIGHT)
            {
                self.ui.geometry.width = bounds.size.width.into();
                self.ui.geometry.height = bounds.size.height.into();
                let scale = window.scale_factor();
                self.ui.geometry.x = Some((f32::from(bounds.origin.x) * scale).round() as i32);
                self.ui.geometry.y = Some((f32::from(bounds.origin.y) * scale).round() as i32);
            }
        }
    }

    pub(super) fn effects(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mode = if self.ui.dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        };
        if Theme::global(cx).mode != mode {
            Theme::change(mode, Some(window), cx);
        }
        gpui_kit::component::set_locale(&self.ui.locale);
        self.sync_report_rate(window, cx);
        self.sync_scope_pickers(window, cx);
        if let Some(message) = self.controller.pending_notification.take() {
            let close_label = self.tr("a11y.dismiss_notification");
            window.defer(cx, move |window, cx| {
                window.push_notification(
                    notification(message, close_label).id::<FeedbackNotification>(),
                    cx,
                );
            });
        }
        if self.last_progress_message != self.ui.progress_message {
            self.last_progress_message = self.ui.progress_message.clone();
            let message = self.last_progress_message.clone();
            let close_label = self.tr("a11y.dismiss_notification");
            window.defer(cx, move |window, cx| {
                if message.is_empty() {
                    window.remove_notification::<ProgressNotification>(cx);
                } else {
                    window.push_notification(
                        notification(message, close_label)
                            .id::<ProgressNotification>()
                            .autohide(false),
                        cx,
                    );
                }
            });
        }
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

    pub(super) fn key(
        &mut self,
        key: &str,
        down: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        if key == "escape" && down {
            self.dispatch(Command::Binding(BindingCommand::CancelCapture), window, cx);
        } else if let Some(code) = crate::platform::capture_key::virtual_key(key) {
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
}

#[cfg(windows)]
fn native_handle(window: &Window) -> Option<isize> {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    match HasWindowHandle::window_handle(window).ok()?.as_raw() {
        RawWindowHandle::Win32(handle) => Some(handle.hwnd.get()),
        _ => None,
    }
}
