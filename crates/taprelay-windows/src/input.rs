//! Dedicated message-pump thread for WH_KEYBOARD_LL / WH_MOUSE_LL.
//! Callbacks only copy physical edges into a bounded queue and always pass input through.
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
    input::{InputCode, InputEvent, MouseButton},
    ports::BackendError,
};
use tokio::sync::mpsc::Sender;
use windows::Win32::{
    Foundation::*,
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::WindowsAndMessaging::*,
};

thread_local! { static EVENTS: RefCell<Option<Sender<InputEvent>>> = const { RefCell::new(None) }; }
#[derive(Clone, Copy)]
enum StopReason {
    Overflow,
    ReceiverClosed,
    CallbackPanic,
}
impl StopReason {
    fn message(self) -> &'static str {
        match self {
            Self::Overflow => "Input queue overflow; listener stopped to avoid a lost release",
            Self::ReceiverClosed => "Input consumer closed; listener stopped",
            Self::CallbackPanic => "Input callback panicked; listener stopped (see panic log)",
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
pub struct InputHandle {
    id: u32,
    failed: Arc<AtomicBool>,
    reason: Arc<Mutex<Option<String>>>,
    thread: Option<JoinHandle<()>>,
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
    pub fn start(events: Sender<InputEvent>) -> Result<Self, BackendError> {
        let (started, result) = std::sync::mpsc::sync_channel(1);
        let failed = Arc::new(AtomicBool::new(false));
        let failure = failed.clone();
        let reason = Arc::new(Mutex::new(None));
        let worker_reason = reason.clone();
        let thread = thread::Builder::new()
            .name("taprelay-input".into())
            .spawn(move || {
                EVENTS.with(|s| *s.borrow_mut() = Some(events));
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
                EVENTS.with(|s| *s.borrow_mut() = None);
            })
            .map_err(|e| BackendError::Unavailable(e.to_string()))?;
        match result.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(id)) => Ok(Self {
                id,
                failed,
                reason,
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(e) => Err(BackendError::Unavailable(format!("Input startup: {e}"))),
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
fn emit(code: InputCode, down: bool) {
    EVENTS.with(|s| {
        if let Some(tx) = s.borrow().as_ref()
            && let Err(error) = tx.try_send(InputEvent {
                code,
                down,
                captured: Instant::now(),
            })
        {
            // A lost release makes pressed-state unreliable. Fail closed instead of replaying.
            stop(match error {
                tokio::sync::mpsc::error::TrySendError::Full(_) => StopReason::Overflow,
                tokio::sync::mpsc::error::TrySendError::Closed(_) => StopReason::ReceiverClosed,
            });
        }
    });
}
unsafe extern "system" fn keyboard_proc(code: i32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 {
        let outcome = std::panic::catch_unwind(|| {
            // Windows owns this structure for the duration of the hook callback.
            let event = unsafe { &*(lp.0 as *const KBDLLHOOKSTRUCT) };
            #[cfg(test)]
            if event.dwExtraInfo == PROBE_TAG {
                KEYBOARD_PROBE.fetch_or(
                    if wp.0 as u32 == WM_KEYDOWN { 1 } else { 2 },
                    Ordering::SeqCst,
                );
            }
            if !event.flags.contains(LLKHF_INJECTED) && event.vkCode <= 254 {
                match wp.0 as u32 {
                    WM_KEYDOWN | WM_SYSKEYDOWN => emit(InputCode::Key(event.vkCode as u8), true),
                    WM_KEYUP | WM_SYSKEYUP => emit(InputCode::Key(event.vkCode as u8), false),
                    _ => {}
                }
            }
        });
        if outcome.is_err() {
            stop(StopReason::CallbackPanic);
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
        let outcome = std::panic::catch_unwind(|| {
            let event = unsafe { &*(lp.0 as *const MSLLHOOKSTRUCT) };
            if event.flags & LLMHF_INJECTED == 0
                && let Some((b, down)) = mouse_edge(wp.0 as u32, event.mouseData)
            {
                emit(InputCode::Mouse(b), down);
            }
        });
        if outcome.is_err() {
            stop(StopReason::CallbackPanic);
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
            assert_ne!(
                event.code,
                InputCode::Key(0x87),
                "Injected F24 must not reach the recorder"
            );
        }
    }
    #[test]
    #[ignore = "requires pressing and releasing physical F8 on an interactive desktop"]
    fn physical_keyboard_f8_reaches_recorder() {
        use taprelay_core::input::{InputState, Recorder, Trigger};
        let (tx, mut rx) = tokio::sync::mpsc::channel(256);
        let _handle = InputHandle::start(tx).unwrap();
        let mut state = InputState::default();
        let mut recorder = Recorder::default();
        let until = Instant::now() + Duration::from_secs(30);
        eprintln!("Ready: press and release physical F8 within 30 seconds.");
        while Instant::now() < until {
            while let Ok(event) = rx.try_recv() {
                if event.code != InputCode::Key(0x77) {
                    continue;
                }
                eprintln!("F8 down={}", event.down);
                state.update(event);
                if let Some(trigger) = recorder.observe(&state, event) {
                    assert_eq!(trigger, Trigger::Keyboard { keys: vec![0x77] });
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
