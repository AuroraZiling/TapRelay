//! Dedicated message-pump thread for WH_KEYBOARD_LL / WH_MOUSE_LL.
//! Callbacks make a bounded, synchronous core-router decision. They never wait
//! for Bluetooth or the UI; parsed edges and high-frequency pointer deltas are
//! copied to one bounded, ordered queue for the application worker.
use std::{
    cell::{Cell, RefCell},
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
    input_router::{
        InputRouter, PhysicalEvent, PhysicalInput, RouteResult, RoutedInput, RoutedOutput,
    },
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

thread_local! { static DISPATCH: RefCell<Option<Sender<RoutedInput>>> = const { RefCell::new(None) }; }
thread_local! { static POLICY: RefCell<Option<Arc<Mutex<HookPolicy>>>> = const { RefCell::new(None) }; }
thread_local! { static LAST_MOUSE_POINT: Cell<Option<POINT>> = const { Cell::new(None) }; }
thread_local! { static TAPRELAY_WINDOW: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) }; }
thread_local! { static PENDING_MOTION: RefCell<Option<RoutedInput>> = const { RefCell::new(None) }; }
thread_local! { static MOTION_FLUSH_POSTED: Cell<bool> = const { Cell::new(false) }; }
const FLUSH_MOTION_MESSAGE: u32 = WM_APP + 1;
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
            Self::PolicyUnavailable => {
                "Input policy was busy; listener stopped to preserve ordering"
            }
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
    policy: Arc<Mutex<HookPolicy>>,
    thread: Option<JoinHandle<()>>,
}

struct HookPolicy {
    router: InputRouter,
    revision: u64,
    listening: bool,
    recording: bool,
    remote_ready: bool,
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
            remote_ready: false,
        }
    }

    fn configure(
        &mut self,
        configs: &FunctionConfigs,
        listening: bool,
        recording: bool,
        remote_ready: bool,
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
        if remote_ready != self.remote_ready {
            append(&mut result, self.router.set_remote_ready(remote_ready));
            self.remote_ready = remote_ready;
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
}

fn append(target: &mut RouteResult, mut next: RouteResult) {
    target.revision = next.revision;
    target.consume |= next.consume;
    target.outputs.append(&mut next.outputs);
}
impl InputHandle {
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
        remote_ready: bool,
        revision: u64,
    ) -> RouteResult {
        self.policy
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .configure(configs, listening, recording, remote_ready, revision)
    }
    pub fn terminate(&self, reason: taprelay_core::input_router::RouterReason) -> RouteResult {
        self.policy
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .terminate(reason)
    }
    pub fn start(dispatch: Sender<RoutedInput>) -> Result<Self, BackendError> {
        let (started, result) = std::sync::mpsc::sync_channel(1);
        let failed = Arc::new(AtomicBool::new(false));
        let failure = failed.clone();
        let reason = Arc::new(Mutex::new(None));
        let worker_reason = reason.clone();
        let policy = Arc::new(Mutex::new(HookPolicy::new()));
        let worker_policy = policy.clone();
        let thread = thread::Builder::new()
            .name("taprelay-input".into())
            .spawn(move || {
                DISPATCH.with(|s| *s.borrow_mut() = Some(dispatch));
                POLICY.with(|s| *s.borrow_mut() = Some(worker_policy));
                LAST_MOUSE_POINT.with(|point| point.set(None));
                PENDING_MOTION.with(|pending| *pending.borrow_mut() = None);
                MOTION_FLUSH_POSTED.with(|posted| posted.set(false));
                STOP_REASON.with(|r| r.set(None));
                let outcome = unsafe {
                    (|| -> windows::core::Result<()> {
                        // Force a thread queue into existence before the handle can post WM_QUIT.
                        let mut message = MSG::default();
                        let _ = PeekMessageW(&mut message, None, 0, 0, PM_NOREMOVE);
                        let module = GetModuleHandleW(None)?;
                        let keyboard = Hook(SetWindowsHookExW(
                            WH_KEYBOARD_LL,
                            Some(keyboard_proc),
                            Some(module.into()),
                            0,
                        )?);
                        let mouse = Hook(SetWindowsHookExW(
                            WH_MOUSE_LL,
                            Some(mouse_proc),
                            Some(module.into()),
                            0,
                        )?);
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
                                    if message.message == FLUSH_MOTION_MESSAGE {
                                        flush_motion();
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
                DISPATCH.with(|s| *s.borrow_mut() = None);
                POLICY.with(|s| *s.borrow_mut() = None);
                LAST_MOUSE_POINT.with(|point| point.set(None));
            })
            .map_err(|e| BackendError::Unavailable(e.to_string()))?;
        match result.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(id)) => Ok(Self {
                id,
                failed,
                reason,
                policy,
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
                        ki: KEYBDINPUT {
                            wVk: VIRTUAL_KEY(key as u16),
                            dwFlags: if down {
                                KEYBD_EVENT_FLAGS(0)
                            } else {
                                KEYEVENTF_KEYUP
                            },
                            dwExtraInfo: REPLAY_TAG,
                            ..Default::default()
                        },
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
    // Motion held in the coalescer must be visible before the next edge. A
    // full queue is a listener failure, not permission to drop a release.
    if !flush_pending_motion() {
        stop(StopReason::Overflow);
        return;
    }
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
}
fn emit_motion(motion: PhysicalEvent, result: RouteResult) {
    let next = RoutedInput::Motion {
        event: motion,
        result,
    };
    let merged = PENDING_MOTION.with(|pending| {
        let mut pending = pending.borrow_mut();
        pending
            .as_mut()
            .is_some_and(|previous| merge_motion(previous, &next))
    });
    if merged {
        schedule_motion_flush();
        return;
    }
    if !flush_pending_motion() {
        stop(StopReason::Overflow);
        return;
    }
    if is_coalescible_motion(&next) {
        PENDING_MOTION.with(|pending| *pending.borrow_mut() = Some(next));
        schedule_motion_flush();
    } else {
        dispatch(next);
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
}
fn flush_pending_motion() -> bool {
    MOTION_FLUSH_POSTED.with(|posted| posted.set(false));
    let pending = PENDING_MOTION.with(|slot| slot.borrow_mut().take());
    let Some(input) = pending else {
        return true;
    };
    let mut sent = true;
    DISPATCH.with(|s| {
        if let Some(tx) = s.borrow().as_ref()
            && let Err(error) = tx.try_send(input.clone())
        {
            sent = false;
            tracing::error!(?error, "Unable to flush coalesced input");
        }
    });
    if !sent {
        PENDING_MOTION.with(|slot| *slot.borrow_mut() = Some(input));
    }
    sent
}
fn flush_motion() {
    if !flush_pending_motion() {
        stop(StopReason::Overflow);
    }
}
fn schedule_motion_flush() {
    let should_post = MOTION_FLUSH_POSTED.with(|posted| {
        if posted.get() {
            false
        } else {
            posted.set(true);
            true
        }
    });
    if should_post {
        let posted = unsafe {
            PostThreadMessageW(
                GetCurrentThreadId(),
                FLUSH_MOTION_MESSAGE,
                WPARAM(0),
                LPARAM(0),
            )
        };
        if let Err(error) = posted {
            MOTION_FLUSH_POSTED.with(|flag| flag.set(false));
            tracing::error!(?error, "Unable to schedule coalesced input");
            stop(StopReason::ReceiverClosed);
        }
    }
}
fn is_coalescible_motion(input: &RoutedInput) -> bool {
    let RoutedInput::Motion { event, result } = input else {
        return false;
    };
    matches!(
        event.input,
        PhysicalInput::Motion { .. } | PhysicalInput::Wheel { .. }
    ) && result.outputs.len() == 1
        && matches!(
            result.outputs[0],
            RoutedOutput::Local(_) | RoutedOutput::Remote(_)
        )
}
fn merge_motion(previous: &mut RoutedInput, next: &RoutedInput) -> bool {
    if !is_coalescible_motion(previous) || !is_coalescible_motion(next) {
        return false;
    }
    let RoutedInput::Motion {
        event: previous_event,
        result: previous_result,
    } = previous
    else {
        return false;
    };
    let RoutedInput::Motion {
        event: next_event,
        result: next_result,
    } = next
    else {
        return false;
    };
    if previous_result.consume != next_result.consume {
        return false;
    }
    let (previous_input, next_input, remote) =
        match (&previous_result.outputs[0], &next_result.outputs[0]) {
            (RoutedOutput::Local(previous), RoutedOutput::Local(next)) => (*previous, *next, false),
            (RoutedOutput::Remote(previous), RoutedOutput::Remote(next)) => {
                (*previous, *next, true)
            }
            _ => return false,
        };
    let Some(input) = add_motion(previous_input, next_input) else {
        return false;
    };
    previous_event.input = input;
    previous_event.captured = previous_event.captured.max(next_event.captured);
    previous_result.outputs[0] = if remote {
        RoutedOutput::Remote(input)
    } else {
        RoutedOutput::Local(input)
    };
    true
}
fn add_motion(left: PhysicalInput, right: PhysicalInput) -> Option<PhysicalInput> {
    match (left, right) {
        (PhysicalInput::Motion { dx, dy }, PhysicalInput::Motion { dx: rx, dy: ry }) => {
            Some(PhysicalInput::Motion {
                dx: dx.checked_add(rx)?,
                dy: dy.checked_add(ry)?,
            })
        }
        (
            PhysicalInput::Wheel {
                vertical,
                horizontal,
            },
            PhysicalInput::Wheel {
                vertical: rv,
                horizontal: rh,
            },
        ) => Some(PhysicalInput::Wheel {
            vertical: vertical.checked_add(rv)?,
            horizontal: horizontal.checked_add(rh)?,
        }),
        _ => None,
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
        let Some(policy) = policy.borrow().as_ref().cloned() else {
            stop(StopReason::PolicyUnavailable);
            return RouteResult::default();
        };
        let Ok(mut policy) = policy.try_lock() else {
            stop(StopReason::PolicyUnavailable);
            return RouteResult::default();
        };
        policy.edge(event)
    })
}
fn policy_local_edge(event: InputEvent) -> RouteResult {
    POLICY.with(|policy| {
        let Some(policy) = policy.borrow().as_ref().cloned() else {
            stop(StopReason::PolicyUnavailable);
            return RouteResult::default();
        };
        let Ok(mut policy) = policy.try_lock() else {
            stop(StopReason::PolicyUnavailable);
            return RouteResult::default();
        };
        policy.local_edge(event)
    })
}
fn policy_motion(motion: PhysicalInput) -> RouteResult {
    POLICY.with(|policy| {
        let Some(policy) = policy.borrow().as_ref().cloned() else {
            stop(StopReason::PolicyUnavailable);
            return RouteResult::default();
        };
        let Ok(mut policy) = policy.try_lock() else {
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
    let outputs = std::mem::take(&mut result.outputs);
    let mut kept = Vec::with_capacity(outputs.len());
    for output in outputs {
        match output {
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

unsafe extern "system" fn keyboard_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let mut consume = false;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
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
                let edge = match wp.0 as u32 {
                    WM_KEYDOWN | WM_SYSKEYDOWN => keyboard_code(event).map(|code| (code, true)),
                    WM_KEYUP | WM_SYSKEYUP => keyboard_code(event).map(|code| (code, false)),
                    _ => None,
                };
                if let Some((code, down)) = edge {
                    let input = InputEvent {
                        code,
                        down,
                        captured: Instant::now(),
                    };
                    let mut result = if taprelay_foreground() {
                        policy_local_edge(input)
                    } else {
                        policy_edge(input)
                    };
                    if prepare_hook_result(&mut result) {
                        consume = result.consume;
                        emit(input, result);
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
unsafe extern "system" fn mouse_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let mut consume = false;
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let event = unsafe { &*(lp.0 as *const MSLLHOOKSTRUCT) };
            if event.flags & LLMHF_INJECTED == 0 {
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
                                emit_motion(
                                    PhysicalEvent {
                                        input: motion,
                                        captured: Instant::now(),
                                    },
                                    result,
                                );
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
                        emit_motion(
                            PhysicalEvent {
                                input: motion,
                                captured: Instant::now(),
                            },
                            result,
                        );
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
                        emit_motion(
                            PhysicalEvent {
                                input: motion,
                                captured: Instant::now(),
                            },
                            result,
                        );
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
