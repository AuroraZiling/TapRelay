use taprelay_core::ports::BackendError;
use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE},
        System::{
            Threading::CreateMutexW,
            WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
        },
    },
    core::PCWSTR,
};

/// Owned handles acquired before a larger object is fully constructed must also
/// close on `?` paths. Each wrapper is confined to its creating GUI thread.
pub(crate) struct OwnedWindow(pub windows::Win32::Foundation::HWND);
impl Drop for OwnedWindow {
    fn drop(&mut self) {
        unsafe {
            if let Err(e) = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(self.0) {
                tracing::warn!("DestroyWindow: {e}");
            }
        }
    }
}
pub(crate) struct OwnedIcon(pub windows::Win32::UI::WindowsAndMessaging::HICON);
impl Drop for OwnedIcon {
    fn drop(&mut self) {
        unsafe {
            if let Err(e) = windows::Win32::UI::WindowsAndMessaging::DestroyIcon(self.0) {
                tracing::warn!("DestroyIcon: {e}");
            }
        }
    }
}
pub(crate) struct OwnedRegistryKey(pub windows::Win32::System::Registry::HKEY);
impl Drop for OwnedRegistryKey {
    fn drop(&mut self) {
        unsafe {
            if let Err(e) = windows::Win32::System::Registry::RegCloseKey(self.0).ok() {
                tracing::warn!("RegCloseKey: {e}");
            }
        }
    }
}
/// Owned by the thread that uses WinRT; deliberately not Send/Sync.
pub struct Apartment(std::marker::PhantomData<std::rc::Rc<()>>);
impl Apartment {
    pub fn new() -> Result<Self, BackendError> {
        unsafe {
            RoInitialize(RO_INIT_MULTITHREADED)
                .map_err(|e| super::native_error("RoInitialize", e))?;
        }
        Ok(Self(std::marker::PhantomData))
    }
}
impl Drop for Apartment {
    fn drop(&mut self) {
        unsafe { RoUninitialize() }
    }
}
pub struct InstanceLock(HANDLE);
impl InstanceLock {
    pub fn acquire() -> Result<Self, BackendError> {
        let name: Vec<u16> = "Local\\TapRelay.GUI.Runtime.v2\0".encode_utf16().collect();
        unsafe {
            let handle = CreateMutexW(None, false, PCWSTR(name.as_ptr()))
                .map_err(|e| super::native_error("CreateMutexW", e))?;
            if GetLastError() == ERROR_ALREADY_EXISTS {
                let _ = CloseHandle(handle);
                return Err(BackendError::Unavailable(
                    "TapRelay is already running.".into(),
                ));
            }
            Ok(Self(handle))
        }
    }
}
impl Drop for InstanceLock {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
