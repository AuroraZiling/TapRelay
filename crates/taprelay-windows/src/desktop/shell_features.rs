//! Native gaps in tray-icon: balloon notifications and GUID-aware tooltip updates.
use super::{DesktopEvent, emit, truncate_utf16};
use tray_icon::TrayIcon;
use windows::{
    Win32::{Foundation::*, UI::Shell::*},
    core::GUID,
};

pub(super) const TRAY_CALLBACK: u32 = 6002;
const SUBCLASS_ID: usize = 1;

pub(super) struct ShellFeatures {
    // Own a clone so the HWND outlives the subclass even during Desktop teardown.
    tray: TrayIcon,
    guid: GUID,
}

impl ShellFeatures {
    pub fn new(tray: &TrayIcon, guid: u128) -> windows::core::Result<Self> {
        unsafe {
            SetWindowSubclass(HWND(tray.window_handle()), Some(procedure), SUBCLASS_ID, 0).ok()?;
        }
        Ok(Self {
            tray: tray.clone(),
            guid: GUID::from_u128(guid),
        })
    }

    pub fn notify(&self, title: &str, body: &str) {
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: HWND(self.tray.window_handle()),
            guidItem: self.guid,
            uFlags: NIF_GUID | NIF_INFO,
            dwInfoFlags: NIIF_INFO,
            ..Default::default()
        };
        copy_wide(&mut data.szInfoTitle, title);
        copy_wide(&mut data.szInfo, body);
        if !unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.as_bool() {
            tracing::warn!("Tray notification failed");
        }
    }

    pub fn set_tooltip(&self, text: &str) -> windows::core::Result<()> {
        // tray-icon 0.25.1 omits NIF_GUID in set_tooltip, so the shell cannot find
        // our icon. Keep this workaround until upstream applies the GUID there.
        // After Explorer recovery, Desktop's next periodic update restores it.
        let mut data = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: HWND(self.tray.window_handle()),
            guidItem: self.guid,
            uFlags: NIF_GUID | NIF_TIP,
            ..Default::default()
        };
        copy_wide(&mut data.szTip, text);
        unsafe { Shell_NotifyIconW(NIM_MODIFY, &data) }.ok()
    }
}

impl Drop for ShellFeatures {
    fn drop(&mut self) {
        unsafe {
            let _ = RemoveWindowSubclass(
                HWND(self.tray.window_handle()),
                Some(procedure),
                SUBCLASS_ID,
            );
        }
    }
}

unsafe extern "system" fn procedure(
    hwnd: HWND,
    msg: u32,
    wp: WPARAM,
    lp: LPARAM,
    _: usize,
    _: usize,
) -> LRESULT {
    if msg == TRAY_CALLBACK && lp.0 as u32 == NIN_BALLOONUSERCLICK {
        emit(DesktopEvent::Show);
        return LRESULT(0);
    }
    unsafe { DefSubclassProc(hwnd, msg, wp, lp) }
}

fn copy_wide(dst: &mut [u16], value: &str) {
    dst.fill(0);
    let value = truncate_utf16(value, dst.len().saturating_sub(1));
    for (dst, unit) in dst.iter_mut().zip(value.encode_utf16()) {
        *dst = unit;
    }
}
