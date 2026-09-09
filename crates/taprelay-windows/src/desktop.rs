//! Windows shell integration, isolated from Slint and the application state model.
use super::native::{OwnedIcon, OwnedRegistryKey, OwnedWindow};
use std::{
    collections::VecDeque,
    mem::size_of,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
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
static MENU_LABELS: Mutex<[String; 4]> =
    Mutex::new([String::new(), String::new(), String::new(), String::new()]);
static LISTENING: AtomicBool = AtomicBool::new(false);
static TASKBAR: AtomicU32 = AtomicU32::new(0);
const ACTIVATE: u32 = WM_APP + 71;
const TRAY: u32 = WM_APP + 72;
#[derive(Clone, Copy)]
pub enum DesktopEvent {
    Show,
    Toggle,
    Quit,
    Resume,
    TrayReset,
}
fn emit(e: DesktopEvent) {
    if let Ok(mut q) = EVENTS.lock() {
        q.push_back(e);
    }
}
pub fn events() -> Vec<DesktopEvent> {
    EVENTS
        .lock()
        .map(|mut q| q.drain(..).collect())
        .unwrap_or_default()
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn copy_wide(dst: &mut [u16], s: &str) {
    dst.fill(0);
    let count = dst.len().saturating_sub(1);
    for (d, c) in dst.iter_mut().take(count).zip(s.encode_utf16()) {
        *d = c;
    }
}
pub fn any_input_held() -> bool {
    unsafe {
        (1..=254)
            .filter(|k| !matches!(k, 3 | 7 | 0x10..=0x12))
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
        if msg == TASKBAR.load(Ordering::Relaxed) && msg != 0 {
            emit(DesktopEvent::TrayReset);
            return LRESULT(0);
        }
        match msg {
            ACTIVATE => emit(DesktopEvent::Show),
            WM_POWERBROADCAST if wp.0 == 18 || wp.0 == 7 => emit(DesktopEvent::Resume),
            TRAY => match lp.0 as u32 {
                WM_LBUTTONUP | NIN_BALLOONUSERCLICK => emit(DesktopEvent::Show),
                WM_RBUTTONUP => {
                    if let Ok(menu) = CreatePopupMenu() {
                        let labels = MENU_LABELS
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .clone();
                        let texts = [
                            &labels[0],
                            &labels[if LISTENING.load(Ordering::Relaxed) {
                                2
                            } else {
                                1
                            }],
                            &labels[3],
                        ];
                        for (i, t) in texts.iter().enumerate() {
                            let text = wide(t);
                            let _ = AppendMenuW(menu, MF_STRING, i + 1, PCWSTR(text.as_ptr()));
                        }
                        let mut point = POINT::default();
                        let _ = GetCursorPos(&mut point);
                        let _ = SetForegroundWindow(hwnd);
                        let selected = TrackPopupMenu(
                            menu,
                            TPM_RETURNCMD | TPM_RIGHTBUTTON,
                            point.x,
                            point.y,
                            Some(0),
                            hwnd,
                            None,
                        )
                        .0;
                        match selected {
                            1 => emit(DesktopEvent::Show),
                            2 => emit(DesktopEvent::Toggle),
                            3 => emit(DesktopEvent::Quit),
                            _ => {}
                        }
                        let _ = DestroyMenu(menu);
                        let _ = PostMessageW(Some(hwnd), WM_NULL, WPARAM(0), LPARAM(0));
                    }
                }
                _ => {}
            },
            _ => return DefWindowProcW(hwnd, msg, wp, lp),
        }
        LRESULT(0)
    }
}
pub struct Desktop {
    _window: OwnedWindow,
    icon: OwnedIcon,
    data: NOTIFYICONDATAW,
    state: u8,
    shell_failed: bool,
}
impl Desktop {
    pub fn new(labels: [String; 4]) -> windows::core::Result<Self> {
        *MENU_LABELS.lock().unwrap_or_else(|e| e.into_inner()) = labels;
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
            let hwnd = window.0;
            // Only these data-free messages may cross the Explorer/UAC boundary.
            ChangeWindowMessageFilterEx(hwnd, ACTIVATE, MSGFLT_ALLOW, None)?;
            let taskbar = RegisterWindowMessageW(w!("TaskbarCreated"));
            if taskbar == 0 {
                return Err(windows::core::Error::from_thread());
            }
            ChangeWindowMessageFilterEx(hwnd, taskbar, MSGFLT_ALLOW, None)?;
            TASKBAR.store(taskbar, Ordering::Relaxed);
            let icon = OwnedIcon(make_icon(1)?);
            let mut data = NOTIFYICONDATAW {
                cbSize: size_of::<NOTIFYICONDATAW>() as u32,
                hWnd: hwnd,
                uID: 1,
                uFlags: NIF_ICON | NIF_TIP | NIF_MESSAGE,
                uCallbackMessage: TRAY,
                hIcon: icon.0,
                ..Default::default()
            };
            copy_wide(&mut data.szTip, "TapRelay");
            if !Shell_NotifyIconW(NIM_ADD, &data).as_bool() {
                return Err(windows::core::Error::from_thread());
            }
            Ok(Self {
                _window: window,
                icon,
                data,
                state: 1,
                shell_failed: false,
            })
        }
    }
    pub fn update(&mut self, state: u8, tip: &str, listening: bool, labels: [String; 4]) {
        unsafe {
            LISTENING.store(listening, Ordering::Relaxed);
            *MENU_LABELS.lock().unwrap_or_else(|e| e.into_inner()) = labels;
            if self.state != state
                && let Ok(icon) = make_icon(state)
            {
                self.icon = OwnedIcon(icon);
                self.data.hIcon = icon;
                self.state = state;
            }
            copy_wide(&mut self.data.szTip, tip);
            let ok = Shell_NotifyIconW(NIM_MODIFY, &self.data).as_bool();
            if !ok && !self.shell_failed {
                tracing::warn!("Tray update failed; attempting to restore icon");
            }
            self.shell_failed = !ok;
            if !ok {
                self.restore();
            }
        }
    }
    pub fn restore(&mut self) {
        unsafe {
            let restored = Shell_NotifyIconW(NIM_ADD, &self.data).as_bool();
            if !restored && !self.shell_failed {
                tracing::warn!("Tray restore failed");
            }
            if restored && self.shell_failed {
                tracing::info!("Tray icon restored");
            }
            self.shell_failed = !restored;
        }
    }
    pub fn notify(&self, title: &str, body: &str) {
        unsafe {
            let mut data = self.data;
            data.uFlags = NIF_INFO;
            data.dwInfoFlags = NIIF_INFO;
            copy_wide(&mut data.szInfoTitle, title);
            copy_wide(&mut data.szInfo, body);
            if !Shell_NotifyIconW(NIM_MODIFY, &data).as_bool() {
                tracing::warn!("Tray notification failed");
            }
        }
    }
}
impl Drop for Desktop {
    fn drop(&mut self) {
        unsafe {
            if !Shell_NotifyIconW(NIM_DELETE, &self.data).as_bool() {
                tracing::debug!("Tray icon was already absent at shutdown");
            }
        }
    }
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
fn make_icon(state: u8) -> windows::core::Result<HICON> {
    unsafe {
        let mut bytes = icon_rgba(state);
        for pixel in bytes.chunks_exact_mut(4) {
            pixel.swap(0, 2);
        }
        let color = CreateBitmap(32, 32, 1, 32, Some(bytes.as_ptr().cast()));
        let mask = CreateBitmap(32, 32, 1, 1, Some([0u8; 128].as_ptr().cast()));
        let icon = CreateIconIndirect(&ICONINFO {
            fIcon: true.into(),
            hbmColor: color,
            hbmMask: mask,
            ..Default::default()
        });
        let _ = DeleteObject(color.into());
        let _ = DeleteObject(mask.into());
        icon
    }
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
pub fn system_chinese() -> bool {
    unsafe {
        let mut locale = [0u16; 85];
        let len = windows::Win32::Globalization::GetUserDefaultLocaleName(&mut locale);
        len > 2
            && String::from_utf16_lossy(&locale[..len.saturating_sub(1) as usize]).starts_with("zh")
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
impl Desktop {
    pub fn copy_text(&self, text: &str) -> windows::core::Result<()> {
        unsafe {
            use windows::Win32::System::{DataExchange::*, Memory::*};
            OpenClipboard(Some(self._window.0))?;
            struct Clipboard;
            impl Drop for Clipboard {
                fn drop(&mut self) {
                    unsafe {
                        let _ = CloseClipboard();
                    }
                }
            }
            let _clipboard = Clipboard;
            let text = wide(text);
            let memory = GlobalAlloc(GMEM_MOVEABLE, text.len() * 2)?;
            let dst = GlobalLock(memory);
            if dst.is_null() {
                let _ = GlobalFree(Some(memory));
                return Err(windows::core::Error::from_thread());
            }
            std::ptr::copy_nonoverlapping(text.as_ptr(), dst.cast(), text.len());
            let _ = GlobalUnlock(memory);
            if let Err(e) = EmptyClipboard() {
                let _ = GlobalFree(Some(memory));
                return Err(e);
            }
            match SetClipboardData(13, Some(HANDLE(memory.0))) {
                Ok(_) => Ok(()),
                Err(e) => {
                    let _ = GlobalFree(Some(memory));
                    Err(e)
                }
            }
        }
    }
}
/// The owner HWND belongs to the GUI, which remains alive for the export operation.
/// Only the dialog runs on this STA; no Controller borrow or Slint value crosses threads.
pub fn export_logs(
    default: std::path::PathBuf,
    text: String,
) -> std::io::Result<std::sync::mpsc::Receiver<Result<Option<std::path::PathBuf>, String>>> {
    let owner = unsafe { GetForegroundWindow().0 as usize };
    export_with(default, text, move |default| {
        use windows::Win32::System::Com::*;
        unsafe {
            CoInitializeEx(None, COINIT_APARTMENTTHREADED)
                .ok()
                .map_err(|e| e.to_string())?;
        }
        struct Com;
        impl Drop for Com {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        let _com = Com;
        save_file(default, HWND(owner as *mut _)).map_err(|e| e.to_string())
    })
}
type ExportResult = Result<Option<std::path::PathBuf>, String>;
fn export_with(
    default: std::path::PathBuf,
    text: String,
    choose: impl FnOnce(&std::path::Path) -> ExportResult + Send + 'static,
) -> std::io::Result<std::sync::mpsc::Receiver<ExportResult>> {
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    std::thread::Builder::new()
        .name("taprelay-export".into())
        .spawn(move || {
            let result = (|| -> ExportResult {
                let selected = choose(&default)?;
                if let Some(path) = &selected {
                    std::fs::write(path, text).map_err(|e| e.to_string())?;
                }
                Ok(selected)
            })();
            let _ = tx.send(result);
        })?;
    Ok(rx)
}

fn save_file(
    default: &std::path::Path,
    owner: HWND,
) -> windows::core::Result<Option<std::path::PathBuf>> {
    unsafe {
        use windows::Win32::UI::Controls::Dialogs::*;
        let mut file = [0u16; 32768];
        copy_wide(&mut file, &default.to_string_lossy());
        let mut dialog = OPENFILENAMEW {
            hwndOwner: owner,
            lStructSize: size_of::<OPENFILENAMEW>() as u32,
            lpstrFile: windows::core::PWSTR(file.as_mut_ptr()),
            nMaxFile: file.len() as u32,
            lpstrFilter: w!("Log files\0*.log\0All files\0*.*\0"),
            lpstrDefExt: w!("log"),
            Flags: OFN_OVERWRITEPROMPT | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
            ..Default::default()
        };
        if GetSaveFileNameW(&mut dialog).as_bool() {
            let end = file.iter().position(|&c| c == 0).unwrap_or(file.len());
            Ok(Some(String::from_utf16_lossy(&file[..end]).into()))
        } else {
            let code = CommDlgExtendedError();
            if code == COMMON_DLG_ERRORS(0) {
                Ok(None)
            } else {
                Err(windows::core::Error::new(
                    windows::core::HRESULT(0x80004005u32 as i32),
                    format!("Save dialog error: {:#x}", code.0),
                ))
            }
        }
    }
}

#[cfg(test)]
mod export_tests {
    use super::*;
    #[test]
    fn pending_dialog_does_not_block_its_caller() {
        let (release, wait) = std::sync::mpsc::channel();
        let result = export_with("unused.log".into(), "log snapshot".into(), move |_| {
            wait.recv_timeout(std::time::Duration::from_secs(2))
                .map_err(|e| e.to_string())?;
            Ok(None)
        })
        .unwrap();
        // Returning from export_with before releasing the simulated modal dialog
        // is what permits the GUI timer to continue consuming native input.
        assert!(matches!(
            result.try_recv(),
            Err(std::sync::mpsc::TryRecvError::Empty)
        ));
        release.send(()).unwrap();
        assert_eq!(
            result
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap(),
            Ok(None)
        );
    }
}
