//! UAC elevation uses the Windows shell, never a command interpreter or a stored password.
use std::{
    ffi::{OsStr, OsString},
    mem::size_of,
    os::windows::ffi::OsStrExt,
    path::Path,
};
use taprelay_core::ports::BackendError;
use windows::{
    Win32::{
        Foundation::*,
        Security::*,
        System::{Com::*, Threading::*},
        UI::{Shell::*, WindowsAndMessaging::SW_SHOWNORMAL},
    },
    core::{BOOL, PCWSTR, w},
};

#[derive(Debug, Clone, Copy, Default)]
pub struct Status {
    pub elevated: bool,
    pub account_admin: bool,
}
struct Handle(HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

pub fn status() -> Result<Status, BackendError> {
    unsafe {
        (|| -> windows::core::Result<Status> {
            let mut token = HANDLE::default();
            OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token)?;
            let token = Handle(token);
            let mut size = 0;
            let mut elevation = TOKEN_ELEVATION::default();
            GetTokenInformation(
                token.0,
                TokenElevation,
                Some((&mut elevation as *mut TOKEN_ELEVATION).cast()),
                size_of::<TOKEN_ELEVATION>() as u32,
                &mut size,
            )?;
            let mut kind = TOKEN_ELEVATION_TYPE::default();
            GetTokenInformation(
                token.0,
                TokenElevationType,
                Some((&mut kind as *mut TOKEN_ELEVATION_TYPE).cast()),
                size_of::<TOKEN_ELEVATION_TYPE>() as u32,
                &mut size,
            )?;
            // Limited split tokens belong to administrator accounts even though their admin SID is disabled.
            let mut sid = [0u32; 17];
            let mut size = size_of_val(&sid) as u32;
            let sid = PSID(sid.as_mut_ptr().cast());
            CreateWellKnownSid(WinBuiltinAdministratorsSid, None, Some(sid), &mut size)?;
            let mut member = BOOL::default();
            CheckTokenMembership(None, sid, &mut member)?;
            Ok(Status {
                elevated: elevation.TokenIsElevated != 0,
                account_admin: member.as_bool() || kind == TokenElevationTypeLimited,
            })
        })()
        .map_err(|e| super::native_error("Administrator status", e))
    }
}
fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}
/// Quote one Windows CRT argument, including embedded quotes and trailing backslashes.
fn quote(value: &OsStr) -> Vec<u16> {
    let mut result = vec![b'"' as u16];
    let mut slashes = 0;
    for c in value.encode_wide() {
        if c == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        result.extend(std::iter::repeat_n(
            b'\\' as u16,
            if c == b'"' as u16 {
                slashes * 2 + 1
            } else {
                slashes
            },
        ));
        result.push(c);
        slashes = 0;
    }
    result.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    result.push(b'"' as u16);
    result
}
/// Returns false on UAC cancellation so the caller can keep its current session alive.
pub fn restart(arguments: &[OsString], directory: &Path) -> Result<bool, BackendError> {
    let executable =
        std::env::current_exe().map_err(|e| BackendError::Unavailable(e.to_string()))?;
    let executable = wide(executable.as_os_str());
    let directory = wide(directory.as_os_str());
    let mut parameters = vec![];
    for argument in arguments {
        if !parameters.is_empty() {
            parameters.push(b' ' as u16);
        }
        parameters.extend(quote(argument));
    }
    parameters.push(0);
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|e| super::native_error("CoInitializeEx", e))?;
        struct Com;
        impl Drop for Com {
            fn drop(&mut self) {
                unsafe {
                    CoUninitialize();
                }
            }
        }
        let _com = Com;
        let mut info = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC | SEE_MASK_FLAG_NO_UI,
            lpVerb: w!("runas"),
            lpFile: PCWSTR(executable.as_ptr()),
            lpParameters: PCWSTR(parameters.as_ptr()),
            lpDirectory: PCWSTR(directory.as_ptr()),
            nShow: SW_SHOWNORMAL.0,
            ..Default::default()
        };
        match ShellExecuteExW(&mut info) {
            Ok(()) => {
                let _process = Handle(info.hProcess);
                Ok(true)
            }
            Err(e) if e.code() == windows::core::HRESULT::from_win32(ERROR_CANCELLED.0) => {
                Ok(false)
            }
            Err(e) => Err(super::native_error("Restart as administrator", e)),
        }
    }
}
/// An elevated replacement waits before opening the old process's mutex and GATT services.
pub fn wait_for_parent(pid: u32) -> Result<(), BackendError> {
    if pid == 0 || pid == std::process::id() {
        return Err(BackendError::Unavailable("Invalid restart parent".into()));
    }
    unsafe {
        let process = match OpenProcess(PROCESS_SYNCHRONIZE, false, pid) {
            Ok(handle) => Handle(handle),
            Err(e) if e.code() == windows::core::HRESULT::from_win32(ERROR_INVALID_PARAMETER.0) => {
                return Ok(());
            }
            Err(e) => return Err(super::native_error("Open restart parent", e)),
        };
        match WaitForSingleObject(process.0, 15000) {
            WAIT_OBJECT_0 => Ok(()),
            WAIT_TIMEOUT => Err(BackendError::Unavailable(
                "Previous TapRelay process did not exit within 15 seconds".into(),
            )),
            _ => Err(super::native_error(
                "Wait for restart parent",
                windows::core::Error::from_thread(),
            )),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn arguments_preserve_spaces_quotes_unicode_and_trailing_slashes() {
        for (input, expected) in [
            ("", "\"\""),
            ("C:\\目录 space\\", "\"C:\\目录 space\\\\\""),
            ("a\"b", "\"a\\\"b\""),
            ("a\\\"b", "\"a\\\\\\\"b\""),
            ("a&b", "\"a&b\""),
        ] {
            assert_eq!(
                String::from_utf16(&quote(OsStr::new(input))).unwrap(),
                expected
            );
        }
    }
    #[test]
    fn current_token_can_be_inspected() {
        let s = status().unwrap();
        assert!(!s.elevated || s.account_admin);
    }
    #[test]
    fn restart_handoff_waits_for_process_exit_and_rejects_self() {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--list")
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        wait_for_parent(child.id()).unwrap();
        assert!(child.wait().unwrap().success());
        assert!(wait_for_parent(std::process::id()).is_err());
    }
}
