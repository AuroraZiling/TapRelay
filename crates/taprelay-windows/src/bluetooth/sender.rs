use super::{api, buffer};
use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
use taprelay_core::{
    command::{CommandPhase, MediaCommand},
    hid::{self, ConsumerState, InputReports, ReportKind},
    passthrough::{Event, InputLink, KeyUsage, Packet, ReportSchedule},
    ports::BackendError,
};
use windows::Devices::Bluetooth::GenericAttributeProfile::{
    GattCommunicationStatus, GattLocalCharacteristic, GattSubscribedClient,
};

#[derive(Clone)]
pub(super) struct Endpoint {
    pub characteristic: GattLocalCharacteristic,
    pub client: GattSubscribedClient,
    pub current: Arc<Mutex<Vec<u8>>>,
}

enum Command {
    Detach(mpsc::SyncSender<()>),
    Configure {
        generation: u64,
        endpoints: [Option<Endpoint>; 3],
        ready: bool,
        interval: Option<Duration>,
    },
    Notify {
        generation: u64,
        kind: ReportKind,
        refresh: bool,
        endpoint: Endpoint,
        reply: mpsc::SyncSender<Result<(), BackendError>>,
    },
    Media {
        generation: u64,
        created: Instant,
        action: MediaCommand,
        down: bool,
        endpoint: Endpoint,
        reply: mpsc::SyncSender<Result<(), BackendError>>,
    },
}

pub(super) struct Sender {
    commands: mpsc::SyncSender<Command>,
    pub link: InputLink,
    stopping: Arc<AtomicBool>,
    detaching: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}

impl Sender {
    pub fn new(revision: Arc<AtomicU64>) -> Result<Self, BackendError> {
        let (link, input) = InputLink::channel(revision.clone());
        let (commands, control) = mpsc::sync_channel(16);
        let stopping = Arc::new(AtomicBool::new(false));
        let worker_stop = stopping.clone();
        let detaching = Arc::new(AtomicBool::new(false));
        let worker_detaching = detaching.clone();
        let worker_link = link.clone();
        let worker = thread::Builder::new()
            .name("taprelay-hid".into())
            .spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let _apartment = crate::Apartment::new()?;
                    let mut worker = Worker {
                        link: worker_link.clone(),
                        revision,
                        stopping: worker_stop,
                        detaching: worker_detaching,
                        endpoints: std::array::from_fn(|_| None),
                        generation: 0,
                        authorized: false,
                        epoch: 0,
                        reports: InputReports::default(),
                        consumer: ConsumerState::default(),
                        pulses: Vec::new(),
                        next_owner: 0,
                        schedule: ReportSchedule::default(),
                    };
                    worker.run(control, input);
                    Ok::<_, BackendError>(())
                }));
                let message = match result {
                    Ok(Ok(())) => "HID sender stopped".to_string(),
                    Ok(Err(e)) => e.to_string(),
                    Err(_) => "HID sender panicked".to_string(),
                };
                worker_link.fail(message);
            })
            .map_err(|e| BackendError::Unavailable(e.to_string()))?;
        link.set_worker(worker.thread().clone());
        Ok(Self {
            commands,
            link,
            stopping,
            detaching,
            worker: Some(worker),
        })
    }

    fn submit(&self, command: Command) -> Result<(), BackendError> {
        self.commands
            .try_send(command)
            .map_err(|_| BackendError::Unavailable("HID control queue unavailable".into()))?;
        self.link.wake();
        Ok(())
    }

    pub fn detach(&self) -> Result<(), BackendError> {
        self.detaching.store(true, Ordering::Release);
        self.link.end();
        let (reply, result) = mpsc::sync_channel(1);
        self.submit(Command::Detach(reply))?;
        result.recv_timeout(Duration::from_secs(1)).map_err(|_| {
            BackendError::Unavailable("HID sender did not release its endpoints".into())
        })
    }

    pub fn configure(
        &self,
        generation: u64,
        endpoints: [Option<Endpoint>; 3],
        ready: bool,
        interval: Option<Duration>,
    ) {
        if let Err(e) = self.submit(Command::Configure {
            generation,
            endpoints,
            ready,
            interval,
        }) {
            self.link.fail(e.to_string());
        }
    }

    pub fn notify(
        &self,
        kind: ReportKind,
        refresh: bool,
        endpoint: Endpoint,
    ) -> Result<(), BackendError> {
        let (reply, result) = mpsc::sync_channel(1);
        self.submit(Command::Notify {
            generation: self.link.generation(),
            kind,
            refresh,
            endpoint,
            reply,
        })?;
        result.recv_timeout(Duration::from_secs(1)).map_err(|_| {
            BackendError::Unavailable("HID sender did not complete notification".into())
        })?
    }

    pub fn media(
        &self,
        action: MediaCommand,
        phase: CommandPhase,
        created: Instant,
        endpoint: Endpoint,
    ) -> Result<(), BackendError> {
        let (reply, result) = mpsc::sync_channel(1);
        self.submit(Command::Media {
            generation: self.link.generation(),
            created,
            action,
            down: phase == CommandPhase::Press,
            endpoint,
            reply,
        })?;
        result.recv_timeout(Duration::from_secs(1)).map_err(|_| {
            BackendError::Unavailable("HID sender did not complete media command".into())
        })?
    }
}

impl Drop for Sender {
    fn drop(&mut self) {
        self.link.end();
        self.stopping.store(true, Ordering::Release);
        self.link.wake();
        if let Some(worker) = self.worker.take() {
            let until = Instant::now() + Duration::from_secs(1);
            while !worker.is_finished() && Instant::now() < until {
                thread::sleep(Duration::from_millis(5));
            }
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}

struct Worker {
    link: InputLink,
    revision: Arc<AtomicU64>,
    stopping: Arc<AtomicBool>,
    detaching: Arc<AtomicBool>,
    endpoints: [Option<Endpoint>; 3],
    generation: u64,
    authorized: bool,
    epoch: u64,
    reports: InputReports,
    consumer: ConsumerState,
    pulses: Vec<(Instant, u64, MediaCommand)>,
    next_owner: u64,
    schedule: ReportSchedule,
}

impl Worker {
    fn fail(&mut self, error: &BackendError) {
        tracing::warn!("Passthrough: {error}");
        self.authorized = false;
        self.link.fail(error.to_string());
        self.revision.fetch_add(1, Ordering::AcqRel);
    }

    fn reset(&mut self) -> Result<(), BackendError> {
        self.reports = InputReports::default();
        self.consumer = ConsumerState::default();
        self.pulses.clear();
        self.schedule.clear_input();
        let mut error = None;
        for kind in ReportKind::ALL {
            if let Some(endpoint) = self.endpoints[kind.index()].clone()
                && let Err(e) = self.notify(kind, &hid::neutral(kind), &endpoint)
            {
                error.get_or_insert(e);
            }
        }
        error.map_or(Ok(()), Err)
    }

    fn reconcile(&mut self) {
        if self.detaching.load(Ordering::Acquire) {
            self.authorized = false;
            self.link.end();
            return;
        }
        let active = self.link.epoch();
        if self.epoch != active || self.generation != self.link.generation() {
            let changed = self.generation != self.link.generation();
            if self.epoch == 0
                && active != 0
                && !self.consumer.is_empty()
                && let Err(error) = self.reset()
            {
                self.fail(&error);
            }
            if self.epoch != 0 || (changed && !self.consumer.is_empty()) {
                self.link.end();
                if let Err(e) = self.reset() {
                    self.fail(&e);
                }
            }
            self.epoch = 0;
            if self.generation != self.link.generation() {
                self.authorized = false;
                self.link.end();
                self.generation = self.link.generation();
                self.reports = InputReports::default();
                self.consumer.clear();
                self.pulses.clear();
                self.schedule.clear_input();
                self.endpoints = std::array::from_fn(|_| None);
            }
            if self.authorized {
                self.link.set_ready(self.generation);
            }
            self.epoch = self.link.epoch();
        }
    }

    fn run(&mut self, commands: mpsc::Receiver<Command>, input: mpsc::Receiver<Packet>) {
        while !self.stopping.load(Ordering::Acquire) {
            self.reconcile();
            let command = match commands.try_recv() {
                Ok(command) => Some(command),
                Err(mpsc::TryRecvError::Disconnected) => break,
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(command) = command {
                self.command(command);
                continue;
            }
            if self.schedule.wait(Instant::now()).is_zero()
                && let Some(index) = self
                    .pulses
                    .iter()
                    .position(|(due, _, _)| *due <= Instant::now())
            {
                let (_, owner, action) = self.pulses.remove(index);
                self.consumer.release(owner, action);
                if let Err(e) = self.send_report(ReportKind::Consumer, &self.consumer.report()) {
                    self.fail(&e);
                }
                continue;
            }
            if self.schedule.wait(Instant::now()).is_zero() {
                match self.schedule.take(&input, &self.link, Instant::now()) {
                    Ok(Some(packet)) => {
                        if !self.link.accepts(&packet) {
                            continue;
                        }
                        self.epoch = packet.epoch;
                        if let Err(e) = self.event(packet.event) {
                            self.fail(&e);
                        }
                        continue;
                    }
                    Err(error) => {
                        self.fail(&BackendError::Unavailable(error.into()));
                        continue;
                    }
                    Ok(None) => {}
                }
            }
            let wait = self
                .pulses
                .iter()
                .map(|(due, _, _)| due.saturating_duration_since(Instant::now()))
                .min()
                .unwrap_or(Duration::from_millis(10))
                .max(self.schedule.wait(Instant::now()))
                .min(Duration::from_millis(10));
            thread::park_timeout(wait);
        }
        self.link.end();
    }

    fn command(&mut self, command: Command) {
        match command {
            Command::Detach(reply) => {
                self.link.end();
                self.authorized = false;
                self.epoch = 0;
                self.endpoints = std::array::from_fn(|_| None);
                self.reports = InputReports::default();
                self.consumer.clear();
                self.pulses.clear();
                self.schedule.clear_input();
                self.detaching.store(false, Ordering::Release);
                let _ = reply.send(());
            }
            Command::Configure {
                generation,
                endpoints,
                ready,
                interval,
            } => {
                if self.detaching.load(Ordering::Acquire) || generation != self.link.generation() {
                    return;
                }
                if generation != self.generation {
                    self.link.end();
                    self.epoch = 0;
                    self.reports = InputReports::default();
                    self.consumer = ConsumerState::default();
                    self.pulses.clear();
                }
                self.generation = generation;
                let previous = self.schedule.interval();
                self.schedule.set_connection_interval(interval);
                if previous != self.schedule.interval() {
                    tracing::info!(
                        interval_us = self.schedule.interval().as_micros() as u64,
                        "HID report cadence changed"
                    );
                }
                self.endpoints = endpoints;
                self.authorized = ready && self.endpoints.iter().all(Option::is_some);
                if self.authorized {
                    self.link.set_ready(generation);
                } else {
                    self.link.end();
                }
            }
            Command::Notify {
                generation,
                kind,
                refresh,
                endpoint,
                reply,
            } => {
                let result = if generation != self.link.generation() {
                    Err(BackendError::Stale)
                } else if refresh && self.link.epoch() != 0 {
                    Ok(())
                } else {
                    let bytes = if refresh {
                        endpoint
                            .current
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .clone()
                    } else {
                        if kind == ReportKind::Consumer {
                            self.consumer.clear();
                            self.pulses.clear();
                        }
                        hid::neutral(kind)
                    };
                    self.notify(kind, &bytes, &endpoint)
                };
                // A refresh can wait for its slot while the hook enables
                // capture. Skipping that refresh must not revoke the new mode.
                let result = match result {
                    Err(BackendError::Stale)
                        if refresh
                            && generation == self.link.generation()
                            && self.link.epoch() != 0 =>
                    {
                        Ok(())
                    }
                    result => result,
                };
                if let Err(ref error) = result
                    && !matches!(error, BackendError::Stale)
                {
                    self.fail(error);
                }
                let _ = reply.send(result);
            }
            Command::Media {
                generation,
                created,
                action,
                down,
                endpoint,
                reply,
            } => {
                let result =
                    if generation != self.link.generation() || !self.link.accepts_media(created) {
                        Err(BackendError::Stale)
                    } else {
                        self.endpoints[0] = Some(endpoint);
                        self.media(action, down)
                    };
                if let Err(ref error) = result
                    && !matches!(error, BackendError::Stale)
                {
                    self.fail(error);
                }
                let _ = reply.send(result);
            }
        }
    }

    fn media(&mut self, action: MediaCommand, down: bool) -> Result<(), BackendError> {
        let owner = u64::from(action.usage());
        if !down {
            self.consumer.release(owner, action);
        } else if matches!(action, MediaCommand::Rewind | MediaCommand::FastForward) {
            self.consumer.press(owner, action);
        } else {
            self.next_owner = self.next_owner.wrapping_add(1).max(1);
            let owner = self.next_owner | (1 << 63);
            self.consumer.press(owner, action);
            self.pulses
                .push((Instant::now() + Duration::from_millis(40), owner, action));
        }
        self.send_report(ReportKind::Consumer, &self.consumer.report())
    }

    fn event(&mut self, event: Event) -> Result<(), BackendError> {
        let report = match event {
            Event::Key {
                usage: KeyUsage::Keyboard(usage),
                down,
            } => Some((
                ReportKind::Keyboard,
                self.reports
                    .key(usage, down)
                    .map_err(|e| BackendError::Unavailable(e.into()))?,
            )),
            Event::Key {
                usage: KeyUsage::Consumer(usage),
                down,
            } => {
                let owner = 0x10000 + u64::from(usage);
                if down {
                    self.consumer.press_usage(owner, usage);
                } else {
                    self.consumer.release_usage(owner, usage);
                }
                Some((ReportKind::Consumer, self.consumer.report()))
            }
            Event::Button { button, down } => {
                Some((ReportKind::Mouse, self.reports.button(button, down)))
            }
            Event::Motion { dx, dy } => Some((
                ReportKind::Mouse,
                self.reports
                    .motion(dx, dy)
                    .map_err(|e| BackendError::Unavailable(e.into()))?,
            )),
            Event::Wheel {
                vertical,
                horizontal,
            } => self
                .reports
                .wheel_with_direction(vertical, horizontal, self.link.reverse_scroll())
                .map_err(|e| BackendError::Unavailable(e.into()))?
                .map(|bytes| (ReportKind::Mouse, bytes)),
            Event::Media { action, down } => {
                return self.media(action, down);
            }
            Event::Pointer {
                dx,
                dy,
                vertical,
                horizontal,
            } => Some((
                ReportKind::Mouse,
                self.reports
                    .pointer(dx, dy, vertical, horizontal, self.link.reverse_scroll())
                    .map_err(|e| BackendError::Unavailable(e.into()))?,
            )),
        };
        if let Some((kind, bytes)) = report {
            self.send_report(kind, &bytes)?;
        }
        Ok(())
    }

    fn send_report(&mut self, kind: ReportKind, bytes: &[u8]) -> Result<(), BackendError> {
        if kind == ReportKind::Consumer && self.consumer.exceeds_capacity() {
            return Err(BackendError::Unavailable(
                "Consumer HID report capacity exceeded".into(),
            ));
        }
        let endpoint = self.endpoints[kind.index()]
            .clone()
            .ok_or_else(|| BackendError::Unavailable("HID subscriber unavailable".into()))?;
        self.notify(kind, bytes, &endpoint)
    }

    fn notify(
        &mut self,
        kind: ReportKind,
        bytes: &[u8],
        endpoint: &Endpoint,
    ) -> Result<(), BackendError> {
        if self.detaching.load(Ordering::Acquire) {
            return Err(BackendError::Stale);
        }
        let generation = self.link.generation();
        let epoch = self.link.epoch();
        while !self.schedule.wait(Instant::now()).is_zero() {
            if self.stopping.load(Ordering::Acquire)
                || self.detaching.load(Ordering::Acquire)
                || generation != self.link.generation()
                || epoch != self.link.epoch()
            {
                return Err(BackendError::Stale);
            }
            thread::park_timeout(
                self.schedule
                    .wait(Instant::now())
                    .min(Duration::from_millis(5)),
            );
        }
        self.schedule.sent(Instant::now());
        let mut current = bytes.to_vec();
        if kind == ReportKind::Mouse {
            current[1..].fill(0);
        }
        *endpoint.current.lock().unwrap_or_else(|e| e.into_inner()) = current;
        let pending = api(
            "HID.Notify",
            endpoint.characteristic.NotifyValueForSubscribedClientAsync(
                &api("DataWriter", buffer(bytes))?,
                &endpoint.client,
            ),
        )?;
        let (tx, rx) = mpsc::sync_channel(1);
        let completion_error = api(
            "HID.Completed",
            pending.SetCompleted(&windows_future::AsyncOperationCompletedHandler::new(
                move |_, _| {
                    let _ = tx.try_send(());
                    Ok(())
                },
            )),
        )
        .err();
        if let Some(error) = &completion_error {
            self.fail(error);
        }
        let started = Instant::now();
        let mut timed_out = false;
        loop {
            match rx.recv_timeout(Duration::from_millis(5)) {
                Ok(()) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    thread::sleep(Duration::from_millis(5));
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if completion_error.is_some()
                && pending
                    .Status()
                    .is_ok_and(|status| status != windows_future::AsyncStatus::Started)
            {
                break;
            }
            if !timed_out && started.elapsed() >= Duration::from_millis(250) {
                timed_out = true;
                self.fail(&BackendError::Unavailable(
                    "HID notification stalled; input returned to this computer".into(),
                ));
            }
            if self.stopping.load(Ordering::Acquire) || self.detaching.load(Ordering::Acquire) {
                let _ = pending.Cancel();
                return Err(BackendError::Stale);
            }
        }
        if let Some(error) = completion_error {
            return Err(error);
        }
        let result = api("HID.GetResults", pending.GetResults())?;
        if timed_out {
            return Err(BackendError::Stale);
        }
        let status = api("HID.Status", result.Status())?;
        if status != GattCommunicationStatus::Success {
            return Err(BackendError::Unavailable(format!(
                "HID notification: {status:?}"
            )));
        }
        Ok(())
    }
}
