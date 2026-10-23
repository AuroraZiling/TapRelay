//! Physical relative motion has one owner: IOHID. Quartz suppresses movement
//! but never supplies a second copy or reconstructs deltas from screen positions.
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const CAPACITY: usize = 256;
pub(super) const COALESCE: Duration = Duration::from_millis(5);

pub(super) struct Sample {
    device: usize,
    stamp: u64,
    pub captured: Instant,
    pub delta: [i32; 2],
}

#[derive(Default)]
pub(super) struct Motion {
    pending: VecDeque<Sample>,
    last_raw: Option<Instant>,
    waiting_since: Option<Instant>,
}

impl Motion {
    pub fn push(
        &mut self,
        device: usize,
        stamp: u64,
        captured: Instant,
        delta: [i32; 2],
    ) -> Result<(), &'static str> {
        if delta == [0, 0] {
            return Ok(());
        }
        self.last_raw = Some(captured);
        self.waiting_since = None;
        // A report's X/Y elements arrive separately with the same HID timestamp.
        if let Some(sample) = self
            .pending
            .iter_mut()
            .find(|s| s.device == device && s.stamp == stamp)
        {
            for (target, value) in sample.delta.iter_mut().zip(delta) {
                *target = target.saturating_add(value);
            }
            return Ok(());
        }
        if self.pending.len() == CAPACITY {
            return Err("Physical mouse queue full; capture stopped");
        }
        let index = self
            .pending
            .iter()
            .position(|s| s.captured > captured)
            .unwrap_or(self.pending.len());
        self.pending.insert(
            index,
            Sample {
                device,
                stamp,
                captured,
                delta,
            },
        );
        Ok(())
    }

    pub fn drain(&mut self, now: Instant) -> Vec<Sample> {
        let mut samples = Vec::new();
        while self
            .pending
            .front()
            .is_some_and(|s| now.saturating_duration_since(s.captured) >= COALESCE)
        {
            samples.push(self.pending.pop_front().unwrap());
        }
        samples
    }

    pub fn clear(&mut self) {
        self.pending.clear();
        self.last_raw = None;
        self.waiting_since = None;
    }

    pub fn observe_tap(&mut self, now: Instant) {
        // A descriptor alone does not prove that raw callbacks keep arriving.
        // Bound how long we suppress observed movement without physical data.
        if self.last_raw.is_none_or(|raw| {
            now.saturating_duration_since(raw) > taprelay_core::passthrough::MAX_INPUT_AGE
        }) {
            self.waiting_since.get_or_insert(now);
        }
    }

    pub fn stalled(&self, now: Instant) -> bool {
        self.waiting_since.is_some_and(|since| {
            now.saturating_duration_since(since) > taprelay_core::passthrough::MAX_INPUT_AGE
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn physical_motion_needs_no_quartz_counterpart_and_preserves_each_device() {
        let mut motion = Motion::default();
        let now = Instant::now();
        motion.push(1, 123, now, [7, 0]).unwrap();
        motion.push(1, 123, now, [0, -2]).unwrap();
        motion.push(2, 123, now, [3, 0]).unwrap();
        assert!(motion.drain(now).is_empty());
        let samples = motion.drain(now + COALESCE);
        assert_eq!(
            samples.iter().map(|s| s.delta).collect::<Vec<_>>(),
            [[7, -2], [3, 0]]
        );
        assert!(samples.iter().all(|s| s.captured == now));
        assert!(motion.drain(now + COALESCE).is_empty());
    }
    #[test]
    fn capacity_and_reset_preserve_capture_boundaries() {
        let now = Instant::now();
        let mut motion = Motion::default();
        for stamp in 0..CAPACITY {
            motion.push(1, stamp as u64, now, [1, 0]).unwrap();
        }
        assert!(motion.push(1, CAPACITY as u64, now, [1, 0]).is_err());
        motion.clear();
        assert!(motion.drain(now + COALESCE).is_empty());
    }

    #[test]
    fn missing_raw_motion_is_bounded_but_a_stationary_mouse_is_not_a_failure() {
        let start = Instant::now();
        let mut motion = Motion::default();
        assert!(!motion.stalled(start + Duration::from_secs(1)));
        motion.observe_tap(start);
        assert!(!motion.stalled(start + Duration::from_millis(10)));
        assert!(motion.stalled(start + Duration::from_secs(1)));
        motion.push(1, 1, start, [1, 0]).unwrap();
        assert!(!motion.stalled(start + Duration::from_secs(1)));
        motion.observe_tap(start + Duration::from_millis(1));
        assert!(!motion.stalled(start + Duration::from_secs(1)));
        motion.observe_tap(start + Duration::from_millis(300));
        assert!(motion.stalled(start + Duration::from_secs(1)));
        motion.clear();
        assert!(!motion.stalled(start + Duration::from_secs(1)));
    }

    #[test]
    fn movement_after_idle_allows_raw_callback_to_arrive_after_tap() {
        let start = Instant::now();
        let mut motion = Motion::default();
        motion.push(1, 1, start, [1, 0]).unwrap();
        let resumed = start + Duration::from_secs(10);
        motion.observe_tap(resumed);
        assert!(!motion.stalled(resumed + COALESCE));
        motion.push(1, 2, resumed + COALESCE, [1, 0]).unwrap();
        assert!(!motion.stalled(resumed + Duration::from_secs(1)));
    }
}
