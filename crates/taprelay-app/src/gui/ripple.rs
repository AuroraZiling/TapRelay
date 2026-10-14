use std::time::{Duration, Instant};

const DURATION: Duration = Duration::from_millis(800);

#[derive(Default)]
pub(super) struct Ripples {
    previous: Option<u64>,
    pulses: Vec<(Instant, [f32; 4])>,
}

pub(super) struct RippleFrame {
    pub progress: f32,
    pub opacity: f32,
    pub color: [f32; 4],
}

impl Ripples {
    pub fn update(&mut self, count: u64, color: [f32; 4], now: Instant, visible: bool) {
        let previous = self.previous.replace(count);
        self.pulses
            .retain(|(started, _)| now.saturating_duration_since(*started) < DURATION);
        if !visible {
            self.pulses.clear();
            self.previous = None;
            return;
        }
        if let Some(previous) = previous
            && count > previous
        {
            let count = (count - previous).min(16) as usize;
            let keep = 16 - count;
            if self.pulses.len() > keep {
                self.pulses.drain(..self.pulses.len() - keep);
            }
            self.pulses.extend(std::iter::repeat_n((now, color), count));
        }
    }

    pub fn frames(&self, now: Instant) -> Vec<RippleFrame> {
        self.pulses
            .iter()
            .filter_map(|(started, color)| {
                let elapsed = now.saturating_duration_since(*started);
                (elapsed < DURATION).then(|| {
                    let progress = elapsed.as_secs_f32() / DURATION.as_secs_f32();
                    RippleFrame {
                        progress,
                        opacity: 0.45 * (1.0 - progress),
                        color: *color,
                    }
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn input_expands_and_fades_without_replaying_the_initial_count() {
        let now = Instant::now();
        let mut ripples = Ripples::default();
        let color = [0.3, 0.5, 0.7, 1.0];
        ripples.update(42, color, now, true);
        assert!(ripples.frames(now).is_empty());
        ripples.update(43, color, now, true);
        assert_eq!(ripples.frames(now).len(), 1);
        let halfway = ripples.frames(now + Duration::from_millis(400));
        assert_eq!(halfway[0].progress, 0.5);
        assert_eq!(halfway[0].color, color);
        assert!((halfway[0].opacity - 0.225).abs() < 0.001);
        assert!(ripples.frames(now + Duration::from_millis(800)).is_empty());
    }

    #[test]
    fn burst_inputs_overlap_and_keep_at_most_sixteen_pulses() {
        let now = Instant::now();
        let mut ripples = Ripples::default();
        ripples.update(0, [0.; 4], now, true);
        ripples.update(3, [1.; 4], now, true);
        assert_eq!(ripples.frames(now).len(), 3);
        ripples.update(1000, [0.5; 4], now, true);
        assert_eq!(ripples.frames(now).len(), 16);
        assert!(ripples.frames(now).iter().all(|f| f.color == [0.5; 4]));
    }

    #[test]
    fn returning_to_a_status_page_does_not_replay_hidden_inputs() {
        let now = Instant::now();
        let mut ripples = Ripples::default();
        ripples.update(1, [0.; 4], now, true);
        ripples.update(2, [0.; 4], now, true);
        ripples.update(10, [0.; 4], now, false);
        ripples.update(15, [0.; 4], now, true);
        assert!(ripples.frames(now).is_empty());
        ripples.update(16, [0.; 4], now, true);
        assert_eq!(ripples.frames(now).len(), 1);
    }
}
