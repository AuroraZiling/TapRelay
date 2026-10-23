#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("Tap expired or belongs to a previous connection/input session")]
    Stale,
    #[error("Bluetooth delivery timed out")]
    TimedOut,
    #[error("Release failed: {release}; press error: {press:?}")]
    Release {
        press: Option<Box<BackendError>>,
        release: Box<BackendError>,
    },
    #[error("Unsupported platform: {0}")]
    Unsupported(&'static str),
    #[error("Unavailable: {0}")]
    Unavailable(String),
    #[error("{api}: {code:#010x}: {context}")]
    Native {
        api: &'static str,
        code: i32,
        context: String,
    },
}
/// Plain desktop capability values shared by native adapters and presentation.
/// Reading or changing OS authorization remains the platform adapter's job.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct InputPermissions {
    pub accessibility: bool,
    pub monitoring: bool,
    pub posting: bool,
}
impl InputPermissions {
    pub fn ready(self) -> bool {
        self.accessibility && self.monitoring && self.posting
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BluetoothPermission {
    #[default]
    Unknown,
    NotDetermined,
    Restricted,
    Denied,
    Allowed,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum NotificationPermission {
    #[default]
    Unknown,
    NotDetermined,
    Denied,
    Allowed,
    Quiet,
    Unavailable,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LoginStatus {
    Disabled,
    Enabled,
    RequiresApproval,
    NotFound,
    #[default]
    Unknown,
}
