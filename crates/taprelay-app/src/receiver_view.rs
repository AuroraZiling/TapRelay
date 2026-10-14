//! Typed receiver state projected into localized presentation at one boundary.
use crate::{DeviceRow, i18n::keys};
use std::time::{Duration, Instant};
use taprelay_core::{
    devices::*,
    state::{Knowledge, Snapshot, Target},
};

#[derive(Default)]
pub struct SessionPresentation {
    target: Option<String>,
    suspended: bool,
    ready_since: Option<Instant>,
}

impl SessionPresentation {
    pub fn key(&mut self, state: &Snapshot, now: Instant) -> &'static str {
        if self.target != state.selected {
            self.target = state.selected.clone();
            self.suspended = false;
            self.ready_since = None;
        }
        let connected = state.selected_target().is_some_and(|target| {
            target.link == Knowledge::Yes && target.subscribed == Knowledge::Yes
        });
        if !connected || state.device_error.is_some() || state.profile_switching {
            self.suspended = false;
            self.ready_since = None;
        } else if state.hid_suspended {
            self.suspended = true;
            self.ready_since = None;
        } else if self.suspended {
            if state.ready {
                let since = *self.ready_since.get_or_insert(now);
                if now.saturating_duration_since(since) >= Duration::from_millis(500) {
                    self.suspended = false;
                    self.ready_since = None;
                }
            } else {
                self.ready_since = None;
            }
        }
        if self.suspended {
            keys::RECEIVER_SUSPENDED
        } else {
            session_status_key(state)
        }
    }
}

/// Text and dot share the same interpretation of session readiness and listening.
pub fn relay_presentation(
    key: &'static str,
    listening: bool,
    has_bindings: bool,
) -> (&'static str, i32) {
    if key == "fault" {
        return (key, 3);
    }
    let key = if key == keys::RECEIVER_READY {
        if !listening {
            keys::RECEIVER_LISTENING_PAUSED
        } else if !has_bindings {
            keys::RECEIVER_NO_BINDINGS
        } else {
            keys::RECEIVER_FORWARDING
        }
    } else {
        key
    };
    (
        key,
        if !listening {
            1
        } else if key == keys::RECEIVER_FORWARDING {
            0
        } else {
            2
        },
    )
}

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
            DeviceError::ConnectionFailed => keys::RECEIVER_CONNECT_FAILED,
            DeviceError::PairingLaunchFailed => keys::RECEIVER_PAIR_FAILED,
        };
    }
    match state.discovery {
        DiscoveryState::Failed => keys::RECEIVER_SCAN_FAILED,
        DiscoveryState::Scanning => keys::RECEIVER_SCANNING,
        DiscoveryState::ResultsAvailable
            if !state
                .targets
                .iter()
                .any(|target| target.pairing == Knowledge::Yes) =>
        {
            keys::RECEIVER_EMPTY
        }
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
    let Some(target) = state.selected_target() else {
        return keys::RECEIVER_DEVICE_MISSING;
    };
    match target.connection {
        Connection::Disconnected if target.availability == Availability::Unavailable => {
            keys::RECEIVER_DEVICE_MISSING
        }
        Connection::Disconnected if target.pairing == Knowledge::No => keys::RECEIVER_UNPAIRED,
        Connection::Disconnected => keys::RECEIVER_PAIRED_DISCONNECTED,
        Connection::AwaitingHostSubscription => keys::RECEIVER_WAIT_SUBSCRIPTION,
        Connection::Synchronizing => keys::RECEIVER_SYNCHRONIZING,
        Connection::Connected if state.ready && !state.profile_switching => keys::RECEIVER_READY,
        Connection::Connected => keys::RECEIVER_SYNCHRONIZING,
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
    } else if target.link == Knowledge::Yes
        && (target.subscribed == Knowledge::Yes
            || (state.service_paused && target.pairing == Knowledge::Yes))
    {
        ("device", keys::RECEIVER_CONNECT)
    } else if target.pairing == Knowledge::No {
        ("pair-device", keys::RECEIVER_PAIR)
    } else {
        // A bond alone is not a live receiver. Require a connected link,
        // plus its HID subscription while the service is published. A linked
        // paired receiver may restart a paused service to subscribe again.
        ("", keys::RECEIVER_HOST_HINT)
    };
    let connection = if selected
        && target.connection == Connection::Connected
        && (!state.ready || state.hid_suspended || state.profile_switching)
    {
        Connection::Synchronizing
    } else {
        target.connection
    };
    let detail = if selected && state.hid_suspended {
        keys::RECEIVER_SUSPENDED
    } else if target.availability == Availability::Unavailable {
        keys::RECEIVER_DEVICE_MISSING
    } else {
        match connection {
            Connection::AwaitingHostSubscription => keys::RECEIVER_HOST_HINT,
            Connection::Synchronizing => keys::RECEIVER_SYNCHRONIZING,
            Connection::Connected => keys::RECEIVER_READY,
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
        id: target.id.clone(),
        name: if target.name.trim().is_empty() {
            target.id.clone()
        } else {
            target.name.clone()
        },
        device_kind: device_kind(target.kind).into(),
        status: match connection {
            Connection::Connected => "ready",
            Connection::Synchronizing => "busy",
            Connection::Failed => "failed",
            _ if target.availability == Availability::Unavailable => "unavailable",
            _ if target.link == Knowledge::Yes => "linked",
            _ if target.pairing == Knowledge::Yes => "paired",
            _ => "unknown",
        }
        .into(),
        detail,
        selected,
        action: action.into(),
        action_label: tr(label),
        enabled: selected
            || (!action.is_empty()
                && state.adapter_state == AdapterState::Available
                && target.availability != Availability::Unavailable
                && !matches!(target.connection, Connection::Synchronizing)),
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
    fn revoked_readiness_cannot_display_a_connected_row_as_ready() {
        let state = Snapshot {
            selected: Some("tablet".into()),
            ready: false,
            targets: vec![Target {
                id: "tablet".into(),
                connection: Connection::Connected,
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_ne!(session_status_key(&state), keys::RECEIVER_READY);
        let projected = row(&state.targets[0], &state, |key| key.into());
        assert_ne!(projected.status, "ready");
        assert_ne!(projected.detail, keys::RECEIVER_READY);
    }

    #[test]
    fn forwarding_text_and_dot_agree_with_listening_and_bindings() {
        assert_eq!(
            relay_presentation(keys::RECEIVER_READY, false, true),
            (keys::RECEIVER_LISTENING_PAUSED, 1)
        );
        assert_eq!(
            relay_presentation(keys::RECEIVER_READY, true, false),
            (keys::RECEIVER_NO_BINDINGS, 2)
        );
        assert_eq!(
            relay_presentation(keys::RECEIVER_READY, true, true),
            (keys::RECEIVER_FORWARDING, 0)
        );
        for key in [
            keys::RECEIVER_SYNCHRONIZING,
            keys::RECEIVER_SUSPENDED,
            "switching",
            "identity",
        ] {
            assert_ne!(relay_presentation(key, true, true).1, 0);
            assert_ne!(
                relay_presentation(key, false, true).0,
                keys::RECEIVER_FORWARDING
            );
        }
        assert_eq!(relay_presentation("fault", false, true), ("fault", 3));
    }

    #[test]
    fn short_suspend_resume_cycles_stay_paused_until_readiness_is_stable() {
        let mut view = SessionPresentation::default();
        let now = Instant::now();
        let mut state = Snapshot {
            selected: Some("tablet".into()),
            targets: vec![Target {
                id: "tablet".into(),
                link: Knowledge::Yes,
                subscribed: Knowledge::Yes,
                connection: Connection::Connected,
                ..Default::default()
            }],
            ..Default::default()
        };
        for cycle in 0..5 {
            let at = now + Duration::from_secs(cycle * 3);
            state.hid_suspended = true;
            state.ready = false;
            assert_eq!(view.key(&state, at), keys::RECEIVER_SUSPENDED);
            state.hid_suspended = false;
            assert_eq!(
                view.key(&state, at + Duration::from_secs(1)),
                keys::RECEIVER_SUSPENDED
            );
            state.ready = true;
            assert_eq!(
                view.key(&state, at + Duration::from_secs(2)),
                keys::RECEIVER_SUSPENDED
            );
            assert_eq!(
                view.key(&state, at + Duration::from_millis(2100)),
                keys::RECEIVER_SUSPENDED
            );
        }
        assert_eq!(
            view.key(&state, now + Duration::from_millis(14500)),
            keys::RECEIVER_READY
        );
        state.hid_suspended = true;
        assert_eq!(
            view.key(&state, now + Duration::from_secs(15)),
            keys::RECEIVER_SUSPENDED
        );
        state.selected = None;
        state.hid_suspended = false;
        assert_eq!(
            view.key(&state, now + Duration::from_secs(16)),
            keys::RECEIVER_DEVICE_MISSING
        );
    }
    #[test]
    fn compact_row_preserves_identity_and_distinguishes_link_from_session() {
        let state = Snapshot::default();
        let mut target = Target {
            id: "unnamed-endpoint".into(),
            pairing: Knowledge::Yes,
            ..Default::default()
        };
        let project = |t: &Target| row(t, &state, |key| key.into());
        assert!(project(&target).name.contains("unnamed-endpoint"));
        assert_eq!(project(&target).status, "paired");
        target.link = Knowledge::Yes;
        assert_eq!(project(&target).status, "linked");
        assert_eq!(project(&target).detail, keys::RECEIVER_LINKED);
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
        let project = |t: &Target, s: &Snapshot| {
            row(t, s, |key| rust_i18n::t!(key, locale = "en").into_owned())
        };
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
        target.link = Knowledge::Yes;
        target.subscribed = Knowledge::Yes;
        assert_eq!(project(&target, &state).action_label, "Start session");
        state.selected = Some("stable".into());
        target.connection = Connection::Connected;
        assert_eq!(project(&target, &state).action, "disconnect-device");
        target.connection = Connection::Synchronizing;
        assert!(project(&target, &state).enabled);
        state.adapter_state = AdapterState::Disabled;
        assert!(project(&target, &state).enabled);
    }
    #[test]
    fn paired_devices_require_a_live_link_to_start_a_session_when_service_is_paused() {
        let state = Snapshot {
            adapter_state: AdapterState::Available,
            service_paused: true,
            ..Default::default()
        };
        let mut target = Target {
            id: "receiver".into(),
            pairing: Knowledge::Yes,
            availability: Availability::Nearby,
            ..Default::default()
        };
        for link in [Knowledge::No, Knowledge::Unknown] {
            target.link = link;
            let device = row(&target, &state, |key| key.into());
            assert!(
                !device.enabled,
                "paired {link:?} link must not start a session"
            );
            assert_ne!(device.action, "device");
        }
        target.link = Knowledge::Yes;
        let device = row(&target, &state, |key| key.into());
        assert!(device.enabled);
        assert_eq!(device.action, "device");
    }

    #[test]
    fn stale_subscription_does_not_allow_a_disconnected_receiver_to_start_a_session() {
        let mut state = Snapshot {
            adapter_state: AdapterState::Available,
            ..Default::default()
        };
        let mut target = Target {
            id: "receiver".into(),
            pairing: Knowledge::Yes,
            subscribed: Knowledge::Yes,
            availability: Availability::Nearby,
            ..Default::default()
        };
        for paused in [false, true] {
            state.service_paused = paused;
            for link in [Knowledge::No, Knowledge::Unknown] {
                target.link = link;
                let device = row(&target, &state, |key| key.into());
                assert!(!device.enabled);
                assert_ne!(device.action, "device");
            }
            target.link = Knowledge::Yes;
            let device = row(&target, &state, |key| key.into());
            assert!(device.enabled);
            assert_eq!(device.action, "device");
        }
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
        let device = row(&target, &state, |key| {
            rust_i18n::t!(key, locale = "en").into_owned()
        });
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
