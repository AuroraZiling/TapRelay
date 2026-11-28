use super::*;
use std::{
    os::windows::ffi::OsStringExt,
    time::{Duration, Instant},
};
use uuid::Uuid;
use windows::{
    Win32::{
        Foundation::STILL_ACTIVE,
        System::{Registry::*, Variant::VARIANT},
    },
    core::{BSTR, Interface},
};

fn event_name(id: Uuid) -> Vec<u16> {
    wide(OsStr::new(&format!("Local\\TapRelay.Update.{id}")))
}

fn wait_for_parent_release(
    parent: u32,
    mutex_name: PCWSTR,
    timeout: Duration,
) -> windows::core::Result<()> {
    let deadline = Instant::now() + timeout;
    unsafe {
        loop {
            match CreateMutexW(None, false, mutex_name) {
                Ok(handle) => {
                    let already_exists = GetLastError() == ERROR_ALREADY_EXISTS;
                    let _ = CloseHandle(handle);
                    if !already_exists {
                        break;
                    }
                }
                Err(e)
                    if matches!(
                        e.code(),
                        c if c == windows::core::HRESULT::from_win32(ERROR_ALREADY_EXISTS.0)
                            || c == windows::core::HRESULT::from_win32(ERROR_ACCESS_DENIED.0)
                    ) => {}
                Err(e) => return Err(e),
            }
            if Instant::now() >= deadline {
                return Err(windows::core::Error::new(
                    E_ABORT,
                    "The original TapRelay instance did not exit",
                ));
            }
            std::thread::sleep(Duration::from_millis(50));
        }

        let process = match OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, parent) {
            Ok(handle) => Handle(handle),
            Err(e)
                if matches!(
                    e.code(),
                    c if c == windows::core::HRESULT::from_win32(ERROR_INVALID_PARAMETER.0)
                        || c == windows::core::HRESULT::from_win32(ERROR_ACCESS_DENIED.0)
                ) =>
            {
                return Ok(());
            }
            Err(e) => return Err(e),
        };
        loop {
            let mut exit_code = 0u32;
            GetExitCodeProcess(process.0, &mut exit_code)?;
            if exit_code != STILL_ACTIVE.0 as u32 {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(windows::core::Error::new(
                    E_ABORT,
                    "The original TapRelay instance did not exit",
                ));
            }
            std::thread::sleep(Duration::from_millis(25));
        }
    }
}

pub fn restart_for_update(tag: &str, log_level: &str) -> Result<(), BackendError> {
    let exe = std::env::current_exe().map_err(|e| BackendError::Unavailable(e.to_string()))?;
    unsafe {
        CoInitializeEx(None, COINIT_APARTMENTTHREADED)
            .ok()
            .map_err(|e| super::super::native_error("Update handoff apartment", e))?;
        let launch = (|| -> windows::core::Result<_> {
            let id = Uuid::from_u128(CoCreateGuid()?.to_u128());
            let shell: IShellWindows = CoCreateInstance(&ShellWindows, None, CLSCTX_LOCAL_SERVER)?;
            let desktop = shell.FindWindowSW(
                &VARIANT::from(CSIDL_DESKTOP as i32),
                &VARIANT::default(),
                SWC_DESKTOP,
                &mut 0,
                SWFO_NEEDDISPATCH,
            )?;
            let browser: IShellBrowser = desktop
                .cast::<IServiceProvider>()?
                .QueryService(&SID_STopLevelBrowser)?;
            let background: IDispatch = browser
                .QueryActiveShellView()?
                .GetItemObject(SVGIO_BACKGROUND)?;
            let shell: IShellDispatch2 = background
                .cast::<IShellFolderViewDual>()?
                .Application()?
                .cast()?;
            let pid = std::process::id().to_string();
            let nonce = id.to_string();
            let arguments = [
                "--user",
                "--handoff",
                &pid,
                "--confirmed-update",
                tag,
                &nonce,
                "--log-level",
                log_level,
            ];
            let parameters = arguments
                .map(|arg| quote(OsStr::new(arg)))
                .join(&(b' ' as u16));
            shell.ShellExecute(
                &BSTR::from_wide(&exe.as_os_str().encode_wide().collect::<Vec<_>>()),
                &VARIANT::from(BSTR::from_wide(&parameters)),
                &VARIANT::default(),
                &VARIANT::from("open"),
                &VARIANT::default(),
            )?;
            Ok(id)
        })();
        CoUninitialize();
        let name = event_name(
            launch.map_err(|e| super::super::native_error("Launch TapRelay from Explorer", e))?,
        );
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            match OpenEventW(EVENT_MODIFY_STATE, false, PCWSTR(name.as_ptr())) {
                Ok(event) => {
                    let event = Handle(event);
                    return SetEvent(event.0)
                        .map_err(|e| super::super::native_error("Confirm update handoff", e));
                }
                Err(e)
                    if e.code() == windows::core::HRESULT::from_win32(ERROR_FILE_NOT_FOUND.0)
                        && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) => return Err(super::super::native_error("Wait for installing user", e)),
            }
        }
    }
}

pub fn accept_update_handoff(parent: u32, nonce: &str) -> Result<(), BackendError> {
    let id = Uuid::parse_str(nonce)
        .map_err(|_| BackendError::Unavailable("Invalid update handoff ID".into()))?;
    if parent == 0 || parent == std::process::id() || status()?.elevated {
        return Err(BackendError::Unavailable(
            "The update requires a separate non-elevated process".into(),
        ));
    }
    let exe = std::env::current_exe().map_err(|e| BackendError::Unavailable(e.to_string()))?;
    unsafe {
        (|| -> windows::core::Result<()> {
            let mut path = [0u16; 32768];
            let mut size = size_of_val(&path) as u32;
            RegGetValueW(
                HKEY_CURRENT_USER,
                w!("Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\TapRelay"),
                w!("InstallLocation"),
                RRF_RT_REG_SZ,
                None,
                Some(path.as_mut_ptr().cast()),
                Some(&mut size),
            )
            .ok()?;
            let end = path.iter().position(|ch| *ch == 0).unwrap_or(path.len());
            let installed = std::path::PathBuf::from(OsString::from_wide(&path[..end]))
                .join("current")
                .join("TapRelay.exe");
            if std::fs::canonicalize(installed)? != std::fs::canonicalize(exe)? {
                return Err(windows::core::Error::new(
                    E_ACCESSDENIED,
                    "This account does not own the installation",
                ));
            }
            let name = event_name(id);
            // The non-elevated owner creates the event; an elevated event would deny a medium-integrity writer.
            let ready = Handle(CreateEventW(None, true, false, PCWSTR(name.as_ptr()))?);
            if GetLastError() == ERROR_ALREADY_EXISTS
                || WaitForSingleObject(ready.0, 20000) != WAIT_OBJECT_0
            {
                return Err(windows::core::Error::new(
                    E_ABORT,
                    "The original TapRelay instance did not confirm the update",
                ));
            }
            wait_for_parent_release(
                parent,
                w!("Local\\TapRelay.GUI.Runtime.v2"),
                Duration::from_secs(15),
            )?;
            Ok(())
        })()
        .map_err(|e| super::super::native_error("Accept update handoff", e))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn waits_for_held_mutex_and_child_exit_without_synchronize_access() {
        let mutex_name = format!("Local\\TapRelay.UpdateTest.{}", Uuid::new_v4());
        let wide_name = wide(OsStr::new(&mutex_name));
        let (created_tx, created_rx) = mpsc::channel();
        let thread_name = wide_name.clone();
        let holder = std::thread::spawn(move || {
            let mutex = unsafe { CreateMutexW(None, false, PCWSTR(thread_name.as_ptr())) }.unwrap();
            let _guard = Handle(mutex);
            created_tx.send(()).unwrap();
            std::thread::sleep(Duration::from_millis(120));
        });
        created_rx.recv().unwrap();

        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .arg("--list")
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let started = Instant::now();
        wait_for_parent_release(
            child.id(),
            PCWSTR(wide_name.as_ptr()),
            Duration::from_secs(5),
        )
        .unwrap();
        assert!(started.elapsed() >= Duration::from_millis(80));
        holder.join().unwrap();
        assert!(child.wait().unwrap().success());
    }

    #[test]
    fn times_out_when_mutex_or_parent_stays_active() {
        let mutex_name = format!("Local\\TapRelay.UpdateTimeout.{}", Uuid::new_v4());
        let wide_name = wide(OsStr::new(&mutex_name));
        let (created_tx, created_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let thread_name = wide_name.clone();
        let holder = std::thread::spawn(move || {
            let mutex = unsafe { CreateMutexW(None, false, PCWSTR(thread_name.as_ptr())) }.unwrap();
            let _guard = Handle(mutex);
            created_tx.send(()).unwrap();
            let _ = release_rx.recv();
        });
        created_rx.recv().unwrap();
        assert!(
            wait_for_parent_release(
                std::process::id(),
                PCWSTR(wide_name.as_ptr()),
                Duration::from_millis(80),
            )
            .is_err()
        );
        let _ = release_tx.send(());
        holder.join().unwrap();

        let free_name = wide(OsStr::new(&format!(
            "Local\\TapRelay.UpdateFree.{}",
            Uuid::new_v4()
        )));
        assert!(
            wait_for_parent_release(
                std::process::id(),
                PCWSTR(free_name.as_ptr()),
                Duration::from_millis(80),
            )
            .is_err()
        );
    }
}
