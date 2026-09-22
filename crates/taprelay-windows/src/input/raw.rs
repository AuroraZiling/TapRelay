use std::{
    cell::{Cell, RefCell},
    mem::size_of,
    time::{Duration, Instant},
};
use taprelay_core::{input::MouseButton, input_router::PhysicalInput};
use windows::{
    Win32::{
        Foundation::*,
        System::{
            LibraryLoader::GetModuleHandleW, RemoteDesktop::*, StationsAndDesktops::*,
            SystemInformation::GetTickCount,
        },
        UI::{Input::*, WindowsAndMessaging::*},
    },
    core::w,
};

thread_local! {
    static WINDOW: Cell<HWND> = const { Cell::new(HWND(std::ptr::null_mut())) };
    static REGISTERED: Cell<bool> = const { Cell::new(false) };
    static PREVIOUS: RefCell<[Option<RAWINPUTDEVICE>; 2]> = const { RefCell::new([None; 2]) };
    static STARTED: Cell<u32> = const { Cell::new(0) };
}
const HEALTH_TIMER: usize = 2;

pub(super) fn captured_at(message_tick: u32) -> Instant {
    message_time(Instant::now(), unsafe { GetTickCount() }, message_tick)
}

fn message_time(now: Instant, current_tick: u32, message_tick: u32) -> Instant {
    let age = (current_tick.wrapping_sub(message_tick) as i32).max(0) as u64;
    now.checked_sub(Duration::from_millis(age)).unwrap_or(now)
}

pub(super) struct Capture(HWND);

impl Capture {
    pub fn new() -> windows::core::Result<Self> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: instance.into(),
                lpszClassName: w!("TapRelay.RawInput"),
                ..Default::default()
            };
            RegisterClassW(&class);
            let window = CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class.lpszClassName,
                w!(""),
                WINDOW_STYLE::default(),
                0,
                0,
                0,
                0,
                Some(HWND_MESSAGE),
                None,
                Some(instance.into()),
                None,
            )?;
            WINDOW.with(|current| current.set(window));
            let capture = Self(window);
            WTSRegisterSessionNotification(window, NOTIFY_FOR_THIS_SESSION)?;
            if SetTimer(Some(window), HEALTH_TIMER, 20, None) == 0 {
                return Err(windows::core::Error::from_thread());
            }
            Ok(capture)
        }
    }
}

impl Drop for Capture {
    fn drop(&mut self) {
        disable();
        WINDOW.with(|current| current.set(HWND(std::ptr::null_mut())));
        unsafe {
            let _ = WTSUnRegisterSessionNotification(self.0);
            let _ = KillTimer(Some(self.0), HEALTH_TIMER);
            let _ = DestroyWindow(self.0);
        }
    }
}

pub(super) fn enable() -> windows::core::Result<()> {
    unsafe {
        let mut count = 0;
        let size = size_of::<RAWINPUTDEVICE>() as u32;
        if GetRegisteredRawInputDevices(None, &mut count, size) == u32::MAX {
            return Err(windows::core::Error::from_thread());
        }
        let mut registrations = vec![RAWINPUTDEVICE::default(); count as usize];
        if count > 0
            && GetRegisteredRawInputDevices(Some(registrations.as_mut_ptr()), &mut count, size)
                == u32::MAX
        {
            return Err(windows::core::Error::from_thread());
        }
        let old = [2, 6].map(|usage| {
            registrations
                .iter()
                .find(|r| r.usUsagePage == 1 && r.usUsage == usage)
                .copied()
        });
        let registrations = [2, 6].map(|usage| RAWINPUTDEVICE {
            usUsagePage: 1,
            usUsage: usage,
            dwFlags: RIDEV_INPUTSINK | RIDEV_DEVNOTIFY,
            hwndTarget: WINDOW.with(Cell::get),
        });
        RegisterRawInputDevices(&registrations, size)?;
        PREVIOUS.with(|previous| *previous.borrow_mut() = old);
        REGISTERED.with(|registered| registered.set(true));
        let mut message = MSG::default();
        while PeekMessageW(
            &mut message,
            Some(registrations[0].hwndTarget),
            WM_INPUT,
            WM_INPUT,
            PM_REMOVE,
        )
        .as_bool()
        {
            DefWindowProcW(
                message.hwnd,
                message.message,
                message.wParam,
                message.lParam,
            );
        }
        STARTED.with(|started| started.set(GetTickCount()));
        Ok(())
    }
}

pub(super) fn disable() {
    if !REGISTERED.with(|registered| registered.replace(false)) {
        return;
    }
    let previous = PREVIOUS.with(|previous| std::mem::take(&mut *previous.borrow_mut()));
    let registrations = restore_registrations(previous);
    if let Err(error) =
        unsafe { RegisterRawInputDevices(&registrations, size_of::<RAWINPUTDEVICE>() as u32) }
    {
        tracing::error!("Restore raw input registrations: {error}");
    }
}

fn restore_registrations(previous: [Option<RAWINPUTDEVICE>; 2]) -> [RAWINPUTDEVICE; 2] {
    std::array::from_fn(|index| {
        previous[index].unwrap_or(RAWINPUTDEVICE {
            usUsagePage: 1,
            usUsage: [2, 6][index],
            dwFlags: RIDEV_REMOVE,
            hwndTarget: HWND(std::ptr::null_mut()),
        })
    })
}

fn on_default_desktop() -> bool {
    unsafe {
        let Ok(desktop) = OpenInputDesktop(DESKTOP_CONTROL_FLAGS(0), false, DESKTOP_READOBJECTS)
        else {
            return false;
        };
        let mut name = [0u16; 128];
        let mut length = 0;
        let result = GetUserObjectInformationW(
            HANDLE(desktop.0),
            UOI_NAME,
            Some(name.as_mut_ptr().cast()),
            std::mem::size_of_val(&name) as u32,
            Some(&mut length),
        );
        let _ = CloseDesktop(desktop);
        result.is_ok()
            && String::from_utf16_lossy(
                &name[..name.iter().position(|v| *v == 0).unwrap_or(name.len())],
            )
            .eq_ignore_ascii_case("Default")
    }
}

unsafe extern "system" fn procedure(hwnd: HWND, message: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match message {
        WM_TIMER if wp.0 == HEALTH_TIMER => {
            super::sync_passthrough();
            if super::passthrough_active() && !on_default_desktop() {
                super::end_passthrough();
            }
        }
        WM_WTSSESSION_CHANGE
            if matches!(
                wp.0 as u32,
                WTS_SESSION_LOCK
                    | WTS_SESSION_LOGOFF
                    | WTS_CONSOLE_DISCONNECT
                    | WTS_REMOTE_DISCONNECT
            ) =>
        {
            super::end_passthrough()
        }
        WM_INPUT_DEVICE_CHANGE if wp.0 as u32 == GIDC_REMOVAL => super::end_passthrough(),
        WM_INPUT => {
            super::sync_passthrough();
            if !super::passthrough_active() {
                return;
            }
            let time = unsafe { GetMessageTime() } as u32;
            let captured = captured_at(time);
            if (time.wrapping_sub(STARTED.with(Cell::get)) as i32) < 0 {
                return;
            }
            let mut raw = RAWINPUT::default();
            let mut size = size_of::<RAWINPUT>() as u32;
            let read = unsafe {
                GetRawInputData(
                    HRAWINPUT(lp.0 as *mut _),
                    RID_INPUT,
                    Some((&mut raw as *mut RAWINPUT).cast()),
                    &mut size,
                    size_of::<RAWINPUTHEADER>() as u32,
                )
            };
            if read == u32::MAX {
                super::end_passthrough();
                return;
            }
            if raw.header.dwType == RIM_TYPEKEYBOARD.0 {
                super::raw_keyboard(unsafe { raw.data.keyboard }, time);
                return;
            }
            if raw.header.dwType != RIM_TYPEMOUSE.0 {
                return;
            }
            let mouse = unsafe { raw.data.mouse };
            if mouse.ulExtraInformation == super::REPLAY_TAG as u32 {
                return;
            }
            if mouse.usFlags.0 & MOUSE_MOVE_ABSOLUTE.0 != 0 {
                super::end_passthrough();
                return;
            }
            let flags = unsafe { mouse.Anonymous.Anonymous.usButtonFlags };
            for (button, down, up) in [
                (
                    MouseButton::Left,
                    RI_MOUSE_LEFT_BUTTON_DOWN,
                    RI_MOUSE_LEFT_BUTTON_UP,
                ),
                (
                    MouseButton::Right,
                    RI_MOUSE_RIGHT_BUTTON_DOWN,
                    RI_MOUSE_RIGHT_BUTTON_UP,
                ),
                (
                    MouseButton::Middle,
                    RI_MOUSE_MIDDLE_BUTTON_DOWN,
                    RI_MOUSE_MIDDLE_BUTTON_UP,
                ),
                (
                    MouseButton::Side1,
                    RI_MOUSE_BUTTON_4_DOWN,
                    RI_MOUSE_BUTTON_4_UP,
                ),
                (
                    MouseButton::Side2,
                    RI_MOUSE_BUTTON_5_DOWN,
                    RI_MOUSE_BUTTON_5_UP,
                ),
            ] {
                if flags & down as u16 != 0 {
                    super::raw_mouse_edge(button, true, captured);
                }
                if flags & up as u16 != 0 {
                    super::raw_mouse_edge(button, false, captured);
                }
                if !super::passthrough_active() {
                    return;
                }
            }
            if mouse.lLastX != 0 || mouse.lLastY != 0 {
                super::send_remote(
                    PhysicalInput::Motion {
                        dx: mouse.lLastX,
                        dy: mouse.lLastY,
                    },
                    captured,
                );
            }
            // The low-level hook owns wheel input before swallowing it.
            // Reading it here as well would duplicate reports on some drivers.
        }
        _ => {}
    }));
    if result.is_err() {
        super::end_passthrough();
    }
    unsafe { DefWindowProcW(hwnd, message, wp, lp) }
}
