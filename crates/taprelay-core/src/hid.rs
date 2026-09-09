use crate::{command::MediaCommand, ports::BackendError};
pub const REPORT_MAP: &[u8] = &[
    0x05, 0x0c, // Usage Page: Consumer
    0x09, 0x01, // Usage: Consumer Control
    0xa1, 0x01, // Application collection
    0x85, 0x01, // Report ID 1, carried by the GATT Report Reference descriptor
    0x15, 0x00, 0x25, 0x01, // Logical range 0..1
    0x09, 0xcd, // Usage: Play/Pause
    0x75, 0x01, 0x95, 0x01, 0x81, 0x02, // One data bit
    0x75, 0x07, 0x95, 0x01, 0x81, 0x03, // Seven constant padding bits
    0xc0, // End collection; GATT payload is one byte, without a report ID prefix
];
pub const REPORT_REFERENCE: [u8; 2] = [1, 1]; // Report ID 1, Input report
pub const HID_INFORMATION: [u8; 4] = [0x11, 0x01, 0, 2];
pub const NEUTRAL: [u8; 1] = [0];
pub fn press(command: MediaCommand) -> [u8; 1] {
    match command {
        MediaCommand::PlayPause => [1],
    }
}
/// Serial caller owns the transport. Release is attempted even after an uncertain press.
pub fn click(
    command: MediaCommand,
    mut notify: impl FnMut(&[u8]) -> Result<(), BackendError>,
    delay: impl FnOnce(),
) -> Result<(), BackendError> {
    let pressed = notify(&press(command));
    if pressed.is_ok() {
        delay();
    }
    let released = notify(&NEUTRAL);
    match released {
        Ok(()) => pressed,
        Err(release) => Err(BackendError::Release {
            press: pressed.err().map(Box::new),
            release: Box::new(release),
        }),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn descriptor_and_encoding_match() {
        assert!(
            REPORT_MAP
                .windows(2)
                .any(|b| b == [0x85, REPORT_REFERENCE[0]])
        );
        assert!(REPORT_MAP.windows(2).any(|b| b == [0x09, 0xcd]));
        assert_eq!(REPORT_REFERENCE[1], 1);
        assert_eq!(press(MediaCommand::PlayPause), [1]);
        // One data bit plus seven constant bits: no report ID in GATT payload.
        assert!(REPORT_MAP.windows(2).any(|b| b == [0x75, 7]));
        assert_eq!(NEUTRAL, [0]);
    }
    #[test]
    fn release_after_success_or_failure() {
        for fail in [false, true] {
            let mut reports = Vec::new();
            let result = click(
                MediaCommand::PlayPause,
                |b| {
                    reports.push(b.to_vec());
                    if fail && b[0] == 1 {
                        Err(BackendError::Unavailable("uncertain".into()))
                    } else {
                        Ok(())
                    }
                },
                || {},
            );
            assert_eq!(reports, [vec![1], vec![0]]);
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
