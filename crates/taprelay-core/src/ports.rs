#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("Tap expired or belongs to a previous connection/input session")]
    Stale,
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
