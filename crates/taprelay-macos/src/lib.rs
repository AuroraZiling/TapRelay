//! Native macOS adapters
#[cfg(target_os = "macos")]
pub mod bluetooth;

/// System-wide monotonic clock, shared by the UI and isolated BLE process.
/// CLOCK_MONOTONIC_RAW includes sleep and never depends on wall-clock changes.
#[cfg(target_os = "macos")]
pub fn monotonic_millis() -> u64 {
    let mut time = std::mem::MaybeUninit::<libc::timespec>::uninit();
    let result = unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC_RAW, time.as_mut_ptr()) };
    assert_eq!(result, 0, "macOS monotonic clock unavailable");
    let time = unsafe { time.assume_init() };
    (time.tv_sec as u64).saturating_mul(1000) + time.tv_nsec as u64 / 1_000_000
}

pub mod keys;
#[cfg(target_os = "macos")]
pub mod paths;

#[cfg(target_os = "macos")]
pub mod desktop;
#[cfg(target_os = "macos")]
pub mod foreground_apps;
#[cfg(target_os = "macos")]
pub mod input;
#[cfg(target_os = "macos")]
pub mod overlay;
#[cfg(target_os = "macos")]
pub use desktop::InstanceLock;
