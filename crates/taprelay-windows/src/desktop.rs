//! Windows shell integration, isolated from Slint and the application state model.
mod shell_features;
use super::native::{OwnedRegistryKey, OwnedWindow};
use std::{collections::VecDeque, sync::Mutex};
use tray_icon::{
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    menu::{Menu, MenuEvent, MenuItem},
};
use windows::{
    Win32::{
        Foundation::*,
        Graphics::Gdi::*,
        System::{LibraryLoader::GetModuleHandleW, Registry::*, Threading::*},
        UI::{Input::KeyboardAndMouse::GetAsyncKeyState, Shell::*, WindowsAndMessaging::*},
    },
    core::{PCWSTR, w},
};
static EVENTS: Mutex<VecDeque<DesktopEvent>> = Mutex::new(VecDeque::new());
const ACTIVATE: u32 = WM_APP + 71;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopEvent {
    Show,
    Toggle,
    Quit,
    Resume,
}
fn emit(e: DesktopEvent) {
    if let Ok(mut q) = EVENTS.lock() {
        q.push_back(e);
    }
}
pub fn events() -> Vec<DesktopEvent> {
    for event in TrayIconEvent::receiver().try_iter() {
        if let Some(event) = tray_event(&event) {
            emit(event);
        }
    }
    for event in MenuEvent::receiver().try_iter() {
        if let Some(event) = menu_event(event.id.as_ref()) {
            emit(event);
        }
    }
    EVENTS
        .lock()
        .map(|mut q| q.drain(..).collect())
        .unwrap_or_default()
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
pub fn any_input_held() -> bool {
    unsafe {
        (1..=254)
            .filter(|k| !matches!(k, 3 | 7))
            .any(|k| GetAsyncKeyState(k) < 0)
    }
}
pub fn virtual_key_for_character(ch: char) -> Option<u8> {
    let ch = u16::try_from(ch as u32).ok()?;
    let key = unsafe { windows::Win32::UI::Input::KeyboardAndMouse::VkKeyScanW(ch) };
    (key != -1).then_some((key & 0xff) as u8)
}
pub fn ui_input_is_injected() -> bool {
    use windows::Win32::UI::Input::*;
    let mut source = INPUT_MESSAGE_SOURCE::default();
    unsafe { GetCurrentInputMessageSource(&mut source).is_ok() && source.originId == IMO_INJECTED }
}
pub fn activate_existing() -> bool {
    unsafe {
        if let Ok(hwnd) = FindWindowW(w!("TapRelay.Desktop.v2"), None) {
            let mut pid = 0;
            GetWindowThreadProcessId(hwnd, Some(&mut pid));
            let _ = AllowSetForegroundWindow(pid);
            return PostMessageW(Some(hwnd), ACTIVATE, WPARAM(0), LPARAM(0)).is_ok();
        }
    }
    false
}
unsafe extern "system" fn procedure(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
    unsafe {
        match msg {
            ACTIVATE => emit(DesktopEvent::Show),
            WM_POWERBROADCAST if wp.0 == 18 || wp.0 == 7 => emit(DesktopEvent::Resume),
            _ => return DefWindowProcW(hwnd, msg, wp, lp),
        }
        LRESULT(0)
    }
}

const TRAY_ID: &str = "taprelay";
const SHOW_ID: &str = "taprelay.show";
const TOGGLE_ID: &str = "taprelay.toggle";
const QUIT_ID: &str = "taprelay.quit";

fn menu_event(id: &str) -> Option<DesktopEvent> {
    match id {
        SHOW_ID => Some(DesktopEvent::Show),
        TOGGLE_ID => Some(DesktopEvent::Toggle),
        QUIT_ID => Some(DesktopEvent::Quit),
        _ => None,
    }
}

fn tray_event(event: &TrayIconEvent) -> Option<DesktopEvent> {
    match event {
        TrayIconEvent::Click {
            id,
            button: MouseButton::Left,
            button_state: MouseButtonState::Up,
            ..
        } if id.as_ref() == TRAY_ID => Some(DesktopEvent::Show),
        _ => None,
    }
}

fn tray_error(error: impl std::fmt::Display) -> windows::core::Error {
    windows::core::Error::new(E_FAIL, error.to_string())
}

fn tray_guid(path: &std::path::Path) -> u128 {
    // Unsigned portable executables must not share a GUID across different paths.
    // A name UUID keeps the identity stable for subsequent launches at this path.
    use std::os::windows::ffi::OsStrExt;
    let path: Vec<_> = path
        .as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect();
    uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, &path).as_u128()
}

pub struct Desktop {
    _window: OwnedWindow,
    tray: TrayIcon,
    show: MenuItem,
    toggle: MenuItem,
    quit: MenuItem,
    shell: shell_features::ShellFeatures,
    state: u8,
    shell_failed: bool,
}
impl Desktop {
    pub fn new(labels: [String; 4]) -> windows::core::Result<Self> {
        unsafe {
            let instance = GetModuleHandleW(None)?;
            let class = WNDCLASSW {
                lpfnWndProc: Some(procedure),
                hInstance: instance.into(),
                lpszClassName: w!("TapRelay.Desktop.v2"),
                ..Default::default()
            };
            if RegisterClassW(&class) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
                return Err(windows::core::Error::from_thread());
            }
            let window = OwnedWindow(CreateWindowExW(
                WINDOW_EX_STYLE::default(),
                class.lpszClassName,
                w!("TapRelay"),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                None,
                None,
                Some(instance.into()),
                None,
            )?);
            // The activation host remains separate from the library-owned tray window.
            ChangeWindowMessageFilterEx(window.0, ACTIVATE, MSGFLT_ALLOW, None)?;
            let show = MenuItem::with_id(SHOW_ID, &labels[0], true, None);
            let toggle = MenuItem::with_id(TOGGLE_ID, &labels[1], true, None);
            let quit = MenuItem::with_id(QUIT_ID, &labels[3], true, None);
            let menu = Menu::with_items(&[&show, &toggle, &quit]).map_err(tray_error)?;
            let guid = tray_guid(&std::env::current_exe().map_err(tray_error)?);
            let tray = TrayIconBuilder::new()
                .with_id(TRAY_ID)
                .with_guid(guid)
                .with_icon(Icon::from_rgba(icon_rgba(1), 32, 32).map_err(tray_error)?)
                .with_tooltip("TapRelay")
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .build()
                .map_err(tray_error)?;
            let shell = shell_features::ShellFeatures::new(&tray, guid)?;
            Ok(Self {
                _window: window,
                tray,
                show,
                toggle,
                quit,
                shell,
                state: 1,
                shell_failed: false,
            })
        }
    }

    pub fn update(&mut self, state: u8, tip: &str, listening: bool, labels: [String; 4]) {
        self.show.set_text(&labels[0]);
        self.toggle.set_text(&labels[if listening { 2 } else { 1 }]);
        self.quit.set_text(&labels[3]);
        let update = || -> windows::core::Result<()> {
            if self.state != state || self.shell_failed {
                self.tray
                    .set_icon(Some(
                        Icon::from_rgba(icon_rgba(state), 32, 32).map_err(tray_error)?,
                    ))
                    .map_err(tray_error)?;
            }
            self.shell.set_tooltip(tip)
        };
        match update() {
            Ok(()) => {
                if self.shell_failed {
                    tracing::info!("Tray icon restored");
                }
                self.state = state;
                self.shell_failed = false;
            }
            Err(error) => {
                if !self.shell_failed {
                    tracing::warn!(%error, "Tray update failed; attempting recovery");
                }
                self.shell_failed = true;
                // Let tray-icon re-register its own icon, menu, visibility and callback state.
                unsafe {
                    let message = RegisterWindowMessageW(w!("TaskbarCreated"));
                    if message != 0 {
                        let _ = PostMessageW(
                            Some(HWND(self.tray.window_handle())),
                            message,
                            WPARAM(0),
                            LPARAM(0),
                        );
                    }
                }
            }
        }
    }

    pub fn notify(&self, title: &str, body: &str) {
        self.shell.notify(title, body);
    }
}

fn truncate_utf16(value: &str, units: usize) -> String {
    let mut used = 0;
    value
        .chars()
        .take_while(|ch| {
            used += ch.len_utf16();
            used <= units
        })
        .collect()
}

/// App artwork, plus lower-right state dot. Colors: green/gray/amber/red.
pub fn icon_rgba(state: u8) -> Vec<u8> {
    let mut rgba = include_bytes!("../resources/taprelay-32.rgba").to_vec();
    let dot = match state {
        0 => [46, 180, 112, 255],
        1 => [139, 145, 154, 255],
        2 => [232, 174, 54, 255],
        _ => [230, 79, 85, 255],
    };
    for y in 0i32..32 {
        for x in 0i32..32 {
            let i = ((y * 32 + x) * 4) as usize;
            if state < 4 && (x - 25).pow(2) + (y - 25).pow(2) <= 49 {
                rgba[i..i + 4].copy_from_slice(&[38, 47, 65, 255]);
            }
            if state < 4 && (x - 25).pow(2) + (y - 25).pow(2) <= 25 {
                rgba[i..i + 4].copy_from_slice(&dot);
            }
        }
    }
    rgba
}
pub fn open(target: &str) -> windows::core::Result<()> {
    unsafe {
        let value = wide(target);
        let result = ShellExecuteW(
            None,
            w!("open"),
            PCWSTR(value.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        );
        if result.0 as isize <= 32 {
            Err(windows::core::Error::from_thread())
        } else {
            Ok(())
        }
    }
}
pub fn system_dark() -> bool {
    unsafe {
        let mut value = 1u32;
        let mut size = 4;
        let _ = RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize"),
            w!("AppsUseLightTheme"),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut value as *mut u32).cast()),
            Some(&mut size),
        );
        value == 0
    }
}
pub fn system_locale_id() -> Option<String> {
    unsafe {
        // LOCALE_NAME_MAX_LENGTH, which GetUserDefaultLocaleName never exceeds.
        let mut locale = [0u16; 85];
        let len = windows::Win32::Globalization::GetUserDefaultLocaleName(&mut locale);
        if len <= 1 {
            return None;
        }
        Some(String::from_utf16_lossy(
            &locale[..len.saturating_sub(1) as usize],
        ))
    }
}
pub fn set_autostart(enabled: bool) -> windows::core::Result<()> {
    unsafe {
        let mut key = HKEY::default();
        RegCreateKeyExW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
            None,
            None,
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE,
            None,
            &mut key,
            None,
        )
        .ok()?;
        let key = OwnedRegistryKey(key);
        if enabled {
            let exe = std::env::current_exe().map_err(|_| windows::core::Error::from_thread())?;
            let command = wide(&format!("\"{}\"", exe.display()));
            let bytes =
                std::slice::from_raw_parts(command.as_ptr().cast::<u8>(), command.len() * 2);
            RegSetValueExW(key.0, w!("TapRelay"), None, REG_SZ, Some(bytes)).ok()
        } else {
            let r = RegDeleteValueW(key.0, w!("TapRelay"));
            if r == ERROR_FILE_NOT_FOUND {
                Ok(())
            } else {
                r.ok()
            }
        }
    }
}
pub fn autostart_enabled() -> bool {
    unsafe {
        let mut text = [0u16; 32768];
        let mut size = (text.len() * 2) as u32;
        let ok = RegGetValueW(
            HKEY_CURRENT_USER,
            w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run"),
            w!("TapRelay"),
            RRF_RT_REG_SZ,
            None,
            Some(text.as_mut_ptr().cast()),
            Some(&mut size),
        )
        .is_ok();
        let len = text.iter().position(|&c| c == 0).unwrap_or(text.len());
        ok && std::env::current_exe().is_ok_and(|p| {
            String::from_utf16_lossy(&text[..len]).to_lowercase()
                == format!("\"{}\"", p.display()).to_lowercase()
        })
    }
}
pub fn foreground_is_ours() -> bool {
    unsafe {
        let hwnd = GetForegroundWindow();
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        pid == GetCurrentProcessId()
    }
}
pub fn visible_position(x: i32, y: i32) -> bool {
    unsafe { !MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONULL).is_invalid() }
}
pub fn show_error(text: &str) {
    unsafe {
        let text = wide(text);
        let _ = MessageBoxW(
            None,
            PCWSTR(text.as_ptr()),
            w!("TapRelay"),
            MB_OK | MB_ICONERROR,
        );
    }
}
pub fn foreground_app() {
    unsafe {
        // Slint owns the actual visible application window, the hidden shell host has a distinct title.
        unsafe extern "system" fn find(hwnd: HWND, _: LPARAM) -> windows::core::BOOL {
            unsafe {
                let mut pid = 0;
                GetWindowThreadProcessId(hwnd, Some(&mut pid));
                if pid == GetCurrentProcessId() && IsWindowVisible(hwnd).as_bool() {
                    let _ = SetForegroundWindow(hwnd);
                    return false.into();
                }
                true.into()
            }
        }
        let _ = EnumWindows(Some(find), LPARAM(0));
    }
}
