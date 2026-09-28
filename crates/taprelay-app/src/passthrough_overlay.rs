#[cfg(any(windows, test))]
use std::time::Duration;
use taprelay_core::state::{Snapshot, Target};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Controlling,
    Stopped,
    Disconnected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Notice {
    pub status: Status,
    pub device: String,
}

#[cfg(any(windows, test))]
impl Notice {
    pub fn duration(&self) -> Duration {
        Duration::from_secs(if self.status == Status::Disconnected {
            5
        } else {
            3
        })
    }

    pub fn text_parts(&self, locale: &str) -> (String, String, String) {
        use crate::i18n::{self, keys};
        let key = match self.status {
            Status::Controlling => keys::OVERLAY_CONTROLLING,
            Status::Stopped => keys::OVERLAY_STOPPED,
            Status::Disconnected => keys::OVERLAY_DISCONNECTED,
        };
        let template = i18n::text(locale, key);
        let (prefix, suffix) = template.split_once("{device}").expect("device placeholder");
        let device = if self.device.trim().is_empty() {
            i18n::text(locale, keys::OVERLAY_UNKNOWN_DEVICE)
        } else {
            // Device names are external input; keep the capsule on one line.
            self.device.chars().filter(|c| !c.is_control()).collect()
        };
        (prefix.into(), device, suffix.into())
    }
}

#[derive(Default)]
pub struct Tracker {
    active: Option<(Target, u64)>,
}

impl Tracker {
    pub fn update(
        &mut self,
        state: &Snapshot,
        epoch: u64,
        disconnected_epoch: u64,
    ) -> Option<Notice> {
        let active = state
            .selected_target()
            .filter(|_| epoch != 0 && state.ready);
        if let Some(target) = active {
            let changed = self
                .active
                .as_ref()
                .is_none_or(|(old, old_epoch)| !old.same_device(target) || *old_epoch != epoch);
            self.active = Some((target.clone(), epoch));
            return changed.then(|| Notice {
                status: Status::Controlling,
                device: target.name.clone(),
            });
        }
        let (previous, previous_epoch) = self.active.take()?;
        let switched = state
            .selected
            .as_deref()
            .is_some_and(|id| !previous.matches_id(id));
        let disconnected = disconnected_epoch == previous_epoch
            || (!state.ready && !state.profile_switching && !switched);
        Some(Notice {
            status: if disconnected {
                Status::Disconnected
            } else {
                Status::Stopped
            },
            device: previous.name,
        })
    }
}

#[cfg(windows)]
#[derive(Default)]
pub struct Overlay {
    native: std::rc::Rc<std::cell::RefCell<Option<taprelay_windows::overlay::Overlay>>>,
    timer: std::rc::Rc<slint::Timer>,
    notice: Option<Notice>,
    appearance: Option<(String, [u8; 3], [u8; 3])>,
}

#[cfg(windows)]
impl Overlay {
    pub fn update(
        &mut self,
        notice: Option<Notice>,
        enabled: bool,
        locale: &str,
        background: [u8; 3],
        foreground: [u8; 3],
    ) {
        let mut native = self.native.borrow_mut();
        if !enabled {
            if let Some(native) = native.as_mut() {
                native.hide();
            }
            self.timer.stop();
            self.notice = None;
            return;
        }
        let changed = notice.is_some();
        if let Some(notice) = notice {
            self.notice = Some(notice);
        }
        let appearance = (locale.to_owned(), background, foreground);
        let restyle = self.appearance.as_ref() != Some(&appearance);
        self.appearance = Some(appearance);
        let result = (|| -> anyhow::Result<()> {
            if (changed || restyle)
                && let Some(notice) = &self.notice
            {
                if native.is_none() {
                    *native = Some(taprelay_windows::overlay::Overlay::new()?);
                }
                let (prefix, device, suffix) = notice.text_parts(locale);
                native.as_mut().unwrap().update(
                    &prefix,
                    &device,
                    &suffix,
                    background,
                    foreground,
                    changed.then_some(notice.duration()),
                )?;
            }
            Ok(())
        })();
        if let Err(error) = result {
            tracing::warn!(%error, "Passthrough overlay unavailable");
            *native = None;
            self.notice = None;
        }
        if native.as_ref().is_some_and(|native| native.is_visible()) && !self.timer.running() {
            let native = self.native.clone();
            let timer = std::rc::Rc::downgrade(&self.timer);
            self.timer.start(
                slint::TimerMode::Repeated,
                Duration::from_millis(16),
                move || {
                    let mut native = native.borrow_mut();
                    if let Some(window) = native.as_mut()
                        && let Err(error) = window.tick()
                    {
                        tracing::warn!(%error, "Passthrough overlay animation failed");
                        *native = None;
                    }
                    if native.as_ref().is_none_or(|window| !window.is_visible())
                        && let Some(timer) = timer.upgrade()
                    {
                        timer.stop();
                    }
                },
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn connected() -> Snapshot {
        Snapshot {
            ready: true,
            selected: Some("a".into()),
            targets: vec![Target {
                id: "a".into(),
                name: "iPad".into(),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    #[test]
    fn connection_and_pending_request_do_not_imply_control() {
        let mut tracker = Tracker::default();
        let mut state = connected();
        assert_eq!(tracker.update(&state, 0, 0), None);
        state.profile_switching = true;
        assert_eq!(tracker.update(&state, 0, 0), None);
        state.ready = false;
        assert_eq!(tracker.update(&state, 0, 0), None);
    }

    #[test]
    fn enter_exit_and_disconnect_preserve_actual_device() {
        let mut tracker = Tracker::default();
        let mut state = connected();
        assert_eq!(
            tracker.update(&state, 2, 0).unwrap().status,
            Status::Controlling
        );
        assert_eq!(tracker.update(&state, 2, 0), None);
        let stopped = tracker.update(&state, 0, 0).unwrap();
        assert_eq!(stopped.status, Status::Stopped);
        assert_eq!(stopped.duration(), Duration::from_secs(3));
        assert_eq!(tracker.update(&state, 0, 0), None);
        tracker.update(&state, 4, 0);
        state.ready = false;
        state.targets.clear();
        state.selected = None;
        let lost = tracker.update(&state, 0, 0).unwrap();
        assert_eq!(lost.status, Status::Disconnected);
        assert_eq!(lost.device, "iPad");
        assert_eq!(lost.duration(), Duration::from_secs(5));
        assert_eq!(tracker.update(&state, 0, 0), None);
    }

    #[test]
    fn switch_does_not_claim_new_device_is_disconnected() {
        let mut tracker = Tracker::default();
        let mut state = connected();
        tracker.update(&state, 2, 0);
        state.ready = false;
        state.selected = Some("b".into());
        let stopped = tracker.update(&state, 0, 0).unwrap();
        assert_eq!(stopped.status, Status::Stopped);
        assert_eq!(stopped.device, "iPad");
        state.targets.push(Target {
            id: "b".into(),
            name: "Phone".into(),
            ..Default::default()
        });
        state.ready = true;
        assert_eq!(tracker.update(&state, 4, 0).unwrap().device, "Phone");
    }

    #[test]
    fn a_new_epoch_refreshes_control_even_when_ui_missed_the_exit() {
        let mut tracker = Tracker::default();
        let state = connected();
        tracker.update(&state, 2, 0);
        assert_eq!(
            tracker.update(&state, 4, 0).unwrap().status,
            Status::Controlling
        );
    }

    #[test]
    fn disconnect_remains_visible_after_automatic_profile_recovery() {
        let mut tracker = Tracker::default();
        let mut state = connected();
        tracker.update(&state, 2, 0);
        state.ready = false;
        state.profile_switching = true;
        assert_eq!(
            tracker.update(&state, 0, 2).unwrap().status,
            Status::Disconnected
        );
        state.ready = true;
        state.profile_switching = false;
        tracker.update(&state, 4, 2);
        assert_eq!(
            tracker.update(&state, 0, 2).unwrap().status,
            Status::Stopped
        );
    }

    #[test]
    fn translated_device_placeholder_keeps_status_suffix_separate() {
        let notice = Notice {
            status: Status::Disconnected,
            device: "Artemis\niPad".into(),
        };
        assert_eq!(
            notice.text_parts("zh-cn"),
            ("已断开与 ".into(), "ArtemisiPad".into(), " 的连接".into())
        );
        assert_eq!(
            notice.text_parts("en"),
            ("Disconnected from ".into(), "ArtemisiPad".into(), "".into())
        );
    }
}
