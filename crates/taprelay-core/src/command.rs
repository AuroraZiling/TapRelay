use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaCommand {
    #[serde(rename = "play-pause")]
    PlayPause,
}
/// Interactive input loses intent quickly: discard queued presses after 250ms.
/// This is an admission deadline, not permission to cancel an in-flight release.
pub const COMMAND_TTL: Duration = Duration::from_millis(250);
#[derive(Debug)]
pub struct QueuedCommand {
    pub action: MediaCommand,
    pub target: String,
    pub generation: u64,
    pub created: Instant,
}
impl QueuedCommand {
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
            && now.saturating_duration_since(self.created) <= COMMAND_TTL
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disconnected_expired_or_replaced_commands_are_dropped() {
        let now = Instant::now();
        let c = QueuedCommand {
            action: MediaCommand::PlayPause,
            target: "a".into(),
            generation: 1,
            created: now,
        };
        assert!(c.valid(Some("a"), 1, true, now, now));
        assert!(!c.valid(Some("a"), 1, false, now, now));
        assert!(!c.valid(Some("a"), 2, true, now, now));
        assert!(!c.valid(Some("b"), 1, true, now, now));
        assert!(!c.valid(Some("a"), 1, true, now + Duration::from_secs(1), now));
        // Even if app processing assigns the new generation to a queued input,
        // its original capture timestamp must not cross a reconnect boundary.
        let reconnect = now + Duration::from_millis(1);
        assert!(!c.valid(Some("a"), 1, true, reconnect, reconnect));
    }
}
