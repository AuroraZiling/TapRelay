use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use taprelay_core::function::{self, FunctionConfigs};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema: u32,
    pub functions: FunctionConfigs,
    pub remembered_device: Option<Device>,
    pub wizard: Wizard,
    pub options: Options,
    pub window: Geometry,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            schema: 3,
            functions: function::default_configs(),
            remembered_device: None,
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
impl Theme {
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "system" => Some(Self::System),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    System,
    Chinese,
    English,
    Locale(String),
}
impl Language {
    pub fn locale(&self) -> &'static str {
        match self {
            Self::Chinese => "zh-cn",
            Self::English => "en",
            Self::Locale(id) => crate::i18n::resolve(id).unwrap_or(crate::i18n::locale::SOURCE),
            Self::System => crate::i18n::locale::SOURCE,
        }
    }
    pub fn from_name(name: &str) -> Option<Self> {
        if let Some(id) = crate::i18n::resolve(name) {
            return match id {
                "en" => Some(Self::English),
                "zh-cn" => Some(Self::Chinese),
                _ => Some(Self::Locale(id.to_owned())),
            };
        }
        match name {
            "system" => Some(Self::System),
            "chinese" => Some(Self::Chinese),
            "english" => Some(Self::English),
            _ => None,
        }
    }
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
    pub identity: Vec<String>,
    pub aliases: Vec<String>,
}
impl Device {
    pub fn from_target(target: &taprelay_core::state::Target) -> Self {
        Self {
            id: target.id.clone(),
            name: target.name.clone(),
            identity: target.identity.clone(),
            aliases: target.aliases.clone(),
        }
    }

    pub fn as_target(&self) -> taprelay_core::state::Target {
        taprelay_core::state::Target {
            id: self.id.clone(),
            name: self.name.clone(),
            identity: self.identity.clone(),
            aliases: self.aliases.clone(),
            ..Default::default()
        }
    }
}
impl Config {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.schema == 3,
            "Unsupported configuration format (expected schema 3)"
        );
        ensure!(
            function::valid_configs(&self.functions),
            "Invalid or duplicate shortcut"
        );
        ensure!(self.wizard.page <= 3, "Invalid wizard page");
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
                c.functions
                    .entry(taprelay_core::function::FunctionId::AppToggleListening)
                    .or_default();
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
    fn remembered_device_defaults_empty_and_round_trips() {
        let mut json = serde_json::to_value(Config::default()).unwrap();
        assert!(json["remembered_device"].is_null());
        json["remembered_device"] = serde_json::json!({
            "id": "gatt-endpoint",
            "name": "Tablet",
            "identity": ["container:tablet"],
            "aliases": ["classic-endpoint"]
        });
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        let config = Config::load(&path).unwrap();
        assert_eq!(
            config.remembered_device.as_ref().unwrap().id,
            "gatt-endpoint"
        );
        config.save(&path).unwrap();
        assert_eq!(
            Config::load(&path).unwrap().remembered_device,
            config.remembered_device
        );
    }
    #[test]
    fn drafts_and_empty_functions_persist_without_verification() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.json");
        let mut c = Config::default();
        c.wizard.page = 1;
        c.save(&p).unwrap();
        assert_eq!(Config::load(&p).unwrap().wizard.page, 1);
        assert!(
            Config::load(&p)
                .unwrap()
                .functions
                .values()
                .all(|function| function.shortcuts.is_empty() && !function.enabled)
        );
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
    fn invalid_config_is_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.json");
        std::fs::write(&p, "{\"verified\":true}").unwrap();
        assert!(Config::load(&p).is_err());
        assert_eq!(std::fs::read_to_string(p).unwrap(), "{\"verified\":true}");
    }

    #[test]
    fn theme_and_language_names_match_the_persisted_config() {
        // The settings page sends these names, so they must stay identical to
        // what serde writes; otherwise a click would change nothing on reload.
        for (name, theme) in [
            ("system", Theme::System),
            ("light", Theme::Light),
            ("dark", Theme::Dark),
        ] {
            assert_eq!(Theme::from_name(name), Some(theme));
            assert_eq!(
                serde_json::to_value(theme).unwrap(),
                serde_json::json!(name)
            );
        }
        for (name, language) in [
            ("system", Language::System),
            ("chinese", Language::Chinese),
            ("english", Language::English),
        ] {
            assert_eq!(Language::from_name(name), Some(language.clone()));
            assert_eq!(
                serde_json::to_value(&language).unwrap(),
                serde_json::json!(name)
            );
        }
        assert_eq!(Theme::from_name("Dark"), None);
        assert_eq!(Language::from_name(""), None);
    }

    #[test]
    fn locale_identifiers_select_the_matching_catalog() {
        assert_eq!(Language::from_name("zh-CN"), Some(Language::Chinese));
        assert_eq!(Language::from_name("en"), Some(Language::English));
        assert_eq!(Language::from_name("de"), None);
    }

    #[test]
    fn unsupported_schema_and_unknown_function_are_rejected_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let unsupported_schema = dir.path().join("unsupported-schema.json");
        let mut invalid = serde_json::to_value(Config::default()).unwrap();
        invalid["schema"] = serde_json::json!(4);
        let invalid_bytes = serde_json::to_vec(&invalid).unwrap();
        std::fs::write(&unsupported_schema, &invalid_bytes).unwrap();
        assert!(Config::load(&unsupported_schema).is_err());
        assert_eq!(std::fs::read(&unsupported_schema).unwrap(), invalid_bytes);

        let unknown = dir.path().join("unknown-function.json");
        let mut invalid = serde_json::to_value(Config::default()).unwrap();
        invalid["functions"] = serde_json::json!({
            "media.not-a-function": {
                "enabled": true,
                "shortcuts": []
            }
        });
        let invalid_bytes = serde_json::to_vec(&invalid).unwrap();
        std::fs::write(&unknown, &invalid_bytes).unwrap();
        assert!(Config::load(&unknown).is_err());
        assert_eq!(std::fs::read(&unknown).unwrap(), invalid_bytes);
    }
    #[test]
    fn legacy_config_only_gains_the_app_function_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut original = Config::default();
        original
            .functions
            .get_mut(&taprelay_core::function::FunctionId::MediaMute)
            .unwrap()
            .enabled = true;
        let mut json = serde_json::to_value(&original).unwrap();
        json["functions"]
            .as_object_mut()
            .unwrap()
            .remove("app.toggle-listening");
        let bytes = serde_json::to_vec(&json).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let loaded = Config::load(&path).unwrap();
        assert_eq!(loaded.functions, original.functions);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        loaded.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap().functions, original.functions);
        json["functions"]
            .as_object_mut()
            .unwrap()
            .remove("media.mute");
        std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        assert!(Config::load(&path).is_err());
    }
}
