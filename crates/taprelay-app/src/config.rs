mod storage;

pub use storage::{check_writable, path};

use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use taprelay_core::function::{self, FunctionConfigs};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub schema: u32,
    pub functions: FunctionConfigs,
    #[serde(default)]
    pub custom_mappings: Vec<taprelay_core::mapping::CustomMapping>,
    #[serde(default)]
    pub foreground_app_rules: taprelay_core::foreground_app::ForegroundAppRules,
    pub remembered_device: Option<Device>,
    pub wizard: Wizard,
    pub options: Options,
    pub window: Geometry,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            schema: 4,
            custom_mappings: Vec::new(),
            functions: function::default_configs(),
            foreground_app_rules: Default::default(),
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
#[serde(default, deny_unknown_fields)]
pub struct Options {
    pub sidebar_collapsed: bool,
    pub passthrough_overlay: bool,
    pub passthrough_mouse_percent: u16,
    pub passthrough_mouse_report_rate: u16,
    pub passthrough_reverse_scroll: bool,
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
            sidebar_collapsed: false,
            passthrough_overlay: true,
            passthrough_mouse_percent: 50,
            passthrough_mouse_report_rate: 0,
            passthrough_reverse_scroll: false,
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
    Locale(String),
}
impl Language {
    pub fn locale(&self) -> &'static str {
        match self {
            Self::Locale(id) => crate::i18n::resolve(id).unwrap_or(crate::i18n::locale::SOURCE),
            Self::System => crate::i18n::locale::SOURCE,
        }
    }
    pub fn from_name(name: &str) -> Option<Self> {
        match name {
            "system" => Some(Self::System),
            _ => crate::i18n::resolve(name).map(|id| Self::Locale(id.to_owned())),
        }
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Geometry {
    pub width: f32,
    pub height: f32,
    pub x: Option<i32>,
    pub y: Option<i32>,
    pub maximized: bool,
}
impl Geometry {
    pub const MIN_WIDTH: f32 = 900.;
    pub const MIN_HEIGHT: f32 = 500.;
}
impl Default for Geometry {
    fn default() -> Self {
        Self {
            width: Self::MIN_WIDTH,
            height: Self::MIN_HEIGHT,
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
            taprelay_core::passthrough::MOUSE_REPORT_RATES
                .contains(&self.options.passthrough_mouse_report_rate),
            "Invalid passthrough mouse report rate"
        );
        ensure!(
            (1..=100).contains(&self.options.passthrough_mouse_percent),
            "Passthrough mouse percentage must be between 1 and 100"
        );
        ensure!(
            self.schema == 4,
            "Unsupported configuration format (expected schema 4)"
        );
        ensure!(
            self.foreground_app_rules.valid_bindings(&self.functions),
            "Invalid application group or conflicting shortcut"
        );
        ensure!(
            taprelay_core::mapping::valid(
                &self.functions,
                &self.foreground_app_rules,
                &self.custom_mappings
            ),
            "Invalid custom mapping or conflicting shortcut"
        );
        ensure!(self.wizard.page <= 3, "Invalid wizard page");
        ensure!(
            self.window.width.is_finite() && self.window.height.is_finite(),
            "Invalid window size"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests;
