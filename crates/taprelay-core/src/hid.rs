//! Standard HID-over-GATT report descriptions, state, and pure encoders.
//!
//! GATT report characteristics carry the payload only. The report ID is
//! declared by the Report Reference descriptor and is therefore not prefixed
//! to a notification value.

use crate::{command::MediaCommand, ports::BackendError};
use std::collections::{BTreeMap, BTreeSet};
mod input;
pub use input::{InputReports, keyboard_usage};

pub const CONSUMER_REPORT_ID: u8 = 1;
pub const CONSUMER_REPORT_REFERENCE: [u8; 2] = [CONSUMER_REPORT_ID, 1];
pub const CONSUMER_REPORT_LENGTH: usize = 12; // six 16-bit usage-array entries

/// Keyboard, Consumer Control and relative Mouse application collections.
pub const REPORT_MAP: &[u8] = &[
    // Put the keyboard first for hosts that classify by the primary collection.
    0x05,
    0x01,
    0x09,
    0x06,
    0xa1,
    0x01,
    0x85,
    0x02,
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
    0x75,
    0x08,
    0x95,
    0x01,
    0x81,
    0x01,
    0x19,
    0x00,
    0x29,
    0xdf,
    0x15,
    0x00,
    0x26,
    0xdf,
    0x00,
    0x75,
    0x08,
    0x95,
    0x12,
    0x81,
    0x00,
    0xc0,
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
    0x05,
    0x01,
    0x09,
    0x02,
    0xa1,
    0x01,
    0x85,
    0x03,
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
    0x01,
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
    0x05,
    0x0c,
    0x0a,
    0x38,
    0x02,
    0x81,
    0x06,
    0xc0,
    0xc0,
];

pub const HID_INFORMATION: [u8; 4] = [0x11, 0x01, 0, 2];
pub const PROTOCOL_MODE: [u8; 1] = [1];

pub const MEDIA_REPORT_MAP: &[u8] = &[
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
    0x19,
    0x00,
    0x2a,
    0xff,
    0x03,
    0x81,
    0x00,
    0xc0,
];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Profile {
    Full,
    #[default]
    MediaOnly,
}

impl Profile {
    pub fn report_map(self) -> &'static [u8] {
        match self {
            Self::Full => REPORT_MAP,
            Self::MediaOnly => MEDIA_REPORT_MAP,
        }
    }

    pub fn reports(self) -> &'static [ReportKind] {
        match self {
            Self::Full => &ReportKind::ALL,
            Self::MediaOnly => &[ReportKind::Consumer],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ReportKind {
    Consumer,
    Keyboard,
    Mouse,
}

impl ReportKind {
    pub const ALL: [Self; 3] = [Self::Consumer, Self::Keyboard, Self::Mouse];

    pub const fn index(self) -> usize {
        self.id() as usize - 1
    }

    pub const fn id(self) -> u8 {
        match self {
            Self::Consumer => CONSUMER_REPORT_ID,
            Self::Keyboard => 2,
            Self::Mouse => 3,
        }
    }

    pub const fn reference(self) -> [u8; 2] {
        [self.id(), 1]
    }

    pub const fn payload_len(self) -> usize {
        match self {
            Self::Consumer => CONSUMER_REPORT_LENGTH,
            Self::Keyboard => 20,
            Self::Mouse => 7,
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

/// Media usages are owned by independent gestures.
/// Removing one owner never releases another, which is what lets a tapped
/// pulse and a held seek share a shortcut without releasing each other.
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

    pub fn exceeds_capacity(&self) -> bool {
        self.owners.len() > CONSUMER_REPORT_LENGTH / 2
    }

    pub fn clear(&mut self) {
        self.owners.clear();
    }
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
    fn applications(map: &[u8]) -> Vec<(u32, u32)> {
        let mut page = 0;
        let mut usage = 0;
        let mut depth = 0;
        let mut offset = 0;
        let mut collections = Vec::new();
        while offset < map.len() {
            let prefix = map[offset];
            let size = match prefix & 3 {
                3 => 4,
                size => size as usize,
            };
            let value = map[offset + 1..offset + 1 + size]
                .iter()
                .enumerate()
                .fold(0u32, |value, (index, byte)| {
                    value | (u32::from(*byte) << (8 * index))
                });
            match prefix & 0xfc {
                0x04 => page = value,
                0x08 => usage = value,
                0xa0 => {
                    if depth == 0 && value == 1 {
                        collections.push((page, usage));
                    }
                    depth += 1;
                }
                0xc0 => depth -= 1,
                _ => {}
            }
            offset += size + 1;
        }
        assert_eq!(depth, 0);
        collections
    }

    #[test]
    fn media_only_publication_has_no_keyboard_or_mouse_application() {
        assert_eq!(
            applications(Profile::MediaOnly.report_map()),
            vec![(0x0c, 1)]
        );
        assert_eq!(Profile::MediaOnly.reports(), &[ReportKind::Consumer]);
        assert_eq!(
            applications(Profile::Full.report_map()),
            vec![(1, 6), (0x0c, 1), (1, 2)]
        );
    }
    #[test]
    fn descriptor_exposes_keyboard_media_and_mouse_reports() {
        let ids: Vec<_> = REPORT_MAP
            .windows(2)
            .filter(|bytes| bytes[0] == 0x85)
            .map(|bytes| bytes[1])
            .collect();
        assert_eq!(ids, vec![2, CONSUMER_REPORT_ID, 3]);
        assert_eq!(
            ReportKind::ALL,
            [
                ReportKind::Consumer,
                ReportKind::Keyboard,
                ReportKind::Mouse
            ]
        );
    }

    // Interpret the Consumer Array's selector table independently of the encoder.
    // Array values index the declared usages; they are not implicitly usage IDs.
    fn consumer_array_usages(map: &[u8]) -> (u32, Vec<u16>) {
        let mut report_id = 0;
        let mut logical_min = 0;
        let mut usage_min = 0;
        let mut usages = Vec::new();
        let mut offset = 0;
        while offset < map.len() {
            let prefix = map[offset];
            let size = match prefix & 3 {
                3 => 4,
                n => n as usize,
            };
            let value = map[offset + 1..offset + 1 + size]
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
        let (minimum, usages) = consumer_array_usages(REPORT_MAP);
        assert_eq!(
            consumer_array_usages(MEDIA_REPORT_MAP),
            (minimum, usages.clone())
        );
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
