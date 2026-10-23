//! Join raw relative motion to its Quartz event, regardless of callback order.
//! Unmatched Quartz events always fall back to their own delta. Raw samples
//! never grant permission to discard another device's or remote source's input.
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const JOIN_WINDOW: Duration = Duration::from_millis(5);
const CAPACITY: usize = 256;

pub(super) struct Motion {
    pub captured: Instant,
    pub delta: [i32; 2],
}
struct Tap {
    stamp: u64,
    motion: Motion,
}
struct Raw {
    device: usize,
    stamp: u64,
    captured: Instant,
    delta: [i32; 2],
}

#[derive(Default)]
pub(super) struct MotionSources {
    taps: VecDeque<Tap>,
    raw: VecDeque<Raw>,
    resolved: VecDeque<u64>,
}

impl MotionSources {
    pub fn clear(&mut self) {
        self.taps.clear();
        self.raw.clear();
        self.resolved.clear();
    }

    pub fn tap(
        &mut self,
        stamp: u64,
        captured: Instant,
        delta: [i32; 2],
    ) -> Result<(), &'static str> {
        if self.taps.len() == CAPACITY {
            return Err("Mouse event queue full; capture stopped");
        }
        self.taps.push_back(Tap {
            stamp,
            motion: Motion { captured, delta },
        });
        Ok(())
    }

    pub fn raw(
        &mut self,
        device: usize,
        stamp: u64,
        captured: Instant,
        delta: [i32; 2],
    ) -> Result<(), &'static str> {
        if stamp == 0 || self.resolved.contains(&stamp) {
            return Ok(());
        }
        // IOHID delivers the X and Y elements separately with one timestamp.
        if let Some(sample) = self
            .raw
            .iter_mut()
            .find(|r| r.device == device && r.stamp == stamp)
        {
            for (target, value) in sample.delta.iter_mut().zip(delta) {
                *target = target.saturating_add(value);
            }
            return Ok(());
        }
        self.raw.retain(|sample| {
            captured.saturating_duration_since(sample.captured) <= JOIN_WINDOW * 2
        });
        if self.raw.len() == CAPACITY {
            return Err("Raw mouse queue full; capture stopped");
        }
        self.raw.push_back(Raw {
            device,
            stamp,
            captured,
            delta,
        });
        Ok(())
    }

    pub fn remove(&mut self, device: usize) {
        self.raw.retain(|sample| sample.device != device);
    }

    pub fn drain(&mut self, now: Instant) -> Vec<Motion> {
        let mut output = Vec::new();
        // Let both axes and either callback order finish before joining. The
        // original timestamp survives this bounded delay and BLE age checks.
        while self
            .taps
            .front()
            .is_some_and(|tap| now.saturating_duration_since(tap.motion.captured) >= JOIN_WINDOW)
        {
            let mut tap = self.taps.pop_front().unwrap();
            let mut relative = [0i32; 2];
            let mut device = None;
            let mut ambiguous = false;
            self.raw.retain(|sample| {
                if tap.stamp != 0 && sample.stamp == tap.stamp {
                    if device.is_some_and(|device| device != sample.device) {
                        ambiguous = true;
                    }
                    device = Some(sample.device);
                    for (target, value) in relative.iter_mut().zip(sample.delta) {
                        *target = target.saturating_add(value);
                    }
                    false
                } else {
                    true
                }
            });
            // Quartz exposes no HID device id. Timestamp collisions across
            // devices are ambiguous, so preserve the Quartz delta in that case.
            if device.is_some() && !ambiguous {
                tap.motion.delta = relative;
            }
            if self.resolved.len() == CAPACITY {
                self.resolved.pop_front();
            }
            self.resolved.push_back(tap.stamp);
            if tap.motion.delta != [0, 0] {
                output.push(tap.motion);
            }
        }
        self.raw
            .retain(|sample| now.saturating_duration_since(sample.captured) <= JOIN_WINDOW * 2);
        output
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_callback_orders_join_axes_once_and_unrelated_input_survives() {
        for raw_first in [true, false] {
            let mut sources = MotionSources::default();
            let now = Instant::now();
            if !raw_first {
                sources.tap(100, now, [90, 80]).unwrap();
            }
            sources.raw(1, 100, now, [4, 0]).unwrap();
            sources.raw(1, 100, now, [0, 3]).unwrap();
            if raw_first {
                sources.tap(100, now, [90, 80]).unwrap();
            }
            // A different device/source with no raw observation is preserved.
            sources.tap(200, now, [7, -2]).unwrap();
            let output = sources.drain(now + JOIN_WINDOW);
            assert_eq!(
                output.iter().map(|m| m.delta).collect::<Vec<_>>(),
                [[4, 3], [7, -2]]
            );
            assert_eq!(output[0].captured, now);
            sources.raw(1, 100, now, [4, 3]).unwrap();
            assert!(sources.drain(now + JOIN_WINDOW * 3).is_empty());
        }
    }
    #[test]
    fn timestamp_collision_across_devices_uses_quartz_fallback() {
        let now = Instant::now();
        let mut sources = MotionSources::default();
        sources.raw(1, 100, now, [4, 0]).unwrap();
        sources.raw(2, 100, now, [0, 3]).unwrap();
        sources.tap(100, now, [8, 6]).unwrap();
        assert_eq!(sources.drain(now + JOIN_WINDOW)[0].delta, [8, 6]);
    }
    #[test]
    fn removal_drops_only_that_devices_raw_sample() {
        let now = Instant::now();
        let mut sources = MotionSources::default();
        sources.raw(1, 100, now, [1, 0]).unwrap();
        sources.raw(2, 200, now, [2, 0]).unwrap();
        sources.remove(1);
        sources.tap(100, now, [10, 0]).unwrap();
        sources.tap(200, now, [20, 0]).unwrap();
        assert_eq!(
            sources
                .drain(now + JOIN_WINDOW)
                .iter()
                .map(|m| m.delta)
                .collect::<Vec<_>>(),
            [[10, 0], [2, 0]]
        );
    }
    #[test]
    fn bounded_queue_and_reset_do_not_carry_input_into_next_capture() {
        let now = Instant::now();
        let mut sources = MotionSources::default();
        for stamp in 0..CAPACITY {
            sources.tap(stamp as u64, now, [1, 0]).unwrap();
        }
        assert!(sources.tap(999, now, [1, 0]).is_err());
        sources.clear();
        assert!(sources.drain(now + JOIN_WINDOW).is_empty());
    }
}
