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
