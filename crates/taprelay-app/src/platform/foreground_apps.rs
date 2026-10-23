//! Presentation metadata adapters. Native discovery and image decoding stay in
//! their platform crates; the GUI uses one cache contract on both platforms.
pub use super::native::foreground_apps::IconPixels;
#[cfg(target_os = "macos")]
pub use super::native::foreground_apps::{RunningApp, identity_for_selection, metadata, running};

#[cfg(target_os = "macos")]
pub const CACHE_MAX_AGE: Option<std::time::Duration> = Some(std::time::Duration::from_secs(60));
#[cfg(windows)]
pub const CACHE_MAX_AGE: Option<std::time::Duration> = None;

#[cfg(windows)]
#[derive(Clone, Debug)]
pub struct RunningApp {
    pub identity: String,
    pub name: Option<String>,
    pub path: Option<std::path::PathBuf>,
}

#[cfg(windows)]
pub fn running() -> anyhow::Result<Vec<RunningApp>> {
    Ok(super::native::foreground_apps::running()?
        .into_iter()
        .map(|identity| RunningApp {
            identity,
            name: None,
            path: None,
        })
        .collect())
}

#[cfg(windows)]
pub fn identity_for_selection(value: &str) -> std::io::Result<String> {
    Ok(value.to_owned())
}

#[cfg(windows)]
pub struct Metadata {
    pub path: Option<std::path::PathBuf>,
    pub name: Option<String>,
    pub missing: bool,
}

#[cfg(windows)]
pub fn metadata(id: &str, _: Option<&std::path::Path>, _: Option<&str>) -> Metadata {
    Metadata {
        path: Some(id.into()),
        name: super::native::foreground_apps::executable_name(id),
        missing: !std::path::Path::new(id).is_file(),
    }
}

#[cfg(windows)]
impl Metadata {
    pub fn icon(&self) -> Option<IconPixels> {
        super::native::foreground_apps::executable_icon(self.path.as_ref()?.to_str()?)
    }
}
