use serde::Serialize;
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub enum TransportActivity {
    #[default]
    Idle,
    CheckingEnvironment,
    Publishing,
    ResolvingDevice,
    Synchronizing,
    Sending,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub enum Knowledge {
    Yes,
    No,
    #[default]
    Unknown,
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Target {
    pub id: String,
    pub name: String,
    pub kind: DeviceKind,
    pub pairing: Knowledge,
    pub link: Knowledge,
    pub subscribed: Knowledge,
    /// Platform-provided physical identity keys; display names are never identity.
    pub identity: Vec<String>,
    pub aliases: Vec<String>,
    pub availability: crate::devices::Availability,
    pub connection: crate::devices::Connection,
}
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub enum DeviceKind {
    #[default]
    Unknown,
    Computer,
    Phone,
    Tablet,
}
impl Target {
    pub fn matches_id(&self, id: &str) -> bool {
        self.id == id || self.aliases.iter().any(|a| a == id)
    }
    pub fn same_device(&self, other: &Self) -> bool {
        self.matches_id(&other.id)
            || other.aliases.iter().any(|id| self.matches_id(id))
            || self
                .identity
                .iter()
                .any(|key| !key.is_empty() && other.identity.contains(key))
    }
}
pub fn upsert_target(targets: &mut Vec<Target>, mut target: Target) {
    // Revisit the list after every merge: a row can bridge two endpoint identities.
    while let Some(index) = targets.iter().position(|t| t.same_device(&target)) {
        let mut previous = targets.remove(index);
        let mut aliases = previous.aliases.clone();
        aliases.extend(target.aliases.iter().cloned());
        aliases.extend([previous.id.clone(), target.id.clone()]);
        aliases.sort();
        aliases.dedup();
        let mut identity = previous.identity.clone();
        identity.extend(target.identity.iter().cloned());
        identity.sort();
        identity.dedup();
        let previous_ready = previous.subscribed == Knowledge::Yes;
        let target_ready = target.subscribed == Knowledge::Yes;
        if (previous_ready && !target_ready)
            || (previous_ready == target_ready && previous.id < target.id)
        {
            std::mem::swap(&mut target, &mut previous);
        }
        // Unknown is absence of evidence, not a negative pairing result. Only
        // supplement from an endpoint already proven to be the same entity.
        if target.pairing == Knowledge::Unknown {
            target.pairing = previous.pairing;
        }
        if target.kind == DeviceKind::Unknown {
            target.kind = previous.kind;
        }
        if previous.availability == crate::devices::Availability::Nearby {
            target.availability = previous.availability;
        }
        if previous.link == Knowledge::Yes {
            target.link = Knowledge::Yes;
        }
        target.identity = identity;
        target.aliases = aliases;
    }
    targets.push(target);
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovery_and_subscriber_of_same_device_produce_one_row() {
        let classic = Target {
            kind: DeviceKind::Tablet,
            id: "classic-endpoint".into(),
            name: "Artemis iPad".into(),
            identity: vec!["container:ipad".into()],
            ..Default::default()
        };
        let le = Target {
            id: "le-endpoint".into(),
            ..classic.clone()
        };
        let subscriber = Target {
            kind: DeviceKind::Unknown,
            subscribed: Knowledge::Yes,
            ..le.clone()
        };
        let mut rows = vec![classic, le];
        upsert_target(&mut rows, subscriber);
        assert_eq!(rows.len(), 1, "One physical device must have one row");
        assert_eq!(rows[0].subscribed, Knowledge::Yes);
        assert_eq!(rows[0].kind, DeviceKind::Tablet);
        assert!(rows[0].matches_id("classic-endpoint"));
        assert!(rows[0].matches_id("le-endpoint"));
    }
    #[test]
    fn same_name_different_devices_remain_separate() {
        let mut rows = vec![];
        for id in ["one", "two"] {
            upsert_target(
                &mut rows,
                Target {
                    id: id.into(),
                    name: "iPad".into(),
                    identity: vec![format!("container:{id}")],
                    ..Default::default()
                },
            );
        }
        assert_eq!(rows.len(), 2);
    }
    #[test]
    fn refresh_does_not_downgrade_a_subscribed_endpoint() {
        let mut rows = vec![];
        let subscriber = Target {
            id: "hid".into(),
            identity: vec!["physical".into()],
            subscribed: Knowledge::Yes,
            pairing: Knowledge::Unknown,
            ..Default::default()
        };
        upsert_target(&mut rows, subscriber);
        for _ in 0..4 {
            upsert_target(
                &mut rows,
                Target {
                    id: "discovery".into(),
                    identity: vec!["physical".into()],
                    pairing: Knowledge::Yes,
                    ..Default::default()
                },
            );
        }
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].id, "hid");
        assert_eq!(rows[0].pairing, Knowledge::Yes);
    }
}
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Snapshot {
    pub adapter_state: crate::devices::AdapterState,
    pub discovery: crate::devices::DiscoveryState,
    pub pairing_handoff: crate::devices::PairingHandoff,
    pub device_error: Option<crate::devices::DeviceError>,
    pub connection_capability: crate::devices::ConnectionCapability,
    pub activity: TransportActivity,
    pub adapter: bool,
    pub peripheral: bool,
    pub service: bool,
    pub broadcasting: bool,
    pub targets: Vec<Target>,
    pub selected: Option<String>,
    pub target_status: Option<Target>,
    pub ready: bool,
    /// The three report subscriptions are tracked independently. `ready`
    /// remains the media/Consumer readiness used by the diagnostics flow.
    pub consumer_ready: bool,
    pub keyboard_ready: bool,
    pub mouse_ready: bool,
    pub passthrough_ready: bool,
    pub hid_suspended: bool,
    pub generation: u64,
    pub input: bool,
    pub last_error: Option<String>,
}
