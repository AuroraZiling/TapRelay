//! Physical input state and release-to-confirm recording. Fixed bitsets avoid allocations in matching.
use serde::{Deserialize, Serialize};
use std::{fmt, time::Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Side1,
    Side2,
}
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum Trigger {
    Keyboard {
        keys: Vec<u8>,
    },
    Mouse {
        button: MouseButton,
    },
    Mixed {
        modifiers: Vec<u8>,
        button: MouseButton,
    },
}
impl Trigger {
    pub fn valid(&self) -> bool {
        match self {
            Self::Mouse { .. } => true,
            Self::Mixed { modifiers, .. } => {
                !modifiers.is_empty()
                    && modifiers.iter().all(|&k| modifier(k))
                    && modifiers.windows(2).all(|w| w[0] < w[1])
            }
            Self::Keyboard { keys } => {
                !keys.is_empty()
                    && keys.windows(2).all(|w| w[0] < w[1])
                    && keys.iter().all(|&k| (8..=254).contains(&k))
                    && keys.iter().any(|&k| !modifier(k))
            }
        }
    }
    pub fn specificity(&self) -> usize {
        match self {
            Self::Keyboard { keys } => keys.len(),
            Self::Mixed { modifiers, .. } => modifiers.len() + 1,
            Self::Mouse { .. } => 1,
        }
    }
    pub fn contains(&self, code: InputCode) -> bool {
        match (self, code) {
            (Self::Keyboard { keys }, InputCode::Key(k)) => keys.contains(&k),
            (Self::Mouse { button } | Self::Mixed { button, .. }, InputCode::Mouse(b)) => {
                *button == b
            }
            _ => false,
        }
    }
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
impl fmt::Display for Trigger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Mouse { button } => write!(
                f,
                "mouse.{}",
                match button {
                    MouseButton::Left => "left",
                    MouseButton::Right => "right",
                    MouseButton::Middle => "middle",
                    MouseButton::Side1 => "side1",
                    MouseButton::Side2 => "side2",
                }
            ),
            Self::Mixed { modifiers, button } => write!(
                f,
                "{}+{}",
                modifiers
                    .iter()
                    .map(|&k| key_name(k))
                    .collect::<Vec<_>>()
                    .join("+"),
                Self::Mouse { button: *button }
            ),
            Self::Keyboard { keys } => {
                let names: Vec<_> = keys
                    .iter()
                    .filter(|&&k| modifier(k))
                    .chain(keys.iter().filter(|&&k| !modifier(k)))
                    .map(|&k| key_name(k))
                    .collect();
                write!(f, "{}", names.join("+"))
            }
        }
    }
}
impl Trigger {
    /// Structured key labels; never split display text because a key can contain '+'.
    pub fn key_labels(&self) -> Vec<String> {
        match self {
            Self::Keyboard { keys } => keys
                .iter()
                .filter(|&&key| modifier(key))
                .chain(keys.iter().filter(|&&key| !modifier(key)))
                .map(|&key| key_name(key))
                .collect(),
            Self::Mouse { .. } => vec![self.to_string()],
            Self::Mixed { modifiers, button } => modifiers
                .iter()
                .map(|&key| key_name(key))
                .chain(Some(Self::Mouse { button: *button }.to_string()))
                .collect(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Runtime-only representation: configuration stays human-readable and schema-compatible.
pub struct CompiledTrigger {
    keys: Keys,
    mouse: Option<MouseButton>,
    exact: bool,
    pub specificity: usize,
}
impl CompiledTrigger {
    pub fn new(trigger: &Trigger) -> Self {
        let mut keys = Keys::default();
        let (codes, mouse, exact) = match trigger {
            Trigger::Keyboard { keys } => (keys.as_slice(), None, true),
            Trigger::Mouse { button } => (&[][..], Some(*button), false),
            Trigger::Mixed { modifiers, button } => (modifiers.as_slice(), Some(*button), false),
        };
        for &key in codes {
            keys.set(key, true);
        }
        Self {
            keys,
            mouse,
            exact,
            specificity: trigger.specificity(),
        }
    }
    pub fn matches(&self, state: &InputState) -> bool {
        let keys_match = if self.exact {
            state.held == self.keys
        } else {
            state
                .held
                .0
                .iter()
                .zip(self.keys.0)
                .all(|(held, required)| held & required == required)
        };
        keys_match && self.mouse.is_none_or(|b| state.mouse & (1 << b as u8) != 0)
    }
}
#[derive(Debug, Clone, Copy)]
pub struct InputEvent {
    pub code: InputCode,
    pub down: bool,
    pub captured: Instant,
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
    fn keys(self) -> Vec<u8> {
        (0..=255)
            .filter(|&k| self.0[k as usize / 64] & (1 << (k % 64)) != 0)
            .collect()
    }
}
#[derive(Default)]
pub struct InputState {
    held: Keys,
    mouse: u8,
}
impl InputState {
    /// Returns true only when physical state changes; callers choose down/up semantics.
    pub fn update(&mut self, e: InputEvent) -> bool {
        match e.code {
            InputCode::Key(k) => self.held.set(k, e.down),
            InputCode::Mouse(b) => {
                let mask = 1 << b as u8;
                let changed = (self.mouse & mask != 0) != e.down;
                if e.down {
                    self.mouse |= mask;
                } else {
                    self.mouse &= !mask;
                }
                changed
            }
        }
    }
    pub fn empty(&self) -> bool {
        self.held.count() == 0 && self.mouse == 0
    }
    pub fn description(&self) -> String {
        self.key_labels().join(" + ")
    }
    pub fn key_labels(&self) -> Vec<String> {
        let mut parts: Vec<String> = self.held.keys().into_iter().map(key_name).collect();
        for b in [
            MouseButton::Left,
            MouseButton::Right,
            MouseButton::Middle,
            MouseButton::Side1,
            MouseButton::Side2,
        ] {
            if self.mouse & (1 << b as u8) != 0 {
                parts.push(Trigger::Mouse { button: b }.to_string());
            }
        }
        parts
    }
    pub fn matches(&self, trigger: &Trigger) -> bool {
        match trigger {
            Trigger::Keyboard { keys } => {
                self.held.count() == keys.len() as u32
                    && keys
                        .iter()
                        .all(|&k| self.held.0[k as usize / 64] & (1 << (k % 64)) != 0)
            }
            Trigger::Mouse { button } => self.mouse & (1 << *button as u8) != 0,
            Trigger::Mixed { modifiers, button } => {
                self.mouse & (1 << *button as u8) != 0
                    && modifiers
                        .iter()
                        .all(|&k| self.held.0[k as usize / 64] & (1 << (k % 64)) != 0)
            }
        }
    }
}
/// A single recorder recognizes keyboard, mouse and mixed input automatically.
#[derive(Default)]
pub struct Recorder {
    peak: Keys,
    mouse: Option<MouseButton>,
}
impl Recorder {
    pub fn observe(&mut self, state: &InputState, event: InputEvent) -> Option<Trigger> {
        if state.held.count() > self.peak.count() {
            self.peak = state.held;
        }
        if let InputCode::Mouse(b) = event.code
            && event.down
            && self.mouse.is_none()
        {
            self.mouse = Some(b);
        }
        if !event.down && state.empty() {
            let keys = std::mem::take(&mut self.peak).keys();
            if let Some(button) = self.mouse.take() {
                return Some(if keys.is_empty() {
                    Trigger::Mouse { button }
                } else {
                    Trigger::Mixed {
                        modifiers: keys,
                        button,
                    }
                });
            }
            if !keys.is_empty() {
                return Some(Trigger::Keyboard { keys });
            }
        }
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn event(k: u8, down: bool) -> InputEvent {
        InputEvent {
            code: InputCode::Key(k),
            down,
            captured: Instant::now(),
        }
    }
    #[test]
    fn chord_requires_overlap_and_all_releases() {
        let mut s = InputState::default();
        let mut r = Recorder::default();
        for e in [
            event(0xa2, true),
            event(0x4b, true),
            event(0x4b, true),
            event(0xa2, false),
        ] {
            s.update(e);
            assert!(r.observe(&s, e).is_none());
        }
        let e = event(0x4b, false);
        s.update(e);
        let t = r.observe(&s, e).unwrap();
        assert!(t.valid());
        assert_eq!(t.to_string(), "LCtrl+K");
        for e in [
            event(0xa2, true),
            event(0xa2, false),
            event(0x4b, true),
            event(0x4b, false),
        ] {
            s.update(e);
            if let Some(t) = r.observe(&s, e) {
                assert_eq!(
                    t.valid(),
                    matches!(t, Trigger::Keyboard { ref keys } if keys == &[0x4b])
                );
            }
        }
    }
    #[test]
    fn repeats_extra_keys_and_left_right_modifiers() {
        let mut s = InputState::default();
        let t = Trigger::Keyboard {
            keys: vec![0x4b, 0xa2],
        };
        assert!(s.update(event(0xa2, true)));
        assert!(s.update(event(0x4b, true)));
        assert!(s.matches(&t));
        assert!(!s.update(event(0x4b, true)));
        s.update(event(0xa3, true));
        assert!(!s.matches(&t));
        s.update(event(0xa2, false));
        assert!(!s.matches(&t));
    }
    #[test]
    fn all_mouse_buttons_confirm_on_release() {
        for b in [
            MouseButton::Left,
            MouseButton::Right,
            MouseButton::Middle,
            MouseButton::Side1,
            MouseButton::Side2,
        ] {
            let mut s = InputState::default();
            let mut r = Recorder::default();
            for down in [true, false] {
                let e = InputEvent {
                    code: InputCode::Mouse(b),
                    down,
                    captured: Instant::now(),
                };
                s.update(e);
                assert_eq!(
                    r.observe(&s, e),
                    (!down).then_some(Trigger::Mouse { button: b })
                );
            }
        }
    }
}
