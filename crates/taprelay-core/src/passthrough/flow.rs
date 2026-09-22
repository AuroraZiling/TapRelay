use super::{Event, InputLink, MAX_INPUT_AGE, Packet, QUEUE_CAPACITY};
use std::{
    sync::mpsc,
    time::{Duration, Instant},
};

const MIN_INTERVAL: Duration = Duration::from_micros(8_334);
const FALLBACK_INTERVAL: Duration = Duration::from_millis(30);

pub struct ReportSchedule {
    pending: Option<Packet>,
    last_sent: Option<Instant>,
    interval: Duration,
    motion_owner: (u64, u64, u16),
    remainder: (i32, i32),
}

impl Default for ReportSchedule {
    fn default() -> Self {
        Self {
            pending: None,
            last_sent: None,
            interval: FALLBACK_INTERVAL,
            motion_owner: (0, 0, 0),
            remainder: (0, 0),
        }
    }
}

impl ReportSchedule {
    pub fn set_connection_interval(&mut self, interval: Option<Duration>) {
        self.interval = interval
            .filter(|value| !value.is_zero())
            .unwrap_or(FALLBACK_INTERVAL)
            .max(MIN_INTERVAL);
    }
    pub fn interval(&self) -> Duration {
        self.interval
    }
    pub fn wait(&self, now: Instant) -> Duration {
        self.last_sent.map_or(Duration::ZERO, |sent| {
            self.interval
                .saturating_sub(now.saturating_duration_since(sent))
        })
    }
    pub fn sent(&mut self, now: Instant) {
        self.last_sent = Some(now);
    }
    pub fn clear_input(&mut self) {
        self.pending = None;
        self.remainder = (0, 0);
        self.motion_owner = (0, 0, 0);
    }
    pub fn take(
        &mut self,
        input: &mpsc::Receiver<Packet>,
        link: &InputLink,
        now: Instant,
    ) -> Result<Option<Packet>, &'static str> {
        if !self.wait(now).is_zero() {
            return Ok(None);
        }
        let mut batch: Option<Packet> = None;
        for _ in 0..QUEUE_CAPACITY {
            let Some(packet) = self.pending.take().or_else(|| input.try_recv().ok()) else {
                break;
            };
            if !link.accepts(&packet) {
                continue;
            }
            if now.saturating_duration_since(packet.captured) > MAX_INPUT_AGE {
                return Err("Passthrough input backlog exceeded 250 ms");
            }
            let Some(first) = batch.as_mut() else {
                if pointer_components(packet.event).is_none() {
                    return Ok(Some(packet));
                }
                batch = Some(packet);
                continue;
            };
            if first.epoch == packet.epoch
                && first.generation == packet.generation
                && merge_continuous(&mut first.event, packet.event)
            {
                first.captured = first.captured.min(packet.captured);
            } else {
                self.pending = Some(packet);
                break;
            }
        }
        Ok(batch
            .filter(|packet| link.accepts(packet))
            .map(|mut packet| {
                if let Event::Motion { dx, dy } | Event::Pointer { dx, dy, .. } = &mut packet.event
                {
                    let percent = link.mouse_percent();
                    let owner = (packet.epoch, packet.generation, percent);
                    if self.motion_owner != owner {
                        self.motion_owner = owner;
                        self.remainder = (0, 0);
                    }
                    let x = i64::from(*dx) * i64::from(percent) + i64::from(self.remainder.0);
                    let y = i64::from(*dy) * i64::from(percent) + i64::from(self.remainder.1);
                    *dx = (x / 100) as i32;
                    *dy = (y / 100) as i32;
                    self.remainder = ((x % 100) as i32, (y % 100) as i32);
                }
                packet
            }))
    }
}

fn pointer_components(event: Event) -> Option<[i32; 4]> {
    match event {
        Event::Motion { dx, dy } => Some([dx, dy, 0, 0]),
        Event::Wheel {
            vertical,
            horizontal,
        } => Some([0, 0, vertical, horizontal]),
        Event::Pointer {
            dx,
            dy,
            vertical,
            horizontal,
        } => Some([dx, dy, vertical, horizontal]),
        _ => None,
    }
}

fn merge_continuous(first: &mut Event, next: Event) -> bool {
    let (Some(mut sums), Some(additions)) = (pointer_components(*first), pointer_components(next))
    else {
        return false;
    };
    for axis in 0..4 {
        let value = sums[axis];
        let addition = additions[axis];
        if axis >= 2 && value != 0 && addition != 0 && value.signum() != addition.signum() {
            return false;
        }
        let Some(sum) = value.checked_add(addition) else {
            return false;
        };
        let (min, max) = if axis < 2 {
            (i32::from(i16::MIN), i32::from(i16::MAX))
        } else {
            (-127 * 120, 127 * 120)
        };
        if !(min..=max).contains(&sum) {
            return false;
        }
        sums[axis] = sum;
    }
    let [dx, dy, vertical, horizontal] = sums;
    *first = match (*first, next) {
        (Event::Motion { .. }, Event::Motion { .. }) => Event::Motion { dx, dy },
        (Event::Wheel { .. }, Event::Wheel { .. }) => Event::Wheel {
            vertical,
            horizontal,
        },
        _ => Event::Pointer {
            dx,
            dy,
            vertical,
            horizontal,
        },
    };
    true
}
