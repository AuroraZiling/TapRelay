//! Active event tap. The callback only consults core policy and bounded queues.
use crate::keys;
use core_foundation::{
    base::CFRelease,
    runloop::{CFRunLoopRunInMode, kCFRunLoopDefaultMode},
};
use objc2_core_graphics::{
    CGEventField, CGEventFlags, CGEventSourceStateID, CGEventTapLocation, CGEventTapOptions,
    CGEventTapPlacement, CGEventType,
};
use std::{
    ffi::c_void,
    ptr,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use taprelay_core::{
    foreground_app::ForegroundAppRules,
    function::{AppCommand, FunctionAction, FunctionConfigs},
    input::{InputCode, InputEvent, MouseButton},
    input_router::{
        InputRouter, PhysicalInput, RouteResult, RoutedInput, RoutedOutput, RouterReason,
    },
    mapping::CustomMapping,
    passthrough::{Event, InputLink, KeyUsage},
    ports::BackendError,
};
use tokio::sync::mpsc::Sender;
mod capture;
mod cursor;
pub(super) mod keyboard;
mod motion;
mod raw;
mod sources;
mod tap;

type NativeEvent = *mut c_void;
static SESSION_ACTIVE: AtomicBool = AtomicBool::new(true);
pub fn set_session_active(active: bool) {
    SESSION_ACTIVE.store(active, Ordering::Release);
}
#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn IsSecureEventInputEnabled() -> bool;
}
const TAG: i64 = 0x54524c59;
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(
        tap: CGEventTapLocation,
        place: CGEventTapPlacement,
        options: CGEventTapOptions,
        mask: u64,
        callback: unsafe extern "C" fn(
            *mut c_void,
            CGEventType,
            NativeEvent,
            *mut c_void,
        ) -> NativeEvent,
        info: *mut c_void,
    ) -> *mut c_void;
    fn CGEventTapEnable(tap: *mut c_void, enable: bool);
    fn CGEventGetIntegerValueField(event: NativeEvent, field: CGEventField) -> i64;
    fn CGEventGetFlags(event: NativeEvent) -> CGEventFlags;
    fn CGEventSourceKeyState(state: CGEventSourceStateID, key: u16) -> bool;
    fn CGEventSourceButtonState(state: CGEventSourceStateID, button: u32) -> bool;
    fn CGEventCreateKeyboardEvent(source: *const c_void, key: u16, down: bool) -> NativeEvent;
    fn CGEventCreateMouseEvent(
        source: *const c_void,
        kind: CGEventType,
        point: Point,
        button: u32,
    ) -> NativeEvent;
    fn CGEventCreate(source: *const c_void) -> NativeEvent;
    fn CGEventGetLocation(event: NativeEvent) -> Point;
    fn CGEventGetTimestamp(event: NativeEvent) -> u64;
    fn CGEventSetIntegerValueField(event: NativeEvent, field: CGEventField, value: i64);
    fn CGEventPost(tap: CGEventTapLocation, event: NativeEvent);
    fn CGPreflightListenEventAccess() -> bool;
    fn CGPreflightPostEventAccess() -> bool;
    fn AXIsProcessTrusted() -> bool;
}
#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFMachPortCreateRunLoopSource(
        allocator: *const c_void,
        port: *mut c_void,
        order: isize,
    ) -> *const c_void;
    fn CFRunLoopGetCurrent() -> *mut c_void;
    fn CFRunLoopAddSource(run_loop: *mut c_void, source: *const c_void, mode: *const c_void);
    fn CFRunLoopRemoveSource(run_loop: *mut c_void, source: *const c_void, mode: *const c_void);
}
#[repr(C)]
#[derive(Clone, Copy)]
struct Point {
    x: f64,
    y: f64,
}
pub use taprelay_core::ports::InputPermissions as Permissions;
pub fn permission_status() -> Permissions {
    unsafe {
        Permissions {
            accessibility: AXIsProcessTrusted(),
            monitoring: CGPreflightListenEventAccess(),
            posting: CGPreflightPostEventAccess(),
        }
    }
}
pub fn permissions() -> bool {
    permission_status().ready()
}
pub fn any_input_held() -> bool {
    unsafe {
        keys::KEYS
            .iter()
            .any(|k| CGEventSourceKeyState(CGEventSourceStateID::CombinedSessionState, k.native.0))
            || (0..5)
                .any(|b| CGEventSourceButtonState(CGEventSourceStateID::CombinedSessionState, b))
    }
}
pub fn key_name(key: u8) -> String {
    keys::name(key)
}
pub fn keyboard_layout() -> usize {
    0
} // names use the explicit physical-key vocabulary

struct Policy {
    router: InputRouter,
    link: Option<InputLink>,
    sender: Sender<RoutedInput>,
    foreground: Option<String>,
    ours: bool,
    available: bool,
    capture: capture::Session<cursor::Capture>,
    replay_pending: std::collections::VecDeque<PhysicalInput>,
    caps_lock: Option<bool>,
    sources: sources::MotionSources,
    motion: motion::Motion,
    error: Option<String>,
    revision: u64,
}
impl Policy {
    fn new(sender: Sender<RoutedInput>) -> Self {
        Self {
            router: InputRouter::new(&taprelay_core::function::default_configs(), 0),
            link: None,
            sender,
            foreground: None,
            ours: false,
            available: false,
            capture: capture::Session::default(),
            replay_pending: Default::default(),
            caps_lock: None,
            sources: sources::MotionSources::default(),
            motion: motion::Motion::default(),
            error: None,
            revision: u64::MAX,
        }
    }

    fn process(&mut self, mut result: RouteResult, captured: Instant) -> RouteResult {
        let mut kept = vec![];
        for output in std::mem::take(&mut result.outputs) {
            match output {
                RoutedOutput::Remote(PhysicalInput::Motion { dx: 0, dy: 0 }) => {}
                RoutedOutput::Remote(input) => {
                    let event = match input {
                        PhysicalInput::Edge {
                            code: InputCode::Key(key),
                            down,
                        } => keys::usage(key).map(|usage| Event::Key {
                            usage: KeyUsage::Keyboard(usage),
                            down,
                        }),
                        PhysicalInput::Edge {
                            code: InputCode::Mouse(button),
                            down,
                        } => Some(Event::Button { button, down }),
                        PhysicalInput::Motion { dx, dy } => Some(Event::Motion { dx, dy }),
                        PhysicalInput::Wheel {
                            vertical,
                            horizontal,
                        } => Some(Event::Wheel {
                            vertical,
                            horizontal,
                        }),
                    };
                    if let Some(link) = &self.link {
                        if let Some(event) = event {
                            link.submit(event, captured);
                        } else {
                            link.fail("Unmapped macOS key; capture stopped");
                        }
                    }
                }
                RoutedOutput::Mapping {
                    id,
                    output,
                    token,
                    down,
                    created,
                } => {
                    if let Some(link) = &self.link {
                        link.submit_mapping(output, token, down, created);
                    }
                    kept.push(RoutedOutput::Mapping {
                        id,
                        output,
                        token,
                        down,
                        created,
                    });
                }
                RoutedOutput::Replay(input) => {
                    if self.replay_pending.len() == 256 {
                        self.error = Some("Local replay queue full; listener stopped".into());
                        self.stop_capture();
                    } else {
                        self.replay_pending.push_back(input);
                    }
                }
                RoutedOutput::EndPassthrough => {
                    self.stop_capture();
                }
                RoutedOutput::Function {
                    action: FunctionAction::App(AppCommand::TogglePassthrough),
                    down: true,
                    ..
                } => {
                    if self.router.passthrough()
                        || self.link.as_ref().is_some_and(InputLink::capture_requested)
                    {
                        self.end_capture();
                    } else if let Some(link) = &self.link {
                        link.request_profile();
                    }
                }
                RoutedOutput::Function {
                    action: FunctionAction::Media(action),
                    down,
                    ..
                } if self.router.passthrough() => {
                    if let Some(link) = &self.link {
                        link.submit(Event::Media { action, down }, captured);
                    }
                }
                output => kept.push(output),
            }
        }
        result.outputs = kept;
        result
    }
    fn dispatch(&mut self, event: Option<InputEvent>, result: RouteResult) {
        if result.outputs.is_empty() {
            return;
        }
        let routed = match event {
            Some(event) => RoutedInput::Edge { event, result },
            None => RoutedInput::Control(result),
        };
        if self.sender.try_send(routed).is_err() {
            self.error = Some("Input queue unavailable; capture stopped".into());
            self.end_capture();
        }
    }
    fn stop_capture(&mut self) {
        if let Some(link) = &self.link {
            link.end();
        }
        self.capture.stop();
        self.sources.clear();
        self.motion = motion::Motion::default();
    }
    fn end_capture(&mut self) {
        let result = self.router.set_passthrough(false);
        let result = self.process(result, Instant::now());
        self.stop_capture();
        self.dispatch(None, result);
    }
    fn drain_effects(&mut self) {
        if self.capture.release() != capture::Transition::Unchanged {
            tracing::info!("Input capture released");
        }
        while let Some(input) = self.replay_pending.pop_front() {
            if let Err(error) = replay(input) {
                self.error = Some(error.to_string());
                self.end_capture();
            }
        }
    }
    fn sync(&mut self, context: capture::Context) {
        if self.router.passthrough() && !self.link.as_ref().is_some_and(|link| link.epoch() != 0) {
            self.end_capture();
        }
        match self
            .capture
            .tick(self.link.as_ref(), context, cursor::Capture::request)
        {
            capture::Transition::Started => {
                self.motion = motion::Motion::default();
                self.sources.clear();
                let result = self.router.set_passthrough(true);
                let result = self.process(result, context.now);
                self.dispatch(None, result);
                tracing::info!("Input capture started");
            }
            capture::Transition::Stopped => {
                tracing::info!("Input capture stopped");
                self.end_capture();
            }
            capture::Transition::Failed(error) => {
                self.error = Some(error.into());
                self.end_capture();
            }
            capture::Transition::Unchanged => {}
        }
        self.drain_effects();
        self.flush_motion(context.now);
    }
    fn flush_motion(&mut self, now: Instant) {
        for sample in self.sources.drain(now) {
            if self.router.passthrough() {
                let result = self.router.route_motion(sample.delta[0], sample.delta[1]);
                let result = self.process(result, sample.captured);
                self.dispatch(None, result);
            }
        }
    }
}
type PolicyCommand = Box<dyn FnOnce(&mut Policy) -> RouteResult + Send>;
struct PolicyRequest {
    command: PolicyCommand,
    reply: std::sync::mpsc::SyncSender<RouteResult>,
    dispatch: bool,
}
struct Shared {
    // Only the event-tap thread accesses policy. Other threads send commands.
    policy: Mutex<Policy>,
    failure: Mutex<Option<String>>,
    stop: AtomicBool,
    callback_failure: AtomicU8,
}
#[repr(u8)]
enum CallbackFailure {
    TapReentered = 1,
    RawReentered,
    TapPanicked,
    RawPanicked,
}
impl CallbackFailure {
    fn message(value: u8) -> Option<&'static str> {
        match value {
            1 => Some("Event tap policy reentered or poisoned"),
            2 => Some("Raw HID policy reentered or poisoned"),
            3 => Some("Event tap callback panicked"),
            4 => Some("Raw HID callback panicked"),
            _ => None,
        }
    }
}
impl Shared {
    fn stop_after_callback(&self, failure: CallbackFailure) {
        // Never lock or format a diagnostic from a reentrant/failed C callback.
        // The event loop records the first cause after leaving the callback.
        let _ = self.callback_failure.compare_exchange(
            0,
            failure as u8,
            Ordering::AcqRel,
            Ordering::Acquire,
        );
        self.stop.store(true, Ordering::Release);
    }
}
pub struct InputHandle {
    shared: Arc<Shared>,
    commands: std::sync::mpsc::SyncSender<PolicyRequest>,
    link: Mutex<Option<InputLink>>,
    configuration: Mutex<Option<(bool, bool, u64, bool)>>,
    transport_ready: AtomicBool,
    thread: Option<JoinHandle<()>>,
}
impl InputHandle {
    pub fn start(sender: Sender<RoutedInput>) -> Result<Self, BackendError> {
        let status = permission_status();
        let Permissions {
            accessibility,
            monitoring,
            posting,
        } = status;
        tracing::info!(
            accessibility,
            monitoring,
            posting,
            "macOS input permission checks"
        );
        if !status.ready() {
            return Err(BackendError::Unavailable(format!(
                "Input permissions: Accessibility={accessibility}, Input Monitoring={monitoring}, Event Posting={posting}. Enable TapRelay in System Settings → Privacy & Security → Accessibility and Input Monitoring, then return to TapRelay."
            )));
        }
        keyboard::initialize()?;
        let shared = Arc::new(Shared {
            policy: Mutex::new(Policy::new(sender)),
            failure: Mutex::new(None),
            stop: AtomicBool::new(false),
            callback_failure: AtomicU8::new(0),
        });
        let (commands, requests) = std::sync::mpsc::sync_channel::<PolicyRequest>(16);
        let state = shared.clone();
        let (ready, result) = std::sync::mpsc::sync_channel(1);
        let thread = thread::Builder::new()
            .name("taprelay-event-tap".into())
            .spawn(move || {
                let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    objc2::rc::autoreleasepool(|_| unsafe {
                        let raw = match raw::Capture::new(Arc::as_ptr(&state) as *mut c_void) {
                            Ok(raw) => raw,
                            Err(error) => {
                                let _ = ready.send(Err(error));
                                return;
                            }
                        };
                        let tap = match tap::EventTap::install(Arc::as_ptr(&state) as *mut c_void) {
                            Ok(tap) => tap,
                            Err(error) => {
                                let _ = ready.send(Err(error.into()));
                                return;
                            }
                        };
                        let _ = ready.send(Ok(()));
                        let mut foreground = crate::foreground_apps::Foreground::default();
                        let mut refreshed = Instant::now() - Duration::from_millis(50);
                        while !state.stop.load(Ordering::Acquire) {
                            let running = objc2::rc::autoreleasepool(|_| {
                                for request in requests.try_iter().take(16) {
                                    let mut policy =
                                        state.policy.lock().unwrap_or_else(|e| e.into_inner());
                                    let result = (request.command)(&mut policy);
                                    if request.dispatch {
                                        policy.dispatch(None, result.clone());
                                    }
                                    let _ = request.reply.try_send(result);
                                }
                                CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.005, 1);
                                let mut p = state.policy.lock().unwrap_or_else(|e| e.into_inner());
                                if state.stop.load(Ordering::Acquire) || p.error.is_some() {
                                    return false;
                                }
                                let mut result = RouteResult::default();
                                if refreshed.elapsed() >= Duration::from_millis(50) {
                                    refreshed = Instant::now();
                                    let current = foreground.path();
                                    // Cursor ownership must not change the app scope that
                                    // selected this session. Copy identity only on change.
                                    if !p.capture.owns_cursor()
                                        && p.foreground.as_deref() != current
                                    {
                                        p.foreground = current.map(str::to_owned);
                                        result = p.router.set_foreground(current);
                                    }
                                    p.ours = foreground.pid() == Some(std::process::id() as i32);
                                }
                                let session_active = SESSION_ACTIVE.load(Ordering::Acquire);
                                let secure_input = IsSecureEventInputEnabled();
                                let permitted = permissions();
                                if !session_active || (p.router.passthrough() && secure_input) {
                                    p.error = Some(
                                        "Capture revoked by session change, sleep, or Secure Input"
                                            .into(),
                                    );
                                    p.end_capture();
                                    return false;
                                }
                                if !permitted {
                                    p.error = Some("Input permission revoked".into());
                                    p.end_capture();
                                    return false;
                                }
                                let ours = p.ours;
                                p.sync(capture::Context {
                                    now: Instant::now(),
                                    ours,
                                    permitted,
                                    session_active,
                                    secure_input,
                                });
                                append(&mut result, p.router.tick(Instant::now()));
                                let result = p.process(result, Instant::now());
                                p.dispatch(None, result);
                                p.error.is_none()
                            });
                            if !running {
                                break;
                            }
                        }
                        drop(tap);
                        drop(raw);
                    });
                }));
                // Both normal exit and a Rust panic release cursor/input ownership
                // on the same thread, after callbacks have been unscheduled.
                objc2::rc::autoreleasepool(|_| {
                    let mut p = state.policy.lock().unwrap_or_else(|e| e.into_inner());
                    if outcome.is_err() {
                        p.error = Some("macOS input thread panicked".into());
                    } else if p.error.is_none() {
                        p.error = CallbackFailure::message(
                            state.callback_failure.load(Ordering::Acquire),
                        )
                        .map(str::to_owned);
                    }
                    p.end_capture();
                    p.drain_effects();
                    if let Some(error) = &p.error {
                        tracing::error!(%error, "macOS input listener stopped");
                        let mut failure = state.failure.lock().unwrap_or_else(|e| e.into_inner());
                        failure.get_or_insert_with(|| error.clone());
                    }
                });
            })
            .map_err(|e| BackendError::Unavailable(e.to_string()))?;
        if let Err(error) = result
            .recv_timeout(Duration::from_secs(3))
            .unwrap_or_else(|_| Err("Input event thread did not start".into()))
        {
            shared.stop.store(true, Ordering::Release);
            return Err(BackendError::Unavailable(error));
        }
        Ok(Self {
            commands,
            link: Mutex::new(None),
            configuration: Mutex::new(None),
            transport_ready: AtomicBool::new(false),
            shared,
            thread: Some(thread),
        })
    }
    pub fn is_finished(&self) -> bool {
        self.thread.as_ref().is_none_or(|t| t.is_finished())
    }
    fn request(&self, command: PolicyCommand, dispatch: bool) -> RouteResult {
        let (reply, result) = std::sync::mpsc::sync_channel(1);
        if self
            .commands
            .try_send(PolicyRequest {
                command,
                reply,
                dispatch,
            })
            .is_ok()
            && let Ok(result) = result.recv_timeout(Duration::from_millis(500))
        {
            return result;
        }
        self.shared.stop.store(true, Ordering::Release);
        *self
            .shared
            .failure
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some("Input policy thread unavailable".into());
        if let Some(link) = self.link.lock().unwrap_or_else(|e| e.into_inner()).as_ref() {
            link.disconnect();
        }
        RouteResult::default()
    }
    pub fn failure(&self) -> Option<String> {
        self.shared
            .failure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    pub fn passthrough_error(&self) -> Option<String> {
        self.link
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(InputLink::take_failure)
    }
    pub fn attach_passthrough(&self, link: Option<InputLink>) {
        let mut previous = self.link.lock().unwrap_or_else(|e| e.into_inner());
        if matches!((&*previous, &link), (None, None))
            || previous
                .as_ref()
                .zip(link.as_ref())
                .is_some_and(|(a, b)| a.same(b))
        {
            return;
        }
        *previous = link.clone();
        drop(previous);
        self.request(
            Box::new(move |p| {
                p.end_capture();
                p.link = link;
                RouteResult::default()
            }),
            true,
        );
    }
    pub fn set_transport_ready(&self, ready: bool) {
        self.transport_ready.store(ready, Ordering::Release);
    }
    pub fn configure(
        &self,
        configs: &FunctionConfigs,
        mappings: &[CustomMapping],
        rules: &ForegroundAppRules,
        listening: bool,
        recording: bool,
        revision: u64,
    ) -> RouteResult {
        let mut previous = self.configuration.lock().unwrap_or_else(|e| e.into_inner());
        let ready = self.transport_ready.load(Ordering::Acquire);
        if *previous == Some((listening, recording, revision, ready)) {
            return RouteResult::default();
        }
        *previous = Some((listening, recording, revision, ready));
        drop(previous);
        let mut configs = configs.clone();
        if !ready {
            for (id, config) in &mut configs {
                if taprelay_core::function::function_definition(*id).category
                    == taprelay_core::function::CategoryId::Media
                {
                    config.enabled = false;
                }
            }
        }
        let mappings = mappings.to_vec();
        let rules = rules.clone();
        self.request(
            Box::new(move |p| {
                let mut result = RouteResult::default();
                if p.revision != revision || p.available != ready {
                    append(&mut result, p.router.update_config(&configs, revision));
                    append(
                        &mut result,
                        p.router
                            .update_mappings(if ready { &mappings } else { &[] }),
                    );
                    append(&mut result, p.router.set_foreground_app_rules(&rules));
                    p.revision = revision;
                    p.available = ready;
                }
                append(&mut result, p.router.set_listening(listening));
                append(&mut result, p.router.set_recording(recording));
                p.process(result, Instant::now())
            }),
            false,
        )
    }
    pub fn terminate(&self, reason: RouterReason) -> RouteResult {
        self.request(
            Box::new(move |p| {
                let result = p.router.terminate(reason);
                let result = p.process(result, Instant::now());
                p.stop_capture();
                result
            }),
            false,
        )
    }
    pub fn replay(&self, input: PhysicalInput) -> Result<(), BackendError> {
        replay(input)
    }
}
impl Drop for InputHandle {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take()
            && thread.join().is_err()
        {
            tracing::error!("macOS input thread panicked during shutdown");
        }
    }
}
fn append(target: &mut RouteResult, mut next: RouteResult) {
    target.revision = next.revision;
    target.consume |= next.consume;
    target.outputs.append(&mut next.outputs);
}
unsafe extern "C" fn callback(
    _proxy: *mut c_void,
    kind: CGEventType,
    event: NativeEvent,
    info: *mut c_void,
) -> NativeEvent {
    let shared = unsafe { &*(info as *const Shared) };
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| unsafe {
        let disabled = matches!(
            kind,
            CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
        );
        if shared.stop.load(Ordering::Acquire) {
            return event;
        }
        if !disabled && CGEventGetIntegerValueField(event, CGEventField::EventSourceUserData) == TAG
        {
            return event;
        }
        let Ok(mut p) = shared.policy.try_lock() else {
            shared.stop_after_callback(CallbackFailure::TapReentered);
            return event;
        };
        if disabled {
            p.error = Some("Event tap disabled by macOS; listener stopped".into());
            p.end_capture();
            return event;
        }
        // Keyboard, buttons and wheel always retain their event-tap path.
        // Raw relative motion is paired per event.
        let captured = Instant::now();
        let mut caps_pulse = false;
        let edge = match kind {
            CGEventType::KeyDown | CGEventType::KeyUp | CGEventType::FlagsChanged => {
                let native = keyboard::normalize(
                    CGEventGetIntegerValueField(event, CGEventField::KeyboardEventKeycode) as u16,
                    event,
                );
                let Some(key) = keys::internal(native) else {
                    if native == 63 && p.router.passthrough() {
                        return ptr::null_mut(); // Fn has no keyboard HID usage.
                    }
                    if p.router.passthrough() {
                        // Report the unsupported native key once, then fail open locally.
                        p.error = Some(format!("Unmapped macOS key {native}; capture stopped"));
                        p.end_capture();
                    }
                    return event;
                };
                let flags = CGEventGetFlags(event);
                let previous_caps = p
                    .caps_lock
                    .replace(flags.contains(CGEventFlags::MaskAlphaShift));
                let down = if kind == CGEventType::FlagsChanged {
                    if native == 57 {
                        // Caps Lock carries a toggle state, not physical down/up.
                        // Send a complete tap for either transition; ignore repeats.
                        if previous_caps == p.caps_lock {
                            return if p.router.passthrough() {
                                ptr::null_mut()
                            } else {
                                event
                            };
                        }
                        caps_pulse = true;
                        true
                    } else if let Some(down) = keyboard::modifier_down(native, flags) {
                        down
                    } else {
                        return event;
                    }
                } else {
                    kind == CGEventType::KeyDown
                };
                Some(InputEvent {
                    code: InputCode::Key(key),
                    down,
                    captured,
                })
            }
            CGEventType::LeftMouseDown
            | CGEventType::LeftMouseUp
            | CGEventType::RightMouseDown
            | CGEventType::RightMouseUp
            | CGEventType::OtherMouseDown
            | CGEventType::OtherMouseUp => {
                let button = match CGEventGetIntegerValueField(
                    event,
                    CGEventField::MouseEventButtonNumber,
                ) {
                    0 => MouseButton::Left,
                    1 => MouseButton::Right,
                    2 => MouseButton::Middle,
                    3 => MouseButton::Side1,
                    4 => MouseButton::Side2,
                    _ => return event,
                };
                Some(InputEvent {
                    code: InputCode::Mouse(button),
                    down: matches!(
                        kind,
                        CGEventType::LeftMouseDown
                            | CGEventType::RightMouseDown
                            | CGEventType::OtherMouseDown
                    ),
                    captured,
                })
            }
            _ => None,
        };
        if edge.is_some() || kind == CGEventType::ScrollWheel {
            // Preserve movement-before-click/key ordering, even when a raw
            // counterpart has not arrived. A later raw sample is retired.
            p.flush_motion(captured + Duration::from_millis(5));
        }
        let result = if let Some(edge) = edge {
            if p.ours && !p.capture.owns_cursor() {
                p.router.route_local_event(edge)
            } else {
                p.router.route_event(edge)
            }
        } else if kind == CGEventType::ScrollWheel {
            p.router.route_wheel(
                (CGEventGetIntegerValueField(event, CGEventField::ScrollWheelEventDeltaAxis1)
                    .saturating_mul(120))
                .clamp(i32::MIN as i64, i32::MAX as i64) as i32,
                (CGEventGetIntegerValueField(event, CGEventField::ScrollWheelEventDeltaAxis2)
                    .saturating_mul(120))
                .clamp(i32::MIN as i64, i32::MAX as i64) as i32,
            )
        } else {
            let dx = CGEventGetIntegerValueField(event, CGEventField::MouseEventDeltaX) as i32;
            let dy = CGEventGetIntegerValueField(event, CGEventField::MouseEventDeltaY) as i32;
            let [dx, dy] = p.motion.relative(
                CGEventGetIntegerValueField(event, CGEventField::EventSourceUnixProcessID),
                CGEventGetLocation(event),
                [dx, dy],
            );
            if p.router.passthrough() {
                let source =
                    CGEventGetIntegerValueField(event, CGEventField::EventSourceUnixProcessID);
                if source == 0 {
                    if let Err(error) =
                        p.sources
                            .tap(CGEventGetTimestamp(event), captured, [dx, dy])
                    {
                        p.error = Some(error.into());
                        p.end_capture();
                    }
                    RouteResult {
                        consume: true,
                        ..Default::default()
                    }
                } else {
                    p.flush_motion(captured + Duration::from_millis(5));
                    p.router.route_motion(dx, dy)
                }
            } else {
                p.router.route_motion(dx, dy)
            }
        };
        let result = p.process(result, captured);
        let mut consume = result.consume;
        p.dispatch(edge, result);
        if caps_pulse && let Some(edge) = edge {
            let release = InputEvent {
                down: false,
                ..edge
            };
            let result = if p.ours && !p.capture.owns_cursor() {
                p.router.route_local_event(release)
            } else {
                p.router.route_event(release)
            };
            let result = p.process(result, captured);
            consume |= result.consume;
            p.dispatch(Some(release), result);
        }
        if consume { ptr::null_mut() } else { event }
    })) {
        Ok(result) => result,
        Err(_) => {
            shared.stop_after_callback(CallbackFailure::TapPanicked);
            event
        }
    }
}
fn replay(input: PhysicalInput) -> Result<(), BackendError> {
    unsafe {
        let event = match input {
            PhysicalInput::Edge {
                code: InputCode::Key(key),
                down,
            } => {
                let native = keys::native(key)
                    .ok_or_else(|| BackendError::Unavailable("Unknown replay key".into()))?;
                let event = CGEventCreateKeyboardEvent(ptr::null(), native, down);
                if !event.is_null() {
                    CGEventSetIntegerValueField(
                        event,
                        CGEventField::KeyboardEventKeycode,
                        i64::from(keyboard::normalize(native, event)),
                    );
                }
                event
            }
            PhysicalInput::Edge {
                code: InputCode::Mouse(button),
                down,
            } => {
                let location = CGEventCreate(ptr::null());
                if location.is_null() {
                    return Err(BackendError::Unavailable("Cannot read cursor".into()));
                }
                let point = CGEventGetLocation(location);
                CFRelease(location);
                let button = match button {
                    MouseButton::Left => 0,
                    MouseButton::Right => 1,
                    MouseButton::Middle => 2,
                    MouseButton::Side1 => 3,
                    MouseButton::Side2 => 4,
                };
                let kind = match (button, down) {
                    (0, true) => CGEventType::LeftMouseDown,
                    (0, false) => CGEventType::LeftMouseUp,
                    (1, true) => CGEventType::RightMouseDown,
                    (1, false) => CGEventType::RightMouseUp,
                    (_, true) => CGEventType::OtherMouseDown,
                    (_, false) => CGEventType::OtherMouseUp,
                };
                CGEventCreateMouseEvent(ptr::null(), kind, point, button)
            }
            _ => {
                return Err(BackendError::Unavailable(
                    "Unsupported local replay event".into(),
                ));
            }
        };
        if event.is_null() {
            return Err(BackendError::Unavailable(
                "CGEvent allocation failed".into(),
            ));
        }
        CGEventSetIntegerValueField(event, CGEventField::EventSourceUserData, TAG);
        CGEventPost(CGEventTapLocation::SessionEventTap, event);
        CFRelease(event);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;
    use taprelay_core::function::{FunctionId, ModifierSet, Shortcut};

    unsafe extern "C" {
        fn CGEventSetFlags(event: NativeEvent, flags: CGEventFlags);
    }
    // These events are owned test data: never post them or install an OS tap.
    unsafe fn key_callback(
        shared: &Shared,
        native: u16,
        kind: CGEventType,
        flags: CGEventFlags,
    ) -> bool {
        unsafe {
            let event = CGEventCreateKeyboardEvent(ptr::null(), native, kind != CGEventType::KeyUp);
            assert!(!event.is_null());
            CGEventSetFlags(event, flags);
            let consumed = callback(
                ptr::null_mut(),
                kind,
                event,
                shared as *const Shared as *mut c_void,
            )
            .is_null();
            CFRelease(event);
            consumed
        }
    }

    #[test]
    fn quartz_alone_delivers_modifiers_extended_keys_and_complete_caps_lock_taps() {
        let (sender, _events) = tokio::sync::mpsc::channel(16);
        let (link, reports) = InputLink::channel(Arc::new(AtomicU64::new(1)));
        link.set_ready(1);
        assert!(link.begin());
        let mut policy = Policy::new(sender);
        policy.router.set_passthrough(true);
        policy.link = Some(link);
        let shared = Shared {
            policy: Mutex::new(policy),
            failure: Mutex::new(None),
            stop: AtomicBool::new(false),
            callback_failure: AtomicU8::new(0),
        };
        // Public-only flags and both-side chords must each produce exactly one edge.
        for (native, flags, usage, down) in [
            (56, CGEventFlags::MaskShift, 0xe1, true),
            (56, CGEventFlags::empty(), 0xe1, false),
            (56, CGEventFlags::MaskShift | CGEventFlags(0x02), 0xe1, true),
            (60, CGEventFlags::MaskShift | CGEventFlags(0x06), 0xe5, true),
            (
                56,
                CGEventFlags::MaskShift | CGEventFlags(0x04),
                0xe1,
                false,
            ),
            (60, CGEventFlags::empty(), 0xe5, false),
        ] {
            assert!(unsafe { key_callback(&shared, native, CGEventType::FlagsChanged, flags) });
            assert_eq!(
                reports.try_recv().unwrap().event,
                Event::Key {
                    usage: KeyUsage::Keyboard(usage),
                    down
                }
            );
            assert!(reports.try_recv().is_err());
        }
        for (native, usage) in [
            (0, 4),
            (64, 0x6c),
            (79, 0x6d),
            (80, 0x6e),
            (90, 0x6f),
            (93, 0x89),
            (94, 0x87),
            (95, 0x85),
            (102, 0x91),
            (104, 0x90),
            (110, 0x65),
        ] {
            for down in [true, false] {
                let kind = if down {
                    CGEventType::KeyDown
                } else {
                    CGEventType::KeyUp
                };
                assert!(unsafe { key_callback(&shared, native, kind, CGEventFlags::empty()) });
                assert_eq!(
                    reports.try_recv().unwrap().event,
                    Event::Key {
                        usage: KeyUsage::Keyboard(usage),
                        down
                    }
                );
            }
            assert!(reports.try_recv().is_err());
        }
        for flags in [
            CGEventFlags::MaskAlphaShift,
            CGEventFlags::empty(),
            CGEventFlags::MaskAlphaShift,
        ] {
            assert!(unsafe { key_callback(&shared, 57, CGEventType::FlagsChanged, flags) });
            for down in [true, false] {
                assert_eq!(
                    reports.try_recv().unwrap().event,
                    Event::Key {
                        usage: KeyUsage::Keyboard(0x39),
                        down
                    }
                );
            }
            assert!(unsafe { key_callback(&shared, 57, CGEventType::FlagsChanged, flags) });
            assert!(
                reports.try_recv().is_err(),
                "repeated lock-state notifications are not new presses"
            );
        }
    }

    #[test]
    fn reentrant_callback_fails_open_and_retains_the_first_diagnostic() {
        let (sender, _events) = tokio::sync::mpsc::channel(16);
        let shared = Shared {
            policy: Mutex::new(Policy::new(sender)),
            failure: Mutex::new(None),
            stop: AtomicBool::new(false),
            callback_failure: AtomicU8::new(0),
        };
        let _guard = shared.policy.lock().unwrap();
        assert!(!unsafe { key_callback(&shared, 0, CGEventType::KeyDown, CGEventFlags::empty()) });
        shared.stop_after_callback(CallbackFailure::RawPanicked);
        assert!(shared.stop.load(Ordering::Acquire));
        assert_eq!(
            CallbackFailure::message(shared.callback_failure.load(Ordering::Acquire)),
            Some("Event tap policy reentered or poisoned")
        );
    }

    #[test]
    fn event_tap_keeps_exit_and_pointer_routing_without_any_raw_hid_callback() {
        {
            let (sender, _events) = tokio::sync::mpsc::channel(16);
            let (link, reports) = InputLink::channel(Arc::new(AtomicU64::new(1)));
            link.set_ready(1);
            assert!(link.begin());
            let mut configs = taprelay_core::function::default_configs();
            let exit = configs.get_mut(&FunctionId::AppTogglePassthrough).unwrap();
            exit.enabled = true;
            exit.shortcuts = vec![Shortcut::keyboard(ModifierSet::empty(), 0x77)];
            let mut policy = Policy::new(sender);
            policy.router = InputRouter::new(&configs, 1);
            policy.router.set_passthrough(true);
            policy.link = Some(link.clone());
            // No cursor is captured and no event tap is installed in this test.
            // Drive the real callback with owned events, without posting them.
            let shared = Arc::new(Shared {
                policy: Mutex::new(policy),
                failure: Mutex::new(None),
                stop: AtomicBool::new(false),
                callback_failure: AtomicU8::new(0),
            });
            unsafe {
                let event = CGEventCreateMouseEvent(
                    ptr::null(),
                    CGEventType::MouseMoved,
                    Point { x: 0., y: 0. },
                    0,
                );
                assert!(!event.is_null());
                CGEventSetIntegerValueField(event, CGEventField::MouseEventDeltaX, 9);
                CGEventSetIntegerValueField(event, CGEventField::EventSourceUnixProcessID, 99);
                assert!(
                    callback(
                        ptr::null_mut(),
                        CGEventType::MouseMoved,
                        event,
                        Arc::as_ptr(&shared) as *mut c_void
                    )
                    .is_null()
                );
                CFRelease(event);
                assert_eq!(
                    reports.try_recv().unwrap().event,
                    Event::Motion { dx: 9, dy: 0 }
                );
                for down in [true, false] {
                    let event =
                        CGEventCreateKeyboardEvent(ptr::null(), keys::native(0x77).unwrap(), down);
                    assert!(!event.is_null());
                    callback(
                        ptr::null_mut(),
                        if down {
                            CGEventType::KeyDown
                        } else {
                            CGEventType::KeyUp
                        },
                        event,
                        Arc::as_ptr(&shared) as *mut c_void,
                    );
                    CFRelease(event);
                }
            }
            assert_eq!(link.epoch(), 0, "F8 must exit even with zero raw events");
            assert!(!shared.policy.lock().unwrap().router.passthrough());
        }
    }

    #[test]
    fn coordinate_only_background_mouse_events_reach_the_shared_input_link() {
        let (sender, _events) = tokio::sync::mpsc::channel(16);
        let (link, reports) = InputLink::channel(Arc::new(AtomicU64::new(1)));
        link.set_ready(1);
        assert!(link.begin());
        let mut policy = Policy::new(sender);
        policy.link = Some(link.clone());
        policy.router.set_passthrough(true);
        // The callback runs directly on owned events. It never installs a tap,
        // captures the real cursor, or posts system input during this test.
        let shared = Arc::new(Shared {
            policy: Mutex::new(policy),
            failure: Mutex::new(None),
            stop: AtomicBool::new(false),
            callback_failure: AtomicU8::new(0),
        });
        let info = Arc::as_ptr(&shared) as *mut c_void;
        unsafe {
            for (point, native_delta, expected) in [
                ([500., 400.], [0, 0], None),
                ([504., 398.], [0, 0], Some((4, -2))),
                ([504., 398.], [0, 0], None),
                ([505.25, 398.5], [0, 0], Some((1, 0))),
                ([506., 399.], [0, 0], Some((1, 1))),
                ([506., 399.], [7, -3], Some((7, -3))),
                // Ignore recentering after a native relative event.
                ([100., 100.], [0, 0], None),
                ([102., 103.], [0, 0], Some((2, 3))),
            ] {
                let event = CGEventCreateMouseEvent(
                    ptr::null(),
                    CGEventType::MouseMoved,
                    Point {
                        x: point[0],
                        y: point[1],
                    },
                    0,
                );
                CGEventSetIntegerValueField(event, CGEventField::EventSourceUnixProcessID, 99);
                CGEventSetIntegerValueField(event, CGEventField::MouseEventDeltaX, native_delta[0]);
                CGEventSetIntegerValueField(event, CGEventField::MouseEventDeltaY, native_delta[1]);
                assert!(callback(ptr::null_mut(), CGEventType::MouseMoved, event, info).is_null());
                CFRelease(event);
                if let Some((dx, dy)) = expected {
                    assert_eq!(reports.try_recv().unwrap().event, Event::Motion { dx, dy });
                } else {
                    assert!(
                        reports.try_recv().is_err(),
                        "zero displacement must not fill the HID queue"
                    );
                }
            }
            for down in [true, false] {
                let event = CGEventCreateMouseEvent(
                    ptr::null(),
                    if down {
                        CGEventType::LeftMouseDown
                    } else {
                        CGEventType::LeftMouseUp
                    },
                    Point { x: 102., y: 103. },
                    0,
                );
                assert!(
                    callback(
                        ptr::null_mut(),
                        if down {
                            CGEventType::LeftMouseDown
                        } else {
                            CGEventType::LeftMouseUp
                        },
                        event,
                        info
                    )
                    .is_null()
                );
                assert_eq!(
                    reports.try_recv().unwrap().event,
                    Event::Button {
                        button: MouseButton::Left,
                        down
                    }
                );
                CFRelease(event);
            }
            let replay = CGEventCreateKeyboardEvent(ptr::null(), 18, true);
            CGEventSetIntegerValueField(replay, CGEventField::EventSourceUserData, TAG);
            assert_eq!(
                callback(ptr::null_mut(), CGEventType::KeyDown, replay, info),
                replay
            );
            assert!(reports.try_recv().is_err());
            CFRelease(replay);
        }
        assert_eq!(link.epoch(), 2);
    }

    #[test]
    fn exit_cancels_pending_capture_before_cursor_ownership_is_ready() {
        let (sender, _events) = tokio::sync::mpsc::channel(16);
        let (link, reports) = InputLink::channel(Arc::new(AtomicU64::new(1)));
        link.set_ready(1);
        link.set_profile_available(true);
        assert!(link.request_profile());
        let mut configs = taprelay_core::function::default_configs();
        let exit = configs.get_mut(&FunctionId::AppTogglePassthrough).unwrap();
        exit.enabled = true;
        exit.shortcuts = vec![Shortcut::keyboard(ModifierSet::empty(), 0x77)];
        let mut policy = Policy::new(sender);
        policy.router = InputRouter::new(&configs, 1);
        policy.link = Some(link.clone());
        let shared = Shared {
            policy: Mutex::new(policy),
            failure: Mutex::new(None),
            stop: AtomicBool::new(false),
            callback_failure: AtomicU8::new(0),
        };
        unsafe {
            let info = &shared as *const Shared as *mut c_void;
            // Ordinary input must remain local during activation, not queue
            // up for transmission once the cursor becomes available.
            let event = CGEventCreateKeyboardEvent(ptr::null(), 18, true);
            assert_eq!(
                callback(ptr::null_mut(), CGEventType::KeyDown, event, info),
                event
            );
            CFRelease(event);
            assert!(reports.try_recv().is_err());
            let event = CGEventCreateKeyboardEvent(ptr::null(), keys::native(0x77).unwrap(), true);
            assert!(callback(ptr::null_mut(), CGEventType::KeyDown, event, info).is_null());
            CFRelease(event);
        }
        assert!(!link.capture_requested());
        assert!(!link.profile_requested());
        assert!(!shared.policy.lock().unwrap().router.passthrough());
    }
}
