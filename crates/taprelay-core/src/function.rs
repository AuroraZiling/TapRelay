//! The stable function catalog and its persisted, user-editable configuration.
//!
//! The catalog is deliberately static. A function's display metadata,
//! activation semantics, and output operation live here so the UI, recorder,
//! and runtime cannot grow separate lists of special cases.

use crate::{
    command::MediaCommand,
    input::{InputCode, MouseButton, key_name, modifier},
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CategoryId {
    Media,
    Virtual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Activation {
    Press,
    Hold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionAction {
    Media(MediaCommand),
    TogglePassthrough,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FunctionDefinition {
    pub id: FunctionId,
    pub category: CategoryId,
    pub activation: Activation,
    pub action: FunctionAction,
    pub name_key: &'static str,
    pub category_key: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum FunctionId {
    #[serde(rename = "media.play-pause")]
    MediaPlayPause,
    #[serde(rename = "media.previous")]
    MediaPrevious,
    #[serde(rename = "media.next")]
    MediaNext,
    #[serde(rename = "media.mute")]
    MediaMute,
    #[serde(rename = "media.rewind")]
    MediaRewind,
    #[serde(rename = "media.fast-forward")]
    MediaFastForward,
    #[serde(rename = "virtual.passthrough")]
    VirtualPassthrough,
}

impl FunctionId {
    pub const fn stable_id(self) -> &'static str {
        match self {
            Self::MediaPlayPause => "media.play-pause",
            Self::MediaPrevious => "media.previous",
            Self::MediaNext => "media.next",
            Self::MediaMute => "media.mute",
            Self::MediaRewind => "media.rewind",
            Self::MediaFastForward => "media.fast-forward",
            Self::VirtualPassthrough => "virtual.passthrough",
        }
    }

    pub fn from_stable_id(value: &str) -> Option<Self> {
        Some(match value {
            "media.play-pause" => Self::MediaPlayPause,
            "media.previous" => Self::MediaPrevious,
            "media.next" => Self::MediaNext,
            "media.mute" => Self::MediaMute,
            "media.rewind" => Self::MediaRewind,
            "media.fast-forward" => Self::MediaFastForward,
            "virtual.passthrough" => Self::VirtualPassthrough,
            _ => return None,
        })
    }
}

pub const FUNCTION_CATALOG: [FunctionDefinition; 7] = [
    FunctionDefinition {
        id: FunctionId::MediaPlayPause,
        category: CategoryId::Media,
        activation: Activation::Press,
        action: FunctionAction::Media(MediaCommand::PlayPause),
        name_key: "function.media.playpause",
        category_key: "function.category.media",
    },
    FunctionDefinition {
        id: FunctionId::MediaPrevious,
        category: CategoryId::Media,
        activation: Activation::Press,
        action: FunctionAction::Media(MediaCommand::Previous),
        name_key: "function.media.previous",
        category_key: "function.category.media",
    },
    FunctionDefinition {
        id: FunctionId::MediaNext,
        category: CategoryId::Media,
        activation: Activation::Press,
        action: FunctionAction::Media(MediaCommand::Next),
        name_key: "function.media.next",
        category_key: "function.category.media",
    },
    FunctionDefinition {
        id: FunctionId::MediaMute,
        category: CategoryId::Media,
        activation: Activation::Press,
        action: FunctionAction::Media(MediaCommand::Mute),
        name_key: "function.media.mute",
        category_key: "function.category.media",
    },
    FunctionDefinition {
        id: FunctionId::MediaRewind,
        category: CategoryId::Media,
        activation: Activation::Hold,
        action: FunctionAction::Media(MediaCommand::Rewind),
        name_key: "function.media.rewind",
        category_key: "function.category.media",
    },
    FunctionDefinition {
        id: FunctionId::MediaFastForward,
        category: CategoryId::Media,
        activation: Activation::Hold,
        action: FunctionAction::Media(MediaCommand::FastForward),
        name_key: "function.media.fastforward",
        category_key: "function.category.media",
    },
    FunctionDefinition {
        id: FunctionId::VirtualPassthrough,
        category: CategoryId::Virtual,
        activation: Activation::Press,
        action: FunctionAction::TogglePassthrough,
        name_key: "function.virtual.passthrough",
        category_key: "function.category.virtual",
    },
];

pub fn function_definition(id: FunctionId) -> &'static FunctionDefinition {
    FUNCTION_CATALOG
        .iter()
        .find(|definition| definition.id == id)
        .expect("every FunctionId is present in the static catalog")
}

pub fn function_ids() -> impl Iterator<Item = FunctionId> {
    FUNCTION_CATALOG.iter().map(|definition| definition.id)
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModifierSet {
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub alt: bool,
    #[serde(default)]
    pub win: bool,
}

impl ModifierSet {
    pub const fn empty() -> Self {
        Self {
            ctrl: false,
            shift: false,
            alt: false,
            win: false,
        }
    }

    pub fn from_keys(keys: impl IntoIterator<Item = u8>) -> Self {
        let mut result = Self::empty();
        for key in keys {
            match key {
                0x10 | 0xa0 | 0xa1 => result.shift = true,
                0x11 | 0xa2 | 0xa3 => result.ctrl = true,
                0x12 | 0xa4 | 0xa5 => result.alt = true,
                0x5b | 0x5c => result.win = true,
                _ => {}
            }
        }
        result
    }

    pub fn contains_key(self, key: u8) -> bool {
        Self::from_keys([key]).is_subset_of(self)
    }

    pub fn is_subset_of(self, other: Self) -> bool {
        (!self.ctrl || other.ctrl)
            && (!self.shift || other.shift)
            && (!self.alt || other.alt)
            && (!self.win || other.win)
    }

    pub fn labels(self) -> Vec<String> {
        let mut labels = Vec::with_capacity(4);
        if self.ctrl {
            labels.push("Ctrl".into());
        }
        if self.shift {
            labels.push("Shift".into());
        }
        if self.alt {
            labels.push("Alt".into());
        }
        if self.win {
            labels.push("Win".into());
        }
        labels
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum PrimaryInput {
    Keyboard { key: u8 },
    Mouse { button: MouseButton },
}

impl PrimaryInput {
    pub fn keyboard(key: u8) -> Self {
        Self::Keyboard { key }
    }

    pub fn mouse(button: MouseButton) -> Self {
        Self::Mouse { button }
    }

    pub fn code(&self) -> InputCode {
        match self {
            Self::Keyboard { key } => InputCode::Key(*key),
            Self::Mouse { button } => InputCode::Mouse(*button),
        }
    }

    pub fn valid(&self) -> bool {
        match self {
            Self::Keyboard { key } => (8..=254).contains(key) && !modifier(*key),
            Self::Mouse { .. } => true,
        }
    }

    pub fn label(&self) -> String {
        match self {
            Self::Keyboard { key } => key_name(*key),
            Self::Mouse { button } => format!(
                "Mouse {}",
                match button {
                    MouseButton::Left => "Left",
                    MouseButton::Right => "Right",
                    MouseButton::Middle => "Middle",
                    MouseButton::Side1 => "Side 1",
                    MouseButton::Side2 => "Side 2",
                }
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Shortcut {
    #[serde(default)]
    pub modifiers: ModifierSet,
    pub primary: PrimaryInput,
}

impl Shortcut {
    pub fn new(modifiers: ModifierSet, primary: PrimaryInput) -> Self {
        Self { modifiers, primary }
    }

    pub fn keyboard(modifiers: ModifierSet, key: u8) -> Self {
        Self::new(modifiers, PrimaryInput::keyboard(key))
    }

    pub fn mouse(modifiers: ModifierSet, button: MouseButton) -> Self {
        Self::new(modifiers, PrimaryInput::mouse(button))
    }

    pub fn valid(&self) -> bool {
        self.primary.valid()
    }

    pub fn primary_code(&self) -> InputCode {
        self.primary.code()
    }

    pub fn key_labels(&self) -> Vec<String> {
        let mut labels = self.modifiers.labels();
        labels.push(self.primary.label());
        labels
    }

    pub fn display(&self) -> String {
        self.key_labels().join("+")
    }

    pub fn from_physical(modifier_keys: &[u8], primary: InputCode) -> Option<Self> {
        let shortcut = Self::new(
            ModifierSet::from_keys(modifier_keys.iter().copied().filter(|key| modifier(*key))),
            match primary {
                InputCode::Key(key) => PrimaryInput::keyboard(key),
                InputCode::Mouse(button) => PrimaryInput::mouse(button),
            },
        );
        shortcut.valid().then_some(shortcut)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FunctionConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub shortcuts: Vec<Shortcut>,
}

pub type FunctionConfigs = BTreeMap<FunctionId, FunctionConfig>;

pub fn default_configs() -> FunctionConfigs {
    function_ids()
        .map(|id| (id, FunctionConfig::default()))
        .collect()
}

/// Add newly-known functions without changing a user's existing entries.
pub fn complete_configs(configs: &mut FunctionConfigs) {
    for id in function_ids() {
        configs.entry(id).or_default();
    }
}

pub fn valid_configs(configs: &FunctionConfigs) -> bool {
    let mut seen = HashSet::new();
    configs.values().all(|config| {
        config.shortcuts.len() <= 2
            && config
                .shortcuts
                .iter()
                .all(|shortcut| shortcut.valid() && seen.insert(shortcut.clone()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_stable_order_and_all_functions_default_off() {
        assert_eq!(FUNCTION_CATALOG.len(), 7);
        assert_eq!(FUNCTION_CATALOG[0].id, FunctionId::MediaPlayPause);
        assert_eq!(FUNCTION_CATALOG[6].id, FunctionId::VirtualPassthrough);
        assert!(default_configs().values().all(|config| !config.enabled));
    }

    #[test]
    fn modifier_normalization_merges_left_and_right_keys() {
        assert_eq!(
            ModifierSet::from_keys([0xa2, 0xa3]),
            ModifierSet {
                ctrl: true,
                ..Default::default()
            }
        );
        assert_eq!(
            ModifierSet::from_keys([0xa0, 0x10]),
            ModifierSet {
                shift: true,
                ..Default::default()
            }
        );
    }

    #[test]
    fn shortcut_validation_allows_exact_modifier_variants() {
        let x = Shortcut::keyboard(ModifierSet::empty(), 0x58);
        let ctrl_x = Shortcut::keyboard(
            ModifierSet {
                ctrl: true,
                ..Default::default()
            },
            0x58,
        );
        let ctrl_shift_x = Shortcut::keyboard(
            ModifierSet {
                ctrl: true,
                shift: true,
                ..Default::default()
            },
            0x58,
        );
        assert!(x.valid() && ctrl_x.valid() && ctrl_shift_x.valid());
        let mut configs = default_configs();
        configs
            .get_mut(&FunctionId::MediaNext)
            .unwrap()
            .shortcuts
            .push(x);
        configs
            .get_mut(&FunctionId::MediaPrevious)
            .unwrap()
            .shortcuts
            .push(ctrl_x);
        configs
            .get_mut(&FunctionId::MediaMute)
            .unwrap()
            .shortcuts
            .push(ctrl_shift_x);
        assert!(valid_configs(&configs));
    }

    #[test]
    fn duplicates_are_rejected_across_disabled_functions_and_slots() {
        let shortcut = Shortcut::keyboard(ModifierSet::empty(), 0x58);
        let mut configs = default_configs();
        configs
            .get_mut(&FunctionId::MediaNext)
            .unwrap()
            .shortcuts
            .push(shortcut.clone());
        configs
            .get_mut(&FunctionId::MediaPrevious)
            .unwrap()
            .shortcuts
            .push(shortcut);
        assert!(!valid_configs(&configs));
    }
}
