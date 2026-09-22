use std::time::{Duration, Instant};

pub(super) struct Publication {
    enabled: bool,
    restart: bool,
    retry_at: Instant,
    pub selection: Option<String>,
}

impl Publication {
    pub fn new(enabled: bool, now: Instant) -> Self {
        Self {
            enabled,
            restart: enabled,
            retry_at: now,
            selection: None,
        }
    }

    pub fn due(&self, has_service: bool, adapter_available: bool, now: Instant) -> bool {
        self.enabled
            && adapter_available
            && (self.restart || (!has_service && now >= self.retry_at))
    }

    pub fn attempted(&mut self, now: Instant) {
        self.restart = false;
        self.retry_at = now + Duration::from_secs(5);
    }

    pub fn adapter_changed(&mut self) {
        self.restart = self.enabled;
    }

    pub fn select(&mut self, id: String) {
        self.restart = self.enabled;
        self.selection = Some(id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_only_worker_never_publishes_after_adapter_recovery() {
        let now = Instant::now();
        let mut publication = Publication::new(false, now);
        publication.attempted(now);
        publication.adapter_changed();
        publication.select("phone".into());
        assert!(!publication.due(false, true, now + Duration::from_secs(60)));
    }

    #[test]
    fn publication_retry_retains_target_after_failure() {
        let now = Instant::now();
        let mut publication = Publication::new(true, now);
        publication.select("phone".into());
        assert!(!publication.due(false, false, now));
        assert!(publication.due(false, true, now));
        publication.attempted(now);
        assert!(!publication.due(false, true, now));
        assert!(publication.due(false, true, now + Duration::from_secs(5)));
        assert_eq!(publication.selection.as_deref(), Some("phone"));
    }
}
