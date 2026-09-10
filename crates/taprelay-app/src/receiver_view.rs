//! Typed receiver state projected into localized presentation at one boundary.
use crate::{DeviceRow, i18n::keys};
use taprelay_core::{
    devices::*,
    state::{Knowledge, Snapshot, Target},
};

pub fn adapter_label_key(adapter: AdapterState) -> &'static str {
    match adapter {
        AdapterState::Unknown => keys::BLUETOOTH_CHECKING,
        AdapterState::Available => keys::BLUETOOTH_ENABLED,
        AdapterState::Disabled => keys::BLUETOOTH_DISABLED,
        AdapterState::Unavailable => keys::BLUETOOTH_UNAVAILABLE,
    }
}

pub fn page_status_key(state: &Snapshot) -> &'static str {
    match state.adapter_state {
        AdapterState::Disabled => return keys::RECEIVER_ADAPTER_OFF,
        AdapterState::Unavailable => return keys::RECEIVER_ADAPTER_MISSING,
        AdapterState::Unknown => return keys::RECEIVER_PREPARING,
        AdapterState::Available => {}
    }
    match state.pairing_handoff {
        PairingHandoff::WaitingForOS => return keys::RECEIVER_PAIR_WAIT,
        PairingHandoff::ConfirmedPaired if state.ready => return keys::RECEIVER_READY,
        PairingHandoff::ConfirmedPaired if state.device_error.is_none() => {
            return keys::RECEIVER_PAIR_COMPLETE;
        }
        PairingHandoff::ConfirmedPaired => {}
        PairingHandoff::StillUnpaired => return keys::RECEIVER_PAIR_INCOMPLETE,
        PairingHandoff::TargetUnavailable => return keys::RECEIVER_PAIR_MISSING,
        PairingHandoff::Idle => {}
    }
    if let Some(error) = state.device_error {
        return match error {
            DeviceError::DiscoveryFailed => keys::RECEIVER_SCAN_FAILED,
            DeviceError::ConnectionFailed => keys::RECEIVER_CONNECT_FAILED,
            DeviceError::Disconnected => keys::RECEIVER_DISCONNECTED,
            DeviceError::BluetoothOff => keys::RECEIVER_ADAPTER_OFF,
            DeviceError::PairingLaunchFailed => keys::RECEIVER_PAIR_FAILED,
        };
    }
    match state.discovery {
        DiscoveryState::Failed => keys::RECEIVER_SCAN_FAILED,
        DiscoveryState::Scanning => keys::RECEIVER_SCANNING,
        DiscoveryState::ResultsAvailable if state.targets.is_empty() => keys::RECEIVER_EMPTY,
        _ => keys::RECEIVER_HELP,
    }
}

pub fn session_status_key(state: &Snapshot) -> &'static str {
    if state.hid_suspended {
        return keys::RECEIVER_SUSPENDED;
    }
    if state.pairing_handoff == PairingHandoff::WaitingForOS {
        return keys::RECEIVER_WAIT_PAIRING;
    }
    let Some(target) = &state.target_status else {
        return keys::RECEIVER_DEVICE_MISSING;
    };
    match target.connection {
        Connection::Disconnected if target.availability == Availability::Unavailable => {
            keys::RECEIVER_DEVICE_MISSING
        }
        Connection::Disconnected if target.pairing == Knowledge::No => keys::RECEIVER_UNPAIRED,
        Connection::Disconnected => keys::RECEIVER_PAIRED_DISCONNECTED,
        Connection::Connecting => keys::RECEIVER_CONNECTING,
        Connection::AwaitingHostSubscription => keys::RECEIVER_WAIT_SUBSCRIPTION,
        Connection::Synchronizing => keys::RECEIVER_SYNCHRONIZING,
        Connection::Connected => keys::RECEIVER_READY,
        Connection::Disconnecting => keys::RECEIVER_DISCONNECTING,
        Connection::Failed => keys::RECEIVER_CONNECT_FAILED,
    }
}

pub fn row(target: &Target, state: &Snapshot, tr: impl Fn(&'static str) -> String) -> DeviceRow {
    let selected = state
        .selected
        .as_ref()
        .is_some_and(|id| target.matches_id(id));
    let (action, label) = if selected {
        ("disconnect-device", keys::RECEIVER_DISCONNECT)
    } else if target.subscribed == Knowledge::Yes
        || state.connection_capability == ConnectionCapability::Native
    {
        ("device", keys::RECEIVER_CONNECT)
    } else if target.pairing == Knowledge::No {
        ("pair-device", keys::RECEIVER_PAIR)
    } else {
        // The receiver must initiate the HID subscription. Keep this as a
        // non-interactive hint until that subscription is observable; the
        // Connect action appears once the receiver is actually subscribed.
        ("", keys::RECEIVER_HOST_HINT)
    };
    let detail = if target.availability == Availability::Unavailable {
        keys::RECEIVER_DEVICE_MISSING
    } else {
        match target.connection {
            Connection::Connecting => keys::RECEIVER_CONNECTING,
            Connection::AwaitingHostSubscription => keys::RECEIVER_HOST_HINT,
            Connection::Synchronizing => keys::RECEIVER_SYNCHRONIZING,
            Connection::Connected => keys::RECEIVER_READY,
            Connection::Disconnecting => keys::RECEIVER_DISCONNECTING,
            Connection::Failed => keys::RECEIVER_CONNECT_FAILED,
            Connection::Disconnected if target.link == Knowledge::Yes => keys::RECEIVER_LINKED,
            Connection::Disconnected => match target.pairing {
                Knowledge::Yes => keys::RECEIVER_PAIRED_DISCONNECTED,
                Knowledge::No => keys::RECEIVER_UNPAIRED,
                Knowledge::Unknown => keys::RECEIVER_PAIR_UNKNOWN,
            },
        }
    };
    let detail = tr(detail);
    DeviceRow {
        id: target.id.clone().into(),
        name: if target.name.trim().is_empty() {
            target.id.clone()
        } else {
            target.name.clone()
        }
        .into(),
        device_kind: device_kind(target.kind).into(),
        status: match target.connection {
            Connection::Connected => "ready",
            Connection::Connecting | Connection::Synchronizing | Connection::Disconnecting => {
                "busy"
            }
            Connection::Failed => "failed",
            _ if target.availability == Availability::Unavailable => "unavailable",
            _ if target.link == Knowledge::Yes => "linked",
            _ if target.pairing == Knowledge::Yes => "paired",
            _ => "unknown",
        }
        .into(),
        detail: detail.into(),
        selected,
        action: action.into(),
        action_label: tr(label).into(),
        enabled: selected
            || (!action.is_empty()
                && state.adapter_state == AdapterState::Available
                && target.availability != Availability::Unavailable
                && !matches!(
                    target.connection,
                    Connection::Connecting | Connection::Synchronizing | Connection::Disconnecting
                )),
        paired: target.pairing == Knowledge::Yes,
    }
}

fn device_kind(kind: taprelay_core::state::DeviceKind) -> &'static str {
    use taprelay_core::state::DeviceKind;
    match kind {
        DeviceKind::Tablet => "tablet",
        DeviceKind::Phone => "phone",
        DeviceKind::Computer => "computer",
        DeviceKind::Unknown => "bluetooth",
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_row_preserves_identity_and_distinguishes_link_from_session() {
        let state = Snapshot::default();
        let mut target = Target {
            id: "unnamed-endpoint".into(),
            pairing: Knowledge::Yes,
            ..Default::default()
        };
        let project = |t: &Target| row(t, &state, |key| crate::i18n::text(false, key).into());
        assert!(project(&target).name.contains("unnamed-endpoint"));
        assert_eq!(project(&target).status, "paired");
        target.link = Knowledge::Yes;
        assert_eq!(project(&target).status, "linked");
        assert!(project(&target).detail.contains("session not ready"));
        target.connection = Connection::Connected;
        assert_eq!(project(&target).status, "ready");
        target.name = "Artemis iPad".into();
        assert_eq!(project(&target).device_kind, "bluetooth");
        target.kind = taprelay_core::state::DeviceKind::Tablet;
        assert_eq!(project(&target).device_kind, "tablet");
    }
    #[test]
    fn device_actions_and_sections_follow_typed_state() {
        let mut state = Snapshot {
            adapter_state: AdapterState::Available,
            ..Default::default()
        };
        let mut target = Target {
            id: "stable".into(),
            pairing: Knowledge::No,
            ..Default::default()
        };
        let project =
            |t: &Target, s: &Snapshot| row(t, s, |key| crate::i18n::text(false, key).into());
        let unpaired = project(&target, &state);
        assert_eq!(unpaired.action, "pair-device");
        assert!(unpaired.enabled);
        target.pairing = Knowledge::Yes;
        let paired = project(&target, &state);
        assert_eq!(paired.action, "");
        assert_eq!(
            paired.action_label,
            "On the receiver, open Bluetooth and connect to this PC. TapRelay detects its HID subscription automatically."
        );
        assert!(!paired.enabled);
        assert!(paired.paired);
        target.subscribed = Knowledge::Yes;
        assert_eq!(project(&target, &state).action_label, "Start session");
        state.selected = Some("stable".into());
        target.connection = Connection::Connected;
        assert_eq!(project(&target, &state).action, "disconnect-device");
        target.connection = Connection::Connecting;
        assert!(project(&target, &state).enabled);
        state.adapter_state = AdapterState::Disabled;
        assert!(project(&target, &state).enabled);
    }
    #[test]
    fn a_device_is_not_hidden_or_rejected_before_subscription() {
        let state = Snapshot {
            adapter_state: AdapterState::Available,
            selected: Some("ipad".into()),
            ..Default::default()
        };
        let target = Target {
            id: "ipad".into(),
            name: "Artemis iPad".into(),
            pairing: Knowledge::Unknown,
            availability: Availability::Nearby,
            connection: Connection::AwaitingHostSubscription,
            ..Default::default()
        };
        let device = row(&target, &state, |key| crate::i18n::text(false, key).into());
        assert_eq!(device.action, "disconnect-device");
        assert!(device.enabled);
        assert!(!device.detail.contains("Compatibility"));
        assert!(!device.detail.contains("不兼容"));
    }
    #[test]
    fn empty_scan_failure_adapter_and_pairing_states_have_distinct_copy() {
        let mut state = Snapshot::default();
        assert_eq!(page_status_key(&state), keys::RECEIVER_PREPARING);
        state.adapter_state = AdapterState::Disabled;
        assert_eq!(page_status_key(&state), keys::RECEIVER_ADAPTER_OFF);
        state.adapter_state = AdapterState::Unavailable;
        assert_eq!(page_status_key(&state), keys::RECEIVER_ADAPTER_MISSING);
        state.adapter_state = AdapterState::Available;
        state.discovery = DiscoveryState::Scanning;
        assert_eq!(page_status_key(&state), keys::RECEIVER_SCANNING);
        state.discovery = DiscoveryState::ResultsAvailable;
        assert_eq!(page_status_key(&state), keys::RECEIVER_EMPTY);
        state.targets.push(Target {
            pairing: Knowledge::Yes,
            ..Default::default()
        });
        assert_eq!(page_status_key(&state), keys::RECEIVER_HELP);
        state.discovery = DiscoveryState::Failed;
        assert_eq!(page_status_key(&state), keys::RECEIVER_SCAN_FAILED);
        state.pairing_handoff = PairingHandoff::WaitingForOS;
        assert_eq!(page_status_key(&state), keys::RECEIVER_PAIR_WAIT);
    }
}
