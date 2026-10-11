use std::time::{Duration, Instant};

/// Shared schedule for synchronization and idle neutral reports. Session
/// flapping must not reset backoff; only success or user selection does.
pub(super) struct Maintenance {
    pub failures: u32,
    next: Instant,
}
impl Maintenance {
    pub fn new(now: Instant) -> Self {
        Self {
            failures: 0,
            next: now,
        }
    }
    pub fn due(&self, now: Instant) -> bool {
        now >= self.next
    }
    pub fn refresh_due(&self, now: Instant) -> bool {
        self.failures == 0 || self.due(now)
    }
    pub fn success(&mut self, now: Instant) {
        self.failures = 0;
        self.next = now + Duration::from_secs(3);
    }
    pub fn failure(&mut self, now: Instant) {
        self.failures = self.failures.saturating_add(1);
        self.next =
            now + Duration::from_secs((3u64 << self.failures.saturating_sub(1).min(5)).min(60));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn receiver_can_recover_after_a_long_outage() {
        let mut now = Instant::now();
        let mut policy = Maintenance::new(now);
        for _ in 0..12 {
            assert!(policy.due(now), "recovery must not permanently stop");
            policy.failure(now);
            now += Duration::from_secs(60);
        }
        policy.success(now);
        assert_eq!(policy.failures, 0);
        assert!(policy.due(now + Duration::from_secs(3)));
    }
    #[test]
    fn failed_native_refresh_obeys_backoff_instead_of_restarting_it_every_poll() {
        let mut now = Instant::now();
        let mut policy = Maintenance::new(now);
        for delay in [3, 6, 12, 24, 48, 60, 60] {
            assert!(policy.refresh_due(now));
            policy.failure(now);
            for tick in 1..delay * 10 {
                assert!(!policy.refresh_due(now + Duration::from_millis(tick * 100)));
            }
            now += Duration::from_secs(delay);
        }
        assert!(policy.refresh_due(now));
        policy.success(now);
        assert!(policy.refresh_due(now + Duration::from_millis(100)));
    }
    #[test]
    fn backoff_is_bounded_and_explicit_reset_recovers() {
        let mut now = Instant::now();
        let mut policy = Maintenance::new(now);
        for delay in [3, 6, 12, 24, 48] {
            assert!(policy.due(now));
            policy.failure(now);
            assert!(!policy.due(now + Duration::from_secs(delay - 1)));
            now += Duration::from_secs(delay);
        }
        assert!(policy.due(now));
        policy.failure(now);
        assert!(!policy.refresh_due(now + Duration::from_secs(59)));
        assert!(policy.due(now + Duration::from_secs(60)));
        policy = Maintenance::new(now);
        assert!(policy.due(now));
        policy.success(now);
        assert!(policy.refresh_due(now));
        assert!(!policy.due(now + Duration::from_secs(2)));
        assert!(policy.due(now + Duration::from_secs(3)));
        policy.success(now + Duration::from_secs(2));
        assert!(!policy.due(now + Duration::from_secs(3)));
    }
}
