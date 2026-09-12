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
        0x10 => "Shift".into(),
        0x11 => "Ctrl".into(),
        0x12 => "Alt".into(),
        0xa0 => "LShift".into(),
        0xa1 => "RShift".into(),
        0xa2 => "LCtrl".into(),
        0xa3 => "RCtrl".into(),
        0xa4 => "LAlt".into(),
        0xa5 => "RAlt".into(),
        0x5b => "LWin".into(),
        0x5c => "RWin".into(),
        0x08 => "Backspace".into(),
        0x09 => "Tab".into(),
        0x0d => "Enter".into(),
        0xe0 => "Num Enter".into(),
        0x1b => "Esc".into(),
        0x20 => "Space".into(),
        0x25 => "Left".into(),
        0x26 => "Up".into(),
        0x27 => "Right".into(),
        0x28 => "Down".into(),
        0x2e => "Delete".into(),
        0x21 => "Page Up".into(),
        0x22 => "Page Down".into(),
        0x23 => "End".into(),
        0x24 => "Home".into(),
        0x2d => "Insert".into(),
        0x60..=0x69 => format!("Num {}", key - 0x60),
        0x6a => "Num *".into(),
        0x6b => "Num +".into(),
        0x6d => "Num -".into(),
        0x6e => "Num .".into(),
        0x6f => "Num /".into(),
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
        _ => format!("VK{key:02X}"),
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
