//! App-facing desktop contract
use super::native;
use anyhow::Result;
pub use native::desktop::{
    DesktopEvent, activate_existing, any_input_held, autostart_enabled, events, foreground_is_ours,
    open, set_autostart, show_error, system_dark, system_locale_id, virtual_key_for_character,
};
#[cfg(windows)]
pub use native::desktop::{display_at_position, set_window_visible, ui_input_is_injected};
#[cfg(target_os = "macos")]
pub use native::desktop::{
    request_close, request_quit, request_settings, request_show, set_app_window_visible,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Capabilities {
    pub privacy_permissions: bool,
    pub login_approval: bool,
    pub notification_permission: bool,
    pub elevation: bool,
    pub subscription_receivers: bool,
}
impl Default for Capabilities {
    fn default() -> Self {
        CAPABILITIES
    }
}
pub const CAPABILITIES: Capabilities = Capabilities {
    privacy_permissions: cfg!(target_os = "macos"),
    login_approval: cfg!(target_os = "macos"),
    notification_permission: cfg!(target_os = "macos"),
    elevation: cfg!(windows),
    subscription_receivers: cfg!(target_os = "macos"),
};
pub use taprelay_core::ports::{
    BluetoothPermission, InputPermissions, LoginStatus, NotificationPermission,
};
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// The shared UI describes macOS-only states even on Windows.
#[cfg_attr(windows, allow(dead_code))]
pub enum SettingsTarget {
    Bluetooth,
    Accessibility,
    InputMonitoring,
    BluetoothPermission,
    Notifications,
    LoginItems,
}

pub fn open_settings(target: SettingsTarget) -> Result<()> {
    #[cfg(target_os = "macos")]
    {
        if target == SettingsTarget::LoginItems {
            return Ok(native::desktop::open_login_settings()?);
        }
        Ok(open(match target {
            SettingsTarget::Bluetooth => "x-apple.systempreferences:com.apple.BluetoothSettings",
            SettingsTarget::Accessibility => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Accessibility"
            }
            SettingsTarget::InputMonitoring => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_ListenEvent"
            }
            SettingsTarget::BluetoothPermission => {
                "x-apple.systempreferences:com.apple.preference.security?Privacy_Bluetooth"
            }
            SettingsTarget::Notifications => {
                "x-apple.systempreferences:com.apple.Notifications-Settings.extension"
            }
            SettingsTarget::LoginItems => unreachable!(),
        })?)
    }
    #[cfg(windows)]
    match target {
        SettingsTarget::Bluetooth => Ok(open("ms-settings:bluetooth")?),
        _ => anyhow::bail!("This system setting is unavailable on Windows"),
    }
}

pub fn permission_status() -> (InputPermissions, BluetoothPermission) {
    #[cfg(target_os = "macos")]
    {
        (
            native::input::permission_status(),
            native::desktop::bluetooth_permission(),
        )
    }
    #[cfg(windows)]
    {
        (
            InputPermissions {
                accessibility: true,
                monitoring: true,
                posting: true,
            },
            BluetoothPermission::Unknown,
        )
    }
}
pub fn login_status() -> LoginStatus {
    #[cfg(target_os = "macos")]
    {
        native::desktop::login_status()
    }
    #[cfg(windows)]
    {
        if autostart_enabled() {
            LoginStatus::Enabled
        } else {
            LoginStatus::Disabled
        }
    }
}
pub struct Desktop(native::desktop::Desktop);
impl Desktop {
    pub fn new(labels: [String; 4]) -> Result<Self> {
        Ok(Self(native::desktop::Desktop::new(labels)?))
    }
    pub fn update(&mut self, state: u8, tip: &str, listening: bool, labels: [String; 4]) {
        self.0.update(state, tip, listening, labels);
    }
    pub fn notify(&self, title: &str, body: &str) {
        self.0.notify(title, body);
    }
    pub fn notification_permission(&self) -> NotificationPermission {
        #[cfg(target_os = "macos")]
        {
            self.0.notification_permission()
        }
        #[cfg(windows)]
        {
            NotificationPermission::Unavailable
        }
    }
}

pub struct Overlay(native::overlay::Overlay);
impl Overlay {
    pub fn new() -> Result<Self> {
        Ok(Self(native::overlay::Overlay::new()?))
    }
    pub fn update(
        &mut self,
        prefix: &str,
        device: &str,
        suffix: &str,
        background: [u8; 3],
        foreground: [u8; 3],
        duration: Option<std::time::Duration>,
    ) -> Result<()> {
        Ok(self
            .0
            .update(prefix, device, suffix, background, foreground, duration)?)
    }
    pub fn hide(&mut self) {
        self.0.hide();
    }
    pub fn is_visible(&self) -> bool {
        self.0.is_visible()
    }
    pub fn tick(&mut self) -> Result<()> {
        Ok(self.0.tick()?)
    }
}
