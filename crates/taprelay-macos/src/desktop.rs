//! AppKit integration shares GPUI's application main loop.
mod notifications;
use objc2::{MainThreadMarker, class, msg_send, rc::Retained, runtime::AnyObject};
use objc2_foundation::{NSLocale, NSString};
use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    os::{
        fd::AsRawFd,
        unix::{fs::OpenOptionsExt, net::UnixDatagram},
    },
    sync::{Mutex, OnceLock},
};
pub use taprelay_core::ports::{BluetoothPermission, LoginStatus, NotificationPermission};
use tray_icon::{
    Icon, TrayIcon, TrayIconBuilder,
    menu::{Menu, MenuEvent, MenuItem},
};
static EVENTS: Mutex<VecDeque<DesktopEvent>> = Mutex::new(VecDeque::new());
static ACTIVATION: OnceLock<Mutex<UnixDatagram>> = OnceLock::new();
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DesktopEvent {
    Show,
    Toggle,
    Quit,
    Resume,
    Settings,
    Close,
}
fn activation_path() -> std::io::Result<std::path::PathBuf> {
    Ok(crate::paths::application_support()?.join("activate.sock"))
}
pub struct InstanceLock(File);
impl InstanceLock {
    pub fn acquire() -> std::io::Result<Self> {
        let path = activation_path()?;
        std::fs::create_dir_all(path.parent().unwrap())?;
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(path.with_extension("lock"))?;
        if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(std::io::Error::other("TapRelay is already running"));
        }
        let _ = std::fs::remove_file(&path);
        let socket = UnixDatagram::bind(path)?;
        socket.set_nonblocking(true)?;
        ACTIVATION
            .set(Mutex::new(socket))
            .map_err(|_| std::io::Error::other("Activation listener already exists"))?;
        Ok(Self(file))
    }
}
impl Drop for InstanceLock {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
pub fn activate_existing() -> bool {
    activation_path().ok().is_some_and(|path| {
        UnixDatagram::unbound().is_ok_and(|socket| socket.send_to(b"show", path).is_ok())
    })
}
pub fn events() -> Vec<DesktopEvent> {
    if let Some(socket) = ACTIVATION.get() {
        let mut bytes = [0; 32];
        while socket
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .recv(&mut bytes)
            .is_ok()
        {
            EVENTS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push_back(DesktopEvent::Show);
        }
    }
    for event in MenuEvent::receiver().try_iter() {
        if let Some(event) = match event.id.as_ref() {
            "show" => Some(DesktopEvent::Show),
            "toggle" => Some(DesktopEvent::Toggle),
            "quit" => Some(DesktopEvent::Quit),
            _ => None,
        } {
            EVENTS
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push_back(event);
        }
    }
    EVENTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .drain(..)
        .collect()
}
pub struct Desktop {
    _main_thread: MainThreadMarker,
    notifications: notifications::Notifications,
    tray: TrayIcon,
    show: MenuItem,
    toggle: MenuItem,
    quit: MenuItem,
    observers:
        Vec<Retained<objc2::runtime::ProtocolObject<dyn objc2_foundation::NSObjectProtocol>>>,
}
impl Desktop {
    pub fn new(labels: [String; 4]) -> std::io::Result<Self> {
        let main_thread = MainThreadMarker::new()
            .ok_or_else(|| std::io::Error::other("Desktop requires app main thread"))?;
        // Runtime/input startup happens on a worker; Carbon layout queries must
        // finish here before that worker starts reading the immutable key map.
        crate::input::keyboard::initialize().map_err(std::io::Error::other)?;
        let show = MenuItem::with_id("show", &labels[0], true, None);
        let toggle = MenuItem::with_id("toggle", &labels[1], true, None);
        let quit = MenuItem::with_id("quit", &labels[3], true, None);
        let menu = Menu::with_items(&[&show, &toggle, &quit]).map_err(std::io::Error::other)?;
        let tray = TrayIconBuilder::new()
            .with_tooltip("TapRelay")
            .with_icon(Icon::from_rgba(icon_rgba(1), 32, 32).map_err(std::io::Error::other)?)
            .with_menu(Box::new(menu))
            .build()
            .map_err(std::io::Error::other)?;
        Ok(Self {
            _main_thread: main_thread,
            notifications: notifications::Notifications::new(),
            tray,
            show,
            toggle,
            quit,
            observers: observe_session(),
        })
    }
    pub fn update(&mut self, state: u8, tip: &str, listening: bool, labels: [String; 4]) {
        self.show.set_text(&labels[0]);
        self.toggle.set_text(&labels[if listening { 2 } else { 1 }]);
        self.quit.set_text(&labels[3]);
        let _ = self.tray.set_tooltip(Some(tip));
        if let Ok(icon) = Icon::from_rgba(icon_rgba(state), 32, 32) {
            let _ = self.tray.set_icon(Some(icon));
        }
    }
    pub fn notification_permission(&self) -> NotificationPermission {
        self.notifications.permission()
    }
    pub fn notify(&self, title: &str, body: &str) {
        self.notifications.send(title, body);
    }
}
pub fn icon_rgba(state: u8) -> Vec<u8> {
    let mut rgba = include_bytes!("../../../resources/taprelay-32.rgba").to_vec();
    let dot = match state {
        0 => [46, 180, 112, 255],
        1 => [139, 145, 154, 255],
        2 => [232, 174, 54, 255],
        _ => [230, 79, 85, 255],
    };
    for y in 0i32..32 {
        for x in 0i32..32 {
            if (x - 25).pow(2) + (y - 25).pow(2) <= 25 {
                let i = ((y * 32 + x) * 4) as usize;
                rgba[i..i + 4].copy_from_slice(&dot);
            }
        }
    }
    rgba
}
pub fn open(target: &str) -> std::io::Result<()> {
    let status = std::process::Command::new("/usr/bin/open")
        .arg(target)
        .status()?;
    if status.success() {
        Ok(())
    } else {
        Err(std::io::Error::other("Cannot open system target"))
    }
}
pub fn system_dark() -> bool {
    unsafe {
        let defaults: Retained<AnyObject> = msg_send![class!(NSUserDefaults), standardUserDefaults];
        let style: Option<Retained<NSString>> =
            msg_send![&*defaults,stringForKey:&*NSString::from_str("AppleInterfaceStyle")];
        style.is_some_and(|s| s.to_string() == "Dark")
    }
}
pub fn system_locale_id() -> Option<String> {
    NSLocale::preferredLanguages()
        .firstObject()
        .map(|s| s.to_string())
}
pub fn bluetooth_permission() -> BluetoothPermission {
    use objc2_core_bluetooth::{CBManager, CBManagerAuthorization};
    match unsafe { CBManager::authorization_class() } {
        CBManagerAuthorization::NotDetermined => BluetoothPermission::NotDetermined,
        CBManagerAuthorization::Restricted => BluetoothPermission::Restricted,
        CBManagerAuthorization::Denied => BluetoothPermission::Denied,
        CBManagerAuthorization::AllowedAlways => BluetoothPermission::Allowed,
        _ => BluetoothPermission::Unknown,
    }
}
#[link(name = "ServiceManagement", kind = "framework")]
unsafe extern "C" {}
fn login_service() -> Retained<AnyObject> {
    unsafe { msg_send![class!(SMAppService), mainAppService] }
}
pub fn login_status() -> LoginStatus {
    let service = login_service();
    let status: isize = unsafe { msg_send![&*service, status] };
    match status {
        0 => LoginStatus::Disabled,
        1 => LoginStatus::Enabled,
        2 => LoginStatus::RequiresApproval,
        3 => LoginStatus::NotFound,
        _ => LoginStatus::Unknown,
    }
}
pub fn autostart_enabled() -> bool {
    login_status() == LoginStatus::Enabled
}
pub fn open_login_settings() -> std::io::Result<()> {
    unsafe {
        let _: () = msg_send![class!(SMAppService), openSystemSettingsLoginItems];
    }
    Ok(())
}
pub fn set_autostart(enabled: bool) -> std::io::Result<()> {
    let service = login_service();
    unsafe {
        let mut error: *mut objc2_foundation::NSError = std::ptr::null_mut();
        let ok: bool = if enabled {
            msg_send![&*service,registerAndReturnError:&mut error]
        } else {
            msg_send![&*service,unregisterAndReturnError:&mut error]
        };
        if !ok {
            if let Some(error) = error.as_ref() {
                tracing::warn!(error = %error.localizedDescription(), "Login registration failed");
            }
            return Err(std::io::Error::other(
                "Cannot register login item. Check System Settings → General → Login Items.",
            ));
        }
    }
    // Registration can succeed while approval is pending. Read the actual
    // status instead of treating success as permission to run at login.
    Ok(())
}
pub fn foreground_is_ours() -> bool {
    unsafe {
        let workspace: Retained<AnyObject> = msg_send![class!(NSWorkspace), sharedWorkspace];
        let app: Option<Retained<AnyObject>> = msg_send![&*workspace, frontmostApplication];
        app.is_some_and(|app| {
            let pid: i32 = msg_send![&*app, processIdentifier];
            pid == std::process::id() as i32
        })
    }
}
pub fn foreground_app() {
    if let Some(mtm) = MainThreadMarker::new() {
        let app = objc2_app_kit::NSApplication::sharedApplication(mtm);
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    }
}
pub fn show_error(text: &str) {
    if let Some(mtm) = MainThreadMarker::new() {
        let alert = objc2_app_kit::NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str("TapRelay"));
        alert.setInformativeText(&NSString::from_str(text));
        alert.runModal();
    }
}
pub fn any_input_held() -> bool {
    crate::input::any_input_held()
}
pub fn virtual_key_for_character(ch: char) -> Option<u8> {
    if ch.is_ascii_alphanumeric() {
        Some(ch.to_ascii_uppercase() as u8)
    } else {
        match ch {
            ';' => Some(0xba),
            '=' => Some(0xbb),
            ',' => Some(0xbc),
            '-' => Some(0xbd),
            '.' => Some(0xbe),
            '/' => Some(0xbf),
            '`' => Some(0xc0),
            '[' => Some(0xdb),
            '\\' => Some(0xdc),
            ']' => Some(0xdd),
            '\'' => Some(0xde),
            _ => None,
        }
    }
}

pub fn set_app_window_visible(visible: bool) {
    let Some(main_thread) = MainThreadMarker::new() else {
        return;
    };
    let app = objc2_app_kit::NSApplication::sharedApplication(main_thread);
    for window in &app.windows() {
        if window.title().to_string() == "TapRelay" {
            if visible {
                window.makeKeyAndOrderFront(None);
            } else {
                window.orderOut(None);
            }
        }
    }
}

pub fn request_show() {
    emit(DesktopEvent::Show);
}
pub fn request_quit() {
    emit(DesktopEvent::Quit);
}
pub fn request_settings() {
    emit(DesktopEvent::Settings);
}
pub fn request_close() {
    emit(DesktopEvent::Close);
}
fn emit(event: DesktopEvent) {
    EVENTS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push_back(event);
}
fn observe_session()
-> Vec<Retained<objc2::runtime::ProtocolObject<dyn objc2_foundation::NSObjectProtocol>>> {
    use objc2_app_kit::{
        NSWorkspace, NSWorkspaceDidWakeNotification, NSWorkspaceSessionDidBecomeActiveNotification,
        NSWorkspaceSessionDidResignActiveNotification, NSWorkspaceWillSleepNotification,
    };
    let center = NSWorkspace::sharedWorkspace().notificationCenter();
    let mut tokens = vec![];
    unsafe {
        for (name, active) in [
            (NSWorkspaceWillSleepNotification, false),
            (NSWorkspaceSessionDidResignActiveNotification, false),
            (NSWorkspaceDidWakeNotification, true),
            (NSWorkspaceSessionDidBecomeActiveNotification, true),
        ] {
            let block = block2::RcBlock::new(
                move |_: std::ptr::NonNull<objc2_foundation::NSNotification>| {
                    crate::input::set_session_active(active);
                    if active {
                        emit(DesktopEvent::Resume);
                    }
                },
            );
            tokens.push(center.addObserverForName_object_queue_usingBlock(
                Some(name),
                None,
                None,
                &block,
            ));
        }
    }
    tokens
}
impl Drop for Desktop {
    fn drop(&mut self) {
        let center = objc2_app_kit::NSWorkspace::sharedWorkspace().notificationCenter();
        for token in self.observers.drain(..) {
            unsafe {
                center.removeObserver(AsRef::<AnyObject>::as_ref(&*token));
            }
        }
    }
}
