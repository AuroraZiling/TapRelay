use super::Config;
use anyhow::{Context, Result};
use std::{
    io::Write,
    path::{Path, PathBuf},
};

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => {
                let mut c: Self = serde_json::from_slice(&bytes)
                    .context("Invalid configuration; move config.json aside to start fresh")?;
                c.functions
                    .entry(taprelay_core::function::FunctionId::AppToggleListening)
                    .or_default();
                c.functions
                    .entry(taprelay_core::function::FunctionId::AppTogglePassthrough)
                    .or_default();
                for config in c.functions.values_mut() {
                    config
                        .shortcuts
                        .retain(|shortcut| !shortcut.is_standalone_primary_mouse_button());
                }
                if c.schema == 3 {
                    c.schema = 4;
                }
                c.validate()?;
                Ok(c)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        let mut temp =
            tempfile::NamedTempFile::new_in(path.parent().context("Missing data directory")?)?;
        serde_json::to_writer_pretty(&mut temp, self)?;
        temp.write_all(b"\n")?;
        temp.as_file().sync_all()?;
        temp.persist(path)
            .with_context(|| format!("Cannot save {}", path.display()))?;
        Ok(())
    }
}

pub fn path() -> Result<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let dir = taprelay_macos::paths::application_support()?;
        std::fs::create_dir_all(&dir)?;
        Ok(dir.join("config.json"))
    }
    #[cfg(windows)]
    windows_path(
        &std::env::current_exe()?,
        std::env::var_os("APPDATA").as_deref(),
    )
}

#[cfg(windows)]
fn windows_path(executable: &Path, roaming: Option<&std::ffi::OsStr>) -> Result<PathBuf> {
    if crate::installation::is_installed(executable)? {
        let dir = roaming
            .map(Path::new)
            .filter(|path| path.is_absolute())
            .context("Missing roaming application data directory (APPDATA)")?
            .join("TapRelay");
        std::fs::create_dir_all(&dir)?;
        Ok(dir.join("config.json"))
    } else {
        Ok(executable
            .parent()
            .context("Missing executable directory")?
            .join("config.json"))
    }
}
pub fn check_writable(path: &Path) -> Result<()> {
    let dir = path.parent().context("Missing data directory")?;
    let mut probe = tempfile::NamedTempFile::new_in(dir).with_context(|| {
        #[cfg(windows)]
        if std::env::current_exe().is_ok_and(|exe| exe.parent() == Some(dir)) {
            return "Application folder is not writable. Move TapRelay to a writable folder."
                .into();
        }
        format!("Cannot write to the settings folder: {}", dir.display())
    })?;
    probe.write_all(b"TapRelay write check")?;
    probe.as_file().sync_all()?;
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn installed_app_saves_settings_and_logs_outside_the_replaceable_installation() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("Local/TapRelay");
        let current = root.join("current");
        let exe = current.join("TapRelay.exe");
        let roaming = tmp.path().join("Roaming");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(root.join("Update.exe"), []).unwrap();
        std::fs::write(
            current.join("sq.version"),
            "<package><metadata><id>TapRelay</id><version>1.2.3</version><mainExe>TapRelay.exe</mainExe></metadata></package>",
        )
        .unwrap();
        assert!(windows_path(&exe, None).is_err());
        let settings = windows_path(&exe, Some(roaming.as_os_str())).unwrap();
        assert_eq!(settings, roaming.join("TapRelay/config.json"));
        Config::default().save(&settings).unwrap();
        assert_eq!(
            crate::logging::log_directory(&settings).unwrap(),
            roaming.join("TapRelay/logs")
        );
        assert!(!current.join("config.json").exists());
    }

    #[test]
    fn unpackaged_app_keeps_data_beside_executable_without_needing_roaming_appdata() {
        let tmp = tempfile::tempdir().unwrap();
        for folder in ["extracted", "target/debug", "current"] {
            let dir = tmp.path().join(folder);
            std::fs::create_dir_all(&dir).unwrap();
            // An updater belonging to some other program is not a TapRelay installation.
            std::fs::write(tmp.path().join("Update.exe"), []).unwrap();
            let settings = windows_path(&dir.join("TapRelay.exe"), None).unwrap();
            assert_eq!(settings, dir.join("config.json"));
            assert_eq!(
                crate::logging::log_directory(&settings).unwrap(),
                dir.join("logs")
            );
        }
    }
}
