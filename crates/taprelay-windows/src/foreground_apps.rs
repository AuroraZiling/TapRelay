//! Foreground-app executable discovery. Window titles are never rule identities.
mod icons;
mod names;
pub use icons::{IconPixels, executable_icon};
pub use names::executable_name;
use std::time::{Duration, Instant};
use windows::{
    Win32::{
        Foundation::{CloseHandle, HWND, LPARAM},
        System::Threading::{
            OpenProcess, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
            QueryFullProcessImageNameW,
        },
        UI::{
            Controls::Dialogs::*, Input::KeyboardAndMouse::GetActiveWindow, WindowsAndMessaging::*,
        },
    },
    core::{BOOL, PCWSTR, PWSTR},
};

fn process_path(pid: u32) -> Option<String> {
    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buffer = vec![0u16; 32768];
        let mut length = buffer.len() as u32;
        let result = QueryFullProcessImageNameW(
            handle,
            PROCESS_NAME_WIN32,
            PWSTR(buffer.as_mut_ptr()),
            &mut length,
        );
        let _ = CloseHandle(handle);
        result.ok()?;
        String::from_utf16(&buffer[..length as usize]).ok()
    }
}

#[derive(Default)]
pub struct Foreground {
    identity: Option<(usize, u32)>,
    path: Option<String>,
    refreshed: Option<Instant>,
}
impl Foreground {
    pub fn path(&mut self) -> Option<&str> {
        unsafe {
            let window = GetForegroundWindow();
            let mut pid = 0;
            GetWindowThreadProcessId(window, Some(&mut pid));
            let identity = (!window.0.is_null() && pid != 0).then_some((window.0 as usize, pid));
            if identity != self.identity
                || self
                    .refreshed
                    .is_none_or(|time| time.elapsed() >= Duration::from_secs(1))
            {
                self.identity = identity;
                self.path = identity
                    .and_then(|(_, pid)| process_path(pid))
                    .map(|path| taprelay_core::foreground_app::executable_identity(&path));
                self.refreshed = Some(Instant::now());
            }
            self.path.as_deref()
        }
    }
}

pub fn running() -> windows::core::Result<Vec<String>> {
    unsafe extern "system" fn collect(window: HWND, data: LPARAM) -> BOOL {
        // No Rust panic may cross the system callback boundary.
        let _ = std::panic::catch_unwind(|| unsafe {
            if !IsWindowVisible(window).as_bool() || GetWindowTextLengthW(window) == 0 {
                return;
            }
            let mut pid = 0;
            GetWindowThreadProcessId(window, Some(&mut pid));
            if let Some(path) = process_path(pid)
                && taprelay_core::foreground_app::valid_executable(&path)
            {
                (&mut *(data.0 as *mut Vec<String>)).push(path);
            }
        });
        BOOL(1)
    }
    let mut foreground_apps = Vec::<String>::new();
    unsafe {
        EnumWindows(
            Some(collect),
            LPARAM(&mut foreground_apps as *mut _ as isize),
        )?;
    }
    foreground_apps.sort_by_key(|path| taprelay_core::foreground_app::executable_identity(path));
    foreground_apps.dedup_by(|a, b| {
        taprelay_core::foreground_app::executable_identity(a)
            == taprelay_core::foreground_app::executable_identity(b)
    });
    Ok(foreground_apps)
}

pub fn browse() -> windows::core::Result<Option<String>> {
    let mut buffer = vec![0u16; 32768];
    let filter: Vec<u16> = "*.exe\0*.exe\0\0".encode_utf16().collect();
    unsafe {
        let mut dialog = OPENFILENAMEW {
            lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
            hwndOwner: GetActiveWindow(),
            lpstrFile: PWSTR(buffer.as_mut_ptr()),
            nMaxFile: buffer.len() as u32,
            lpstrFilter: PCWSTR(filter.as_ptr()),
            Flags: OFN_EXPLORER | OFN_FILEMUSTEXIST | OFN_PATHMUSTEXIST | OFN_NOCHANGEDIR,
            ..Default::default()
        };
        if GetOpenFileNameW(&mut dialog).as_bool() {
            let length = buffer
                .iter()
                .position(|&ch| ch == 0)
                .unwrap_or(buffer.len());
            Ok(Some(String::from_utf16_lossy(&buffer[..length])))
        } else if CommDlgExtendedError().0 == 0 {
            Ok(None)
        } else {
            Err(windows::core::Error::new(
                windows::core::HRESULT(0x80004005u32 as i32),
                "Cannot open application picker",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn process_identity_uses_the_full_executable_path_and_unknown_pid_is_none() {
        let pid = unsafe { windows::Win32::System::Threading::GetCurrentProcessId() };
        let path = super::process_path(pid).expect("own executable path");
        assert_eq!(
            taprelay_core::foreground_app::executable_identity(&path),
            taprelay_core::foreground_app::executable_identity(
                &std::env::current_exe().unwrap().to_string_lossy()
            )
        );
        assert!(super::process_path(0).is_none());
    }
}
