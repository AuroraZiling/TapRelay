use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use taprelay_core::binding::{self, Binding};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema: u32,
    pub bindings: Vec<Binding>,
    /// Legacy receiver record. Device selection is session-only and is never persisted.
    #[serde(default, skip_serializing)]
    pub device: Option<Device>,
    pub wizard: Wizard,
    pub options: Options,
    pub window: Geometry,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            schema: 2,
            bindings: vec![],
            device: None,
            wizard: Wizard::default(),
            options: Options::default(),
            window: Geometry::default(),
        }
    }
}
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wizard {
    pub dismissed: bool,
    pub page: u8,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Options {
    pub always_admin: bool,
    pub auto_listen: bool,
    pub autostart: bool,
    pub start_hidden: bool,
    pub close_to_tray: bool,
    pub close_hint_seen: bool,
    pub notifications: bool,
    #[serde(default)]
    pub connection_wait_warning: bool,
    pub theme: Theme,
    pub language: Language,
}
impl Default for Options {
    fn default() -> Self {
        Self {
            always_admin: true,
            auto_listen: true,
            autostart: false,
            start_hidden: false,
            close_to_tray: true,
            close_hint_seen: false,
            notifications: true,
            connection_wait_warning: false,
            theme: Theme::System,
            language: Language::System,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    System,
    Light,
    Dark,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    System,
    Chinese,
    English,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Geometry {
    pub width: f32,
    pub height: f32,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub maximized: bool,
}
impl Default for Geometry {
    fn default() -> Self {
        Self {
            width: 900.,
            height: 500.,
            x: None,
            y: None,
            maximized: false,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Device {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub identity: Vec<String>,
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Accepted only so schema-2 configurations written by older builds can load.
    /// Compatibility is no longer a persisted or runtime decision.
    #[serde(default, rename = "verified", skip_serializing)]
    pub legacy_verified: bool,
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(self.schema == 2, "Unsupported configuration format");
        ensure!(
            binding::valid(&self.bindings),
            "Invalid or duplicate shortcut"
        );
        ensure!(self.wizard.page <= 2, "Invalid wizard page");
        ensure!(
            self.window.width.is_finite() && self.window.height.is_finite(),
            "Invalid window size"
        );
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => {
                let mut c: Self = serde_json::from_slice(&bytes)
                    .context("Invalid configuration; move config.json aside to start fresh")?;
                // Drop the legacy record as soon as it is read. Saving any later
                // configuration change also removes it from the file.
                c.device = None;
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
#[derive(Default)]
pub struct SaveQueue {
    due: Option<Instant>,
}
impl SaveQueue {
    pub fn changed(&mut self) {
        self.due = Some(Instant::now() + Duration::from_millis(350));
    }
    pub fn flush(&mut self, config: &Config, path: &Path, force: bool) -> Result<()> {
        if self.due.is_some_and(|d| force || Instant::now() >= d) {
            self.due = None;
            config.save(path)?;
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_receiver_record_is_ignored_on_load() {
        let mut json = serde_json::to_value(Config::default()).unwrap();
        json["device"] = serde_json::json!({ "id": "old-endpoint", "name": "Tablet" });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        let config = Config::load(&path).unwrap();
        assert!(config.device.is_none());
    }
    #[test]
    fn legacy_receiver_record_is_not_written_back() {
        let mut json = serde_json::to_value(Config::default()).unwrap();
        json["device"] = serde_json::json!({
            "id": "old-endpoint",
            "name": "Tablet",
            "verified": true
        });
        let config: Config = serde_json::from_value(json).unwrap();
        assert!(config.device.is_some());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        config.save(&path).unwrap();
        assert!(!std::fs::read_to_string(path).unwrap().contains("device"));
    }
    #[test]
    fn wait_warning_defaults_off_for_existing_configs_and_persists_opt_in() {
        let mut json = serde_json::to_value(Config::default()).unwrap();
        json["options"]
            .as_object_mut()
            .unwrap()
            .remove("connection_wait_warning");
        let mut config: Config = serde_json::from_value(json).unwrap();
        assert!(!config.options.connection_wait_warning);
        config.options.connection_wait_warning = true;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        config.save(&path).unwrap();
        assert!(Config::load(&path).unwrap().options.connection_wait_warning);
    }
    #[test]
    fn drafts_and_empty_bindings_persist_without_verification() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.json");
        let mut c = Config::default();
        c.wizard.page = 1;
        c.save(&p).unwrap();
        assert_eq!(Config::load(&p).unwrap().wizard.page, 1);
        assert!(Config::load(&p).unwrap().bindings.is_empty());
    }
    #[test]
    fn rapid_edits_and_forced_flush_preserve_last_value() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.json");
        let mut c = Config::default();
        let mut q = SaveQueue::default();
        q.changed();
        c.options.start_hidden = true;
        q.changed();
        q.flush(&c, &p, false).unwrap();
        assert!(!p.exists());
        q.flush(&c, &p, true).unwrap();
        assert!(Config::load(&p).unwrap().options.start_hidden);
    }
    #[test]
    fn old_format_is_not_migrated_or_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.json");
        std::fs::write(&p, "{\"verified\":true}").unwrap();
        assert!(Config::load(&p).is_err());
        assert_eq!(std::fs::read_to_string(p).unwrap(), "{\"verified\":true}");
    }
}
