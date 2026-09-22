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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyUsage {
    Keyboard(u8),
    Consumer(u16),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
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
    revision: Arc<AtomicU64>,
    ready: AtomicU64,
    mouse_percent: AtomicU64,
    reverse_scroll: AtomicBool,
    // Zero revokes capture; odd tokens authorize one start, even tokens own
    // an active session. One CAS prevents a racing stop from being undone.
    mode: AtomicU64,
    next_epoch: AtomicU64,
    started: Instant,
    media_boundary: AtomicU64,
    worker: OnceLock<thread::Thread>,
    failure: Mutex<Option<String>>,
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
                    revision,
                    ready: AtomicU64::new(0),
                    mouse_percent: AtomicU64::new(100),
                    reverse_scroll: AtomicBool::new(false),
                    mode: AtomicU64::new(0),
                    next_epoch: AtomicU64::new(1),
                    started: Instant::now(),
                    media_boundary: AtomicU64::new(0),
                    worker: OnceLock::new(),
                    failure: Mutex::new(None),
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
        packet.epoch != 0 && packet.epoch == self.epoch() && packet.generation == self.generation()
    }

    pub fn wake(&self) {
        if let Some(worker) = self.shared.worker.get() {
            worker.unpark();
        }
    }
}
