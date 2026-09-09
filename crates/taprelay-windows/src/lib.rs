#[cfg(windows)]
pub mod administrator;
#[cfg(windows)]
pub mod bluetooth;
#[cfg(windows)]
pub mod desktop;
#[cfg(windows)]
pub mod diagnostics;
#[cfg(windows)]
pub mod input;
#[cfg(windows)]
mod native;
#[cfg(windows)]
pub use native::{Apartment, InstanceLock};
#[cfg(windows)]
pub(crate) fn native_error(
    api: &'static str,
    error: windows::core::Error,
) -> taprelay_core::ports::BackendError {
    taprelay_core::ports::BackendError::Native {
        api,
        code: error.code().0,
        context: error.to_string(),
    }
}
