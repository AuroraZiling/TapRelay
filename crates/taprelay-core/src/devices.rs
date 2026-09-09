//! Receiver lifecycle policy. No native handles, UI strings, or Bluetooth trust logic.
use crate::state::{Knowledge, Snapshot, Target, upsert_target};
use serde::Serialize;
use std::time::{Duration, Instant};

macro_rules! states {
    ($name:ident { $first:ident $(, $rest:ident)* }) => {
        #[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
        pub enum $name { #[default] $first, $($rest),* }
    };
}
states!(AdapterState {
    Unknown,
    Available,
    Disabled,
    Unavailable
});
states!(DiscoveryState {
    Idle,
    Scanning,
    ResultsAvailable,
    Failed
});
states!(Availability {
    Unknown,
    Nearby,
    Unavailable
});
states!(Connection {
    Disconnected,
    Connecting,
    AwaitingHostSubscription,
    Synchronizing,
    Connected,
    Disconnecting,
    Failed
});
states!(PairingHandoff {
    Idle,
    WaitingForOS,
    ConfirmedPaired,
    StillUnpaired,
    TargetUnavailable
});
states!(ConnectionCapability {
    HostInitiated,
    Native
});
states!(DeviceError {
    DiscoveryFailed,
    ConnectionFailed,
    Disconnected,
    BluetoothOff,
    PairingLaunchFailed
});

/// Independent source membership prevents a watcher removal from erasing a bond.
#[derive(Default)]
pub struct Inventory {
    pub known: Vec<Target>,
    pub nearby: Vec<Target>,
}
impl Inventory {
    pub fn merged(&self) -> Vec<Target> {
        let mut rows = vec![];
        for target in self.known.iter().chain(&self.nearby) {
            upsert_target(&mut rows, target.clone());
        }
        // The paired watcher is authoritative for bond membership. A delayed
        // nearby update must not undo a bond just confirmed by the known source.
        for row in &mut rows {
            if self
                .known
                .iter()
                .any(|known| known.same_device(row) && known.pairing == Knowledge::Yes)
            {
                row.pairing = Knowledge::Yes;
            }
        }
        rows
    }
}

#[derive(Default)]
pub struct Coordinator {
    pairing: Option<(Target, Instant)>,
    selected: Option<String>,
    deadline: Option<Instant>,
    was_ready: bool,
    suppressed: bool,
}
impl Coordinator {
    pub fn pairing_pending(&self) -> bool {
        self.pairing.is_some()
    }
    pub fn pair(&mut self, target: Target, now: Instant) -> bool {
        if self
            .pairing
            .as_ref()
            .is_some_and(|(pending, due)| pending.same_device(&target) && now < *due)
        {
            return false;
        }
        self.deadline = None;
        self.pairing = Some((target, now + Duration::from_secs(120)));
        true
    }
    pub fn cancel_pairing(&mut self) {
        self.pairing = None;
    }
    pub fn connect(&mut self, id: String, now: Instant) -> bool {
        if self.selected.as_ref() == Some(&id)
            && !self.suppressed
            && (self.deadline.is_some() || self.was_ready || self.pairing.is_some())
        {
            return false;
        }
        self.selected = Some(id);
        self.pairing = None;
        self.suppressed = false;
        self.was_ready = false;
        self.deadline = Some(now + Duration::from_secs(30));
        true
    }
    pub fn disconnect(&mut self) {
        self.suppressed = true;
        self.deadline = None;
        self.pairing = None;
        self.was_ready = false;
        self.selected = None;
    }
    /// Clear the transient selection after the native session disappears.
    pub fn clear_selection(&mut self) {
        self.pairing = None;
        self.selected = None;
        self.deadline = None;
        self.was_ready = false;
        self.suppressed = false;
    }
    pub fn reconcile(&mut self, state: &mut Snapshot, now: Instant) {
        if state.adapter_state != AdapterState::Available {
            state.discovery = DiscoveryState::Idle;
            state.ready = false;
            self.deadline = None;
            if state.adapter_state == AdapterState::Disabled {
                state.device_error = Some(DeviceError::BluetoothOff);
            }
        } else if state.device_error == Some(DeviceError::BluetoothOff) {
            state.device_error = None;
        }
        if self.suppressed {
            state.ready = false;
        }
        if self.was_ready && !state.ready && !self.suppressed {
            self.clear_selection();
            state.selected = None;
            state.target_status = None;
            if state.adapter_state == AdapterState::Available {
                state.device_error = None;
            }
        }
        if state.ready {
            self.deadline = None;
            state.device_error = None;
        } else if self.deadline.is_some_and(|due| now >= due) {
            self.deadline = None;
            state.device_error = Some(DeviceError::ConnectionFailed);
        }
        self.was_ready = state.ready;
        if let Some((target, due)) = &self.pairing {
            let observed = state.targets.iter().find(|t| t.same_device(target));
            state.pairing_handoff = if observed.is_some_and(|t| t.pairing == Knowledge::Yes) {
                PairingHandoff::ConfirmedPaired
            } else if now >= *due {
                if observed.is_none_or(|t| t.availability == Availability::Unavailable) {
                    PairingHandoff::TargetUnavailable
                } else {
                    PairingHandoff::StillUnpaired
                }
            } else {
                PairingHandoff::WaitingForOS
            };
            if state.pairing_handoff != PairingHandoff::WaitingForOS {
                self.pairing = None;
                if state.pairing_handoff == PairingHandoff::ConfirmedPaired {
                    state.device_error = None;
                    self.deadline = Some(now + Duration::from_secs(30));
                }
            }
        }
        for target in &mut state.targets {
            let selected = state
                .selected
                .as_ref()
                .is_some_and(|id| target.matches_id(id));
            target.connection = if !selected || self.suppressed {
                Connection::Disconnected
            } else if state.ready {
                Connection::Connected
            } else if state.adapter_state != AdapterState::Available
                || target.availability == Availability::Unavailable
            {
                Connection::Disconnected
            } else if state.device_error == Some(DeviceError::ConnectionFailed) {
                Connection::Failed
            } else if target.subscribed == Knowledge::Yes {
                Connection::Synchronizing
            } else if state.connection_capability == ConnectionCapability::Native
                && self.deadline.is_some()
            {
                Connection::Connecting
            } else {
                Connection::AwaitingHostSubscription
            };
        }
        state
            .targets
            .sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        if let Some(id) = &state.selected
            && let Some(t) = state.targets.iter().find(|t| t.matches_id(id))
        {
            state.target_status = Some(t.clone());
        }
    }
}

pub fn receiver_next_allowed(state: &Snapshot) -> bool {
    state.ready
        && state
            .target_status
            .as_ref()
            .is_some_and(|t| t.connection == Connection::Connected)
}

/// A dead worker must not leave an old selection, connected card, or readiness behind.
pub fn revoke_session(state: &mut Snapshot) {
    state.ready = false;
    state.device_error = Some(DeviceError::ConnectionFailed);
    for target in &mut state.targets {
        target.connection = Connection::Disconnected;
    }
    state.selected = None;
    state.target_status = None;
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target(id: &str, paired: Knowledge) -> Target {
        Target {
            id: id.into(),
            name: "Tablet".into(),
            pairing: paired,
            availability: Availability::Nearby,
            identity: vec![format!("physical:{id}")],
            ..Default::default()
        }
    }
    fn snapshot(target: Target) -> Snapshot {
        Snapshot {
            adapter_state: AdapterState::Available,
            selected: Some(target.id.clone()),
            targets: vec![target],
            ..Default::default()
        }
    }
    #[test]
    fn source_merge_deduplicates_and_removal_preserves_known_device() {
        let known = target("paired", Knowledge::Yes);
        let mut nearby = known.clone();
        nearby.id = "discovered-endpoint".into();
        let mut inventory = Inventory {
            known: vec![known],
            nearby: vec![nearby],
        };
        assert_eq!(inventory.merged().len(), 1);
        assert!(inventory.merged()[0].matches_id("discovered-endpoint"));
        inventory.nearby.clear();
        assert_eq!(inventory.merged()[0].pairing, Knowledge::Yes);
        inventory.known.clear();
        assert!(inventory.merged().is_empty());
    }
    #[test]
    fn pairing_confirmed_before_nearby_update_is_not_lost() {
        let now = Instant::now();
        let nearby = target("one", Knowledge::No);
        let mut manager = Coordinator::default();
        manager.pair(nearby.clone(), now);
        let inventory = Inventory {
            known: vec![target("one", Knowledge::Yes)],
            nearby: vec![nearby],
        };
        let mut state = Snapshot {
            adapter_state: AdapterState::Available,
            targets: inventory.merged(),
            ..Default::default()
        };
        manager.reconcile(&mut state, now);
        assert_eq!(state.targets.len(), 1);
        assert_eq!(state.pairing_handoff, PairingHandoff::ConfirmedPaired);
    }
    #[test]
    fn paired_disconnected_and_unpaired_remain_separate() {
        let now = Instant::now();
        let mut state = snapshot(target("one", Knowledge::Yes));
        state.selected = None;
        state.targets.push(target("two", Knowledge::No));
        Coordinator::default().reconcile(&mut state, now);
        assert_eq!(state.targets.len(), 2);
        assert!(
            state
                .targets
                .iter()
                .all(|t| t.connection == Connection::Disconnected)
        );
        assert_eq!(state.targets[1].pairing, Knowledge::No);
        assert!(!receiver_next_allowed(&state));
    }
    #[test]
    fn ready_session_is_allowed_without_a_compatibility_prediction() {
        let target = Target {
            id: "one".into(),
            name: "iPad".into(),
            pairing: Knowledge::Yes,
            link: Knowledge::Yes,
            subscribed: Knowledge::Yes,
            availability: Availability::Nearby,
            connection: Connection::Connected,
            ..Default::default()
        };
        let state = Snapshot {
            adapter_state: AdapterState::Available,
            selected: Some(target.id.clone()),
            targets: vec![target.clone()],
            target_status: Some(target),
            ready: true,
            ..Default::default()
        };
        assert!(receiver_next_allowed(&state));
    }
    #[test]
    fn disconnected_selected_target_is_cleared_instead_of_waiting_again() {
        let now = Instant::now();
        let target = Target {
            id: "ipad".into(),
            name: "Artemis iPad".into(),
            pairing: Knowledge::Yes,
            link: Knowledge::Yes,
            subscribed: Knowledge::Yes,
            availability: Availability::Nearby,
            connection: Connection::Connected,
            ..Default::default()
        };
        let mut state = Snapshot {
            adapter_state: AdapterState::Available,
            selected: Some(target.id.clone()),
            targets: vec![target.clone()],
            target_status: Some(target),
            ready: true,
            ..Default::default()
        };
        let mut manager = Coordinator::default();
        manager.reconcile(&mut state, now);

        state.ready = false;
        state.targets[0].link = Knowledge::No;
        state.targets[0].subscribed = Knowledge::No;
        manager.reconcile(&mut state, now + Duration::from_secs(1));

        assert!(state.selected.is_none());
        assert!(state.target_status.is_none());
        assert_eq!(state.targets[0].connection, Connection::Disconnected);
        manager.reconcile(&mut state, now + Duration::from_secs(60));
        assert!(state.selected.is_none());
        assert!(state.target_status.is_none());
    }
    #[test]
    fn live_subscription_is_enough_when_pairing_metadata_is_unknown() {
        let now = Instant::now();
        let mut manager = Coordinator::default();
        let mut target = target("one", Knowledge::Unknown);
        target.link = Knowledge::Yes;
        target.subscribed = Knowledge::Yes;
        let mut state = snapshot(target);
        manager.connect("one".into(), now);
        manager.reconcile(&mut state, now);
        assert_eq!(state.targets[0].connection, Connection::Synchronizing);
        state.ready = true;
        manager.reconcile(&mut state, now);
        assert_eq!(state.targets[0].connection, Connection::Connected);
        assert!(receiver_next_allowed(&state));
    }
    #[test]
    fn pairing_native_connection_subscription_and_sync_are_distinct() {
        let now = Instant::now();
        let mut manager = Coordinator::default();
        let mut state = snapshot(target("one", Knowledge::No));
        state.connection_capability = ConnectionCapability::Native;
        manager.connect("one".into(), now);
        assert!(manager.pair(state.targets[0].clone(), now));
        assert!(!manager.pair(state.targets[0].clone(), now));
        manager.reconcile(&mut state, now);
        assert_eq!(state.pairing_handoff, PairingHandoff::WaitingForOS);
        state.targets[0].pairing = Knowledge::Yes;
        manager.reconcile(&mut state, now);
        assert_eq!(state.pairing_handoff, PairingHandoff::ConfirmedPaired);
        assert_eq!(state.targets[0].connection, Connection::Connecting);
        assert!(!receiver_next_allowed(&state));
        state.targets[0].subscribed = Knowledge::Yes;
        manager.reconcile(&mut state, now);
        assert_eq!(state.targets[0].connection, Connection::Synchronizing);
        assert!(!receiver_next_allowed(&state));
        state.ready = true;
        manager.reconcile(&mut state, now);
        assert_eq!(state.targets[0].connection, Connection::Connected);
        assert!(receiver_next_allowed(&state));
    }
    #[test]
    fn host_initiation_is_honest_and_attempts_are_bounded() {
        let now = Instant::now();
        let mut manager = Coordinator::default();
        let mut state = snapshot(target("one", Knowledge::Yes));
        assert!(manager.connect("one".into(), now));
        assert!(!manager.connect("one".into(), now));
        manager.reconcile(&mut state, now);
        assert_eq!(
            state.targets[0].connection,
            Connection::AwaitingHostSubscription
        );
        manager.reconcile(&mut state, now + Duration::from_secs(30));
        assert_eq!(state.targets[0].connection, Connection::Failed);
        assert_eq!(state.device_error, Some(DeviceError::ConnectionFailed));
        assert!(manager.connect("one".into(), now + Duration::from_secs(31)));
    }
    #[test]
    fn pairing_timeout_never_claims_success() {
        let now = Instant::now();
        let mut manager = Coordinator::default();
        let mut state = snapshot(target("one", Knowledge::No));
        manager.pair(state.targets[0].clone(), now);
        manager.reconcile(&mut state, now + Duration::from_secs(120));
        assert_eq!(state.pairing_handoff, PairingHandoff::StillUnpaired);
        manager.pair(state.targets[0].clone(), now);
        state.targets.clear();
        manager.reconcile(&mut state, now + Duration::from_secs(120));
        assert_eq!(state.pairing_handoff, PairingHandoff::TargetUnavailable);
    }
    #[test]
    fn disabling_bluetooth_revokes_ready_and_cancels_attempt() {
        let now = Instant::now();
        let mut manager = Coordinator::default();
        let mut state = snapshot(target("one", Knowledge::Yes));
        state.discovery = DiscoveryState::Scanning;
        manager.connect("one".into(), now);
        state.adapter_state = AdapterState::Disabled;
        state.ready = true;
        manager.reconcile(&mut state, now);
        assert!(!state.ready);
        assert!(!receiver_next_allowed(&state));
        assert_eq!(state.discovery, DiscoveryState::Idle);
        manager.reconcile(&mut state, now + Duration::from_secs(60));
        assert_eq!(state.device_error, Some(DeviceError::BluetoothOff));
    }
    #[test]
    fn disappearance_and_live_subscription_have_separate_membership() {
        let now = Instant::now();
        let mut manager = Coordinator::default();
        let mut state = snapshot(target("one", Knowledge::Yes));
        state.targets[0].subscribed = Knowledge::Yes;
        state.ready = true;
        manager.reconcile(&mut state, now);
        state.ready = false;
        state.targets[0].subscribed = Knowledge::No;
        state.targets[0].availability = Availability::Unavailable;
        manager.reconcile(&mut state, now);
        assert_eq!(state.targets[0].connection, Connection::Disconnected);
        assert!(!receiver_next_allowed(&state));
    }
    #[test]
    fn explicit_disconnect_suppresses_late_ready() {
        let now = Instant::now();
        let mut manager = Coordinator::default();
        let mut state = snapshot(target("one", Knowledge::Yes));
        state.ready = true;
        manager.reconcile(&mut state, now);
        manager.disconnect();
        for seconds in [0, 2, 7, 17, 100] {
            state.ready = true;
            manager.reconcile(&mut state, now + Duration::from_secs(seconds));
            assert!(!state.ready);
        }
        assert!(manager.connect("one".into(), now));
    }
}
