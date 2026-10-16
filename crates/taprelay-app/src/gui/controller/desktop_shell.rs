use super::*;

pub(in crate::gui) enum Shell {
    Native(Desktop),
    #[cfg(test)]
    Headless {
        autostart: bool,
        running_apps: Vec<String>,
        notifications: std::cell::RefCell<Vec<(String, String)>>,
        bluetooth_settings_opened: bool,
    },
}

impl Shell {
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

    pub(in crate::gui) fn running_apps(&self) -> Result<Vec<String>> {
        match self {
            Self::Native(_) => Ok(crate::platform::foreground_apps::running()?),
            #[cfg(test)]
            Self::Headless { running_apps, .. } => Ok(running_apps.clone()),
        }
    }

    pub(in crate::gui) fn bluetooth_settings(&mut self) -> Result<()> {
        match self {
            Self::Native(_) => Ok(desktop::open("ms-settings:bluetooth")?),
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
            Self::Headless { autostart, .. } => {
                *autostart = enabled;
                Ok(*autostart)
            }
        }
    }
}
