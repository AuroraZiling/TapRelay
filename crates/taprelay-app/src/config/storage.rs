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
    // Installer mode disabled: installed Windows builds use %LOCALAPPDATA%/TapRelay/config.json.
    Ok(std::env::current_exe()?
        .parent()
        .context("Missing executable directory")?
        .join("config.json"))
}
pub fn check_writable(path: &Path) -> Result<()> {
    let mut probe =
        tempfile::NamedTempFile::new_in(path.parent().context("Missing data directory")?)
            .context("Application folder is not writable. Move TapRelay to a writable folder.")?;
    probe.write_all(b"TapRelay write check")?;
    probe.as_file().sync_all()?;
    Ok(())
}
