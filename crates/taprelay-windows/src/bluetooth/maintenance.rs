use std::time::{Duration, Instant};

/// Shared schedule for synchronization and idle neutral reports. Session
/// flapping must not reset the failure budget; only success or user selection does.
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
        self.failures < 6 && now >= self.next
    }
    pub fn success(&mut self, now: Instant) {
        self.failures = 0;
        self.next = now + Duration::from_secs(3);
    }
    pub fn failure(&mut self, now: Instant) {
        self.failures = self.failures.saturating_add(1);
        self.next =
            now + Duration::from_secs((3u64 << self.failures.min(5).saturating_sub(1)).min(60));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn backoff_stops_and_explicit_reset_recovers() {
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
        assert!(!policy.due(now + Duration::from_secs(3600)));
        policy = Maintenance::new(now);
        assert!(policy.due(now));
        policy.success(now);
        assert!(!policy.due(now + Duration::from_secs(2)));
        assert!(policy.due(now + Duration::from_secs(3)));
        policy.success(now + Duration::from_secs(2));
        assert!(!policy.due(now + Duration::from_secs(3)));
    }
}
