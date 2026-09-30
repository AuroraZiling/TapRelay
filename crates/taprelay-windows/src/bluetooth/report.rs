use taprelay_core::hid::{self, ReportKind};

/// The payload and its native receiver identity must change under one lock.
pub(super) struct ReportState {
    kind: ReportKind,
    owner: Option<(String, u64)>,
    value: Vec<u8>,
}

impl ReportState {
    pub fn new(kind: ReportKind) -> Self {
        Self {
            kind,
            owner: None,
            value: hid::neutral(kind),
        }
    }

    pub fn clear(&mut self) {
        self.owner = None;
        self.value = hid::neutral(self.kind);
    }

    pub fn update(&mut self, id: String, generation: u64, current_generation: u64, bytes: &[u8]) {
        // A queued notification from a revoked endpoint cannot own new state.
        if id.is_empty() || generation != current_generation {
            return;
        }
        self.value = bytes.to_vec();
        if self.kind == ReportKind::Mouse {
            self.value[1..].fill(0);
        }
        self.owner = Some((id, generation));
    }

    pub fn value(&self, requester: Option<&str>, generation: u64) -> Vec<u8> {
        if self.owner.as_ref().is_some_and(|(id, owner_generation)| {
            !id.is_empty() && requester == Some(id.as_str()) && *owner_generation == generation
        }) {
            self.value.clone()
        } else {
            // Hosts may read before selection/subscription. Preserve a valid
            // HID response without disclosing another receiver's input.
            hid::neutral(self.kind)
        }
    }
}

pub(super) fn read_value(value: &[u8], offset: usize) -> Result<&[u8], u8> {
    value.get(offset..).ok_or(7) // ATT Invalid Offset
}

#[cfg(test)]
mod tests {
    use super::*;
    use taprelay_core::{command::MediaCommand, hid::InputReports};

    fn sensitive_report(kind: ReportKind) -> Vec<u8> {
        let mut reports = InputReports::default();
        match kind {
            ReportKind::Consumer => hid::consumer_press(MediaCommand::PlayPause),
            ReportKind::Keyboard => reports.key(4, true).unwrap(),
            ReportKind::Mouse => reports.button(taprelay_core::input::MouseButton::Left, true),
        }
    }

    #[test]
    fn report_reads_expose_input_only_to_its_receiver_in_the_current_generation() {
        for kind in ReportKind::ALL {
            let mut state = ReportState::new(kind);
            let secret = sensitive_report(kind);
            assert_ne!(secret, hid::neutral(kind));
            state.update("selected-session".into(), 1, 1, &secret);
            assert_eq!(state.value(Some("selected-session"), 1), secret);
            for requester in [
                None,
                Some(""),
                Some("other-session"),
                Some("selected-alias"),
            ] {
                let value = state.value(requester, 1);
                assert_eq!(value, hid::neutral(kind));
                for offset in 0..=value.len() {
                    assert_eq!(
                        read_value(&value, offset).unwrap(),
                        &hid::neutral(kind)[offset..]
                    );
                }
            }
            assert_eq!(state.value(Some("selected-session"), 2), hid::neutral(kind));
        }
    }

    #[test]
    fn report_revocation_and_receiver_replacement_cannot_reveal_previous_input() {
        for kind in ReportKind::ALL {
            let mut state = ReportState::new(kind);
            let secret = sensitive_report(kind);
            assert_eq!(state.value(Some("a"), 1), hid::neutral(kind));
            state.update("a".into(), 1, 1, &secret);
            state.clear();
            assert_eq!(state.value(Some("a"), 1), hid::neutral(kind));
            state.update("a".into(), 1, 2, &secret);
            assert_eq!(state.value(Some("a"), 2), hid::neutral(kind));
            state.update("b".into(), 2, 2, &secret);
            state.update("a".into(), 1, 2, &secret);
            assert_eq!(state.value(Some("a"), 2), hid::neutral(kind));
            assert_eq!(state.value(Some("b"), 2), secret);
            state.update("".into(), 2, 2, &secret);
            assert_eq!(state.value(None, 2), hid::neutral(kind));
        }
    }

    #[test]
    fn selected_report_reads_preserve_offsets_and_mouse_button_state() {
        for kind in ReportKind::ALL {
            let mut state = ReportState::new(kind);
            let secret = sensitive_report(kind);
            state.update("selected".into(), 1, 1, &secret);
            let value = state.value(Some("selected"), 1);
            for offset in 0..=value.len() {
                assert_eq!(read_value(&value, offset).unwrap(), &secret[offset..]);
            }
            assert_eq!(read_value(&value, value.len() + 1), Err(7));
            assert_eq!(read_value(&value, usize::MAX), Err(7));
        }
        let mut state = ReportState::new(ReportKind::Mouse);
        state.update("selected".into(), 1, 1, &[1, 2, 3, 4, 5, 6, 7]);
        assert_eq!(state.value(Some("selected"), 1), [1, 0, 0, 0, 0, 0, 0]);
    }
}
