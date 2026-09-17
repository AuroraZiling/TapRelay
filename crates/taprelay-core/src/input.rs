//! Physical input state, shortcut recording, and the platform-neutral input vocabulary.

use crate::function::{ModifierSet, PrimaryInput, Shortcut};
use serde::{Deserialize, Serialize};
use std::{fmt, time::Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[repr(u8)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Side1,
    Side2,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum InputCode {
    Key(u8),
    Mouse(MouseButton),
}

impl InputCode {
    pub fn index(self) -> usize {
        match self {
            Self::Key(key) => key as usize,
            Self::Mouse(button) => 256 + button as usize,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct InputEvent {
    pub code: InputCode,
    pub down: bool,
    pub captured: Instant,
}

pub fn modifier(key: u8) -> bool {
    matches!(key, 0x10..=0x12 | 0x5b..=0x5c | 0xa0..=0xa5)
}

pub fn key_name(key: u8) -> String {
    match key {
        0x30..=0x39 | 0x41..=0x5a => char::from(key).to_string(),
        0x70..=0x87 => format!("F{}", key - 0x6f),
        0x03 => "Break".into(),
        0x0c => "Clear".into(),
        0x10 => "Shift".into(),
        0x11 => "Ctrl".into(),
        0x12 => "Alt".into(),
        0x13 => "Pause".into(),
        0x14 => "Caps Lock".into(),
        0x15 => "Kana / Hangul".into(),
        0x16 => "IME On".into(),
        0x17 => "Junja".into(),
        0x18 => "Final".into(),
        0x19 => "Hanja / Kanji".into(),
        0x1a => "IME Off".into(),
        0x1c => "Convert".into(),
        0x1d => "Nonconvert".into(),
        0x1e => "Accept".into(),
        0x1f => "Mode Change".into(),
        0xa0 => "Left Shift".into(),
        0xa1 => "Right Shift".into(),
        0xa2 => "Left Ctrl".into(),
        0xa3 => "Right Ctrl".into(),
        0xa4 => "Left Alt".into(),
        0xa5 => "Right Alt".into(),
        0x5b => "Left Win".into(),
        0x5c => "Right Win".into(),
        0x08 => "Backspace".into(),
        0x09 => "Tab".into(),
        0x0d => "Enter".into(),
        0xe0 => "Num Enter".into(),
        0x1b => "Escape".into(),
        0x20 => "Space".into(),
        0x25 => "Left Arrow".into(),
        0x26 => "Up Arrow".into(),
        0x27 => "Right Arrow".into(),
        0x28 => "Down Arrow".into(),
        0x29 => "Select".into(),
        0x2a => "Print".into(),
        0x2b => "Execute".into(),
        0x2c => "Print Screen".into(),
        0x2e => "Delete".into(),
        0x2f => "Help".into(),
        0x21 => "Page Up".into(),
        0x22 => "Page Down".into(),
        0x23 => "End".into(),
        0x24 => "Home".into(),
        0x2d => "Insert".into(),
        0x5d => "Application Menu".into(),
        0x5f => "Sleep".into(),
        0x60..=0x69 => format!("Numpad {}", key - 0x60),
        0x6a => "Numpad *".into(),
        0x6b => "Numpad +".into(),
        0x6c => "Numpad Separator".into(),
        0x6d => "Numpad -".into(),
        0x6e => "Numpad .".into(),
        0x6f => "Numpad /".into(),
        0x90 => "Num Lock".into(),
        0x91 => "Scroll Lock".into(),
        0xa6 => "Browser Back".into(),
        0xa7 => "Browser Forward".into(),
        0xa8 => "Browser Refresh".into(),
        0xa9 => "Browser Stop".into(),
        0xaa => "Browser Search".into(),
        0xab => "Browser Favorites".into(),
        0xac => "Browser Home".into(),
        0xad => "Mute".into(),
        0xae => "Volume Down".into(),
        0xaf => "Volume Up".into(),
        0xb0 => "Next Track".into(),
        0xb1 => "Previous Track".into(),
        0xb2 => "Stop Media".into(),
        0xb3 => "Play / Pause".into(),
        0xb4 => "Mail".into(),
        0xb5 => "Media Player".into(),
        0xb6 => "Application 1".into(),
        0xb7 => "Application 2".into(),
        0xc3 => "Gamepad A".into(),
        0xc4 => "Gamepad B".into(),
        0xc5 => "Gamepad X".into(),
        0xc6 => "Gamepad Y".into(),
        0xc7 => "Gamepad Right Shoulder".into(),
        0xc8 => "Gamepad Left Shoulder".into(),
        0xc9 => "Gamepad Left Trigger".into(),
        0xca => "Gamepad Right Trigger".into(),
        0xcb => "Gamepad D-pad Up".into(),
        0xcc => "Gamepad D-pad Down".into(),
        0xcd => "Gamepad D-pad Left".into(),
        0xce => "Gamepad D-pad Right".into(),
        0xcf => "Gamepad Menu / Start".into(),
        0xd0 => "Gamepad View / Back".into(),
        0xd1 => "Gamepad Left Stick Button".into(),
        0xd2 => "Gamepad Right Stick Button".into(),
        0xd3 => "Gamepad Left Stick Up".into(),
        0xd4 => "Gamepad Left Stick Down".into(),
        0xd5 => "Gamepad Left Stick Right".into(),
        0xd6 => "Gamepad Left Stick Left".into(),
        0xd7 => "Gamepad Right Stick Up".into(),
        0xd8 => "Gamepad Right Stick Down".into(),
        0xd9 => "Gamepad Right Stick Right".into(),
        0xda => "Gamepad Right Stick Left".into(),
        0xba => ";".into(),
        0xbb => "=".into(),
        0xbc => ",".into(),
        0xbd => "-".into(),
        0xbe => ".".into(),
        0xbf => "/".into(),
        0xc0 => "`".into(),
        0xdb => "[".into(),
        0xdc | 0xe2 => "\\".into(),
        0xdd => "]".into(),
        0xde => "'".into(),
        0xdf => "Layout-specific key".into(),
        0xe5 => "IME Process".into(),
        0xe7 => "Unicode Input".into(),
        0xf6 => "Attention".into(),
        0xf7 => "Cursor Select".into(),
        0xf8 => "Extend Selection".into(),
        0xf9 => "Erase End of File".into(),
        0xfa => "Play".into(),
        0xfb => "Zoom".into(),
        0xfd => "PA1".into(),
        0xfe => "Clear".into(),
        _ => format!("Unknown key (0x{key:02X})"),
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Keys([u64; 4]);

impl Keys {
    fn set(&mut self, key: u8, down: bool) -> bool {
        let word = &mut self.0[key as usize / 64];
        let mask = 1 << (key % 64);
        let changed = (*word & mask != 0) != down;
        if down {
            *word |= mask;
        } else {
            *word &= !mask;
        }
        changed
    }

    fn count(self) -> u32 {
        self.0.iter().map(|n| n.count_ones()).sum()
    }

    fn contains(self, key: u8) -> bool {
        self.0[key as usize / 64] & (1 << (key % 64)) != 0
    }

    fn keys(self) -> impl Iterator<Item = u8> {
        (0..=255).filter(move |&key| self.contains(key))
    }
}

#[derive(Default)]
pub struct InputState {
    held: Keys,
    mouse: u8,
}

impl InputState {
    /// Returns false for a repeated key/button edge, which is not a new physical down.
    pub fn update(&mut self, event: InputEvent) -> bool {
        match event.code {
            InputCode::Key(key) => self.held.set(key, event.down),
            InputCode::Mouse(button) => {
                let mask = 1 << button as u8;
                let changed = (self.mouse & mask != 0) != event.down;
                if event.down {
                    self.mouse |= mask;
                } else {
                    self.mouse &= !mask;
                }
                changed
            }
        }
    }

    pub fn is_down(&self, code: InputCode) -> bool {
        match code {
            InputCode::Key(key) => self.held.contains(key),
            InputCode::Mouse(button) => self.mouse & (1 << button as u8) != 0,
        }
    }

    pub fn logical_modifiers(&self) -> ModifierSet {
        ModifierSet::from_keys(self.held.keys())
    }

    pub fn empty(&self) -> bool {
        self.held.count() == 0 && self.mouse == 0
    }

    pub fn description(&self) -> String {
        self.description_with(key_name)
    }

    pub fn description_with(&self, key_name: impl Fn(u8) -> String) -> String {
        let mut labels = self.logical_modifiers().labels();
        labels.extend(self.held.keys().filter(|key| !modifier(*key)).map(key_name));
        for button in [
            MouseButton::Left,
            MouseButton::Right,
            MouseButton::Middle,
            MouseButton::Side1,
            MouseButton::Side2,
        ] {
            if self.mouse & (1 << button as u8) != 0 {
                labels.push(PrimaryInput::mouse(button).label());
            }
        }
        labels.join("+")
    }

    pub fn key_labels(&self) -> Vec<String> {
        let mut labels = self.logical_modifiers().labels();
        labels.extend(self.held.keys().filter(|key| !modifier(*key)).map(key_name));
        labels
    }
}

/// Records exactly one physical primary down and the logical modifiers that
/// existed at that edge. It never constructs a shortcut from a peak held set.
#[derive(Default)]
pub struct Recorder {
    primary: Option<PrimaryInput>,
    modifiers: ModifierSet,
    primary_released: bool,
    saw_input: bool,
    invalid: bool,
    completed_invalid: bool,
}

impl Recorder {
    pub fn observe(&mut self, state: &InputState, event: InputEvent) -> Option<Shortcut> {
        self.saw_input = true;
        if event.down {
            match event.code {
                InputCode::Key(key) if modifier(key) => {}
                InputCode::Key(key) => self.observe_primary(PrimaryInput::keyboard(key), state),
                InputCode::Mouse(button) => {
                    self.observe_primary(PrimaryInput::mouse(button), state)
                }
            }
        } else if self
            .primary
            .as_ref()
            .is_some_and(|primary| primary.code() == event.code)
        {
            self.primary_released = true;
        }
        if !event.down && state.empty() {
            let primary = self.primary.take();
            let modifiers = self.modifiers;
            let invalid = self.invalid || primary.is_none();
            self.modifiers = ModifierSet::empty();
            self.primary_released = false;
            self.saw_input = false;
            self.invalid = false;
            if invalid {
                self.completed_invalid = true;
                return None;
            }
            return Some(Shortcut::new(modifiers, primary.expect("checked above")));
        }
        None
    }

    fn observe_primary(&mut self, primary: PrimaryInput, state: &InputState) {
        if self.primary.is_some() {
            // Repeated down for the same physical primary is not another key.
            if self.primary_released || self.primary.as_ref() != Some(&primary) {
                self.invalid = true;
            }
        } else {
            self.modifiers = state.logical_modifiers();
            self.primary = Some(primary);
        }
    }

    pub fn take_invalid(&mut self) -> bool {
        std::mem::take(&mut self.completed_invalid)
    }

    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

impl fmt::Display for Shortcut {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.display())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(code: InputCode, down: bool) -> InputEvent {
        InputEvent {
            code,
            down,
            captured: Instant::now(),
        }
    }

    #[test]
    fn fallback_key_names_are_readable() {
        for key in [0x14, 0x2c, 0x5d, 0x90, 0xa6, 0xaf, 0xb3, 0xe0] {
            let label = key_name(key);
            assert!(!label.starts_with("VK"));
            assert!(!label.starts_with("Unknown"));
        }
        assert_eq!(key_name(0x07), "Unknown key (0x07)");
    }

    #[test]
    fn input_description_uses_the_platform_key_name() {
        let mut state = InputState::default();
        state.update(event(InputCode::Key(0x41), true));
        assert_eq!(
            state.description_with(|key| format!("Layout key {key:02X}")),
            "Layout key 41"
        );
    }

    #[test]
    fn exact_modifier_set_is_captured_at_primary_down() {
        let mut state = InputState::default();
        let mut recorder = Recorder::default();
        for event in [
            event(InputCode::Key(0xa2), true),
            event(InputCode::Key(0x58), true),
            event(InputCode::Key(0xa1), true),
            event(InputCode::Key(0x58), false),
            event(InputCode::Key(0xa2), false),
            event(InputCode::Key(0xa1), false),
        ] {
            state.update(event);
            if let Some(shortcut) = recorder.observe(&state, event) {
                assert_eq!(
                    shortcut,
                    Shortcut::keyboard(
                        ModifierSet {
                            ctrl: true,
                            ..Default::default()
                        },
                        0x58
                    )
                );
            }
        }
    }

    #[test]
    fn second_primary_and_pure_modifiers_are_invalid() {
        let mut state = InputState::default();
        let mut recorder = Recorder::default();
        for event in [
            event(InputCode::Key(0x41), true),
            event(InputCode::Key(0x42), true),
            event(InputCode::Key(0x42), false),
            event(InputCode::Key(0x41), false),
        ] {
            state.update(event);
            recorder.observe(&state, event);
        }
        assert!(recorder.take_invalid());

        for event in [
            event(InputCode::Key(0xa2), true),
            event(InputCode::Key(0xa2), false),
        ] {
            state.update(event);
            recorder.observe(&state, event);
        }
        assert!(recorder.take_invalid());
    }

    #[test]
    fn mouse_primary_confirms_on_down_and_up_does_not_repeat() {
        let mut state = InputState::default();
        let mut recorder = Recorder::default();
        let down = event(InputCode::Mouse(MouseButton::Side1), true);
        state.update(down);
        assert_eq!(recorder.observe(&state, down), None);
        let up = event(InputCode::Mouse(MouseButton::Side1), false);
        state.update(up);
        assert_eq!(
            recorder.observe(&state, up).unwrap().primary,
            PrimaryInput::mouse(MouseButton::Side1)
        );
    }

    #[test]
    fn repeated_down_does_not_change_state() {
        let mut state = InputState::default();
        assert!(state.update(event(InputCode::Key(0x58), true)));
        assert!(!state.update(event(InputCode::Key(0x58), true)));
        assert!(state.is_down(InputCode::Key(0x58)));
    }

    #[test]
    fn a_new_gesture_after_release_can_be_recorded() {
        let mut state = InputState::default();
        let mut recorder = Recorder::default();
        let first_down = event(InputCode::Key(0x41), true);
        state.update(first_down);
        assert_eq!(recorder.observe(&state, first_down), None);
        let first_up = event(InputCode::Key(0x41), false);
        state.update(first_up);
        assert_eq!(
            recorder.observe(&state, first_up).unwrap().primary,
            PrimaryInput::keyboard(0x41)
        );

        let second_down = event(InputCode::Key(0x42), true);
        state.update(second_down);
        assert_eq!(recorder.observe(&state, second_down), None);
        let second_up = event(InputCode::Key(0x42), false);
        state.update(second_up);
        assert_eq!(
            recorder.observe(&state, second_up).unwrap().primary,
            PrimaryInput::keyboard(0x42)
        );
    }
}
