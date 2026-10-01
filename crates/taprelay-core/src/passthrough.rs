use crate::{command::MediaCommand, input::MouseButton};
mod flow;
pub use flow::ReportSchedule;
use std::{
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub const QUEUE_CAPACITY: usize = 1024;
pub const MAX_INPUT_AGE: Duration = Duration::from_millis(250);
pub const MOUSE_REPORT_RATES: [u16; 6] = [0, 60, 125, 250, 500, 1000];

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum KeyUsage {
    Keyboard(u8),
    Consumer(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Event {
    /// End whole-device capture while keeping custom mapping ownership.
    ReleasePhysical,
    Mapping {
        output: crate::mapping::MappingOutput,
        token: u64,
        down: bool,
    },
    Key {
        usage: KeyUsage,
        down: bool,
    },
    Button {
        button: MouseButton,
        down: bool,
    },
    Motion {
        dx: i32,
        dy: i32,
    },
    Wheel {
        vertical: i32,
        horizontal: i32,
    },
    Pointer {
        dx: i32,
        dy: i32,
        vertical: i32,
        horizontal: i32,
    },
    Media {
        action: MediaCommand,
        down: bool,
    },
}

#[derive(Debug)]
pub struct Packet {
    pub epoch: u64,
    pub generation: u64,
    pub captured: Instant,
    pub event: Event,
}

struct Shared {
    mapping_epoch: AtomicU64,
    revision: Arc<AtomicU64>,
    ready: AtomicU64,
    mouse_percent: AtomicU64,
    mouse_report_rate: AtomicU64,
    reverse_scroll: AtomicBool,
    // Zero revokes capture; odd tokens authorize one start, even tokens own
    // an active session. One CAS prevents a racing stop from being undone.
    mode: AtomicU64,
    next_epoch: AtomicU64,
    disconnected_epoch: AtomicU64,
    started: Instant,
    media_boundary: AtomicU64,
    worker: OnceLock<thread::Thread>,
    failure: Mutex<Option<String>>,
    profile_available: AtomicBool,
    // Distinct request tokens reject replies from a cancelled capture even
    // when the user has already requested another capture.
    requested: AtomicU64,
}

#[derive(Clone)]
pub struct InputLink {
    shared: Arc<Shared>,
    sender: mpsc::SyncSender<Packet>,
}

impl InputLink {
    pub fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.shared, &other.shared)
    }
    pub fn channel(revision: Arc<AtomicU64>) -> (Self, mpsc::Receiver<Packet>) {
        let (sender, receiver) = mpsc::sync_channel(QUEUE_CAPACITY);
        (
            Self {
                sender,
                shared: Arc::new(Shared {
                    mapping_epoch: AtomicU64::new(0),
                    revision,
                    ready: AtomicU64::new(0),
                    mouse_percent: AtomicU64::new(100),
                    mouse_report_rate: AtomicU64::new(0),
                    reverse_scroll: AtomicBool::new(false),
                    mode: AtomicU64::new(0),
                    next_epoch: AtomicU64::new(1),
                    disconnected_epoch: AtomicU64::new(0),
                    started: Instant::now(),
                    media_boundary: AtomicU64::new(0),
                    worker: OnceLock::new(),
                    failure: Mutex::new(None),
                    profile_available: AtomicBool::new(false),
                    requested: AtomicU64::new(0),
                }),
            },
            receiver,
        )
    }

    pub fn set_worker(&self, worker: thread::Thread) {
        let _ = self.shared.worker.set(worker);
    }

    pub fn set_mouse_percent(&self, percent: u16) {
        self.shared
            .mouse_percent
            .store(u64::from(percent.clamp(1, 100)), Ordering::Release);
    }

    pub fn mouse_percent(&self) -> u16 {
        self.shared.mouse_percent.load(Ordering::Acquire) as u16
    }

    /// Zero follows the connection interval; other values are target reports per second.
    pub fn set_mouse_report_rate(&self, hz: u16) {
        self.shared
            .mouse_report_rate
            .store(u64::from(hz.min(1000)), Ordering::Release);
        self.wake();
    }

    pub fn mouse_report_rate(&self) -> u16 {
        self.shared.mouse_report_rate.load(Ordering::Acquire) as u16
    }

    pub fn set_reverse_scroll(&self, reverse: bool) {
        self.shared.reverse_scroll.store(reverse, Ordering::Release);
    }

    pub fn reverse_scroll(&self) -> bool {
        self.shared.reverse_scroll.load(Ordering::Acquire)
    }

    pub fn generation(&self) -> u64 {
        self.shared.revision.load(Ordering::Acquire)
    }

    pub fn set_ready(&self, generation: u64) {
        if generation != self.generation() || self.shared.mode.load(Ordering::Acquire) != 0 {
            return;
        }
        self.shared.ready.store(generation, Ordering::Release);
        let token = self.shared.next_epoch.fetch_add(2, Ordering::AcqRel);
        let _ = self
            .shared
            .mode
            .compare_exchange(0, token, Ordering::AcqRel, Ordering::Acquire);
    }

    pub fn ready(&self) -> bool {
        let ready = self.shared.ready.load(Ordering::Acquire);
        self.shared.mode.load(Ordering::Acquire) % 2 == 1
            && ready != 0
            && ready == self.generation()
    }

    pub fn epoch(&self) -> u64 {
        let mode = self.shared.mode.load(Ordering::Acquire);
        if mode.is_multiple_of(2) && self.shared.ready.load(Ordering::Acquire) == self.generation()
        {
            mode
        } else {
            0
        }
    }

    pub fn begin(&self) -> bool {
        let token = self.shared.mode.load(Ordering::Acquire);
        if token.is_multiple_of(2) || !self.ready() {
            return false;
        }
        let epoch = token.wrapping_add(1);
        if self
            .shared
            .mode
            .compare_exchange(token, epoch, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return false;
        }
        self.retire_media();
        self.wake();
        self.epoch() == epoch
    }

    pub fn end(&self) {
        if self.epoch() != 0 && self.mapping_epoch() != 0 {
            self.submit_mapping_event(Event::ReleasePhysical, Instant::now());
        }
        self.update_profile_request(false);
        self.suspend();
    }

    /// Retain which capture lost its transport even after automatic profile recovery.
    /// Ordinary stops must not overwrite this evidence with a later inactive state.
    pub fn disconnect(&self) {
        let epoch = self.epoch();
        if epoch != 0 {
            self.shared
                .disconnected_epoch
                .store(epoch, Ordering::Release);
        }
        self.end();
    }

    pub fn disconnected_epoch(&self) -> u64 {
        self.shared.disconnected_epoch.load(Ordering::Acquire)
    }

    pub fn set_profile_available(&self, available: bool) {
        self.shared
            .profile_available
            .store(available, Ordering::Release);
    }

    pub fn request_profile(&self) -> bool {
        if !self.shared.profile_available.load(Ordering::Acquire) {
            return false;
        }
        self.update_profile_request(true);
        self.wake();
        true
    }

    pub fn profile_requested(&self) -> bool {
        self.profile_request() != 0
    }

    pub fn capture_requested(&self) -> bool {
        !self
            .shared
            .requested
            .load(Ordering::Acquire)
            .is_multiple_of(2)
    }

    /// Request the full HID profile without capturing all keyboard/mouse input.
    pub fn set_mapping_profile(&self, enabled: bool) {
        let current = self.mapping_epoch();
        if enabled && current == 0 {
            let next = self.shared.next_epoch.fetch_add(2, Ordering::AcqRel) | (1 << 63);
            self.shared.mapping_epoch.store(next, Ordering::Release);
            self.wake();
        } else if !enabled && current != 0 {
            self.shared.mapping_epoch.store(0, Ordering::Release);
            self.wake();
        }
    }

    pub fn mapping_epoch(&self) -> u64 {
        self.shared.mapping_epoch.load(Ordering::Acquire)
    }

    pub fn submit_mapping(
        &self,
        output: crate::mapping::MappingOutput,
        token: u64,
        down: bool,
        captured: Instant,
    ) -> bool {
        self.submit_mapping_event(
            Event::Mapping {
                output,
                token,
                down,
            },
            captured,
        )
    }

    /// A child process receives input already routed by its supervisor. Keep
    /// admission independent of whole-device capture so media can coexist.
    pub fn submit_forwarded(&self, event: Event, captured: Instant) -> bool {
        self.submit_mapping_event(event, captured)
    }

    fn submit_mapping_event(&self, event: Event, captured: Instant) -> bool {
        let epoch = self.mapping_epoch();
        if epoch == 0
            || self.shared.mode.load(Ordering::Acquire) == 0
            || self.shared.ready.load(Ordering::Acquire) != self.generation()
        {
            return false;
        }
        let packet = Packet {
            epoch,
            generation: self.generation(),
            captured,
            event,
        };
        if self.sender.try_send(packet).is_err() {
            // Prevent fail -> end -> ReleasePhysical from recursively trying a
            // full queue, and revoke held mapping admission until restarted.
            self.set_mapping_profile(false);
            self.fail("Mapping input queue unavailable; listener must be restarted");
            return false;
        }
        self.wake();
        true
    }

    fn update_profile_request(&self, requested: bool) {
        let mut token = self.shared.requested.load(Ordering::Acquire);
        while !token.is_multiple_of(2) != requested {
            match self.shared.requested.compare_exchange(
                token,
                token.wrapping_add(1),
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => break,
                Err(current) => token = current,
            }
        }
    }

    pub fn profile_request(&self) -> u64 {
        let token = self.shared.requested.load(Ordering::Acquire);
        // Mapping ownership keeps the profile stable across passthrough toggles.
        let mapping = self.mapping_epoch();
        if mapping != 0 {
            mapping
        } else if token.is_multiple_of(2) {
            0
        } else {
            token
        }
    }

    pub fn suspend(&self) {
        let mode = self.shared.mode.load(Ordering::Acquire);
        if mode != 0 && mode.is_multiple_of(2) {
            self.retire_media();
        }
        self.shared.mode.store(0, Ordering::Release);
        self.wake();
    }

    fn retire_media(&self) {
        self.shared.media_boundary.fetch_max(
            self.shared
                .started
                .elapsed()
                .as_nanos()
                .min(u64::MAX as u128) as u64,
            Ordering::AcqRel,
        );
    }

    pub fn accepts_media(&self, created: Instant) -> bool {
        let boundary = self.shared.media_boundary.load(Ordering::Acquire);
        self.epoch() == 0
            && (boundary == 0
                || created
                    .saturating_duration_since(self.shared.started)
                    .as_nanos()
                    > u128::from(boundary))
    }

    pub fn fail(&self, reason: impl Into<String>) {
        self.end();
        *self
            .shared
            .failure
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(reason.into());
    }

    pub fn take_failure(&self) -> Option<String> {
        self.shared
            .failure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
    }

    pub fn submit(&self, event: Event, captured: Instant) -> bool {
        let epoch = self.epoch();
        if epoch == 0 {
            return false;
        }
        let packet = Packet {
            epoch,
            generation: self.generation(),
            captured,
            event,
        };
        if self.sender.try_send(packet).is_err() {
            self.fail("Passthrough input queue unavailable; input returned to this computer");
            return false;
        }
        self.wake();
        true
    }

    pub fn accepts(&self, packet: &Packet) -> bool {
        if packet.epoch != 0 && packet.epoch == self.mapping_epoch() {
            return packet.generation == self.generation()
                && self.shared.mode.load(Ordering::Acquire) != 0;
        }
        packet.epoch != 0 && packet.epoch == self.epoch() && packet.generation == self.generation()
    }

    pub fn wake(&self) {
        if let Some(worker) = self.shared.worker.get() {
            worker.unpark();
        }
    }
}

#[cfg(test)]
mod observation_tests {
    use super::*;

    #[test]
    fn forwarded_keyboard_and_mouse_admission_keeps_media_controls_available() {
        let (link, rx) = InputLink::channel(Arc::new(AtomicU64::new(1)));
        link.set_ready(1);
        link.set_mapping_profile(true);
        for event in [
            Event::Mapping {
                output: crate::mapping::MappingOutput::Keyboard {
                    usage: 0x4f,
                    modifiers: 0,
                },
                token: 5,
                down: true,
            },
            Event::Key {
                usage: KeyUsage::Keyboard(0x4f),
                down: true,
            },
            Event::Button {
                button: MouseButton::Left,
                down: true,
            },
            Event::ReleasePhysical,
        ] {
            assert!(link.submit_forwarded(event, Instant::now()));
            assert!(link.accepts(&rx.try_recv().unwrap()));
        }
        assert_eq!(link.epoch(), 0);
        assert!(link.accepts_media(Instant::now()));
        assert!(link.submit_forwarded(Event::Motion { dx: 5, dy: 0 }, Instant::now()));
        let old = rx.try_recv().unwrap();
        link.set_mapping_profile(false);
        link.set_mapping_profile(true);
        assert!(!link.accepts(&old));
    }

    #[test]
    fn full_mapping_queue_revokes_ownership_without_recursive_failure() {
        let (link, _rx) = InputLink::channel(Arc::new(AtomicU64::new(1)));
        link.set_mapping_profile(true);
        link.set_ready(1);
        assert!(link.begin());
        let output = crate::mapping::MappingOutput::Keyboard {
            usage: 0x4f,
            modifiers: 0,
        };
        for token in 1..=QUEUE_CAPACITY as u64 {
            assert!(link.submit_mapping(output, token, true, Instant::now()));
        }
        assert!(!link.submit_mapping(output, 2000, false, Instant::now()));
        assert_eq!(link.mapping_epoch(), 0);
        assert_eq!(link.epoch(), 0);
        assert!(link.take_failure().is_some());
    }

    #[test]
    fn mappings_request_full_hid_without_requesting_input_capture_and_reject_old_sessions() {
        let (link, input) = InputLink::channel(Arc::new(AtomicU64::new(1)));
        link.set_mapping_profile(true);
        assert!(link.profile_requested());
        assert!(!link.capture_requested());
        assert_eq!(link.epoch(), 0);
        let output = crate::mapping::MappingOutput::Keyboard {
            usage: 0x4f,
            modifiers: 0,
        };
        assert!(!link.submit_mapping(output, 1, true, Instant::now()));
        link.set_ready(1);
        assert!(link.submit_mapping(output, 1, true, Instant::now()));
        let old = input.try_recv().unwrap();
        assert!(link.accepts(&old));
        let request = link.profile_request();
        link.set_profile_available(true);
        assert!(link.request_profile());
        assert!(link.capture_requested());
        assert_eq!(link.profile_request(), request);
        assert!(link.begin());
        link.end();
        link.set_ready(1);
        let cleanup = input.try_recv().unwrap();
        assert_eq!(cleanup.event, Event::ReleasePhysical);
        assert!(link.accepts(&cleanup));
        assert!(link.profile_requested() && !link.capture_requested());
        link.set_mapping_profile(false);
        link.set_mapping_profile(true);
        assert!(!link.accepts(&old));
    }

    #[test]
    fn transport_loss_is_retained_for_its_epoch_but_normal_stops_do_not_mark_loss() {
        let (link, _receiver) = InputLink::channel(Arc::new(AtomicU64::new(1)));
        link.set_ready(1);
        assert!(link.begin());
        let first = link.epoch();
        link.disconnect();
        assert_eq!(link.epoch(), 0);
        assert_eq!(link.disconnected_epoch(), first);
        link.end();
        assert_eq!(link.disconnected_epoch(), first);
        link.set_ready(1);
        assert!(link.begin());
        assert_ne!(link.epoch(), first);
        link.end();
        assert_eq!(link.disconnected_epoch(), first);
    }
}
