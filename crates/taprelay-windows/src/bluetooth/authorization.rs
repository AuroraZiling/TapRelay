//! Selected transport authorization is independent of discovery event traffic.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicU64, Ordering},
};
use taprelay_core::{hid, passthrough::InputLink};

pub(super) struct Refresh {
    pending: AtomicBool,
    worker: std::thread::Thread,
}

impl Refresh {
    pub fn new() -> Self {
        Self {
            pending: AtomicBool::new(false),
            worker: std::thread::current(),
        }
    }

    pub fn signal(&self) {
        self.pending.store(true, Ordering::Release);
        self.worker.unpark();
    }

    pub fn take(&self) -> bool {
        self.pending.swap(false, Ordering::AcqRel)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Peer<S> {
    pub id: String,
    pub session: S,
    pub active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Selected<S> {
    pub reports: [Option<Peer<S>>; 3],
    pub radio_on: bool,
    pub suspended: bool,
}

impl<S> Default for Selected<S> {
    fn default() -> Self {
        Self {
            reports: [None, None, None],
            radio_on: true,
            suspended: false,
        }
    }
}

pub(super) struct Authorization<S> {
    selected: Mutex<Selected<S>>,
    profile: hid::Profile,
    revision: Arc<AtomicU64>,
    link: InputLink,
}

impl<S: Clone + Eq> Authorization<S> {
    pub fn new(profile: hid::Profile, revision: Arc<AtomicU64>, link: InputLink) -> Self {
        Self {
            selected: Mutex::new(Selected::default()),
            profile,
            revision,
            link,
        }
    }

    // Native observations and callback reconciliation share this lock so a
    // refresh cannot reinstall evidence sampled before a selected-peer loss.
    pub fn reconcile<R>(&self, observe: impl FnOnce(&mut Selected<S>) -> R) -> (R, u64) {
        let mut selected = self.selected.lock().unwrap_or_else(|error| {
            self.revoke();
            let mut selected = error.into_inner();
            *selected = Selected::default();
            selected
        });
        let previous = selected.clone();
        let result = observe(&mut selected);
        for kind in hid::ReportKind::ALL {
            if !self.profile.reports().contains(&kind) {
                selected.reports[kind.index()] = None;
            }
        }
        // Radio/power evidence only authorizes a bound selected transport.
        if selected.reports.iter().all(Option::is_none) {
            selected.radio_on = true;
            selected.suspended = false;
        }
        if *selected != previous {
            self.revoke();
        }
        (result, self.revision.load(Ordering::Acquire))
    }

    fn revoke(&self) {
        self.revision.fetch_add(1, Ordering::AcqRel);
        self.link.end();
    }

    pub fn subscription_changed(
        &self,
        kind: hid::ReportKind,
        observe: impl FnOnce(&Peer<S>) -> Option<Peer<S>>,
        refresh: &Refresh,
    ) {
        self.reconcile(|selected| {
            if let Some(peer) = &selected.reports[kind.index()] {
                selected.reports[kind.index()] = observe(peer);
            }
        });
        // All peers, including unselected peers, need a new discovery snapshot.
        refresh.signal();
    }

    pub fn session_changed(&self, session: &S, observe: impl FnOnce() -> bool, refresh: &Refresh) {
        self.reconcile(|selected| {
            if selected
                .reports
                .iter()
                .flatten()
                .any(|peer| &peer.session == session)
            {
                let active = observe();
                for peer in selected.reports.iter_mut().flatten() {
                    if &peer.session == session {
                        peer.active = active;
                    }
                }
            }
        });
        refresh.signal();
    }

    pub fn session_event(&self, session: &S, active: bool, refresh: &Refresh) {
        // The event records a real transition even if the session has already
        // recovered by the time its callback acquires the reconciliation lock.
        self.session_changed(session, || active, refresh);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bluetooth::report::ReportState;
    use taprelay_core::passthrough::Event;

    struct Harness {
        auth: Authorization<u64>,
        revision: Arc<AtomicU64>,
        link: InputLink,
        input: std::sync::mpsc::Receiver<taprelay_core::passthrough::Packet>,
        refresh: Refresh,
    }

    impl Harness {
        fn new(profile: hid::Profile) -> Self {
            let revision = Arc::new(AtomicU64::new(1));
            let (link, input) = InputLink::channel(revision.clone());
            let auth = Authorization::new(profile, revision.clone(), link.clone());
            auth.reconcile(|selected| {
                for &kind in profile.reports() {
                    selected.reports[kind.index()] = Some(Peer {
                        id: "native:selected".into(),
                        session: kind.index() as u64 + 10,
                        active: true,
                    });
                }
            });
            link.set_ready(revision.load(Ordering::Acquire));
            link.set_profile_available(true);
            assert!(link.request_profile());
            assert!(link.begin());
            Self {
                auth,
                revision,
                link,
                input,
                refresh: Refresh::new(),
            }
        }

        fn generation(&self) -> u64 {
            self.revision.load(Ordering::Acquire)
        }
    }

    #[test]
    fn unrelated_subscription_refresh_preserves_selected_input_generation_and_epoch() {
        let h = Harness::new(hid::Profile::Full);
        let generation = h.generation();
        let epoch = h.link.epoch();
        let request = h.link.profile_request();
        let mut cache = ReportState::new(hid::ReportKind::Keyboard);
        let mut bytes = hid::neutral(hid::ReportKind::Keyboard);
        bytes[2] = 4;
        cache.update("native:selected".into(), generation, generation, &bytes);
        for kind in hid::ReportKind::ALL {
            // Native enumeration returns the same selected peer even when an
            // unrelated peer subscribes, unsubscribes, or produces new wrappers.
            h.auth
                .subscription_changed(kind, |peer| Some(peer.clone()), &h.refresh);
            assert!(h.refresh.take());
            assert!(!h.refresh.take());
        }
        h.auth.session_changed(
            &999,
            || panic!("unselected session was queried"),
            &h.refresh,
        );
        assert!(h.refresh.take());
        assert_eq!(h.generation(), generation);
        assert_eq!(h.link.epoch(), epoch);
        assert_eq!(h.link.profile_request(), request);
        assert_eq!(cache.value(Some("native:selected"), h.generation()), bytes);
        assert!(
            h.link
                .submit(Event::Motion { dx: 3, dy: 4 }, std::time::Instant::now())
        );
        assert!(h.link.accepts(&h.input.try_recv().unwrap()));
    }

    #[test]
    fn each_required_subscription_loss_or_replacement_immediately_rejects_old_input() {
        for kind in hid::ReportKind::ALL {
            for replacement in [false, true] {
                let h = Harness::new(hid::Profile::Full);
                let generation = h.generation();
                assert!(
                    h.link
                        .submit(Event::Motion { dx: 3, dy: 4 }, std::time::Instant::now())
                );
                let packet = h.input.try_recv().unwrap();
                let mut cache = ReportState::new(kind);
                let bytes = vec![1; hid::neutral(kind).len()];
                cache.update("native:selected".into(), generation, generation, &bytes);
                h.auth.subscription_changed(
                    kind,
                    |peer| {
                        replacement.then(|| Peer {
                            session: peer.session + 100,
                            ..peer.clone()
                        })
                    },
                    &h.refresh,
                );
                assert!(h.refresh.take());
                assert!(h.generation() > generation);
                assert_eq!(h.link.epoch(), 0);
                assert_eq!(h.link.profile_request(), 0);
                assert!(!h.link.begin());
                assert!(!h.link.accepts(&packet));
                assert_eq!(
                    cache.value(Some("native:selected"), h.generation()),
                    hid::neutral(kind)
                );
            }
        }
    }

    #[test]
    fn selected_session_status_only_revokes_when_activity_actually_changes() {
        for kind in hid::ReportKind::ALL {
            let h = Harness::new(hid::Profile::Full);
            let generation = h.generation();
            let epoch = h.link.epoch();
            let session = kind.index() as u64 + 10;
            h.auth.session_changed(&session, || true, &h.refresh);
            assert_eq!(h.generation(), generation);
            assert_eq!(h.link.epoch(), epoch);
            h.auth.session_changed(&session, || false, &h.refresh);
            assert!(h.generation() > generation);
            assert_eq!(h.link.epoch(), 0);
            let lost = h.generation();
            h.auth.session_changed(&session, || false, &h.refresh);
            assert_eq!(h.generation(), lost);
        }
    }

    #[test]
    fn media_only_ignores_optional_report_events_but_refreshes_inventory() {
        let h = Harness::new(hid::Profile::MediaOnly);
        let generation = h.generation();
        let epoch = h.link.epoch();
        for kind in [hid::ReportKind::Keyboard, hid::ReportKind::Mouse] {
            h.auth.subscription_changed(
                kind,
                |_| panic!("optional report was queried"),
                &h.refresh,
            );
            assert!(h.refresh.take());
            h.auth.reconcile(|selected| {
                selected.reports[kind.index()] = Some(Peer {
                    id: "other".into(),
                    session: 999,
                    active: false,
                })
            });
        }
        assert_eq!(h.generation(), generation);
        assert_eq!(h.link.epoch(), epoch);
        h.auth
            .subscription_changed(hid::ReportKind::Consumer, |_| None, &h.refresh);
        assert!(h.generation() > generation);
    }

    #[test]
    fn loss_and_restore_between_refreshes_cannot_reuse_the_old_generation() {
        let h = Harness::new(hid::Profile::Full);
        let (sampled, generation) = h.auth.reconcile(|selected| selected.clone());
        let session = sampled.reports[2].as_ref().unwrap().session;
        h.auth
            .subscription_changed(hid::ReportKind::Mouse, |_| None, &h.refresh);
        h.auth.subscription_changed(
            hid::ReportKind::Mouse,
            |_| panic!("lost peer was authorized"),
            &h.refresh,
        );
        assert!(h.refresh.take());
        assert_eq!(h.link.epoch(), 0);
        let (_, restored_generation) = h.auth.reconcile(|selected| *selected = sampled);
        assert_ne!(generation, restored_generation);
        assert_eq!(h.link.profile_request(), 0);
        assert!(!h.link.begin());
        // Queued callbacks from a replaced session cannot alter its successor.
        h.auth
            .reconcile(|selected| selected.reports[2].as_mut().unwrap().session = session + 100);
        let restored = h.generation();
        h.auth.session_changed(
            &session,
            || panic!("retired session was queried"),
            &h.refresh,
        );
        assert_eq!(h.generation(), restored);
    }

    #[test]
    fn delayed_selected_loss_event_revokes_even_after_native_session_recovers() {
        let h = Harness::new(hid::Profile::Full);
        let generation = h.generation();
        assert!(
            h.link
                .submit(Event::Motion { dx: 3, dy: 4 }, std::time::Instant::now())
        );
        let packet = h.input.try_recv().unwrap();
        let auth = Arc::new(h.auth);
        let (entered, holding) = std::sync::mpsc::channel();
        let (release, resume) = std::sync::mpsc::channel();
        let blocked = auth.clone();
        let reconcile = std::thread::spawn(move || {
            blocked.reconcile(|_| {
                entered.send(()).unwrap();
                resume.recv().unwrap();
            });
        });
        holding.recv().unwrap();
        let callback_auth = auth.clone();
        let (queued, callback_started) = std::sync::mpsc::channel();
        let event = std::thread::spawn(move || {
            queued.send(()).unwrap();
            callback_auth.session_event(&10, false, &Refresh::new());
        });
        callback_started.recv().unwrap();
        // Native status has recovered to Active before the loss callback gets
        // the lock. Resampling it would hide the recorded Closed transition.
        let native_active = true;
        release.send(()).unwrap();
        reconcile.join().unwrap();
        event.join().unwrap();
        assert!(h.revision.load(Ordering::Acquire) > generation);
        assert_eq!(h.link.profile_request(), 0);
        assert_eq!(h.link.epoch(), 0);
        assert!(!h.link.accepts(&packet));
        auth.session_changed(&10, || native_active, &h.refresh);
        assert!(!h.link.begin());
    }

    #[test]
    fn selected_native_identity_power_and_radio_changes_revoke_authorization() {
        for change in 0..3 {
            let h = Harness::new(hid::Profile::Full);
            let generation = h.generation();
            h.auth.reconcile(|selected| match change {
                0 => selected.reports[0].as_mut().unwrap().id = "native:replacement".into(),
                1 => selected.suspended = true,
                _ => selected.radio_on = false,
            });
            assert!(h.generation() > generation);
            assert_eq!(h.link.epoch(), 0);
        }
    }
}
