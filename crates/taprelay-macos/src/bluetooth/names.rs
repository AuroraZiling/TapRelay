//! Best-effort receiver metadata. A name never selects or authorizes a peer.
use objc2::AnyThread;
use objc2_core_bluetooth::{CBCentralManager, CBManagerState};
use objc2_foundation::{NSArray, NSString, NSUUID};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};
use taprelay_core::state::Target;

#[derive(Default)]
pub(super) struct ReceiverNames {
    entries: BTreeMap<String, Entry>,
}
#[derive(Default)]
struct Entry {
    name: Option<String>,
    next_attempt: Option<Instant>,
    attempts: u8,
}

fn fallback(id: &str) -> String {
    format!(
        "Bluetooth central {}",
        id.chars().take(8).collect::<String>()
    )
}

impl ReceiverNames {
    pub fn new(remembered: Option<&Target>) -> Self {
        let mut names = Self::default();
        if let Some(target) = remembered
            && !target.name.trim().is_empty()
            && target.name != fallback(&target.id)
        {
            names.entries.insert(
                target.id.clone(),
                Entry {
                    name: Some(target.name.clone()),
                    ..Default::default()
                },
            );
        }
        names
    }

    pub fn name(&self, id: &str) -> String {
        self.entries
            .get(id)
            .and_then(|entry| entry.name.clone())
            .unwrap_or_else(|| fallback(id))
    }

    pub fn refresh(&mut self, id: &str) {
        let entry = self.entries.entry(id.to_owned()).or_default();
        entry.next_attempt = Some(Instant::now());
        entry.attempts = 0;
    }

    pub fn resolve(&mut self, manager: &CBCentralManager) -> bool {
        let now = Instant::now();
        let due: Vec<_> = self
            .entries
            .iter()
            .filter(|(_, entry)| entry.next_attempt.is_some_and(|at| at <= now))
            .map(|(id, _)| id.clone())
            .collect();
        if due.is_empty() || unsafe { manager.state() } != CBManagerState::PoweredOn {
            return false;
        }
        let identifiers: Vec<_> = due
            .iter()
            .filter_map(|id| NSUUID::initWithUUIDString(NSUUID::alloc(), &NSString::from_str(id)))
            .collect();
        // Core Bluetooth only promises to return known peripherals. A central
        // may have no corresponding record, even after system pairing.
        let peripherals = unsafe {
            manager.retrievePeripheralsWithIdentifiers(&NSArray::from_retained_slice(&identifiers))
        };
        let resolved: BTreeMap<_, _> = peripherals
            .iter()
            .map(|peripheral| unsafe {
                (
                    peripheral.identifier().UUIDString().to_string(),
                    peripheral.name().map(|name| name.to_string()),
                )
            })
            .collect();
        let mut changed = false;
        for id in due {
            let name = resolved.get(&id).and_then(|name| name.as_deref());
            tracing::info!(central = %id, found = resolved.contains_key(&id), has_name = name.is_some_and(|name| !name.trim().is_empty()), elapsed_ms = now.elapsed().as_millis(), "Bluetooth receiver name lookup");
            changed |= self.record(&id, name, now);
        }
        changed
    }

    fn record(&mut self, id: &str, name: Option<&str>, now: Instant) -> bool {
        let Some(entry) = self.entries.get_mut(id) else {
            return false;
        };
        entry.attempts += 1;
        if let Some(name) = name.filter(|name| !name.trim().is_empty()) {
            let changed = entry.name.as_deref() != Some(name);
            entry.name = Some(name.to_owned());
            entry.next_attempt = None;
            changed
        } else {
            // Allow the system cache to catch up after first pairing, then stop.
            // Reconnection or Refresh explicitly retries; misses retain the last name.
            entry.next_attempt = (entry.attempts < 3).then_some(now + Duration::from_secs(2));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn same_names_do_not_merge_receivers_and_misses_preserve_the_last_name() {
        let mut names = ReceiverNames::default();
        names.refresh("one");
        names.refresh("two");
        let now = Instant::now();
        names.record("one", Some("Tablet"), now);
        names.record("two", Some("Tablet"), now);
        names.record("one", Some("Renamed tablet"), now);
        names.refresh("two");
        for _ in 0..3 {
            names.record("two", Some("  "), now);
        }
        names.record("unrequested", Some("Another device"), now);
        assert_eq!(names.name("one"), "Renamed tablet");
        assert_eq!(names.name("two"), "Tablet");
        assert!(names.entries["two"].next_attempt.is_none());
        assert!(!names.entries.contains_key("unrequested"));
    }

    #[test]
    fn remembered_names_apply_only_to_the_exact_id_and_ignore_old_placeholders() {
        let mut target = Target {
            id: "12345678-old".into(),
            name: "My tablet".into(),
            ..Default::default()
        };
        let names = ReceiverNames::new(Some(&target));
        assert_eq!(names.name(&target.id), "My tablet");
        assert_eq!(names.name("12345678-new"), "Bluetooth central 12345678");
        target.name = fallback(&target.id);
        assert!(ReceiverNames::new(Some(&target)).entries.is_empty());
    }
}
