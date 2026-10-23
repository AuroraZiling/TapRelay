//! HID session transitions shared by native senders
use super::{ConsumerState, InputReports, ReportKind};
use crate::{
    command::MediaCommand,
    mapping::OutputOwners,
    passthrough::{Event, KeyUsage},
    ports::BackendError,
};
use std::time::{Duration, Instant};

pub type Report = (ReportKind, Vec<u8>);
pub type Pulse = (Instant, u64, MediaCommand);

/// Borrow the native sender's state so transport scheduling and recovery stay
/// with their existing owners.
pub struct Session<'a> {
    pub keyboard: &'a mut InputReports,
    pub owners: &'a mut OutputOwners,
    pub consumer: &'a mut ConsumerState,
    pub pulses: &'a mut Vec<Pulse>,
    pub next_owner: &'a mut u64,
}
impl Session<'_> {
    pub fn prepare_media(&mut self, action: MediaCommand, down: bool) -> Option<Vec<u8>> {
        (down && release_volume_pulses(self.consumer, self.pulses, action))
            .then(|| self.consumer.report())
    }
    pub fn media(
        &mut self,
        action: MediaCommand,
        down: bool,
        now: Instant,
    ) -> Result<Vec<u8>, BackendError> {
        let owner = u64::from(action.usage());
        if !down {
            self.consumer.release(owner, action);
        } else if matches!(action, MediaCommand::Rewind | MediaCommand::FastForward) {
            self.consumer.press(owner, action);
        } else {
            *self.next_owner = self.next_owner.wrapping_add(1).max(1);
            let owner = *self.next_owner | (1 << 63);
            self.consumer.press(owner, action);
            self.pulses
                .push((now + Duration::from_millis(40), owner, action));
        }
        self.check_consumer()?;
        Ok(self.consumer.report())
    }
    fn check_consumer(&self) -> Result<(), BackendError> {
        if self.consumer.exceeds_capacity() {
            return Err(BackendError::Unavailable(
                "Consumer HID report capacity exceeded".into(),
            ));
        }
        Ok(())
    }
    pub fn event(
        &mut self,
        event: Event,
        reverse_scroll: bool,
    ) -> Result<impl Iterator<Item = Report> + use<>, BackendError> {
        let report = match event {
            Event::ReleasePhysical => {
                self.owners.clear_physical();
                let (keys, buttons) = self.owners.state();
                let keyboard = self
                    .keyboard
                    .owned_keyboard(keys, buttons)
                    .map_err(|error| BackendError::Unavailable(error.into()))?;
                self.consumer.clear();
                self.pulses.clear();
                return Ok([
                    Some((ReportKind::Keyboard, keyboard)),
                    Some((ReportKind::Mouse, self.keyboard.mouse_neutral())),
                    Some((ReportKind::Consumer, self.consumer.report())),
                ]
                .into_iter()
                .flatten());
            }
            Event::Mapping {
                output,
                token,
                down,
            } => {
                let (keys, buttons) = self.owners.update(token | (1 << 63), output, down);
                let keyboard = self
                    .keyboard
                    .owned_keyboard(keys, buttons)
                    .map_err(|error| BackendError::Unavailable(error.into()))?;
                Some(match output {
                    crate::mapping::MappingOutput::Keyboard { .. } => {
                        (ReportKind::Keyboard, keyboard)
                    }
                    crate::mapping::MappingOutput::Mouse { .. } => {
                        (ReportKind::Mouse, self.keyboard.mouse_neutral())
                    }
                })
            }
            Event::Key {
                usage: KeyUsage::Keyboard(usage),
                down,
            } => {
                let (keys, buttons) = self.owners.update(
                    u64::from(usage),
                    crate::mapping::MappingOutput::Keyboard {
                        usage,
                        modifiers: 0,
                    },
                    down,
                );
                Some((
                    ReportKind::Keyboard,
                    self.keyboard
                        .owned_keyboard(keys, buttons)
                        .map_err(|e| BackendError::Unavailable(e.into()))?,
                ))
            }
            Event::Key {
                usage: KeyUsage::Consumer(usage),
                down,
            } => {
                let owner = 0x10000 + u64::from(usage);
                if down {
                    self.consumer.press_usage(owner, usage);
                } else {
                    self.consumer.release_usage(owner, usage);
                }
                Some((ReportKind::Consumer, self.consumer.report()))
            }
            Event::Button { button, down } => {
                let (keys, buttons) = self.owners.update(
                    0x1000 + button as u64,
                    crate::mapping::MappingOutput::Mouse { button },
                    down,
                );
                self.keyboard
                    .owned_keyboard(keys, buttons)
                    .map_err(|e| BackendError::Unavailable(e.into()))?;
                Some((ReportKind::Mouse, self.keyboard.mouse_neutral()))
            }
            Event::Motion { dx, dy } => Some((
                ReportKind::Mouse,
                self.keyboard
                    .motion(dx, dy)
                    .map_err(|e| BackendError::Unavailable(e.into()))?,
            )),
            Event::Wheel {
                vertical,
                horizontal,
            } => self
                .keyboard
                .wheel_with_direction(vertical, horizontal, reverse_scroll)
                .map_err(|e| BackendError::Unavailable(e.into()))?
                .map(|bytes| (ReportKind::Mouse, bytes)),
            Event::Media { action, down } => {
                return Err(BackendError::Unavailable(format!(
                    "Media {action:?}/{down} requires a pulse transition"
                )));
            }
            Event::Pointer {
                dx,
                dy,
                vertical,
                horizontal,
            } => Some((
                ReportKind::Mouse,
                self.keyboard
                    .pointer(dx, dy, vertical, horizontal, reverse_scroll)
                    .map_err(|e| BackendError::Unavailable(e.into()))?,
            )),
        };
        if report
            .as_ref()
            .is_some_and(|(kind, _)| *kind == ReportKind::Consumer)
        {
            self.check_consumer()?;
        }
        // At most three reports; the outer collection never needs a heap allocation.
        Ok([report, None, None].into_iter().flatten())
    }
}
pub fn release_volume_pulses(
    consumer: &mut ConsumerState,
    pulses: &mut Vec<(Instant, u64, MediaCommand)>,
    action: MediaCommand,
) -> bool {
    if !matches!(action, MediaCommand::VolumeUp | MediaCommand::VolumeDown) {
        return false;
    }
    let mut released = false;
    pulses.retain(|(_, owner, previous)| {
        if matches!(previous, MediaCommand::VolumeUp | MediaCommand::VolumeDown) {
            consumer.release(*owner, *previous);
            released = true;
            false
        } else {
            true
        }
    });
    released
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{input::MouseButton, mapping::MappingOutput};

    #[test]
    fn physical_release_preserves_mapping_owners_and_report_order() {
        let mut session = Session {
            keyboard: &mut InputReports::default(),
            owners: &mut OutputOwners::default(),
            consumer: &mut ConsumerState::default(),
            pulses: &mut Vec::new(),
            next_owner: &mut 0,
        };
        for event in [
            Event::Key {
                usage: KeyUsage::Keyboard(4),
                down: true,
            },
            Event::Button {
                button: MouseButton::Left,
                down: true,
            },
            Event::Mapping {
                output: MappingOutput::Keyboard {
                    usage: 5,
                    modifiers: 2,
                },
                token: 1,
                down: true,
            },
            Event::Mapping {
                output: MappingOutput::Mouse {
                    button: MouseButton::Right,
                },
                token: 2,
                down: true,
            },
        ] {
            assert_eq!(session.event(event, false).unwrap().count(), 1);
        }
        session
            .media(MediaCommand::VolumeUp, true, Instant::now())
            .unwrap();
        let reports: Vec<_> = session
            .event(Event::ReleasePhysical, false)
            .unwrap()
            .collect();
        assert_eq!(
            reports.iter().map(|(kind, _)| *kind).collect::<Vec<_>>(),
            [
                ReportKind::Keyboard,
                ReportKind::Mouse,
                ReportKind::Consumer
            ]
        );
        let mut keyboard = vec![0; 20];
        keyboard[0] = 2; // mapped Shift+B survives physical key release
        keyboard[2] = 5;
        assert_eq!(reports[0].1, keyboard);
        assert_eq!(reports[1].1, [2, 0, 0, 0, 0, 0, 0]); // mapped right mouse button
        assert!(reports[2].1.iter().all(|byte| *byte == 0));
        assert!(session.pulses.is_empty());
    }

    #[test]
    fn pointer_bytes_wheel_remainders_and_range_errors_keep_the_wire_contract() {
        let mut session = Session {
            keyboard: &mut InputReports::default(),
            owners: &mut OutputOwners::default(),
            consumer: &mut ConsumerState::default(),
            pulses: &mut Vec::new(),
            next_owner: &mut 0,
        };
        assert_eq!(
            session
                .event(
                    Event::Wheel {
                        vertical: 60,
                        horizontal: -60
                    },
                    false
                )
                .unwrap()
                .count(),
            0
        );
        let reports: Vec<_> = session
            .event(
                Event::Pointer {
                    dx: -2,
                    dy: 300,
                    vertical: 60,
                    horizontal: -60,
                },
                true,
            )
            .unwrap()
            .collect();
        assert_eq!(
            reports,
            [(ReportKind::Mouse, vec![0, 0xfe, 0xff, 0x2c, 1, 0xff, 1])]
        );
        assert_eq!(
            session
                .event(
                    Event::Wheel {
                        vertical: 0,
                        horizontal: 0
                    },
                    false
                )
                .unwrap()
                .count(),
            0
        );
        assert!(
            session
                .event(
                    Event::Motion {
                        dx: i32::MAX,
                        dy: 0
                    },
                    false
                )
                .is_err()
        );
    }
}
