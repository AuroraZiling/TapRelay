//! User-visible asynchronous states, independent of GUI widgets and wall-clock time.
use std::time::{Duration, Instant};

#[derive(Debug, Default, PartialEq, Eq)]
pub enum TestStatus {
    #[default]
    Idle,
    Pending(u64),
    Succeeded,
    Failed(String),
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
