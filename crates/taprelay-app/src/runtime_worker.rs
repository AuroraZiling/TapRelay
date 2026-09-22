use crate::{
    action::BindingCommand,
    config::{Config, Device},
    feedback::TestStatus,
    runtime::{CaptureSession, Runtime},
};
use anyhow::{Context, Result};
use std::{
    collections::{BTreeMap, VecDeque},
    ops::Deref,
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};
use taprelay_core::{function::FunctionConfigs, input::InputEvent, state::Snapshot};

type Command = Box<dyn FnOnce(&mut Runtime) -> Result<()> + Send>;

pub struct View {
    functions: FunctionConfigs,
    remembered_device: Option<Device>,
    pub state: Snapshot,
    pub matched: u64,
    pub listening: bool,
    capture: Option<CaptureSession>,
    pub test: TestStatus,
    pub bindings_revision: u64,
}
impl View {
    pub fn capture(&self) -> Option<&CaptureSession> {
        self.capture.as_ref()
    }
    fn take(runtime: &Runtime) -> Self {
        Self {
            functions: runtime.functions.clone(),
            remembered_device: runtime.remembered_device.clone(),
            state: runtime.state.clone(),
            matched: runtime.matched,
            listening: runtime.listening,
            capture: runtime.capture().cloned(),
            test: runtime.test.clone(),
            bindings_revision: runtime.bindings_revision,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Poll,
    Edit,
    Cancel,
    Shutdown,
}
struct Request {
    id: u64,
    kind: Kind,
    command: Command,
    keys: Vec<InputEvent>,
    ui_input: Option<Instant>,
}
struct Pending {
    kind: Kind,
    since: Instant,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Notice {
    Error(String),
    Test(TestStatus),
}
enum Response {
    Completed {
        id: u64,
        view: View,
        result: Result<()>,
    },
    Notice(Notice),
    Stopped {
        view: View,
        error: Option<String>,
    },
}

#[derive(Debug)]
pub enum HandoffError {
    Busy,
    Stopped,
    QueueFull,
}
impl std::fmt::Display for HandoffError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Busy => "An operation is still in progress",
            Self::Stopped => "Runtime stopped",
            Self::QueueFull => "Runtime command queue unavailable",
        })
    }
}
impl std::error::Error for HandoffError {}

pub struct RuntimeHandle {
    pub config: Config,
    view: View,
    bindings_changed: bool,
    window_keys: Vec<InputEvent>,
    ui_input: Option<Instant>,
    notices: VecDeque<Notice>,
    pending: BTreeMap<u64, Pending>,
    next_id: u64,
    closing: bool,
    stopped: bool,
    commands: Option<mpsc::SyncSender<Request>>,
    responses: mpsc::Receiver<Response>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Deref for RuntimeHandle {
    type Target = View;
    fn deref(&self) -> &View {
        &self.view
    }
}

fn collect_notices(
    runtime: &mut Runtime,
    last_test: &mut TestStatus,
    tx: &mpsc::Sender<Response>,
    error: Option<&str>,
) {
    if let Some(message) = runtime.error.take()
        && Some(message.as_str()) != error
    {
        let _ = tx.send(Response::Notice(Notice::Error(message)));
    }
    if *last_test != runtime.test {
        *last_test = runtime.test.clone();
        if matches!(last_test, TestStatus::Succeeded | TestStatus::Failed(_)) && error.is_none() {
            let _ = tx.send(Response::Notice(Notice::Test(last_test.clone())));
        }
    }
}

impl RuntimeHandle {
    pub fn new(config: Config) -> Result<Self> {
        Self::spawn(config, Runtime::new, Runtime::tick, 64)
    }

    pub(super) fn spawn(
        config: Config,
        create: impl FnOnce(Config) -> Runtime + Send + 'static,
        tick: fn(&mut Runtime),
        capacity: usize,
    ) -> Result<Self> {
        let runtime_config = config.clone();
        let (commands, rx) = mpsc::sync_channel::<Request>(capacity);
        let (tx, responses) = mpsc::channel();
        let (started, ready) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("taprelay-runtime".into())
            .spawn(move || {
                let mut runtime = create(runtime_config);
                if started.send(View::take(&runtime)).is_err() {
                    return;
                }
                let mut last_test = TestStatus::Idle;
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    loop {
                        tick(&mut runtime);
                        collect_notices(&mut runtime, &mut last_test, &tx, None);
                        match rx.try_recv() {
                            Ok(request) => {
                                if let Some(boundary) = request.ui_input {
                                    runtime.consume_ui_input_at(boundary);
                                }
                                runtime.window_keys.extend(request.keys);
                                // Window edges accepted before an edit cross the same revision barrier as hook input.
                                tick(&mut runtime);
                                collect_notices(&mut runtime, &mut last_test, &tx, None);
                                if request.kind == Kind::Shutdown {
                                    break;
                                }
                                let result = (request.command)(&mut runtime);
                                let error = result.as_ref().err().map(|error| format!("{error:#}"));
                                collect_notices(
                                    &mut runtime,
                                    &mut last_test,
                                    &tx,
                                    error.as_deref(),
                                );
                                if tx
                                    .send(Response::Completed {
                                        id: request.id,
                                        view: View::take(&runtime),
                                        result,
                                    })
                                    .is_err()
                                {
                                    break;
                                }
                            }
                            Err(mpsc::TryRecvError::Disconnected) => break,
                            Err(mpsc::TryRecvError::Empty) => {
                                thread::park_timeout(Duration::from_millis(50))
                            }
                        }
                    }
                }));
                let cleanup =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runtime.shutdown()));
                collect_notices(&mut runtime, &mut last_test, &tx, None);
                let error = (outcome.is_err() || cleanup.is_err()).then(|| {
                    "Runtime worker panicked; unfinished operations were not retried".to_owned()
                });
                let _ = tx.send(Response::Stopped {
                    view: View::take(&runtime),
                    error,
                });
            })?;
        let view = ready.recv().context("Runtime worker failed to start")?;
        Ok(Self {
            config,
            view,
            bindings_changed: false,
            window_keys: vec![],
            ui_input: None,
            notices: VecDeque::new(),
            pending: BTreeMap::new(),
            next_id: 0,
            closing: false,
            stopped: false,
            commands: Some(commands),
            responses,
            worker: Some(worker),
        })
    }

    fn update(
        &mut self,
        command: impl FnOnce(&mut Runtime) -> Result<()> + Send + 'static,
    ) -> Result<()> {
        self.enqueue(Kind::Edit, Box::new(command))
    }

    fn enqueue(&mut self, kind: Kind, command: Command) -> Result<()> {
        self.poll();
        anyhow::ensure!(!self.stopped && !self.closing, HandoffError::Stopped);
        if kind == Kind::Edit && self.busy() {
            return Err(HandoffError::Busy.into());
        }
        if matches!(kind, Kind::Poll | Kind::Cancel)
            && self.pending.values().any(|pending| pending.kind == kind)
        {
            return Ok(());
        }
        let id = self.next_id + 1;
        let request = Request {
            id,
            kind,
            command,
            keys: std::mem::take(&mut self.window_keys),
            ui_input: self.ui_input.take(),
        };
        match self
            .commands
            .as_ref()
            .context("Runtime stopped")?
            .try_send(request)
        {
            Ok(()) => {}
            Err(error) => {
                let (request, reason) = match error {
                    mpsc::TrySendError::Full(request) => (request, HandoffError::QueueFull),
                    mpsc::TrySendError::Disconnected(request) => (request, HandoffError::Stopped),
                };
                self.window_keys = request.keys;
                self.ui_input = request.ui_input;
                return Err(reason.into());
            }
        }
        self.next_id = id;
        self.pending.insert(
            id,
            Pending {
                kind,
                since: Instant::now(),
            },
        );
        if kind == Kind::Shutdown {
            self.closing = true;
        }
        if let Some(worker) = &self.worker {
            worker.thread().unpark();
        }
        Ok(())
    }

    fn apply(&mut self, mut view: View) {
        self.bindings_changed |= view.bindings_revision != self.view.bindings_revision;
        self.config.functions = std::mem::take(&mut view.functions);
        self.config.remembered_device = view.remembered_device.take();
        self.view = view;
    }

    pub fn poll(&mut self) {
        loop {
            match self.responses.try_recv() {
                Ok(Response::Completed { id, view, result }) => {
                    self.pending.remove(&id);
                    self.apply(view);
                    if let Err(error) = result {
                        self.notices.push_back(Notice::Error(format!("{error:#}")));
                    }
                }
                Ok(Response::Notice(notice)) => self.notices.push_back(notice),
                Ok(Response::Stopped { view, error }) => {
                    self.apply(view);
                    if let Some(error) = error {
                        self.notices.push_back(Notice::Error(error));
                    }
                    self.mark_stopped();
                }
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    if !self.stopped {
                        self.notices.push_back(Notice::Error(
                            "Runtime worker stopped before completing queued operations".into(),
                        ));
                        self.mark_stopped();
                    }
                    break;
                }
            }
        }
        if self
            .worker
            .as_ref()
            .is_some_and(|worker| worker.is_finished())
        {
            let _ = self.worker.take().unwrap().join();
        }
    }

    fn mark_stopped(&mut self) {
        self.stopped = true;
        self.pending.clear();
        self.commands.take();
        self.window_keys.clear();
        self.view.capture = None;
        self.view.test = TestStatus::Idle;
        self.view.listening = false;
        self.view.state.input = false;
        self.view.state.ready = false;
    }

    pub fn tick(&mut self) {
        self.poll();
        if !self.stopped
            && !self.closing
            && let Err(error) = self.enqueue(Kind::Poll, Box::new(|_| Ok(())))
        {
            self.notices.push_back(Notice::Error(error.to_string()));
        }
    }
    pub fn busy(&self) -> bool {
        self.pending
            .values()
            .any(|pending| pending.kind != Kind::Poll)
    }
    pub fn slow(&self) -> bool {
        self.pending
            .values()
            .any(|pending| pending.since.elapsed() >= Duration::from_secs(5))
    }
    pub fn progress_pending(&self) -> bool {
        self.pending.values().any(|pending| {
            pending.kind != Kind::Poll && pending.since.elapsed() >= Duration::from_millis(300)
        })
    }
    pub fn stopped(&self) -> bool {
        self.stopped
    }
    pub fn take_notice(&mut self) -> Option<Notice> {
        self.notices.pop_front()
    }
    pub fn take_bindings_changed(&mut self) -> bool {
        std::mem::take(&mut self.bindings_changed)
    }
    pub fn window_key(&mut self, event: InputEvent) {
        if !self.closing && !self.stopped {
            self.window_keys.push(event);
        }
    }
    pub fn consume_ui_input(&mut self) {
        self.ui_input = Some(Instant::now());
    }
    pub fn apply_binding_command(&mut self, command: BindingCommand) -> Result<()> {
        let kind = if command == BindingCommand::CancelCapture {
            Kind::Cancel
        } else {
            Kind::Edit
        };
        self.enqueue(
            kind,
            Box::new(move |runtime| runtime.apply_binding_command(command)),
        )
    }
    pub fn shutdown(&mut self) -> Result<()> {
        self.poll();
        if self.closing || self.stopped {
            return Ok(());
        }
        self.enqueue(Kind::Shutdown, Box::new(|_| Ok(())))
    }
    pub fn resume(&mut self) -> Result<()> {
        self.update(|runtime| {
            runtime.start_bluetooth()?;
            if runtime.listening {
                runtime.apply_binding_command(BindingCommand::CancelCapture)?;
                runtime.set_listening(false)?;
                runtime.set_listening(true)?;
            }
            Ok(())
        })
    }
    pub fn start_bluetooth(&mut self) -> Result<()> {
        self.update(move |r| r.start_bluetooth())
    }
    pub fn set_listening(&mut self, on: bool) -> Result<()> {
        self.update(move |r| r.set_listening(on))
    }
    pub fn set_passthrough_reverse_scroll(&mut self, reverse: bool) -> Result<()> {
        self.update(move |runtime| {
            runtime.set_passthrough_reverse_scroll(reverse);
            Ok(())
        })?;
        self.config.options.passthrough_reverse_scroll = reverse;
        Ok(())
    }
    pub fn pair(&mut self, id: String) -> Result<()> {
        self.update(move |r| r.pair(id))
    }
    pub fn choose(&mut self, id: String) -> Result<()> {
        self.update(move |r| r.choose(id))
    }
    pub fn disconnect(&mut self) -> Result<()> {
        self.update(move |r| r.disconnect())
    }
    pub fn bluetooth_settings(&mut self) -> Result<()> {
        self.update(move |r| r.bluetooth_settings())
    }
    pub fn refresh(&mut self) -> Result<()> {
        self.update(move |r| r.refresh())
    }
    pub fn send(&mut self) -> Result<()> {
        self.update(move |r| r.send())
    }
}
impl Drop for RuntimeHandle {
    fn drop(&mut self) {
        self.commands.take();
        if let Some(worker) = self.worker.take() {
            worker.thread().unpark();
            if worker.is_finished() {
                let _ = worker.join();
            }
        }
    }
}

#[cfg(test)]
mod tests;
