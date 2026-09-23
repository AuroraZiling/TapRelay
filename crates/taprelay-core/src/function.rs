use crate::{
    command::MediaCommand,
    input::{InputCode, MouseButton, modifier},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CategoryId {
    Media,
    App,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppCommand {
    ToggleListening,
    TogglePassthrough,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FunctionAction {
    Media(MediaCommand),
    App(AppCommand),
}

impl FunctionAction {
    pub const fn name_key(self) -> &'static str {
        match self {
            Self::Media(command) => command.name_key(),
            Self::App(AppCommand::ToggleListening) => "function.app.togglelistening",
            Self::App(AppCommand::TogglePassthrough) => "function.app.togglepassthrough",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FunctionDefinition {
    pub id: FunctionId,
    pub category: CategoryId,
    /// Emitted when the shortcut is tapped, or immediately when the function
    /// has no hold gesture.
    pub tap_action: FunctionAction,
    /// Emitted once the shortcut stays held past
    /// [`crate::input_router::HOLD_THRESHOLD`]. `None` makes the function a
    /// plain tap that never waits for the release edge.
    pub hold_action: Option<FunctionAction>,
    pub name_key: &'static str,
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
    #[serde(rename = "app.toggle-listening")]
    AppToggleListening,
    #[serde(rename = "app.toggle-passthrough")]
    AppTogglePassthrough,
}

impl FunctionId {
    pub const fn stable_id(self) -> &'static str {
        match self {
            Self::MediaPlayPause => "media.play-pause",
            Self::MediaPrevious => "media.previous",
            Self::MediaNext => "media.next",
            Self::MediaMute => "media.mute",
            Self::AppToggleListening => "app.toggle-listening",
            Self::AppTogglePassthrough => "app.toggle-passthrough",
        }
    }

    pub fn from_stable_id(value: &str) -> Option<Self> {
        Some(match value {
            "media.play-pause" => Self::MediaPlayPause,
            "media.previous" => Self::MediaPrevious,
            "media.next" => Self::MediaNext,
            "media.mute" => Self::MediaMute,
            "app.toggle-listening" => Self::AppToggleListening,
            "app.toggle-passthrough" => Self::AppTogglePassthrough,
            _ => return None,
        })
    }
}

pub const FUNCTION_CATALOG: [FunctionDefinition; 6] = [
    FunctionDefinition {
        id: FunctionId::MediaPlayPause,
        category: CategoryId::Media,
        tap_action: FunctionAction::Media(MediaCommand::PlayPause),
        hold_action: None,
        name_key: "function.media.playpause",
    },
    FunctionDefinition {
        id: FunctionId::MediaPrevious,
        category: CategoryId::Media,
        tap_action: FunctionAction::Media(MediaCommand::Previous),
        hold_action: Some(FunctionAction::Media(MediaCommand::Rewind)),
        name_key: "function.media.previous",
    },
    FunctionDefinition {
        id: FunctionId::MediaNext,
        category: CategoryId::Media,
        tap_action: FunctionAction::Media(MediaCommand::Next),
        hold_action: Some(FunctionAction::Media(MediaCommand::FastForward)),
        name_key: "function.media.next",
    },
    FunctionDefinition {
        id: FunctionId::MediaMute,
        category: CategoryId::Media,
        tap_action: FunctionAction::Media(MediaCommand::Mute),
        hold_action: None,
        name_key: "function.media.mute",
    },
    FunctionDefinition {
        id: FunctionId::AppToggleListening,
        category: CategoryId::App,
        tap_action: FunctionAction::App(AppCommand::ToggleListening),
        hold_action: None,
        name_key: "function.app.togglelistening",
    },
    FunctionDefinition {
        id: FunctionId::AppTogglePassthrough,
        category: CategoryId::App,
        tap_action: FunctionAction::App(AppCommand::TogglePassthrough),
        hold_action: None,
        name_key: "function.app.togglepassthrough",
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
    pub ctrl: bool,
    pub shift: bool,
    pub alt: bool,
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
        self.label_with(crate::input::key_name)
    }

    pub fn label_with(&self, key_name: impl FnOnce(u8) -> String) -> String {
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
        self.primary.valid() && !self.is_standalone_primary_mouse_button()
    }

    pub fn is_standalone_primary_mouse_button(&self) -> bool {
        self.modifiers == ModifierSet::empty()
            && matches!(
                self.primary,
                PrimaryInput::Mouse {
                    button: MouseButton::Left | MouseButton::Right
                }
            )
    }

    pub fn primary_code(&self) -> InputCode {
        self.primary.code()
    }

    pub fn key_labels(&self) -> Vec<String> {
        self.key_labels_with(crate::input::key_name)
    }

    pub fn key_labels_with(&self, key_name: impl FnOnce(u8) -> String) -> Vec<String> {
        let mut labels = self.modifiers.labels();
        labels.push(self.primary.label_with(key_name));
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
    pub enabled: bool,
    pub shortcuts: Vec<Shortcut>,
}

pub type FunctionConfigs = BTreeMap<FunctionId, FunctionConfig>;

pub fn default_configs() -> FunctionConfigs {
    function_ids()
        .map(|id| (id, FunctionConfig::default()))
        .collect()
}

pub fn valid_configs(configs: &FunctionConfigs) -> bool {
    configs.len() == FUNCTION_CATALOG.len() && crate::binding::valid(configs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standalone_primary_mouse_buttons_are_rejected() {
        for button in [MouseButton::Left, MouseButton::Right] {
            assert!(!Shortcut::mouse(ModifierSet::empty(), button).valid());
            assert!(Shortcut::from_physical(&[], InputCode::Mouse(button)).is_none());
            for key in [0x11, 0x10, 0x12, 0x5b] {
                assert!(Shortcut::from_physical(&[key], InputCode::Mouse(button)).is_some());
            }
        }
        for button in [MouseButton::Middle, MouseButton::Side1, MouseButton::Side2] {
            assert!(Shortcut::mouse(ModifierSet::empty(), button).valid());
        }
    }

    #[test]
    fn skip_functions_pair_a_tap_with_a_held_seek() {
        let previous = function_definition(FunctionId::MediaPrevious);
        assert_eq!(
            previous.tap_action,
            FunctionAction::Media(MediaCommand::Previous)
        );
        assert_eq!(
            previous.hold_action,
            Some(FunctionAction::Media(MediaCommand::Rewind))
        );
        let next = function_definition(FunctionId::MediaNext);
        assert_eq!(next.tap_action, FunctionAction::Media(MediaCommand::Next));
        assert_eq!(
            next.hold_action,
            Some(FunctionAction::Media(MediaCommand::FastForward))
        );
        // Everything else stays a plain tap that never waits for a release.
        for id in [FunctionId::MediaPlayPause, FunctionId::MediaMute] {
            assert_eq!(function_definition(id).hold_action, None);
        }
    }

    #[test]
    fn function_ids_parse_from_their_persisted_names() {
        for id in function_ids() {
            assert_eq!(FunctionId::from_stable_id(id.stable_id()), Some(id));
        }
        assert_eq!(FunctionId::from_stable_id("media.unknown"), None);
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

    #[test]
    fn every_current_function_must_have_a_config() {
        let mut configs = default_configs();
        configs.remove(&FunctionId::MediaMute);
        assert!(!valid_configs(&configs));
    }
    #[test]
    fn app_shortcuts_conflict_with_media_even_when_disabled() {
        let mut configs = default_configs();
        let shortcut = Shortcut::keyboard(ModifierSet::empty(), 0x78);
        configs
            .get_mut(&FunctionId::AppToggleListening)
            .unwrap()
            .shortcuts
            .push(shortcut.clone());
        configs
            .get_mut(&FunctionId::MediaMute)
            .unwrap()
            .shortcuts
            .push(shortcut);
        assert!(!valid_configs(&configs));
        assert!(!crate::binding::valid(&configs));
    }
}
