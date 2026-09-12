//! Standard HID-over-GATT report descriptions, state, and pure encoders.
//!
//! GATT report characteristics carry the payload only. The report ID is
//! declared by the Report Reference descriptor and is therefore not prefixed
//! to a notification value.

use crate::{command::MediaCommand, ports::BackendError};
use std::collections::{BTreeMap, BTreeSet};

pub const CONSUMER_REPORT_ID: u8 = 1;
pub const KEYBOARD_REPORT_ID: u8 = 2;
pub const MOUSE_REPORT_ID: u8 = 3;
pub const CONSUMER_REPORT_REFERENCE: [u8; 2] = [CONSUMER_REPORT_ID, 1];
pub const KEYBOARD_REPORT_REFERENCE: [u8; 2] = [KEYBOARD_REPORT_ID, 1];
pub const MOUSE_REPORT_REFERENCE: [u8; 2] = [MOUSE_REPORT_ID, 1];
pub const CONSUMER_REPORT_LENGTH: usize = 12; // six 16-bit usage-array entries
pub const KEYBOARD_REPORT_LENGTH: usize = 8; // modifiers, reserved, six usages
pub const MOUSE_REPORT_LENGTH: usize = 7; // buttons, x/y, vertical/horizontal wheel

/// Consumer Control, Keyboard, and Mouse application collections in one HOGP
/// Report Map. Keyboard is the compatible six-key boot-style layout; it is not
/// advertised as NKRO.
pub const REPORT_MAP: &[u8] = &[
    // Consumer Control, report 1: six 16-bit usage-array slots.
    0x05,
    0x0c,
    0x09,
    0x01,
    0xa1,
    0x01,
    0x85,
    CONSUMER_REPORT_ID,
    0x15,
    0x00,
    0x26,
    0xff,
    0x03,
    0x75,
    0x10,
    0x95,
    0x06,
    // Array selectors are indices into this usage range. Match the logical
    // range so each encoded usage selects itself, including zero = no control.
    // A six-item explicit usage list would interpret 0x00cd as index 205,
    // not Play/Pause, and would incorrectly map a zero release to Play/Pause.
    0x19,
    0x00,
    0x2a,
    0xff,
    0x03,
    0x81,
    0x00,
    0xc0,
    // Keyboard, report 2: modifier byte, reserved byte, six key usages.
    0x05,
    0x01,
    0x09,
    0x06,
    0xa1,
    0x01,
    0x85,
    KEYBOARD_REPORT_ID,
    0x05,
    0x07,
    0x19,
    0xe0,
    0x29,
    0xe7,
    0x15,
    0x00,
    0x25,
    0x01,
    0x75,
    0x01,
    0x95,
    0x08,
    0x81,
    0x02,
    0x95,
    0x01,
    0x75,
    0x08,
    0x15,
    0x00,
    0x25,
    0x65,
    0x81,
    0x03,
    0x95,
    0x06,
    0x75,
    0x08,
    0x15,
    0x00,
    0x25,
    0x65,
    0x19,
    0x00,
    0x29,
    0x65,
    0x81,
    0x00,
    0xc0,
    // Relative Mouse, report 3: five buttons, relative x/y, two wheels.
    0x05,
    0x01,
    0x09,
    0x02,
    0xa1,
    0x01,
    0x85,
    MOUSE_REPORT_ID,
    0x09,
    0x01,
    0xa1,
    0x00,
    0x05,
    0x09,
    0x19,
    0x01,
    0x29,
    0x05,
    0x15,
    0x00,
    0x25,
    0x01,
    0x75,
    0x01,
    0x95,
    0x05,
    0x81,
    0x02,
    0x75,
    0x03,
    0x95,
    0x01,
    0x81,
    0x03,
    0x05,
    0x01,
    0x09,
    0x30,
    0x09,
    0x31,
    0x16,
    0x00,
    0x80,
    0x26,
    0xff,
    0x7f,
    0x75,
    0x10,
    0x95,
    0x02,
    0x81,
    0x06,
    0x09,
    0x38,
    0x15,
    0x81,
    0x25,
    0x7f,
    0x75,
    0x08,
    0x95,
    0x01,
    0x81,
    0x06,
    0xc0,
    // Consumer AC Pan is the standard horizontal-scroll usage.
    0x05,
    0x0c,
    0x0a,
    0x38,
    0x02,
    0x15,
    0x81,
    0x25,
    0x7f,
    0x75,
    0x08,
    0x95,
    0x01,
    0x81,
    0x06,
    0xc0,
];

pub const HID_INFORMATION: [u8; 4] = [0x11, 0x01, 0, 2];
pub const PROTOCOL_MODE: [u8; 1] = [1];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReportKind {
    Consumer,
    Keyboard,
    Mouse,
}

impl ReportKind {
    pub const ALL: [Self; 3] = [Self::Consumer, Self::Keyboard, Self::Mouse];

    pub const fn id(self) -> u8 {
        match self {
            Self::Consumer => CONSUMER_REPORT_ID,
            Self::Keyboard => KEYBOARD_REPORT_ID,
            Self::Mouse => MOUSE_REPORT_ID,
        }
    }

    pub const fn reference(self) -> [u8; 2] {
        match self {
            Self::Consumer => CONSUMER_REPORT_REFERENCE,
            Self::Keyboard => KEYBOARD_REPORT_REFERENCE,
            Self::Mouse => MOUSE_REPORT_REFERENCE,
        }
    }

    pub const fn payload_len(self) -> usize {
        match self {
            Self::Consumer => CONSUMER_REPORT_LENGTH,
            Self::Keyboard => KEYBOARD_REPORT_LENGTH,
            Self::Mouse => MOUSE_REPORT_LENGTH,
        }
    }
}

pub fn neutral(kind: ReportKind) -> Vec<u8> {
    vec![0; kind.payload_len()]
}

pub fn consumer_press(command: MediaCommand) -> Vec<u8> {
    let mut report = vec![0; CONSUMER_REPORT_LENGTH];
    report[0..2].copy_from_slice(&command.usage().to_le_bytes());
    report
}

pub fn consumer_release() -> Vec<u8> {
    neutral(ReportKind::Consumer)
}

/// Windows virtual-key to USB HID keyboard usage conversion for the standard
/// keys that can be observed by the low-level hook. Text is never produced
/// from an input method; the physical usage is forwarded instead.
pub const fn keyboard_usage(vk: u8) -> Option<u8> {
    match vk {
        0x41..=0x5a => Some(0x04 + (vk - 0x41)),
        0x31..=0x39 => Some(0x1e + (vk - 0x31)),
        0x30 => Some(0x27),
        0x70..=0x7b => Some(0x3a + (vk - 0x70)),
        0x7c..=0x87 => Some(0x68 + (vk - 0x7c)),
        0x08 => Some(0x2a),
        0x09 => Some(0x2b),
        0x0c => Some(0x5d),
        0x0d => Some(0x28),
        // The low-level Windows adapter uses a private key value for the
        // extended keypad Enter edge so it remains distinct from main Enter.
        0xe0 => Some(0x58),
        0x1b => Some(0x29),
        0x20 => Some(0x2c),
        0x25 => Some(0x50),
        0x26 => Some(0x52),
        0x27 => Some(0x4f),
        0x28 => Some(0x51),
        0x2d => Some(0x49),
        0x2e => Some(0x4c),
        0x2f => Some(0x75),
        0x21 => Some(0x4b),
        0x22 => Some(0x4e),
        0x23 => Some(0x4d),
        0x24 => Some(0x4a),
        0x60..=0x69 => Some(0x62 + (vk - 0x60)),
        0x6a => Some(0x55),
        0x6b => Some(0x57),
        0x6c => Some(0x85),
        0x6d => Some(0x56),
        0x6e => Some(0x63),
        0x6f => Some(0x54),
        0xba => Some(0x33),
        0xbb => Some(0x2e),
        0xbc => Some(0x36),
        0xbd => Some(0x2d),
        0xbe => Some(0x37),
        0xbf => Some(0x38),
        0xc0 => Some(0x35),
        0xdb => Some(0x2f),
        0xdc | 0xe2 => Some(0x31),
        0xdd => Some(0x30),
        0xde => Some(0x34),
        0x5d => Some(0x65),
        0x90 => Some(0x53),
        0x14 => Some(0x39),
        0x2c => Some(0x46),
        0x91 => Some(0x47),
        0x13 => Some(0x48),
        _ => None,
    }
}

pub const fn keyboard_modifier_bit(vk: u8) -> Option<u8> {
    match vk {
        0xa0 | 0x10 => Some(1 << 1),
        0xa1 => Some(1 << 5),
        0xa2 | 0x11 => Some(1 << 0),
        0xa3 => Some(1 << 4),
        0xa4 => Some(1 << 2),
        0xa5 | 0x12 => Some(1 << 6),
        0x5b => Some(1 << 3),
        0x5c => Some(1 << 7),
        _ => None,
    }
}

/// Windows media virtual-keys that have a standard Consumer-page equivalent.
/// The physical key remains a Consumer report in passthrough mode; it is not
/// translated into text or a keyboard usage.
pub const fn consumer_usage(vk: u8) -> Option<u16> {
    match vk {
        0xad => Some(0x00e2), // volume mute
        0xb0 => Some(0x00b5), // next track
        0xb1 => Some(0x00b6), // previous track
        0xb3 => Some(0x00cd), // play/pause
        _ => None,
    }
}

/// Consumer usages are owned by independent sources (media functions or
/// physical keyboard input). Removing one owner never releases another.
#[derive(Debug, Default, Clone)]
pub struct ConsumerState {
    owners: BTreeMap<u16, BTreeSet<u64>>,
}

impl ConsumerState {
    pub fn press(&mut self, owner: u64, command: MediaCommand) {
        self.press_usage(owner, command.usage());
    }

    pub fn press_usage(&mut self, owner: u64, usage: u16) {
        self.owners.entry(usage).or_default().insert(owner);
    }

    pub fn release(&mut self, owner: u64, command: MediaCommand) {
        self.release_usage(owner, command.usage());
    }

    pub fn release_usage(&mut self, owner: u64, usage: u16) {
        if let Some(owners) = self.owners.get_mut(&usage) {
            owners.remove(&owner);
            if owners.is_empty() {
                self.owners.remove(&usage);
            }
        }
    }

    pub fn clear_owner(&mut self, owner: u64) {
        let usages: Vec<_> = self
            .owners
            .iter()
            .filter_map(|(&usage, owners)| owners.contains(&owner).then_some(usage))
            .collect();
        for usage in usages {
            self.release_usage(owner, usage);
        }
    }

    pub fn report(&self) -> Vec<u8> {
        let mut report = vec![0; CONSUMER_REPORT_LENGTH];
        for (slot, usage) in self
            .owners
            .keys()
            .take(CONSUMER_REPORT_LENGTH / 2)
            .enumerate()
        {
            report[slot * 2..slot * 2 + 2].copy_from_slice(&usage.to_le_bytes());
        }
        report
    }

    pub fn is_empty(&self) -> bool {
        self.owners.is_empty()
    }

    pub fn clear(&mut self) {
        self.owners.clear();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyboardReport {
    pub modifiers: u8,
    pub keys: [u8; 6],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyboardOverflow;

impl KeyboardReport {
    pub const fn neutral() -> Self {
        Self {
            modifiers: 0,
            keys: [0; 6],
        }
    }

    pub fn encode(self) -> [u8; KEYBOARD_REPORT_LENGTH] {
        let mut bytes = [0; KEYBOARD_REPORT_LENGTH];
        bytes[0] = self.modifiers;
        bytes[2..].copy_from_slice(&self.keys);
        bytes
    }
}

#[derive(Debug, Default, Clone)]
pub struct KeyboardState {
    pub modifiers: u8,
    keys: BTreeSet<u8>,
}

impl KeyboardState {
    pub fn set_modifier(&mut self, bit: u8, down: bool) {
        if down {
            self.modifiers |= bit;
        } else {
            self.modifiers &= !bit;
        }
    }

    pub fn set_key(&mut self, usage: u8, down: bool) -> Result<(), KeyboardOverflow> {
        if down {
            self.keys.insert(usage);
        } else {
            self.keys.remove(&usage);
        }
        if self.keys.len() > 6 {
            Err(KeyboardOverflow)
        } else {
            Ok(())
        }
    }

    pub fn report(&self) -> Result<[u8; KEYBOARD_REPORT_LENGTH], KeyboardOverflow> {
        if self.keys.len() > 6 {
            return Err(KeyboardOverflow);
        }
        let mut report = KeyboardReport {
            modifiers: self.modifiers,
            ..KeyboardReport::neutral()
        };
        for (slot, usage) in self.keys.iter().copied().enumerate() {
            report.keys[slot] = usage;
        }
        Ok(report.encode())
    }

    pub fn clear(&mut self) {
        self.modifiers = 0;
        self.keys.clear();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MouseReport {
    pub buttons: u8,
    pub x: i16,
    pub y: i16,
    pub wheel: i8,
    pub horizontal_wheel: i8,
}

impl MouseReport {
    pub const fn neutral() -> Self {
        Self {
            buttons: 0,
            x: 0,
            y: 0,
            wheel: 0,
            horizontal_wheel: 0,
        }
    }

    pub fn encode(self) -> [u8; MOUSE_REPORT_LENGTH] {
        let mut bytes = [0; MOUSE_REPORT_LENGTH];
        bytes[0] = self.buttons & 0x1f;
        bytes[1..3].copy_from_slice(&self.x.to_le_bytes());
        bytes[3..5].copy_from_slice(&self.y.to_le_bytes());
        bytes[5] = self.wheel as u8;
        bytes[6] = self.horizontal_wheel as u8;
        bytes
    }
}

/// Split a relative movement into report-sized signed values without losing
/// total displacement. Zero is intentionally represented by no movement, not
/// a replay of the previous delta.
pub fn split_relative(mut value: i32) -> impl Iterator<Item = i16> {
    std::iter::from_fn(move || {
        if value == 0 {
            return None;
        }
        let part = value.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        value -= i32::from(part);
        Some(part)
    })
}

/// Serial caller owns the transport. Release is attempted even after an
/// uncertain press. The Windows worker uses the same two reports but schedules
/// the release asynchronously rather than sleeping on the input path.
pub fn click(
    command: MediaCommand,
    mut notify: impl FnMut(&[u8]) -> Result<(), BackendError>,
    delay: impl FnOnce(),
) -> Result<(), BackendError> {
    let pressed = consumer_press(command);
    let press_result = notify(&pressed);
    if press_result.is_ok() {
        delay();
    }
    let released = consumer_release();
    let release_result = notify(&released);
    match release_result {
        Ok(()) => press_result,
        Err(release) => Err(BackendError::Release {
            press: press_result.err().map(Box::new),
            release: Box::new(release),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Interpret the Consumer Array's selector table independently of the encoder.
    // Array values index the declared usages; they are not implicitly usage IDs.
    fn consumer_array_usages() -> (u32, Vec<u16>) {
        let mut report_id = 0;
        let mut logical_min = 0;
        let mut usage_min = 0;
        let mut usages = Vec::new();
        let mut offset = 0;
        while offset < REPORT_MAP.len() {
            let prefix = REPORT_MAP[offset];
            let size = match prefix & 3 {
                3 => 4,
                n => n as usize,
            };
            let value = REPORT_MAP[offset + 1..offset + 1 + size]
                .iter()
                .enumerate()
                .fold(0u32, |v, (i, b)| v | (u32::from(*b) << (8 * i)));
            offset += 1 + size;
            match prefix & 0xfc {
                0x84 => report_id = value,
                0x14 => logical_min = value,
                0x08 => usages.push(value as u16),
                0x18 => usage_min = value as u16,
                0x28 => usages.extend(usage_min..=value as u16),
                0x80 if report_id == u32::from(CONSUMER_REPORT_ID) => {
                    assert_eq!(value & 3, 0, "Consumer input must be a data array");
                    return (logical_min, usages);
                }
                _ => {}
            }
            if prefix & 0x0c == 0 {
                usages.clear();
            }
        }
        panic!("Consumer input missing");
    }

    #[test]
    fn consumer_reports_decode_to_their_declared_media_usages() {
        let (minimum, usages) = consumer_array_usages();
        for command in [
            MediaCommand::PlayPause,
            MediaCommand::Previous,
            MediaCommand::Next,
            MediaCommand::Mute,
            MediaCommand::Rewind,
            MediaCommand::FastForward,
        ] {
            let report = consumer_press(command);
            let selector = u32::from(u16::from_le_bytes([report[0], report[1]]));
            let decoded = selector
                .checked_sub(minimum)
                .and_then(|i| usages.get(i as usize))
                .copied();
            assert_eq!(
                decoded,
                Some(command.usage()),
                "Host must decode {command:?}"
            );
        }
        assert_eq!(
            usages.first(),
            Some(&0),
            "Zero-filled releases must select no control"
        );
    }

    #[test]
    fn descriptor_has_three_reports_and_standard_usages() {
        assert!(
            REPORT_MAP
                .windows(2)
                .any(|b| b == [0x85, CONSUMER_REPORT_ID])
        );
        assert!(
            REPORT_MAP
                .windows(2)
                .any(|b| b == [0x85, KEYBOARD_REPORT_ID])
        );
        assert!(REPORT_MAP.windows(2).any(|b| b == [0x85, MOUSE_REPORT_ID]));
        for usage in [0xcd, 0xb6, 0xb5, 0xe2, 0xb4, 0xb3] {
            assert!(consumer_array_usages().1.contains(&usage));
        }
        assert_eq!(CONSUMER_REPORT_REFERENCE, [1, 1]);
        assert_eq!(KEYBOARD_REPORT_REFERENCE, [2, 1]);
        assert_eq!(MOUSE_REPORT_REFERENCE, [3, 1]);
    }

    #[test]
    fn consumer_owners_and_commands_are_composed() {
        let mut state = ConsumerState::default();
        state.press(1, MediaCommand::Rewind);
        state.press(2, MediaCommand::Rewind);
        state.press(3, MediaCommand::Next);
        let report = state.report();
        let usages: Vec<_> = report
            .chunks(2)
            .map(|bytes| u16::from_le_bytes([bytes[0], bytes[1]]))
            .collect();
        assert!(usages.contains(&MediaCommand::Next.usage()));
        assert!(usages.contains(&MediaCommand::Rewind.usage()));
        state.release(1, MediaCommand::Rewind);
        assert!(!state.is_empty());
        assert!(
            state
                .report()
                .chunks(2)
                .any(|bytes| bytes == MediaCommand::Rewind.usage().to_le_bytes())
        );
        state.release(2, MediaCommand::Rewind);
        assert!(
            !state
                .report()
                .chunks(2)
                .any(|bytes| bytes == MediaCommand::Rewind.usage().to_le_bytes())
        );
    }

    #[test]
    fn keyboard_and_mouse_reports_have_stable_lengths_and_zero_neutral() {
        let keyboard = KeyboardState::default();
        assert_eq!(keyboard.report().unwrap(), [0; KEYBOARD_REPORT_LENGTH]);
        assert_eq!(MouseReport::neutral().encode(), [0; MOUSE_REPORT_LENGTH]);
        assert_eq!(
            neutral(ReportKind::Consumer),
            vec![0; CONSUMER_REPORT_LENGTH]
        );
    }

    #[test]
    fn relative_split_preserves_large_motion() {
        let values: Vec<_> = split_relative(70_000).collect();
        assert_eq!(values.iter().map(|v| i32::from(*v)).sum::<i32>(), 70_000);
        assert!(values.iter().all(|value| *value != 0));
    }

    #[test]
    fn representative_windows_keys_map_to_standard_usages() {
        assert_eq!(keyboard_usage(0x41), Some(0x04));
        assert_eq!(keyboard_usage(0x30), Some(0x27));
        assert_eq!(keyboard_usage(0x70), Some(0x3a));
        assert_eq!(keyboard_usage(0xe0), Some(0x58));
        assert_eq!(keyboard_modifier_bit(0xa3), Some(1 << 4));
        assert_eq!(keyboard_usage(0x11), None);
    }

    #[test]
    fn release_after_success_or_failure() {
        for fail in [false, true] {
            let mut reports = Vec::new();
            let result = click(
                MediaCommand::PlayPause,
                |bytes| {
                    reports.push(bytes.to_vec());
                    if fail && bytes[0] != 0 {
                        Err(BackendError::Unavailable("uncertain".into()))
                    } else {
                        Ok(())
                    }
                },
                || {},
            );
            assert_eq!(
                reports,
                [consumer_press(MediaCommand::PlayPause), consumer_release()]
            );
            assert_eq!(result.is_err(), fail);
        }
    }

    #[test]
    fn failed_release_is_not_hidden_by_press_failure() {
        let result = click(
            MediaCommand::PlayPause,
            |_| Err(BackendError::Unavailable("transport".into())),
            || {},
        );
        assert!(matches!(
            result,
            Err(BackendError::Release { press: Some(_), .. })
        ));
    }
}
