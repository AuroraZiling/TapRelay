//! Dedicated message-pump thread for WH_KEYBOARD_LL / WH_MOUSE_LL.
//! Callbacks make a bounded, synchronous core-router decision. They never wait
//! for Bluetooth or the UI; parsed edges and configuration results are copied
//! to bounded, ordered queues. Active passthrough bypasses the application
//! runtime and sends raw relative mouse input to the HID sender.
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use taprelay_core::{
    function::FunctionConfigs,
    input::{InputCode, InputEvent, MouseButton},
    input_router::{InputRouter, PhysicalInput, RouteResult, RoutedInput, RoutedOutput},
    ports::BackendError,
};
use tokio::sync::mpsc::Sender;
use windows::Win32::{
    Foundation::*,
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::Input::KeyboardAndMouse::*,
    UI::WindowsAndMessaging::*,
};
use windows::core::w;
mod raw;
use taprelay_core::{
    function::{AppCommand, FunctionAction},
    passthrough::{Event as PassthroughEvent, InputLink, KeyUsage},
};
thread_local! { static PASSTHROUGH: RefCell<Option<InputLink>> = const { RefCell::new(None) }; }
thread_local! { static KEY_USAGES: RefCell<[Option<KeyUsage>; 256]> = const { RefCell::new([None; 256]) }; }
thread_local! { static KEYBOARD_HOOK: RefCell<Option<Hook>> = const { RefCell::new(None) }; }
thread_local! { static KEYBOARD_SEEN: RefCell<VecDeque<(KeyboardStamp, bool)>> = const { RefCell::new(VecDeque::new()) }; }

#[derive(Clone, Copy, PartialEq, Eq)]
struct KeyboardStamp {
    time: u32,
    scan: u32,
    key: u32,
    extended: bool,
    down: bool,
}

fn duplicate_keyboard(event: &KBDLLHOOKSTRUCT, message: u32, raw: bool) -> bool {
    let stamp = KeyboardStamp {
        time: event.time,
        scan: event.scanCode,
        key: match keyboard_code(event) {
            Some(InputCode::Key(key)) => u32::from(key),
            _ => event.vkCode,
        },
        extended: event.flags.contains(LLKHF_EXTENDED),
        down: matches!(message, WM_KEYDOWN | WM_SYSKEYDOWN),
    };
    KEYBOARD_SEEN.with(|seen| {
        let mut seen = seen.borrow_mut();
        if let Some(index) = seen
            .iter()
            .position(|(other, source)| *other == stamp && *source != raw)
        {
            seen.remove(index);
            return true;
        }
        if seen.len() == 1024 {
            seen.pop_front();
        }
        seen.push_back((stamp, raw));
        false
    })
}

thread_local! { static DISPATCH: RefCell<Option<Sender<RoutedInput>>> = const { RefCell::new(None) }; }
thread_local! { static POLICY: RefCell<Option<HookPolicy>> = const { RefCell::new(None) }; }
thread_local! { static CONSUMER: RefCell<Option<thread::Thread>> = const { RefCell::new(None) }; }
const POLICY_MESSAGE: u32 = WM_APP + 2;
type PolicyCommand = Box<dyn FnOnce(&mut HookPolicy) -> RouteResult + Send>;
struct PolicyRequest {
    command: PolicyCommand,
    reply: std::sync::mpsc::SyncSender<RouteResult>,
    ordered: bool,
}
thread_local! { static LAST_MOUSE_POINT: Cell<Option<POINT>> = const { Cell::new(None) }; }
thread_local! { static TAPRELAY_WINDOW: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) }; }
// The router decides a tap from a hold by elapsed time alone, so this thread
// must be woken at the pending threshold instead of at the next physical edge.
// The deadline is cached so an unchanged one costs no syscall.
thread_local! { static TIMER_DEADLINE: Cell<Option<Instant>> = const { Cell::new(None) }; }
const POLICY_TIMER: usize = 1;
#[derive(Clone, Copy)]
enum StopReason {
    Overflow,
    ReceiverClosed,
    CallbackPanic,
    PolicyUnavailable,
    ReplayFailed,
}
impl StopReason {
    fn message(self) -> &'static str {
        match self {
            Self::Overflow => "Input queue overflow; listener stopped to avoid a lost release",
            Self::ReceiverClosed => "Input consumer closed; listener stopped",
            Self::CallbackPanic => "Input callback panicked; listener stopped (see panic log)",
            Self::PolicyUnavailable => "Input routing state unavailable; listener stopped",
            Self::ReplayFailed => {
                "Local input replay failed; listener stopped to avoid a stuck key"
            }
        }
    }
}
thread_local! { static STOP_REASON: Cell<Option<StopReason>> = const { Cell::new(None) }; }
fn stop(reason: StopReason) {
    STOP_REASON.with(|r| {
        if r.get().is_none() {
            r.set(Some(reason));
        }
    });
    unsafe {
        PostQuitMessage(1);
    }
}
#[cfg(test)]
static KEYBOARD_PROBE: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);
#[cfg(test)]
const PROBE_TAG: usize = 0x54525052;
const REPLAY_TAG: usize = 0x54524c59;
pub struct InputHandle {
    id: u32,
    failed: Arc<AtomicBool>,
    reason: Arc<Mutex<Option<String>>>,
    commands: std::sync::mpsc::SyncSender<PolicyRequest>,
    configuration: Mutex<Option<(bool, bool, u64)>>,
    passthrough_link: Mutex<Option<InputLink>>,
    thread: Option<JoinHandle<()>>,
}

struct HookPolicy {
    router: InputRouter,
    revision: u64,
    listening: bool,
    recording: bool,
}

impl HookPolicy {
    fn new() -> Self {
        Self {
            router: InputRouter::new(&taprelay_core::function::default_configs(), 0),
            // The first app configuration must be applied even when its
            // persisted revision is also zero.
            revision: u64::MAX,
            listening: false,
            recording: false,
        }
    }

    fn configure(
        &mut self,
        configs: &FunctionConfigs,
        listening: bool,
        recording: bool,
        revision: u64,
    ) -> RouteResult {
        let mut result = RouteResult::default();
        if revision != self.revision {
            append(&mut result, self.router.update_config(configs, revision));
            self.revision = revision;
        }
        if listening != self.listening {
            append(&mut result, self.router.set_listening(listening));
            self.listening = listening;
        }
        if recording != self.recording {
            append(&mut result, self.router.set_recording(recording));
            self.recording = recording;
        }

        result
    }

    fn edge(&mut self, event: InputEvent) -> RouteResult {
        self.router.route_event(event)
    }

    fn local_edge(&mut self, event: InputEvent) -> RouteResult {
        self.router.route_local_event(event)
    }

    fn motion(&mut self, motion: PhysicalInput) -> RouteResult {
        match motion {
            PhysicalInput::Motion { dx, dy } => self.router.route_motion(dx, dy),
            PhysicalInput::Wheel {
                vertical,
                horizontal,
            } => self.router.route_wheel(vertical, horizontal),
            PhysicalInput::Edge { .. } => RouteResult::default(),
        }
    }

    fn terminate(&mut self, reason: taprelay_core::input_router::RouterReason) -> RouteResult {
        self.router.terminate(reason)
    }

    fn tick(&mut self, now: Instant) -> RouteResult {
        self.router.tick(now)
    }

    fn next_deadline(&self) -> Option<Instant> {
        self.router.next_deadline()
    }
}

/// Arm the thread timer for the router's next hold threshold. Re-arming the
/// same timer id replaces it, so only a change in the earliest deadline costs
/// a call. Without this the router would never learn that a held shortcut
/// crossed its threshold, because no further physical edge arrives.
fn arm_policy_timer() {
    let deadline = POLICY.with(|policy| {
        policy
            .borrow()
            .as_ref()
            .and_then(|policy| policy.next_deadline())
    });
    if TIMER_DEADLINE.with(Cell::get) == deadline {
        return;
    }
    TIMER_DEADLINE.with(|current| current.set(deadline));
    unsafe {
        match deadline {
            Some(deadline) => {
                let remaining = deadline.saturating_duration_since(Instant::now());
                let elapsed = remaining.as_millis().clamp(1, u32::MAX as u128) as u32;
                SetTimer(None, POLICY_TIMER, elapsed, None);
            }
            None => {
                let _ = KillTimer(None, POLICY_TIMER);
            }
        }
    }
}

fn policy_tick(now: Instant) -> RouteResult {
    POLICY.with(|policy| {
        policy
            .borrow_mut()
            .as_mut()
            .map(|policy| policy.tick(now))
            .unwrap_or_default()
    })
}

fn append(target: &mut RouteResult, mut next: RouteResult) {
    target.revision = next.revision;
    target.consume |= next.consume;
    target.outputs.append(&mut next.outputs);
}
impl InputHandle {
    pub fn attach_passthrough(&self, link: Option<InputLink>) {
        let mut previous = self
            .passthrough_link
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        if match (&*previous, &link) {
            (None, None) => true,
            (Some(a), Some(b)) => a.same(b),
            _ => false,
        } {
            return;
        }
        *previous = link.clone();
        self.request(
            Box::new(move |policy| {
                let result = policy.router.set_passthrough(false);
                PASSTHROUGH.with(|current| {
                    if let Some(old) = current.borrow_mut().take() {
                        old.end();
                    }
                    *current.borrow_mut() = link;
                });
                result
            }),
            true,
        );
    }

    pub fn passthrough_error(&self) -> Option<String> {
        self.passthrough_link
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(InputLink::take_failure)
    }
    pub fn failure(&self) -> Option<String> {
        self.reason
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn is_finished(&self) -> bool {
        self.failed.load(Ordering::Acquire) || self.thread.as_ref().is_none_or(|t| t.is_finished())
    }
    pub fn configure(
        &self,
        configs: &FunctionConfigs,
        listening: bool,
        recording: bool,
        revision: u64,
    ) -> RouteResult {
        let next = (listening, recording, revision);
        let mut previous = self.configuration.lock().unwrap_or_else(|e| e.into_inner());
        if *previous == Some(next) {
            return RouteResult::default();
        }
        *previous = Some(next);
        let configs = configs.clone();
        self.request(
            Box::new(move |policy| policy.configure(&configs, listening, recording, revision)),
            true,
        )
    }
    pub fn terminate(&self, reason: taprelay_core::input_router::RouterReason) -> RouteResult {
        self.request(Box::new(move |policy| policy.terminate(reason)), false)
    }
    fn request(&self, command: PolicyCommand, ordered: bool) -> RouteResult {
        if self.is_finished() {
            return RouteResult::default();
        }
        let (reply, result) = std::sync::mpsc::sync_channel(1);
        let outcome = (|| {
            self.commands
                .try_send(PolicyRequest {
                    command,
                    reply,
                    ordered,
                })
                .ok()?;
            unsafe {
                PostThreadMessageW(self.id, POLICY_MESSAGE, WPARAM(0), LPARAM(0)).ok()?;
            }
            result.recv_timeout(Duration::from_secs(1)).ok()
        })();
        outcome.unwrap_or_else(|| {
            self.failed.store(true, Ordering::Release);
            *self.reason.lock().unwrap_or_else(|e| e.into_inner()) =
                Some("Input policy command failed".into());
            unsafe {
                let _ = PostThreadMessageW(self.id, WM_QUIT, WPARAM(1), LPARAM(0));
            }
            RouteResult::default()
        })
    }
    pub fn start(sender: Sender<RoutedInput>) -> Result<Self, BackendError> {
        let (started, result) = std::sync::mpsc::sync_channel(1);
        let failed = Arc::new(AtomicBool::new(false));
        let failure = failed.clone();
        let reason = Arc::new(Mutex::new(None));
        let worker_reason = reason.clone();
        let (commands, requests) = std::sync::mpsc::sync_channel::<PolicyRequest>(16);
        let consumer = thread::current();
        let thread = thread::Builder::new()
            .name("taprelay-input".into())
            .spawn(move || {
                DISPATCH.with(|s| *s.borrow_mut() = Some(sender));
                POLICY.with(|s| *s.borrow_mut() = Some(HookPolicy::new()));
                CONSUMER.with(|s| *s.borrow_mut() = Some(consumer));
                LAST_MOUSE_POINT.with(|point| point.set(None));
                STOP_REASON.with(|r| r.set(None));
                let outcome = unsafe {
                    (|| -> windows::core::Result<()> {
                        // Force a thread queue into existence before the handle can post WM_QUIT.
                        let mut message = MSG::default();
                        let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
                        let module = GetModuleHandleW(None)?;
                        let _raw = raw::Capture::new()?;
                        let keyboard = KeyboardHook::new()?;
                        let mouse = Hook(SetWindowsHookExW(
                            WH_MOUSE_LL,
                            Some(mouse_proc),
                            Some(module.into()),
                            0,
                        )?);
                        // Snapshot keys already held before the listener was
                        // installed so shortcut prefixes retain their local ownership.
                        POLICY.with(|p| {
                            let mut p = p.borrow_mut();
                            let p = p.as_mut().expect("policy");
                            for key in 1..=254 {
                                if matches!(key, 3 | 7 | 0x10..=0x12) || GetAsyncKeyState(key) >= 0
                                {
                                    continue;
                                }
                                let code = match key {
                                    1 => InputCode::Mouse(MouseButton::Left),
                                    2 => InputCode::Mouse(MouseButton::Right),
                                    4 => InputCode::Mouse(MouseButton::Middle),
                                    5 => InputCode::Mouse(MouseButton::Side1),
                                    6 => InputCode::Mouse(MouseButton::Side2),
                                    _ => InputCode::Key(key as u8),
                                };
                                p.local_edge(InputEvent {
                                    code,
                                    down: true,
                                    captured: Instant::now(),
                                });
                            }
                        });
                        if started.send(Ok(GetCurrentThreadId())).is_err() {
                            return Ok(());
                        }
                        loop {
                            match GetMessageW(&mut message, None, 0, 0).0 {
                                -1 => return Err(windows::core::Error::from_thread()),
                                0 => {
                                    if message.wParam.0 != 0 {
                                        failure.store(true, Ordering::Release);
                                        let cause = STOP_REASON
                                            .with(|r| r.get())
                                            .map(StopReason::message)
                                            .unwrap_or("Input message loop stopped unexpectedly");
                                        *worker_reason.lock().unwrap_or_else(|e| e.into_inner()) =
                                            Some(cause.into());
                                        tracing::error!(reason = cause, "Input listener stopped");
                                    }
                                    break;
                                }
                                _ => {
                                    if message.message == POLICY_MESSAGE {
                                        while let Ok(request) = requests.try_recv() {
                                            let mut result = POLICY.with(|p| {
                                                (request.command)(
                                                    p.borrow_mut().as_mut().expect("policy"),
                                                )
                                            });
                                            prepare_hook_result(&mut result);
                                            if request.ordered {
                                                if !result.outputs.is_empty() {
                                                    dispatch(RoutedInput::Control(result));
                                                }
                                                let _ = request.reply.send(RouteResult::default());
                                            } else {
                                                let _ = request.reply.send(result);
                                            }
                                        }
                                        // A configuration change can cancel a
                                        // pending gesture, which must retire
                                        // the wake-up with it.
                                        arm_policy_timer();
                                        continue;
                                    }
                                    if message.message == WM_TIMER && message.hwnd.0.is_null() {
                                        let mut result = policy_tick(Instant::now());
                                        if prepare_hook_result(&mut result)
                                            && !result.outputs.is_empty()
                                        {
                                            dispatch(RoutedInput::Control(result));
                                        }
                                        arm_policy_timer();
                                        continue;
                                    }

                                    let _ = TranslateMessage(&message);
                                    DispatchMessageW(&message);
                                }
                            }
                        }
                        drop((keyboard, mouse));
                        Ok(())
                    })()
                };
                if let Err(e) = outcome {
                    failure.store(true, Ordering::Release);
                    let e = super::native_error("Low Level Hook", e);
                    *worker_reason.lock().unwrap_or_else(|e| e.into_inner()) = Some(e.to_string());
                    tracing::error!("{e}");
                    let _ = started.send(Err(e));
                }
                let mut cleanup = POLICY.with(|p| {
                    p.borrow_mut()
                        .as_mut()
                        .map(|p| {
                            p.terminate(taprelay_core::input_router::RouterReason::ListenerStopped)
                        })
                        .unwrap_or_default()
                });
                prepare_hook_result(&mut cleanup);
                if !cleanup.outputs.is_empty() {
                    dispatch(RoutedInput::Control(cleanup));
                }
                DISPATCH.with(|s| *s.borrow_mut() = None);
                CONSUMER.with(|s| {
                    if let Some(t) = s.borrow_mut().take() {
                        t.unpark();
                    }
                });
                POLICY.with(|s| *s.borrow_mut() = None);
                LAST_MOUSE_POINT.with(|point| point.set(None));
            })
            .map_err(|e| BackendError::Unavailable(e.to_string()))?;
        match result.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(id)) => Ok(Self {
                id,
                failed,
                reason,
                commands,
                configuration: Mutex::new(None),
                passthrough_link: Mutex::new(None),
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(e) => Err(BackendError::Unavailable(format!("Input startup: {e}"))),
        }
    }

    /// Replay only the local events that the synchronous hook consumed. The
    /// tag is recognized by this process so the replay cannot recurse into
    /// shortcut matching; other injected input keeps the existing policy.
    pub fn replay(&self, input: PhysicalInput) -> Result<(), BackendError> {
        Self::send_replay(input)
    }

    fn send_replay(input: PhysicalInput) -> Result<(), BackendError> {
        let mut inputs = Vec::new();
        match input {
            PhysicalInput::Edge { code, down } => match code {
                InputCode::Key(key) => inputs.push(INPUT {
                    r#type: INPUT_KEYBOARD,
                    Anonymous: INPUT_0 {
                        ki: replay_key(key, down),
                    },
                }),
                InputCode::Mouse(button) => {
                    let (flags, data) = match (button, down) {
                        (MouseButton::Left, true) => (MOUSEEVENTF_LEFTDOWN, 0),
                        (MouseButton::Left, false) => (MOUSEEVENTF_LEFTUP, 0),
                        (MouseButton::Right, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
                        (MouseButton::Right, false) => (MOUSEEVENTF_RIGHTUP, 0),
                        (MouseButton::Middle, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
                        (MouseButton::Middle, false) => (MOUSEEVENTF_MIDDLEUP, 0),
                        (MouseButton::Side1, true) => (MOUSEEVENTF_XDOWN, XBUTTON1),
                        (MouseButton::Side1, false) => (MOUSEEVENTF_XUP, XBUTTON1),
                        (MouseButton::Side2, true) => (MOUSEEVENTF_XDOWN, XBUTTON2),
                        (MouseButton::Side2, false) => (MOUSEEVENTF_XUP, XBUTTON2),
                    };
                    inputs.push(INPUT {
                        r#type: INPUT_MOUSE,
                        Anonymous: INPUT_0 {
                            mi: MOUSEINPUT {
                                dwFlags: flags,
                                mouseData: data as u32,
                                dwExtraInfo: REPLAY_TAG,
                                ..Default::default()
                            },
                        },
                    });
                }
            },
            PhysicalInput::Motion { dx, dy } => inputs.push(INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx,
                        dy,
                        dwFlags: MOUSEEVENTF_MOVE,
                        dwExtraInfo: REPLAY_TAG,
                        ..Default::default()
                    },
                },
            }),
            PhysicalInput::Wheel {
                vertical,
                horizontal,
            } => {
                if vertical != 0 {
                    inputs.push(INPUT {
                        r#type: INPUT_MOUSE,
                        Anonymous: INPUT_0 {
                            mi: MOUSEINPUT {
                                mouseData: vertical as u32,
                                dwFlags: MOUSEEVENTF_WHEEL,
                                dwExtraInfo: REPLAY_TAG,
                                ..Default::default()
                            },
                        },
                    });
                }
                if horizontal != 0 {
                    inputs.push(INPUT {
                        r#type: INPUT_MOUSE,
                        Anonymous: INPUT_0 {
                            mi: MOUSEINPUT {
                                mouseData: horizontal as u32,
                                dwFlags: MOUSEEVENTF_HWHEEL,
                                dwExtraInfo: REPLAY_TAG,
                                ..Default::default()
                            },
                        },
                    });
                }
            }
        }
        if inputs.is_empty() {
            return Ok(());
        }
        let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
        if sent as usize == inputs.len() {
            Ok(())
        } else {
            Err(BackendError::Unavailable(format!(
                "SendInput replay accepted {sent}/{} events",
                inputs.len()
            )))
        }
    }
}
impl Drop for InputHandle {
    fn drop(&mut self) {
        if let Some(t) = self.thread.take() {
            if !t.is_finished() {
                unsafe {
                    if let Err(e) = PostThreadMessageW(self.id, WM_QUIT, WPARAM(0), LPARAM(0)) {
                        tracing::warn!("Stop input message loop: {e}");
                    }
                }
            }
            if t.join().is_err() {
                tracing::error!("Input worker panicked (see panic log)");
            }
        }
    }
}
struct Hook(HHOOK);
struct KeyboardHook;
impl KeyboardHook {
    fn new() -> windows::core::Result<Self> {
        refresh_keyboard_hook()?;
        Ok(Self)
    }
}
impl Drop for KeyboardHook {
    fn drop(&mut self) {
        KEYBOARD_HOOK.with(|slot| slot.borrow_mut().take());
    }
}
fn refresh_keyboard_hook() -> windows::core::Result<()> {
    let next = unsafe {
        Hook(SetWindowsHookExW(
            WH_KEYBOARD_LL,
            Some(keyboard_proc),
            Some(GetModuleHandleW(None)?.into()),
            0,
        )?)
    };
    KEYBOARD_HOOK.with(|slot| slot.borrow_mut().replace(next));
    Ok(())
}

impl Drop for Hook {
    fn drop(&mut self) {
        unsafe {
            if let Err(e) = UnhookWindowsHookEx(self.0) {
                tracing::warn!("UnhookWindowsHookEx: {e}");
            }
        }
    }
}
fn emit(event: InputEvent, result: RouteResult) {
    // A full queue is a listener failure, not permission to drop a release.

    DISPATCH.with(|s| {
        if let Some(tx) = s.borrow().as_ref()
            && let Err(error) = tx.try_send(RoutedInput::Edge { event, result })
        {
            // A lost release makes pressed-state unreliable. Fail closed instead of replaying.
            stop(match error {
                tokio::sync::mpsc::error::TrySendError::Full(_) => StopReason::Overflow,
                tokio::sync::mpsc::error::TrySendError::Closed(_) => StopReason::ReceiverClosed,
            });
        }
    });
    wake_consumer();
}

fn emit_motion(mut result: RouteResult) {
    result
        .outputs
        .retain(|output| !matches!(output, RoutedOutput::Local(_)));
    if !result.outputs.is_empty() {
        dispatch(RoutedInput::Control(result));
    }
}
fn dispatch(input: RoutedInput) {
    DISPATCH.with(|s| {
        if let Some(tx) = s.borrow().as_ref()
            && let Err(error) = tx.try_send(input)
        {
            stop(match error {
                tokio::sync::mpsc::error::TrySendError::Full(_) => StopReason::Overflow,
                tokio::sync::mpsc::error::TrySendError::Closed(_) => StopReason::ReceiverClosed,
            });
        }
    });
    wake_consumer();
}
fn wake_consumer() {
    CONSUMER.with(|s| {
        if let Some(t) = s.borrow().as_ref() {
            t.unpark();
        }
    });
}

fn passthrough_active() -> bool {
    POLICY.with(|policy| {
        policy
            .borrow()
            .as_ref()
            .is_some_and(|p| p.router.passthrough())
    })
}

fn end_passthrough() {
    if passthrough_active() {
        tracing::info!("Passthrough ended; input returned to this computer");
    }
    let mut result = POLICY.with(|policy| {
        policy
            .borrow_mut()
            .as_mut()
            .map(|p| p.router.set_passthrough(false))
            .unwrap_or_default()
    });
    prepare_hook_result(&mut result);
    if !result.outputs.is_empty() {
        dispatch(RoutedInput::Control(result));
    }
}

fn sync_passthrough() {
    if passthrough_active()
        && !PASSTHROUGH.with(|link| link.borrow().as_ref().is_some_and(|link| link.epoch() != 0))
    {
        end_passthrough();
    }
}

fn toggle_passthrough() {
    if passthrough_active() {
        end_passthrough();
        return;
    }
    let link = PASSTHROUGH.with(|link| link.borrow().clone());
    let Some(link) = link else {
        return;
    };
    if !link.ready() {
        link.fail(
            "Passthrough requires keyboard and mouse HID subscriptions from the selected receiver",
        );
        return;
    }
    // Windows can silently remove a timed-out low-level hook. Start every
    // capture with a fresh registration rather than trusting its old handle.
    if let Err(error) = refresh_keyboard_hook() {
        link.fail(format!("Keyboard capture: {error}"));
        return;
    }
    if let Err(error) = raw::enable() {
        link.fail(format!("Raw input capture: {error}"));
        return;
    }
    if !link.begin() {
        raw::disable();
        return;
    }
    KEYBOARD_SEEN.with(|seen| seen.borrow_mut().clear());
    let mut result = POLICY.with(|policy| {
        policy
            .borrow_mut()
            .as_mut()
            .map(|p| p.router.set_passthrough(true))
            .unwrap_or_default()
    });
    prepare_hook_result(&mut result);
    if !result.outputs.is_empty() {
        dispatch(RoutedInput::Control(result));
    }
    tracing::info!("Passthrough enabled");
}

fn send_remote(input: PhysicalInput, captured: Instant) {
    let event = match input {
        PhysicalInput::Edge {
            code: InputCode::Key(key),
            down,
        } => {
            let usage = KEY_USAGES.with(|usages| usages.borrow()[key as usize]);
            let Some(usage) = usage else {
                PASSTHROUGH.with(|link| {
                    if let Some(link) = link.borrow().as_ref() {
                        link.fail(format!(
                            "Key {key:#x} has no HID mapping; passthrough stopped"
                        ));
                    }
                });
                return;
            };
            PassthroughEvent::Key { usage, down }
        }
        PhysicalInput::Edge {
            code: InputCode::Mouse(button),
            down,
        } => PassthroughEvent::Button { button, down },
        PhysicalInput::Motion { dx, dy } => PassthroughEvent::Motion { dx, dy },
        PhysicalInput::Wheel {
            vertical,
            horizontal,
        } => PassthroughEvent::Wheel {
            vertical,
            horizontal,
        },
    };
    PASSTHROUGH.with(|link| {
        if let Some(link) = link.borrow().as_ref() {
            link.submit(event, captured);
        }
    });
}

fn raw_mouse_edge(button: MouseButton, down: bool, captured: Instant) {
    let event = InputEvent {
        code: InputCode::Mouse(button),
        down,
        captured,
    };
    let mut result = policy_edge(event);
    arm_policy_timer();
    if prepare_hook_result_at(&mut result, captured) && !result.outputs.is_empty() {
        emit(event, result);
    }
}

fn taprelay_foreground() -> bool {
    unsafe { taprelay_window_handle().is_some_and(|window| window == GetForegroundWindow()) }
}

fn taprelay_window_handle() -> Option<HWND> {
    let mut window = None;
    TAPRELAY_WINDOW.with(|cached| {
        let mut value = cached.get();
        if value.0.is_null() {
            value = unsafe { FindWindowW(w!("TapRelay.Desktop.v2"), None).unwrap_or_default() };
            cached.set(value);
        }
        if !value.0.is_null() {
            window = Some(value);
        }
    });
    window
}

fn taprelay_window_at(point: POINT) -> bool {
    unsafe {
        let window = WindowFromPoint(point);
        if window.0.is_null() {
            return false;
        }
        let root = GetAncestor(window, GA_ROOT);
        taprelay_window_handle().is_some_and(|app| app == root)
    }
}

fn policy_edge(event: InputEvent) -> RouteResult {
    POLICY.with(|policy| {
        let mut borrowed = policy.borrow_mut();
        let Some(policy) = borrowed.as_mut() else {
            stop(StopReason::PolicyUnavailable);
            return RouteResult::default();
        };

        policy.edge(event)
    })
}
fn policy_local_edge(event: InputEvent) -> RouteResult {
    POLICY.with(|policy| {
        let mut borrowed = policy.borrow_mut();
        let Some(policy) = borrowed.as_mut() else {
            stop(StopReason::PolicyUnavailable);
            return RouteResult::default();
        };

        policy.local_edge(event)
    })
}
fn policy_motion(motion: PhysicalInput) -> RouteResult {
    POLICY.with(|policy| {
        let mut borrowed = policy.borrow_mut();
        let Some(policy) = borrowed.as_mut() else {
            stop(StopReason::PolicyUnavailable);
            return RouteResult::default();
        };

        policy.motion(motion)
    })
}
/// Replays are the one native operation permitted on the hook path. They are
/// performed before the original unconsumed event is allowed to continue, so
/// a pending modifier cannot arrive after the key or pointer event it prefixes.
fn prepare_hook_result(result: &mut RouteResult) -> bool {
    prepare_hook_result_at(result, Instant::now())
}

fn prepare_hook_result_at(result: &mut RouteResult, captured: Instant) -> bool {
    let outputs = std::mem::take(&mut result.outputs);
    let mut kept = Vec::with_capacity(outputs.len());
    for output in outputs {
        match output {
            RoutedOutput::EndPassthrough => {
                PASSTHROUGH.with(|link| {
                    if let Some(link) = link.borrow().as_ref() {
                        link.end();
                    }
                });
                raw::disable();
            }
            RoutedOutput::Remote(input) => {
                send_remote(input, captured);
            }
            RoutedOutput::Function {
                action: FunctionAction::App(AppCommand::TogglePassthrough),
                down: true,
                ..
            } => {
                toggle_passthrough();
            }
            RoutedOutput::Function {
                action: FunctionAction::Media(action),
                down,
                ..
            } if passthrough_active() => {
                PASSTHROUGH.with(|link| {
                    if let Some(link) = link.borrow().as_ref() {
                        link.submit(PassthroughEvent::Media { action, down }, Instant::now());
                    }
                });
            }
            RoutedOutput::Replay(input) => {
                if let Err(error) = InputHandle::send_replay(input) {
                    tracing::error!(?error, "Local input replay failed");
                    stop(StopReason::ReplayFailed);
                    return false;
                }
            }
            other => kept.push(other),
        }
    }
    result.outputs = kept;
    true
}

fn replay_key(key: u8, down: bool) -> KEYBDINPUT {
    let extended =
        matches!(key, 0xe0 | 0xa3 | 0xa5 | 0x21..=0x28 | 0x2d..=0x2e | 0x5b..=0x5c | 0x6f | 0x90);
    KEYBDINPUT {
        wVk: VIRTUAL_KEY(if key == 0xe0 { 0x0d } else { key as u16 }),
        dwFlags: (if down {
            KEYBD_EVENT_FLAGS(0)
        } else {
            KEYEVENTF_KEYUP
        }) | if extended {
            KEYEVENTF_EXTENDEDKEY
        } else {
            KEYBD_EVENT_FLAGS(0)
        },
        dwExtraInfo: REPLAY_TAG,
        ..Default::default()
    }
}
fn keyboard_code(event: &KBDLLHOOKSTRUCT) -> Option<InputCode> {
    if event.vkCode > 254 {
        return None;
    }
    let key = match event.vkCode as u8 {
        // The low-level hook reports the generic modifier VK. Preserve the
        // physical side for HID output while the core still normalizes it for
        // shortcut matching.
        0x10 => match event.scanCode {
            0x2a => 0xa0,
            0x36 => 0xa1,
            _ => 0x10,
        },
        0x11 if event.flags.contains(LLKHF_EXTENDED) => 0xa3,
        0x11 => 0xa2,
        0x12 if event.flags.contains(LLKHF_EXTENDED) => 0xa5,
        0x12 => 0xa4,
        0x0d if event.flags.contains(LLKHF_EXTENDED) => 0xe0,
        key => key,
    };
    Some(InputCode::Key(key))
}

pub fn key_name(key: u8) -> String {
    unsafe {
        let layout = GetKeyboardLayout(0);
        let (virtual_key, scan_code) = if key == 0xe0 {
            (u32::from(VK_RETURN.0), 0xe01c)
        } else {
            (
                u32::from(key),
                MapVirtualKeyExW(u32::from(key), MAPVK_VK_TO_VSC_EX, Some(layout)),
            )
        };

        if matches!(key, 0x30..=0x5a | 0xba..=0xc0 | 0xdb..=0xdf | 0xe2) {
            let mut text = [0u16; 8];
            let state = [0u8; 256];
            const DO_NOT_CHANGE_KEYBOARD_STATE: u32 = 4;
            let count = ToUnicodeEx(
                virtual_key,
                scan_code,
                &state,
                &mut text,
                DO_NOT_CHANGE_KEYBOARD_STATE,
                Some(layout),
            );
            if count != 0 {
                let length = count.unsigned_abs() as usize;
                let label = String::from_utf16_lossy(&text[..length]);
                if !label.trim().is_empty() {
                    return label.to_uppercase();
                }
            }
        }

        if scan_code != 0 {
            let extended = u32::from(scan_code & 0xff00 != 0) << 24;
            let parameter = ((scan_code & 0xff) << 16) | extended;
            let mut text = [0u16; 64];
            let count = GetKeyNameTextW(parameter as i32, &mut text);
            if count > 0 {
                return String::from_utf16_lossy(&text[..count as usize]);
            }
        }
    }
    taprelay_core::input::key_name(key)
}

pub fn keyboard_layout() -> usize {
    unsafe { GetKeyboardLayout(0).0 as usize }
}

fn raw_keyboard(keyboard: windows::Win32::UI::Input::RAWKEYBOARD, time: u32) {
    if !passthrough_active()
        || keyboard.ExtraInformation == REPLAY_TAG as u32
        || keyboard.VKey == 255
    {
        return;
    }
    let event = KBDLLHOOKSTRUCT {
        vkCode: u32::from(keyboard.VKey),
        scanCode: u32::from(keyboard.MakeCode),
        time,
        flags: if keyboard.Flags & RI_KEY_E0 as u16 != 0 {
            LLKHF_EXTENDED
        } else {
            KBDLLHOOKSTRUCT_FLAGS(0)
        },
        ..Default::default()
    };
    if !duplicate_keyboard(&event, keyboard.Message, true) {
        route_keyboard(&event, keyboard.Message);
    }
}

fn route_keyboard(event: &KBDLLHOOKSTRUCT, message: u32) -> bool {
    let edge = match message {
        WM_KEYDOWN | WM_SYSKEYDOWN => keyboard_code(event).map(|code| (code, true)),
        WM_KEYUP | WM_SYSKEYUP => keyboard_code(event).map(|code| (code, false)),
        _ => None,
    };
    if let Some((code, down)) = edge {
        if let InputCode::Key(key) = code {
            KEY_USAGES.with(|usages| {
                usages.borrow_mut()[key as usize] = taprelay_core::hid::keyboard_usage(
                    event.vkCode as u8,
                    event.scanCode,
                    event.flags.contains(LLKHF_EXTENDED),
                )
            });
        }
        let input = InputEvent {
            code,
            down,
            captured: raw::captured_at(event.time),
        };
        let mut result = if taprelay_foreground() {
            policy_local_edge(input)
        } else {
            policy_edge(input)
        };
        // A merged shortcut starts a hold window on its press edge
        // and ends one on its release edge.
        arm_policy_timer();

        if prepare_hook_result_at(&mut result, input.captured) {
            let consume = result.consume;
            if !passthrough_active() || !result.outputs.is_empty() {
                emit(input, result);
            }
            return consume;
        }
    }
    false
}

unsafe extern "system" fn keyboard_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let mut consume = false;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            sync_passthrough();
            // Windows owns this structure for the duration of the hook callback.
            let event = unsafe { &*(lp.0 as *const KBDLLHOOKSTRUCT) };
            #[cfg(test)]
            if event.dwExtraInfo == PROBE_TAG {
                KEYBOARD_PROBE.fetch_or(
                    if wp.0 as u32 == WM_KEYDOWN { 1 } else { 2 },
                    Ordering::SeqCst,
                );
            }
            if !event.flags.contains(LLKHF_INJECTED) {
                consume = if passthrough_active() && duplicate_keyboard(event, wp.0 as u32, false) {
                    true
                } else {
                    route_keyboard(event, wp.0 as u32)
                };
            }
        }));
        if outcome.is_err() {
            stop(StopReason::CallbackPanic);
        }
        if consume {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wp, lp) }
}
fn mouse_edge(message: u32, data: u32) -> Option<(MouseButton, bool)> {
    match message {
        WM_LBUTTONDOWN => Some((MouseButton::Left, true)),
        WM_LBUTTONUP => Some((MouseButton::Left, false)),
        WM_RBUTTONDOWN => Some((MouseButton::Right, true)),
        WM_RBUTTONUP => Some((MouseButton::Right, false)),
        WM_MBUTTONDOWN => Some((MouseButton::Middle, true)),
        WM_MBUTTONUP => Some((MouseButton::Middle, false)),
        WM_XBUTTONDOWN | WM_XBUTTONUP => match data >> 16 {
            1 => Some((MouseButton::Side1, message == WM_XBUTTONDOWN)),
            2 => Some((MouseButton::Side2, message == WM_XBUTTONDOWN)),
            _ => None,
        },
        _ => None,
    }
}

fn mouse_wheel(message: u32, data: u32) -> Option<PhysicalInput> {
    let delta = (data >> 16) as u16 as i16 as i32;
    match message {
        WM_MOUSEWHEEL => Some(PhysicalInput::Wheel {
            vertical: delta,
            horizontal: 0,
        }),
        WM_MOUSEHWHEEL => Some(PhysicalInput::Wheel {
            vertical: 0,
            horizontal: delta,
        }),
        _ => None,
    }
}
unsafe extern "system" fn mouse_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let mut consume = false;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            sync_passthrough();
            let event = unsafe { &*(lp.0 as *const MSLLHOOKSTRUCT) };

            if event.flags & LLMHF_INJECTED == 0 {
                if passthrough_active() {
                    if let Some(wheel) = mouse_wheel(wp.0 as u32, event.mouseData) {
                        send_remote(wheel, raw::captured_at(event.time));
                    }
                    consume = true;
                    return;
                }
                let local_window = taprelay_window_at(event.pt);
                if let Some((b, down)) = mouse_edge(wp.0 as u32, event.mouseData) {
                    let code = InputCode::Mouse(b);
                    let input = InputEvent {
                        code,
                        down,
                        captured: Instant::now(),
                    };
                    let mut result = if local_window {
                        policy_local_edge(input)
                    } else {
                        policy_edge(input)
                    };
                    arm_policy_timer();
                    if prepare_hook_result(&mut result) {
                        consume = result.consume;
                        emit(input, result);
                    }
                } else if wp.0 as u32 == WM_MOUSEMOVE {
                    let previous = LAST_MOUSE_POINT.with(|point| {
                        let previous = point.get();
                        point.set(Some(event.pt));
                        previous
                    });
                    if !local_window && let Some(previous) = previous {
                        let dx = event.pt.x - previous.x;
                        let dy = event.pt.y - previous.y;
                        if dx != 0 || dy != 0 {
                            let motion = PhysicalInput::Motion { dx, dy };
                            let mut result = policy_motion(motion);
                            if prepare_hook_result(&mut result) {
                                consume = result.consume;
                                emit_motion(result);
                            }
                        }
                    }
                } else if wp.0 as u32 == WM_MOUSEWHEEL {
                    if local_window {
                        return;
                    }
                    let value = (event.mouseData >> 16) as i16 as i32;
                    let motion = PhysicalInput::Wheel {
                        vertical: value,
                        horizontal: 0,
                    };
                    let mut result = policy_motion(motion);
                    if prepare_hook_result(&mut result) {
                        consume = result.consume;
                        emit_motion(result);
                    }
                } else if wp.0 as u32 == WM_MOUSEHWHEEL {
                    if local_window {
                        return;
                    }
                    let value = (event.mouseData >> 16) as i16 as i32;
                    let motion = PhysicalInput::Wheel {
                        vertical: 0,
                        horizontal: value,
                    };
                    let mut result = policy_motion(motion);
                    if prepare_hook_result(&mut result) {
                        consume = result.consume;
                        emit_motion(result);
                    }
                }
            }
        }));
        if outcome.is_err() {
            stop(StopReason::CallbackPanic);
        }
        if consume {
            return LRESULT(1);
        }
    }
    unsafe { CallNextHookEx(None, code, wp, lp) }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_release_preserves_keypad_enter_and_right_modifier_identity() {
        let enter = replay_key(0xe0, false);
        assert_eq!(enter.wVk, VK_RETURN);
        assert_ne!(enter.dwFlags.0 & KEYEVENTF_EXTENDEDKEY.0, 0);
        assert_ne!(enter.dwFlags.0 & KEYEVENTF_KEYUP.0, 0);
        assert_ne!(
            replay_key(0xa3, false).dwFlags.0 & KEYEVENTF_EXTENDEDKEY.0,
            0
        );
        assert_eq!(
            replay_key(0x0d, false).dwFlags.0 & KEYEVENTF_EXTENDEDKEY.0,
            0
        );
    }

    #[test]
    fn native_key_names_never_expose_virtual_key_codes() {
        for key in [0x14, 0x2c, 0x5d, 0x90, 0xa6, 0xaf, 0xb3, 0xba, 0xc3, 0xe0] {
            assert!(!key_name(key).starts_with("VK"));
        }
    }

    #[test]
    #[ignore = "requires an interactive Windows desktop; sends a tagged F24 probe"]
    fn keyboard_hook_receives_os_events_but_rejects_injected_input() {
        use windows::Win32::UI::Input::KeyboardAndMouse::*;
        KEYBOARD_PROBE.store(0, Ordering::SeqCst);
        let (tx, mut rx) = tokio::sync::mpsc::channel(256);
        let _handle = InputHandle::start(tx).unwrap();
        let inputs = [KEYBD_EVENT_FLAGS(0), KEYEVENTF_KEYUP].map(|flags| INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VK_F24,
                    dwFlags: flags,
                    dwExtraInfo: PROBE_TAG,
                    ..Default::default()
                },
            },
        });
        assert_eq!(
            unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) },
            2
        );
        let deadline = Instant::now() + Duration::from_secs(3);
        while KEYBOARD_PROBE.load(Ordering::SeqCst) != 3 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(
            KEYBOARD_PROBE.load(Ordering::SeqCst),
            3,
            "Installed keyboard hook did not receive the OS probe"
        );
        while let Ok(event) = rx.try_recv() {
            if let RoutedInput::Edge { event, .. } = event {
                assert_ne!(
                    event.code,
                    InputCode::Key(0x87),
                    "Injected F24 must not reach the recorder"
                );
            }
        }
    }
    #[test]
    #[ignore = "requires pressing and releasing physical F8 on an interactive desktop"]
    fn physical_keyboard_f8_reaches_recorder() {
        use taprelay_core::input::{InputState, Recorder};
        let (tx, mut rx) = tokio::sync::mpsc::channel(256);
        let _handle = InputHandle::start(tx).unwrap();
        let mut state = InputState::default();
        let mut recorder = Recorder::default();
        let until = Instant::now() + Duration::from_secs(30);
        eprintln!("Ready: press and release physical F8 within 30 seconds.");
        while Instant::now() < until {
            while let Ok(dispatch) = rx.try_recv() {
                let RoutedInput::Edge { event, .. } = dispatch else {
                    continue;
                };
                if event.code != InputCode::Key(0x77) {
                    continue;
                }
                eprintln!("F8 down={}", event.down);
                state.update(event);
                if let Some(shortcut) = recorder.observe(&state, event) {
                    assert_eq!(shortcut.display(), "F8");
                    return;
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("No complete physical F8 press/release received");
    }
    #[test]
    #[ignore = "requires an interactive Windows desktop"]
    fn native_hook_start_stop_restart() {
        for _ in 0..3 {
            let (tx, _rx) = tokio::sync::mpsc::channel(256);
            let handle = InputHandle::start(tx).unwrap();
            assert!(!handle.is_finished());
            drop(handle);
        }
    }
    #[test]
    fn mouse_motion_wheel_and_unknown_xbuttons_are_ignored() {
        assert_eq!(mouse_edge(WM_MOUSEMOVE, 0), None);
        assert_eq!(mouse_edge(WM_MOUSEWHEEL, 120 << 16), None);
        assert_eq!(mouse_edge(WM_XBUTTONDOWN, 3 << 16), None);
        assert_eq!(
            mouse_edge(WM_XBUTTONUP, 2 << 16),
            Some((MouseButton::Side2, false))
        );
    }
}
