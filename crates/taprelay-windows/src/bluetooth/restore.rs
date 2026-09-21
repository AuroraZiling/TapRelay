use std::time::{Duration, Instant};
use taprelay_core::devices::AdapterState;
use taprelay_core::state::{Snapshot, Target};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Waiting,
    Synchronizing,
    Backoff,
    Complete,
    Cancelled,
}

pub(super) struct Restore {
    target: Option<Target>,
    phase: Phase,
    failures: u32,
    next: Instant,
}

impl Restore {
    pub fn new(target: Option<Target>, now: Instant) -> Self {
        Self {
            target,
            phase: Phase::Waiting,
            failures: 0,
            next: now,
        }
    }

    pub fn pending(&self) -> bool {
        self.target.is_some() && !matches!(self.phase, Phase::Complete | Phase::Cancelled)
    }

    pub fn due(&self, now: Instant) -> bool {
        !self.pending() || now >= self.next
    }

    pub fn refresh_due(&self, adapter: AdapterState, now: Instant) -> bool {
        adapter == AdapterState::Available && self.due(now)
    }

    pub fn candidate(
        &self,
        state: &Snapshot,
        suspended: &std::collections::BTreeMap<String, bool>,
        now: Instant,
    ) -> Option<String> {
        if !self.pending() || !self.due(now) || state.selected.is_some() {
            return None;
        }
        super::startup_restore_candidate(self.target.as_ref()?, &state.targets, suspended)
    }

    pub fn started(&mut self) {
        self.transition(Phase::Synchronizing);
    }

    pub fn observe(&mut self, state: &Snapshot, generation: u64) {
        if self.pending()
            && state.ready
            && state.generation == generation
            && state.selected_target().is_some_and(|selected| {
                self.target
                    .as_ref()
                    .is_some_and(|target| selected.same_device(target))
            })
        {
            self.transition(Phase::Complete);
        }
    }

    pub fn failure(&mut self, now: Instant) {
        self.failures = self.failures.saturating_add(1);
        let delay = (3_u64 << self.failures.saturating_sub(1).min(5)).min(60);
        self.next = now + Duration::from_secs(delay);
        self.transition(Phase::Backoff);
        tracing::warn!(
            attempt = self.failures,
            retry_seconds = delay,
            "Startup restore will retry"
        );
    }

    pub fn cancel(&mut self, reason: &str) {
        if self.pending() {
            tracing::info!(reason, "Startup restore cancelled");
            self.transition(Phase::Cancelled);
        }
    }

    fn transition(&mut self, phase: Phase) {
        if self.phase != phase {
            tracing::info!(from = ?self.phase, to = ?phase, "Startup restore state changed");
            self.phase = phase;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use taprelay_core::state::Knowledge;

    fn target() -> Target {
        Target {
            id: "receiver".into(),
            identity: vec!["physical:tablet".into()],
            link: Knowledge::Yes,
            subscribed: Knowledge::Yes,
            ..Default::default()
        }
    }

    #[test]
    fn late_subscription_and_identity_do_not_exhaust_restore() {
        let now = Instant::now();
        let restore = Restore::new(Some(target()), now);
        let suspended = Default::default();
        let mut state = Snapshot::default();
        for seconds in [0, 1, 3, 121, 600] {
            assert!(
                restore
                    .candidate(&state, &suspended, now + Duration::from_secs(seconds))
                    .is_none()
            );
        }
        state.targets.push(Target {
            id: "new-endpoint".into(),
            identity: vec![],
            ..target()
        });
        assert!(restore.candidate(&state, &suspended, now).is_none());
        state.targets[0].identity = target().identity;
        assert_eq!(
            restore
                .candidate(&state, &suspended, now + Duration::from_secs(601))
                .as_deref(),
            Some("new-endpoint")
        );
    }

    #[test]
    fn repeated_failures_recover_only_after_backoff_and_valid_synchronization() {
        let mut now = Instant::now();
        let mut restore = Restore::new(Some(target()), now);
        restore.started();
        for delay in [3, 6, 12, 24, 48, 60, 60, 60] {
            restore.failure(now);
            assert!(!restore.due(now + Duration::from_secs(delay - 1)));
            now += Duration::from_secs(delay);
            assert!(restore.due(now));
            restore.observe(&Snapshot::default(), 0);
            assert!(restore.pending());
        }
        let mut state = Snapshot {
            selected: Some("receiver".into()),
            targets: vec![target()],
            ready: true,
            generation: 1,
            ..Default::default()
        };
        restore.observe(&state, 2);
        assert!(restore.pending());
        state.generation = 2;
        restore.observe(&state, 2);
        assert!(!restore.pending());
        state.selected = None;
        assert!(
            restore
                .candidate(&state, &Default::default(), now)
                .is_none()
        );
    }

    #[test]
    fn explicit_cancellation_cannot_be_revived_by_late_snapshot() {
        for reason in ["select", "disconnect", "pair"] {
            let now = Instant::now();
            let mut restore = Restore::new(Some(target()), now);
            restore.started();
            restore.cancel(reason);
            let mut state = Snapshot {
                targets: vec![target()],
                selected: Some("receiver".into()),
                ready: true,
                ..Default::default()
            };
            restore.observe(&state, 0);
            assert_eq!(restore.phase, Phase::Cancelled);
            state.selected = None;
            assert!(
                restore
                    .candidate(&state, &Default::default(), now)
                    .is_none()
            );
        }
    }

    #[test]
    fn unavailable_environment_and_service_recreation_preserve_pending_target() {
        let now = Instant::now();
        let mut restore = Restore::new(Some(target()), now);
        for adapter in [
            AdapterState::Unknown,
            AdapterState::Unavailable,
            AdapterState::Disabled,
        ] {
            assert!(!restore.refresh_due(adapter, now));
            assert!(restore.pending());
        }
        let state = Snapshot {
            targets: vec![target()],
            ..Default::default()
        };
        for seconds in [5, 10, 600] {
            let now = now + Duration::from_secs(seconds);
            assert!(restore.refresh_due(AdapterState::Available, now));
            assert_eq!(
                restore
                    .candidate(&state, &Default::default(), now)
                    .as_deref(),
                Some("receiver")
            );
            restore.started();
            restore.observe(&Snapshot::default(), 0);
            assert!(restore.pending());
        }
    }
}
