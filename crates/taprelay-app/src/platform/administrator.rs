#[cfg(windows)]
mod windows {
    use anyhow::Result;
    pub use taprelay_windows::administrator::Status;
    pub fn status() -> Result<Status> {
        Ok(taprelay_windows::administrator::status()?)
    }
    pub fn wait_for_parent(pid: u32) -> Result<()> {
        Ok(taprelay_windows::administrator::wait_for_parent(pid)?)
    }
    pub fn restart() -> Result<bool> {
        let mut args = vec![
            std::ffi::OsString::from("--handoff"),
            std::ffi::OsString::from(std::process::id().to_string()),
        ];
        args.extend(crate::logging::arguments().map(std::ffi::OsString::from));
        Ok(taprelay_windows::administrator::restart(
            &args,
            &std::env::current_dir()?,
        )?)
    }
}
#[cfg(windows)]
pub use windows::*;
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, Default)]
pub struct Status {
    pub elevated: bool,
    pub account_admin: bool,
}
#[cfg(target_os = "macos")]
pub fn status() -> anyhow::Result<Status> {
    Ok(Status::default())
}
#[cfg(target_os = "macos")]
pub fn restart() -> anyhow::Result<bool> {
    anyhow::bail!("macOS uses system privacy permissions, not administrator relaunch")
}
#[cfg(target_os = "macos")]
pub fn wait_for_parent(_: u32) -> anyhow::Result<()> {
    anyhow::bail!("Windows UAC handoff is unavailable on macOS")
}
