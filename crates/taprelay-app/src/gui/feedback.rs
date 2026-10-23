//! User-visible asynchronous states, independent of GUI widgets and wall-clock time.
use std::time::{Duration, Instant};

pub fn operation_error_key(message: &str) -> &'static str {
    use crate::i18n::keys;
    match message {
        #[cfg(target_os = "macos")]
        taprelay_macos::input::MISSING_PHYSICAL_KEYBOARD => {
            keys::ERROR_PASSTHROUGH_KEYBOARD_MISSING
        }
        #[cfg(target_os = "macos")]
        taprelay_macos::input::MISSING_RELATIVE_MOUSE => keys::ERROR_PASSTHROUGH_MOUSE_MISSING,
        _ => keys::ERROR_OPERATION,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ErrorScope {
    Connection,
    Startup,
    Runtime,
}

impl ErrorScope {
    pub fn summary_key(self, state: &taprelay_core::state::Snapshot) -> &'static str {
        use crate::i18n::keys;
        use taprelay_core::devices::{AdapterState, DeviceError};
        match self {
            Self::Connection if state.adapter_state == AdapterState::Disabled => {
                keys::RECEIVER_ADAPTER_OFF
            }
            Self::Connection if state.adapter_state == AdapterState::Unavailable => {
                keys::RECEIVER_ADAPTER_MISSING
            }
            Self::Connection if state.hid_suspended => keys::RECEIVER_SUSPENDED,
            Self::Connection if state.device_error == Some(DeviceError::PairingLaunchFailed) => {
                keys::RECEIVER_PAIR_FAILED
            }
            Self::Connection => keys::ERROR_CONNECTION,
            Self::Startup => keys::ERROR_STARTUP,
            Self::Runtime => keys::RUNTIME_STOPPED,
        }
    }

    pub fn can_retry(self) -> bool {
        self == Self::Connection
    }
}

#[derive(Default)]
pub struct WaitWarning {
    target: Option<String>,
    since: Option<Instant>,
    notified: bool,
}
impl WaitWarning {
    /// Stage changes do not restart a continuous outage. Disabled notifications
    /// do not consume the one notification owed for this outage.
    pub fn update(
        &mut self,
        target: Option<&str>,
        ready: bool,
        enabled: bool,
        now: Instant,
    ) -> bool {
        if self.target.as_deref() != target {
            self.target = target.map(str::to_owned);
            self.since = None;
            self.notified = false;
        }
        if ready || target.is_none() {
            self.since = None;
            self.notified = false;
            return false;
        }
        let since = *self.since.get_or_insert(now);
        if enabled
            && !self.notified
            && now.saturating_duration_since(since) >= Duration::from_secs(60)
        {
            self.notified = true;
            return true;
        }
        false
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn outage_timer_does_not_inherit_ready_or_unselected_time() {
        let now = Instant::now();
        let mut warning = WaitWarning::default();
        assert!(!warning.update(None, false, true, now));
        assert!(!warning.update(Some("a"), true, true, now + Duration::from_secs(100)));
        assert!(!warning.update(Some("a"), false, true, now + Duration::from_secs(200)));
        assert!(!warning.update(Some("a"), false, true, now + Duration::from_secs(259)));
        assert!(warning.update(Some("a"), false, true, now + Duration::from_secs(260)));
        assert!(!warning.update(Some("a"), false, true, now + Duration::from_secs(300)));
        assert!(!warning.update(Some("b"), false, true, now + Duration::from_secs(301)));
    }
    #[test]
    fn continuous_wait_is_not_consumed_while_notifications_are_disabled() {
        let now = Instant::now();
        let mut warning = WaitWarning::default();
        assert!(!warning.update(Some("a"), false, false, now));
        assert!(!warning.update(Some("a"), false, false, now + Duration::from_secs(80)));
        assert!(warning.update(Some("a"), false, true, now + Duration::from_secs(81)));
    }
}
