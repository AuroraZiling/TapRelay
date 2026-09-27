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
    mouse_interval: Option<Duration>,
    motion_owner: (u64, u64, u16),
    remainder: (i32, i32),
}

impl Default for ReportSchedule {
    fn default() -> Self {
        Self {
            pending: None,
            last_sent: None,
            interval: FALLBACK_INTERVAL,
            mouse_interval: None,
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
    pub fn set_mouse_report_rate(&mut self, hz: u16) -> bool {
        let interval = (hz != 0)
            .then(|| Duration::from_nanos(1_000_000_000_u64.div_ceil(u64::from(hz.min(1000)))));
        let changed = self.mouse_interval != interval;
        self.mouse_interval = interval;
        changed
    }
    pub fn mouse_interval(&self) -> Duration {
        // ATT notifications have no receiver-consumption acknowledgement. A
        // completed WinRT call is not a credit for another mouse report. Pace
        // conservatively at one report per connection interval, coalescing
        // displacement before submission, where we still control the queue.
        // A manual rate is an upper bound, never permission to flood the link.
        self.mouse_interval
            .unwrap_or(self.interval)
            .max(self.interval)
    }
    pub fn wait(&self, now: Instant) -> Duration {
        self.wait_interval(self.interval, now)
    }
    pub fn mouse_wait(&self, now: Instant) -> Duration {
        self.wait_interval(self.mouse_interval(), now)
    }
    pub fn input_wait(&self, now: Instant) -> Option<Duration> {
        self.pending
            .as_ref()
            .map(|packet| self.event_wait(packet.event, now))
    }
    fn event_wait(&self, event: Event, now: Instant) -> Duration {
        if pointer_components(event).is_some() || matches!(event, Event::Button { .. }) {
            self.mouse_wait(now)
        } else {
            self.wait(now)
        }
    }
    fn wait_interval(&self, interval: Duration, now: Instant) -> Duration {
        self.last_sent.map_or(Duration::ZERO, |sent| {
            interval.saturating_sub(now.saturating_duration_since(sent))
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
                if !self.event_wait(packet.event, now).is_zero() {
                    self.pending = Some(packet);
                    return Ok(None);
                }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, atomic::AtomicU64};

    fn active_link() -> (InputLink, mpsc::Receiver<Packet>) {
        let (link, input) = InputLink::channel(Arc::new(AtomicU64::new(1)));
        link.set_ready(1);
        assert!(link.begin());
        (link, input)
    }

    #[test]
    fn fast_notification_completion_does_not_build_a_slow_receiver_tail() {
        use std::collections::VecDeque;

        // Model an API which completes immediately, while the receiver consumes
        // one notification per 15 ms connection event. Exercise the real capture
        // queue and report scheduler rather than substituting a fake scheduler.
        let (link, input) = active_link();
        let mut schedule = ReportSchedule::default();
        schedule.set_connection_interval(Some(Duration::from_millis(15)));
        schedule.set_mouse_report_rate(1000);
        let start = Instant::now();
        let mut remote = VecDeque::new();
        let mut received_x = 0;
        let mut last_received_ms = 0;
        for ms in 0..1000_u64 {
            let now = start + Duration::from_millis(ms);
            if ms < 100 {
                assert!(link.submit(Event::Motion { dx: 1, dy: 0 }, now));
            }
            if let Some(packet) = schedule.take(&input, &link, now).unwrap() {
                remote.push_back(packet.event);
                schedule.sent(now);
            }
            if ms % 15 == 0
                && let Some(Event::Motion { dx, .. }) = remote.pop_front()
            {
                received_x += dx;
                last_received_ms = ms;
            }
        }
        assert!(
            remote.is_empty() && last_received_ms <= 130,
            "motion stopped at 99 ms, but receiver still moved at {last_received_ms} ms with {} reports queued",
            remote.len()
        );
        assert_eq!(
            received_x, 100,
            "coalescing must preserve total displacement"
        );
    }

    #[test]
    fn slow_notification_completion_coalesces_unsent_motion_without_replaying_each_sample() {
        let (link, input) = active_link();
        let mut schedule = ReportSchedule::default();
        schedule.set_connection_interval(Some(Duration::from_millis(15)));
        schedule.set_mouse_report_rate(1000);
        let start = Instant::now();
        let mut reports = Vec::new();
        for ms in 0..200_u64 {
            let now = start + Duration::from_millis(ms);
            if ms < 100 {
                assert!(link.submit(Event::Motion { dx: 1, dy: -1 }, now));
            }
            // The sender cannot dequeue another report while Notify is pending.
            // Model a completion which takes three connection events.
            if ms % 45 == 0
                && let Some(packet) = schedule.take(&input, &link, now).unwrap()
            {
                reports.push(packet.event);
                schedule.sent(now);
            }
        }
        assert_eq!(
            reports,
            [
                Event::Motion { dx: 1, dy: -1 },
                Event::Motion { dx: 45, dy: -45 },
                Event::Motion { dx: 45, dy: -45 },
                Event::Motion { dx: 9, dy: -9 },
            ]
        );
        assert!(
            schedule
                .take(&input, &link, start + Duration::from_millis(200))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn unavailable_connection_parameters_never_enable_unbounded_manual_sending() {
        let mut schedule = ReportSchedule::default();
        schedule.set_mouse_report_rate(1000);
        for interval in [None, Some(Duration::ZERO)] {
            schedule.set_connection_interval(interval);
            assert_eq!(schedule.mouse_interval(), FALLBACK_INTERVAL);
        }
        schedule.set_connection_interval(Some(Duration::from_micros(11_250)));
        assert_eq!(schedule.mouse_interval(), Duration::from_micros(11_250));
        schedule.set_connection_interval(Some(Duration::from_millis(30)));
        assert_eq!(schedule.mouse_interval(), Duration::from_millis(30));
    }

    #[test]
    fn manual_rates_are_upper_bounds_on_connection_paced_motion() {
        for hz in super::super::MOUSE_REPORT_RATES
            .into_iter()
            .filter(|hz| *hz != 0)
        {
            let (link, input) = active_link();
            let mut schedule = ReportSchedule::default();
            schedule.set_connection_interval(Some(Duration::from_millis(15)));
            schedule.set_mouse_report_rate(hz);
            let now = Instant::now();
            schedule.sent(now);
            assert!(link.submit(Event::Motion { dx: 2, dy: -1 }, now));
            let interval = Duration::from_nanos(1_000_000_000_u64.div_ceil(u64::from(hz)))
                .max(Duration::from_millis(15));
            assert!(
                schedule
                    .take(&input, &link, now + interval - Duration::from_nanos(1))
                    .unwrap()
                    .is_none()
            );
            assert_eq!(schedule.input_wait(now), Some(interval));
            assert_eq!(
                schedule
                    .take(&input, &link, now + interval)
                    .unwrap()
                    .unwrap()
                    .event,
                Event::Motion { dx: 2, dy: -1 }
            );
        }
    }

    #[test]
    fn changing_rate_applies_to_pending_motion_and_auto_restores_connection_pacing() {
        let (link, input) = active_link();
        let mut schedule = ReportSchedule::default();
        schedule.set_connection_interval(Some(Duration::from_millis(15)));
        schedule.set_mouse_report_rate(60);
        let now = Instant::now();
        schedule.sent(now);
        assert!(link.submit(Event::Motion { dx: 2, dy: 3 }, now));
        assert!(
            schedule
                .take(&input, &link, now + Duration::from_millis(15))
                .unwrap()
                .is_none()
        );
        schedule.set_mouse_report_rate(1000);
        assert!(
            schedule
                .take(&input, &link, now + Duration::from_millis(15))
                .unwrap()
                .is_some()
        );
        assert_eq!(schedule.wait(now), Duration::from_millis(15));
        schedule.set_connection_interval(Some(Duration::from_millis(30)));
        assert_eq!(schedule.mouse_wait(now), Duration::from_millis(30));
        schedule.set_mouse_report_rate(0);
        assert_eq!(schedule.mouse_wait(now), Duration::from_millis(30));
        schedule.set_connection_interval(Some(Duration::from_millis(1)));
        assert_eq!(schedule.mouse_wait(now), MIN_INTERVAL);
        schedule.set_connection_interval(None);
        assert_eq!(schedule.mouse_wait(now), FALLBACK_INTERVAL);
    }

    #[test]
    fn high_rate_preserves_motion_totals_and_button_order_without_accelerating_keys() {
        use crate::{input::MouseButton, passthrough::KeyUsage};
        let (link, input) = active_link();
        let mut schedule = ReportSchedule::default();
        schedule.set_connection_interval(Some(Duration::from_millis(15)));
        schedule.set_mouse_report_rate(1000);
        let now = Instant::now();
        let events = [
            Event::Motion { dx: 2, dy: -1 },
            Event::Motion { dx: 3, dy: -2 },
            Event::Button {
                button: MouseButton::Left,
                down: true,
            },
            Event::Motion { dx: 7, dy: 8 },
            Event::Key {
                usage: KeyUsage::Keyboard(4),
                down: true,
            },
        ];
        for event in events {
            assert!(link.submit(event, now));
        }
        for (ms, expected) in [
            (0, Event::Motion { dx: 5, dy: -3 }),
            (15, events[2]),
            (30, events[3]),
        ] {
            let due = now + Duration::from_millis(ms);
            assert_eq!(
                schedule.take(&input, &link, due).unwrap().unwrap().event,
                expected
            );
            schedule.sent(due);
        }
        assert!(
            schedule
                .take(&input, &link, now + Duration::from_millis(31))
                .unwrap()
                .is_none()
        );
        assert_eq!(
            schedule
                .take(&input, &link, now + Duration::from_millis(45))
                .unwrap()
                .unwrap()
                .event,
            events[4]
        );
    }

    #[test]
    fn pending_input_is_discarded_after_capture_ends() {
        let (link, input) = active_link();
        let mut schedule = ReportSchedule::default();
        let now = Instant::now();
        schedule.sent(now);
        assert!(link.submit(Event::Motion { dx: 1, dy: 0 }, now));
        assert!(schedule.take(&input, &link, now).unwrap().is_none());
        link.end();
        assert!(
            schedule
                .take(&input, &link, now + FALLBACK_INTERVAL)
                .unwrap()
                .is_none()
        );
        assert_eq!(schedule.input_wait(now), None);
    }
}
