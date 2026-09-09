//! Subscriber metadata is cached per live endpoint; failed reads are never permanent.
use crate::state::Knowledge;
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct Metadata {
    pub name: String,
    pub pairing: Knowledge,
    pub identity: Vec<String>,
}
impl Metadata {
    fn complete(&self) -> bool {
        !self.name.is_empty()
            && self.name != "Unknown"
            && self.pairing == Knowledge::Yes
            && !self.identity.is_empty()
    }
}
#[derive(Default)]
pub struct MetadataCache(BTreeMap<String, (Metadata, Instant)>);
impl MetadataCache {
    pub fn resolve(
        &mut self,
        id: &str,
        now: Instant,
        resolve: impl FnOnce() -> Metadata,
    ) -> Metadata {
        if let Some((value, retry)) = self.0.get(id)
            && (value.complete() || now < *retry)
        {
            return value.clone();
        }
        let value = resolve();
        // Retry incomplete Windows metadata at most once per endpoint per 3s.
        self.0
            .insert(id.to_owned(), (value.clone(), now + Duration::from_secs(3)));
        value
    }
    pub fn retain(&mut self, mut live: impl FnMut(&str) -> bool) {
        self.0.retain(|id, _| live(id));
    }
    pub fn clear(&mut self) {
        self.0.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn transient_failure_is_retried_without_reconnecting() {
        let mut cache = MetadataCache::default();
        let now = Instant::now();
        let failed = || Metadata {
            name: "Unknown".into(),
            pairing: Knowledge::Unknown,
            identity: vec![],
        };
        cache.resolve("hid", now, failed);
        assert_eq!(
            cache
                .resolve("hid", now + Duration::from_secs(2), || panic!("backoff"))
                .pairing,
            Knowledge::Unknown
        );
        let resolved = cache.resolve("hid", now + Duration::from_secs(3), || Metadata {
            name: "Tablet".into(),
            pairing: Knowledge::Yes,
            identity: vec!["physical".into()],
        });
        assert_eq!(resolved.pairing, Knowledge::Yes);
        assert_eq!(
            cache
                .resolve("hid", now + Duration::from_secs(9), || panic!(
                    "complete metadata is stable during subscription"
                ))
                .name,
            "Tablet"
        );
        cache.retain(|_| false);
        assert_eq!(
            cache.resolve("hid", now, failed).pairing,
            Knowledge::Unknown
        );
    }
    #[test]
    fn recovered_identity_joins_selected_discovery_alias_and_becomes_paired() {
        use crate::state::{Target, upsert_target};
        let now = Instant::now();
        let mut cache = MetadataCache::default();
        for (elapsed, recovered) in [(0, false), (3, true)] {
            let metadata = cache.resolve("hid", now + Duration::from_secs(elapsed), || Metadata {
                name: "Tablet".into(),
                pairing: Knowledge::Unknown,
                identity: if recovered {
                    vec!["physical".into()]
                } else {
                    vec![]
                },
            });
            let mut targets = vec![Target {
                id: "classic".into(),
                pairing: Knowledge::Yes,
                link: Knowledge::Yes,
                identity: vec!["physical".into()],
                ..Default::default()
            }];
            upsert_target(
                &mut targets,
                Target {
                    id: "hid".into(),
                    name: metadata.name,
                    pairing: metadata.pairing,
                    identity: metadata.identity,
                    link: Knowledge::Yes,
                    subscribed: Knowledge::Yes,
                    ..Default::default()
                },
            );
            let selected = targets.iter().find(|t| t.matches_id("classic")).unwrap();
            assert_eq!(
                selected.subscribed == Knowledge::Yes && selected.pairing == Knowledge::Yes,
                recovered
            );
        }
    }
}
