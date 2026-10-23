use super::*;

use crate::platform::foreground_apps::RunningApp;

pub(in crate::gui) enum Shell {
    Native(Desktop),
    #[cfg(test)]
    Headless {
        autostart: bool,
        running_apps: Vec<String>,
        notifications: std::cell::RefCell<Vec<(String, String)>>,
        bluetooth_settings_opened: bool,
        login_status: Option<desktop::LoginStatus>,
        login_settings_opened: bool,
        notification_permission: desktop::NotificationPermission,
        notification_settings_opened: bool,
    },
}

impl Shell {
    pub(in crate::gui) fn notification_permission(&self) -> desktop::NotificationPermission {
        match self {
            Self::Native(desktop) => desktop.notification_permission(),
            #[cfg(test)]
            Self::Headless {
                notification_permission,
                ..
            } => *notification_permission,
        }
    }

    pub(in crate::gui) fn notification_settings(&mut self) -> Result<()> {
        match self {
            Self::Native(_) => desktop::open_settings(desktop::SettingsTarget::Notifications),
            #[cfg(test)]
            Self::Headless {
                notification_settings_opened,
                ..
            } => {
                *notification_settings_opened = true;
                Ok(())
            }
        }
    }
    pub(in crate::gui) fn permission_status(
        &self,
    ) -> (desktop::InputPermissions, desktop::BluetoothPermission) {
        match self {
            Self::Native(_) => desktop::permission_status(),
            #[cfg(test)]
            Self::Headless { .. } => (
                desktop::InputPermissions {
                    accessibility: true,
                    monitoring: true,
                    posting: true,
                },
                desktop::BluetoothPermission::Allowed,
            ),
        }
    }
    pub(in crate::gui) fn login_status(&self) -> desktop::LoginStatus {
        match self {
            Self::Native(_) => desktop::login_status(),
            #[cfg(test)]
            Self::Headless {
                autostart,
                login_status,
                ..
            } => login_status.unwrap_or(if *autostart {
                desktop::LoginStatus::Enabled
            } else {
                desktop::LoginStatus::Disabled
            }),
        }
    }

    pub(in crate::gui) fn login_settings(&mut self) -> Result<()> {
        match self {
            Self::Native(_) => desktop::open_settings(desktop::SettingsTarget::LoginItems),
            #[cfg(test)]
            Self::Headless {
                login_settings_opened,
                ..
            } => {
                *login_settings_opened = true;
                Ok(())
            }
        }
    }
    pub(in crate::gui) fn update(
        &mut self,
        state: u8,
        tip: &str,
        listening: bool,
        labels: [String; 4],
    ) {
        match self {
            Self::Native(desktop) => desktop.update(state, tip, listening, labels),
            #[cfg(test)]
            Self::Headless { .. } => {}
        }
    }
    pub(in crate::gui) fn notify(&self, title: &str, body: &str) {
        match self {
            Self::Native(desktop) => desktop.notify(title, body),
            #[cfg(test)]
            Self::Headless { notifications, .. } => {
                notifications.borrow_mut().push((title.into(), body.into()))
            }
        }
    }

    pub(in crate::gui) fn running_apps(&self) -> Result<Vec<RunningApp>> {
        match self {
            Self::Native(_) => Ok(crate::platform::foreground_apps::running()?),
            #[cfg(test)]
            Self::Headless { running_apps, .. } => Ok(running_apps
                .iter()
                .map(|identity| RunningApp {
                    identity: identity.clone(),
                    name: None,
                    path: None,
                })
                .collect()),
        }
    }

    pub(in crate::gui) fn bluetooth_settings(&mut self) -> Result<()> {
        match self {
            Self::Native(_) => desktop::open_settings(desktop::SettingsTarget::Bluetooth),
            #[cfg(test)]
            Self::Headless {
                bluetooth_settings_opened,
                ..
            } => {
                *bluetooth_settings_opened = true;
                Ok(())
            }
        }
    }

    pub(in crate::gui) fn set_autostart(&mut self, enabled: bool) -> Result<bool> {
        match self {
            Self::Native(_) => {
                desktop::set_autostart(enabled)?;
                Ok(desktop::autostart_enabled())
            }
            #[cfg(test)]
            Self::Headless {
                autostart,
                login_status,
                ..
            } => {
                *autostart = enabled;
                {
                    if !enabled {
                        *login_status = None;
                    }
                    Ok(login_status.unwrap_or(if enabled {
                        desktop::LoginStatus::Enabled
                    } else {
                        desktop::LoginStatus::Disabled
                    }) == desktop::LoginStatus::Enabled)
                }
            }
        }
    }
}
