use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::{
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use taprelay_core::{
    binding,
    function::{self, FunctionConfigs},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema: u32,
    #[serde(default, deserialize_with = "read_functions")]
    pub functions: FunctionConfigs,
    /// Legacy receiver record. Device selection is session-only and is never persisted.
    #[serde(default, skip_serializing)]
    pub device: Option<Device>,
    /// The last receiver that reached a usable HID session. This is only a
    /// hint for one passive restore check during the next application start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remembered_device: Option<Device>,
    pub wizard: Wizard,
    pub options: Options,
    pub window: Geometry,
}
/// A function id that a tap and its long press used to own separately. The
/// merged function kept the skip id, so an old file names the hold half by an
/// id that no longer exists and must be folded in rather than rejected.
fn folded_function(id: &str) -> Option<function::FunctionId> {
    Some(match id {
        "media.rewind" => function::FunctionId::MediaPrevious,
        "media.fast-forward" => function::FunctionId::MediaNext,
        _ => return None,
    })
}

/// A merged function inherits the hold-only binding it replaced, because that
/// was the same physical key the user already pressed to seek. The skip half
/// wins when both were bound: one shortcut can only carry two slots.
fn fold_into(target: &mut function::FunctionConfig, legacy: function::FunctionConfig) {
    target.enabled |= legacy.enabled;
    for shortcut in legacy.shortcuts {
        if target.shortcuts.len() >= binding::MAX_SHORTCUTS_PER_FUNCTION {
            break;
        }
        if !target.shortcuts.contains(&shortcut) {
            target.shortcuts.push(shortcut);
        }
    }
}

fn read_functions<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<FunctionConfigs, D::Error> {
    let entries =
        std::collections::BTreeMap::<String, serde_json::Value>::deserialize(deserializer)?;
    let mut functions = FunctionConfigs::new();
    // Entries arrive in key order, so a folded id can precede the id it folds
    // into ("media.fast-forward" sorts before "media.next"). Collect both and
    // merge afterwards instead of depending on that order.
    let mut folded = Vec::new();
    for (id, value) in entries {
        // Ignore the removed feature in old files without discarding working
        // media bindings or unrelated preferences. Never write it back.
        if id == "virtual.passthrough" {
            continue;
        }
        let config: function::FunctionConfig =
            serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        match folded_function(&id) {
            Some(function) => folded.push((function, config)),
            None => {
                let function = function::FunctionId::from_stable_id(&id)
                    .ok_or_else(|| serde::de::Error::custom(format!("Unknown function: {id}")))?;
                functions.insert(function, config);
            }
        }
    }
    for (function, legacy) in folded {
        fold_into(functions.entry(function).or_default(), legacy);
    }
    Ok(functions)
}
impl Default for Config {
    fn default() -> Self {
        Self {
            schema: 3,
            functions: function::default_configs(),
            device: None,
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
impl Device {
    pub fn from_target(target: &taprelay_core::state::Target) -> Self {
        Self {
            id: target.id.clone(),
            name: target.name.clone(),
            identity: target.identity.clone(),
            aliases: target.aliases.clone(),
            legacy_verified: false,
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

    pub fn matches(&self, target: &taprelay_core::state::Target) -> bool {
        self.as_target().same_device(target)
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
        ensure!(
            binding::valid(&self.functions),
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
                function::complete_configs(&mut c.functions);
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
    fn removed_function_is_discarded_without_losing_media_bindings() {
        let mut config = Config::default();
        let media = config
            .functions
            .get_mut(&function::FunctionId::MediaNext)
            .unwrap();
        media.enabled = true;
        media.shortcuts = vec![function::Shortcut::keyboard(
            function::ModifierSet::empty(),
            0x70,
        )];
        config.options.notifications = false;
        let mut json = serde_json::to_value(&config).unwrap();
        json["functions"]["virtual.passthrough"] =
            serde_json::json!({"enabled": true, "shortcuts": []});
        let loaded: Config = serde_json::from_value(json).unwrap();
        assert_eq!(loaded.functions, config.functions);
        assert!(!loaded.options.notifications);
        assert!(
            serde_json::to_value(loaded).unwrap()["functions"]
                .get("virtual.passthrough")
                .is_none()
        );
    }
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
    fn remembered_device_defaults_empty_and_round_trips() {
        let mut json = serde_json::to_value(Config::default()).unwrap();
        assert!(json.get("remembered_device").is_none());
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
    fn old_format_is_not_migrated_or_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.json");
        std::fs::write(&p, "{\"verified\":true}").unwrap();
        assert!(Config::load(&p).is_err());
        assert_eq!(std::fs::read_to_string(p).unwrap(), "{\"verified\":true}");
    }

    fn legacy_hold_binding(key: u8) -> serde_json::Value {
        serde_json::json!({
            "enabled": true,
            "shortcuts": [{ "primary": { "kind": "keyboard", "key": key } }]
        })
    }

    #[test]
    fn a_removed_hold_function_folds_into_the_merged_shortcut() {
        let mut config = Config::default();
        let previous = config
            .functions
            .get_mut(&function::FunctionId::MediaPrevious)
            .unwrap();
        previous.shortcuts = vec![function::Shortcut::keyboard(
            function::ModifierSet::empty(),
            0x76,
        )];
        let mut json = serde_json::to_value(&config).unwrap();
        json["functions"]["media.rewind"] = legacy_hold_binding(0x77);

        let loaded: Config = serde_json::from_value(json).unwrap();
        let merged = &loaded.functions[&function::FunctionId::MediaPrevious];
        assert!(merged.enabled, "the hold half was the enabled one");
        assert_eq!(
            merged.shortcuts,
            vec![
                function::Shortcut::keyboard(function::ModifierSet::empty(), 0x76),
                function::Shortcut::keyboard(function::ModifierSet::empty(), 0x77),
            ],
            "the skip shortcut stays first and the seek shortcut is adopted"
        );
        loaded.validate().unwrap();
        assert!(
            serde_json::to_value(&loaded).unwrap()["functions"]
                .get("media.rewind")
                .is_none(),
            "the removed id must not be written back"
        );
    }

    #[test]
    fn a_hold_only_configuration_still_enables_the_merged_function() {
        let mut json = serde_json::to_value(Config::default()).unwrap();
        json["functions"]
            .as_object_mut()
            .unwrap()
            .remove("media.previous");
        json["functions"]["media.rewind"] = legacy_hold_binding(0x77);

        let loaded: Config = serde_json::from_value(json).unwrap();
        let merged = &loaded.functions[&function::FunctionId::MediaPrevious];
        assert!(merged.enabled);
        assert_eq!(
            merged.shortcuts,
            vec![function::Shortcut::keyboard(
                function::ModifierSet::empty(),
                0x77
            )]
        );
        loaded.validate().unwrap();
    }

    #[test]
    fn folding_never_exceeds_the_shortcut_slots_of_one_function() {
        let mut config = Config::default();
        let previous = config
            .functions
            .get_mut(&function::FunctionId::MediaPrevious)
            .unwrap();
        previous.shortcuts = vec![
            function::Shortcut::keyboard(function::ModifierSet::empty(), 0x76),
            function::Shortcut::keyboard(function::ModifierSet::empty(), 0x77),
        ];
        let mut json = serde_json::to_value(&config).unwrap();
        json["functions"]["media.rewind"] = legacy_hold_binding(0x78);

        let loaded: Config = serde_json::from_value(json).unwrap();
        assert_eq!(
            loaded.functions[&function::FunctionId::MediaPrevious]
                .shortcuts
                .len(),
            2,
            "a third shortcut has no slot and must not invalidate the file"
        );
        loaded.validate().unwrap();
    }

    #[test]
    fn schema_two_and_unknown_function_are_rejected_without_writing() {
        let dir = tempfile::tempdir().unwrap();
        let schema_two = dir.path().join("schema-two.json");
        let mut old = serde_json::to_value(Config::default()).unwrap();
        old["schema"] = serde_json::json!(2);
        let old_bytes = serde_json::to_vec(&old).unwrap();
        std::fs::write(&schema_two, &old_bytes).unwrap();
        assert!(Config::load(&schema_two).is_err());
        assert_eq!(std::fs::read(&schema_two).unwrap(), old_bytes);

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
}
