use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaCommand {
    #[serde(rename = "play-pause")]
    PlayPause,
    #[serde(rename = "previous")]
    Previous,
    #[serde(rename = "next")]
    Next,
    #[serde(rename = "mute")]
    Mute,
    #[serde(rename = "rewind")]
    Rewind,
    #[serde(rename = "fast-forward")]
    FastForward,
}

impl MediaCommand {
    /// USB HID Consumer usage, as defined by the standard Consumer page.
    pub const fn usage(self) -> u16 {
        match self {
            Self::PlayPause => 0x00cd,
            Self::Previous => 0x00b6,
            Self::Next => 0x00b5,
            Self::Mute => 0x00e2,
            Self::Rewind => 0x00b4,
            Self::FastForward => 0x00b3,
        }
    }

    /// Localization key for this command used on its own, as one gesture of a
    /// function. The function's own label is a separate key: a merged
    /// press/hold function reads as "Previous / Rewind" while the tap gesture
    /// still reads as plain "Previous".
    pub const fn name_key(self) -> &'static str {
        match self {
            Self::PlayPause => "command.media.playpause",
            Self::Previous => "command.media.previous",
            Self::Next => "command.media.next",
            Self::Mute => "command.media.mute",
            Self::Rewind => "command.media.rewind",
            Self::FastForward => "command.media.fastforward",
        }
    }
}

/// Interactive input loses intent quickly: discard queued presses after 250ms.
/// This is an admission deadline, not permission to cancel an in-flight release.
pub const COMMAND_TTL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandPhase {
    Press,
    Release,
}

#[derive(Debug)]
pub struct QueuedCommand {
    pub action: MediaCommand,
    pub phase: CommandPhase,
    pub target: String,
    pub generation: u64,
    pub created: Instant,
}

impl QueuedCommand {
    pub fn press(action: MediaCommand, target: String, generation: u64, created: Instant) -> Self {
        Self {
            action,
            phase: CommandPhase::Press,
            target,
            generation,
            created,
        }
    }

    pub fn release(
        action: MediaCommand,
        target: String,
        generation: u64,
        created: Instant,
    ) -> Self {
        Self {
            action,
            phase: CommandPhase::Release,
            target,
            generation,
            created,
        }
    }

    pub fn is_press(&self) -> bool {
        self.phase == CommandPhase::Press
    }

    pub fn valid(
        &self,
        target: Option<&str>,
        generation: u64,
        ready: bool,
        now: Instant,
        generation_started: Instant,
    ) -> bool {
        ready
            && target == Some(self.target.as_str())
            && generation == self.generation
            && self.created >= generation_started
            && (self.phase == CommandPhase::Release
                || now.saturating_duration_since(self.created) <= COMMAND_TTL)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_usages_are_the_standard_consumer_controls() {
        assert_eq!(MediaCommand::PlayPause.usage(), 0xcd);
        assert_eq!(MediaCommand::Previous.usage(), 0xb6);
        assert_eq!(MediaCommand::Next.usage(), 0xb5);
        assert_eq!(MediaCommand::Mute.usage(), 0xe2);
        assert_eq!(MediaCommand::Rewind.usage(), 0xb4);
        assert_eq!(MediaCommand::FastForward.usage(), 0xb3);
    }

    #[test]
    fn presses_expire_but_required_releases_do_not_use_press_ttl() {
        let now = Instant::now();
        let press = QueuedCommand::press(MediaCommand::PlayPause, "a".into(), 1, now);
        let release = QueuedCommand::release(MediaCommand::PlayPause, "a".into(), 1, now);
        assert!(press.valid(Some("a"), 1, true, now, now));
        assert!(!press.valid(Some("a"), 1, true, now + Duration::from_secs(1), now));
        assert!(release.valid(Some("a"), 1, true, now + Duration::from_secs(1), now));
        assert!(!release.valid(Some("a"), 2, true, now, now));
    }
}
