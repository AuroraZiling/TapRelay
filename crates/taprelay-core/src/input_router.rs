//! A single, platform-neutral input routing state machine.
//!
//! The Windows adapter supplies physical edges and consumes the boolean
//! decision in [`RouteResult`]. Output delivery is intentionally represented
//! as values so the adapter can enqueue it without making the hook wait for
//! Bluetooth, disk, or UI work.

use crate::{
    binding::{BindingIndex, BindingKey},
    function::{Activation, FunctionAction, FunctionConfigs, FunctionId},
    input::{InputCode, InputEvent, InputState, modifier},
};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

/// How long a merged press/hold shortcut must stay down before its long-press
/// gesture takes over. Time is the only thing that separates a tapped
/// "previous" from a held "rewind", so the router must be able to wait.
pub const HOLD_THRESHOLD: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouterReason {
    FunctionDisabled,
    BindingEdited,
    BeginRecording,
    ListenerStopped,
    TransportLost,
    SessionChanged,
    ApplicationExit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysicalInput {
    Edge { code: InputCode, down: bool },
    Motion { dx: i32, dy: i32 },
    Wheel { vertical: i32, horizontal: i32 },
}

/// The Windows input thread is the owner of the router. It publishes the
/// decision made at the physical edge together with the edge itself, so the
/// application thread only applies outputs and never re-runs matching against
/// a second copy of router state.
#[derive(Debug, Clone)]
pub enum RoutedInput {
    Control(RouteResult),
    Edge {
        event: InputEvent,
        result: RouteResult,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutedOutput {
    Local(PhysicalInput),
    /// Re-deliver an earlier physical input that the hook consumed while it
    /// waited to decide a later event (currently pending modifiers).
    Replay(PhysicalInput),
    Function {
        binding: BindingKey,
        action: FunctionAction,
        activation: Activation,
        down: bool,
        token: u64,
        revision: u64,
        created: Instant,
    },
    Feedback {
        binding: BindingKey,
        action: FunctionAction,
    },
}

#[derive(Debug, Default, Clone)]
pub struct RouteResult {
    pub revision: u64,
    pub consume: bool,
    pub outputs: Vec<RoutedOutput>,
}

impl RouteResult {
    fn pass(input: PhysicalInput) -> Self {
        Self {
            revision: 0,
            consume: false,
            outputs: vec![RoutedOutput::Local(input)],
        }
    }
}

#[derive(Debug, Clone)]
struct ActiveToken {
    binding: BindingKey,
    shortcut: crate::function::Shortcut,
    /// The tap gesture, emitted on release unless the hold gesture took over.
    action: FunctionAction,
    hold_action: Option<FunctionAction>,
    token: u64,
    revision: u64,
    created: Instant,
    /// When the hold gesture becomes due. `None` when there is nothing left to
    /// wait for, either because it was emitted or because there is none.
    deadline: Option<Instant>,
    /// The hold gesture was emitted and owns one held output.
    hold_started: bool,
}

/// Owns physical state, exact shortcut matching, captured lifetimes, and the
/// function lifetimes. Callers publish a new config at a revision
/// boundary and then feed subsequent physical events through `route`.
pub struct InputRouter {
    index: BindingIndex,
    revision: u64,
    listening: bool,
    recording: bool,
    physical: InputState,
    active: BTreeMap<InputCode, ActiveToken>,
    held_functions: BTreeMap<FunctionId, usize>,
    // A prefix is a short ordered sequence, not a set. The order matters when
    // an unmatched Ctrl+Shift chord is replayed to the host.
    pending_modifiers: Vec<InputCode>,
    captured_modifiers: BTreeSet<InputCode>,
    // A prefix replayed to the host keeps local ownership until its physical
    // up edge, across configuration changes.
    local_held: BTreeSet<InputCode>,
    suppressed_until_up: BTreeSet<InputCode>,
    next_token: u64,
}

impl InputRouter {
    pub fn new(configs: &FunctionConfigs, revision: u64) -> Self {
        Self {
            index: BindingIndex::new(configs),
            revision,
            listening: false,
            recording: false,
            physical: InputState::default(),
            active: BTreeMap::new(),
            held_functions: BTreeMap::new(),
            pending_modifiers: Vec::new(),
            captured_modifiers: BTreeSet::new(),
            local_held: BTreeSet::new(),
            suppressed_until_up: BTreeSet::new(),
            next_token: 0,
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn set_listening(&mut self, listening: bool) -> RouteResult {
        self.listening = listening;
        if !listening {
            self.terminate(RouterReason::ListenerStopped)
        } else {
            self.stamp(RouteResult::default())
        }
    }

    pub fn set_recording(&mut self, recording: bool) -> RouteResult {
        self.recording = recording;
        if recording {
            self.terminate(RouterReason::BeginRecording)
        } else {
            self.stamp(RouteResult::default())
        }
    }

    pub fn update_config(&mut self, configs: &FunctionConfigs, revision: u64) -> RouteResult {
        if revision < self.revision {
            return self.stamp(RouteResult::default());
        }
        let mut cleanup = RouteResult {
            revision: self.revision,
            consume: true,
            outputs: Vec::new(),
        };
        // An unmatched modifier prefix must be delivered before the new index
        // becomes authoritative. A prefix already captured by a function is
        // instead suppressed through its physical up edge and must not leak
        // while the old binding is being edited or disabled.
        self.flush_pending_unmatched(&mut cleanup);
        let next_index = BindingIndex::new(configs);
        let active_codes: Vec<_> = self
            .active
            .iter()
            .filter_map(|(&code, token)| {
                let replacement = next_index
                    .find(token.binding)
                    .and_then(|index| next_index.binding(index))
                    .is_none_or(|binding| {
                        binding.shortcut != token.shortcut
                            || binding.action != token.action
                            || binding.hold_action != token.hold_action
                    });
                replacement.then_some(code)
            })
            .collect();
        for code in active_codes {
            if let Some(active) = self.active.remove(&code) {
                // A pending tap never produced an output, so dropping it needs
                // no cleanup; only an escalated hold owns a held key.
                if active.hold_started {
                    self.release_hold(&active, &mut cleanup, Instant::now());
                }
                self.suppressed_until_up.insert(code);
            }
        }

        self.index = next_index;
        self.revision = revision;
        self.stamp(cleanup)
    }

    pub fn terminate(&mut self, _reason: RouterReason) -> RouteResult {
        let mut result = RouteResult {
            revision: self.revision,
            consume: true,
            outputs: Vec::new(),
        };
        // A modifier that was held back by the hook has not reached the host
        // yet. Replay only the non-captured prefixes before dropping the
        // router state; a captured shortcut must not leak a lone modifier.
        let pending = self.pending_modifiers.to_vec();
        for code in pending {
            if self.captured_modifiers.contains(&code) {
                // The modifier never reached either endpoint. Do not produce
                // an orphaned up after recording/session cleanup.
                self.suppressed_until_up.insert(code);
            } else {
                self.local_held.insert(code);
                result
                    .outputs
                    .push(RoutedOutput::Replay(PhysicalInput::Edge {
                        code,
                        down: true,
                    }));
            }
        }
        for active in self.active.values() {
            // The corresponding down was consumed by the hook. Keep its
            // trailing physical up consumed even if the session ends before
            // the application has applied the cleanup outputs.
            self.suppressed_until_up
                .insert(active.shortcut.primary_code());
            if active.hold_started
                && let Some(action) = active.hold_action
            {
                result.outputs.push(RoutedOutput::Function {
                    binding: active.binding,
                    action,
                    activation: Activation::Hold,
                    down: false,
                    token: active.token,
                    revision: active.revision,
                    created: active.created,
                });
            }
        }
        self.active.clear();
        self.held_functions.clear();
        self.pending_modifiers.clear();
        self.captured_modifiers.clear();
        self.stamp(result)
    }

    pub fn route(&mut self, input: PhysicalInput, captured: InputEvent) -> RouteResult {
        match input {
            PhysicalInput::Edge { code, down } => self.route_event(InputEvent {
                code,
                down,
                captured: captured.captured,
            }),
            PhysicalInput::Motion { dx, dy } => self.route_motion(dx, dy),
            PhysicalInput::Wheel {
                vertical,
                horizontal,
            } => self.route_wheel(vertical, horizontal),
        }
    }

    pub fn route_event(&mut self, event: InputEvent) -> RouteResult {
        let result = self.route_edge(event);
        self.stamp(result)
    }

    /// Route an edge that belongs to TapRelay's own configuration window.
    /// Physical state is still updated and a captured function is released,
    /// but this path never starts a configured function.
    pub fn route_local_event(&mut self, event: InputEvent) -> RouteResult {
        let changed = self.physical.update(event);
        if !changed {
            if self.active.contains_key(&event.code)
                || self.suppressed_until_up.contains(&event.code)
            {
                return self.route_event(event);
            }

            return self.stamp(RouteResult::default());
        }

        if let Some(active) = self.active.get(&event.code).cloned() {
            let mut result = RouteResult {
                revision: self.revision,
                consume: true,
                outputs: Vec::new(),
            };
            if !event.down {
                self.active.remove(&event.code);
                self.finish(&active, &mut result, event.captured);
            }
            return self.stamp(result);
        }
        if self.suppressed_until_up.contains(&event.code) {
            if !event.down {
                self.suppressed_until_up.remove(&event.code);
            }
            return self.stamp(RouteResult {
                revision: self.revision,
                consume: true,
                outputs: Vec::new(),
            });
        }
        if !self.listening || self.recording {
            if event.down {
                self.local_held.insert(event.code);
            }
            if !event.down {
                self.local_held.remove(&event.code);
            }
            return self.stamp(RouteResult::pass(PhysicalInput::Edge {
                code: event.code,
                down: event.down,
            }));
        }

        if !event.down && self.local_held.remove(&event.code) {
            return self.stamp(RouteResult {
                revision: self.revision,
                consume: false,
                outputs: vec![RoutedOutput::Local(PhysicalInput::Edge {
                    code: event.code,
                    down: false,
                })],
            });
        }

        let mut result = RouteResult {
            revision: self.revision,
            consume: false,
            outputs: Vec::new(),
        };
        self.flush_pending_local(&mut result);
        if event.down {
            self.local_held.insert(event.code);
        }
        result
            .outputs
            .push(RoutedOutput::Local(PhysicalInput::Edge {
                code: event.code,
                down: event.down,
            }));
        self.stamp(result)
    }

    pub fn route_motion(&mut self, dx: i32, dy: i32) -> RouteResult {
        let result = self.route_continuous(PhysicalInput::Motion { dx, dy });
        self.stamp(result)
    }

    pub fn route_wheel(&mut self, vertical: i32, horizontal: i32) -> RouteResult {
        let result = self.route_continuous(PhysicalInput::Wheel {
            vertical,
            horizontal,
        });
        self.stamp(result)
    }

    fn stamp(&self, mut result: RouteResult) -> RouteResult {
        result.revision = self.revision;
        result
    }

    fn route_continuous(&mut self, input: PhysicalInput) -> RouteResult {
        if !self.listening || self.recording {
            return RouteResult::pass(input);
        }
        let mut result = RouteResult {
            revision: self.revision,
            consume: false,
            outputs: Vec::new(),
        };
        self.flush_pending(&mut result);

        result.outputs.push(RoutedOutput::Local(input));

        result
    }

    fn route_edge(&mut self, event: InputEvent) -> RouteResult {
        let changed = self.physical.update(event);
        if !changed {
            if self.active.contains_key(&event.code)
                || self.suppressed_until_up.contains(&event.code)
            {
                return RouteResult {
                    revision: self.revision,
                    consume: true,
                    outputs: Vec::new(),
                };
            }
            if self.pending_modifiers.contains(&event.code) {
                // Modifier repeats do not create another prefix edge.
                return RouteResult {
                    revision: self.revision,
                    consume: true,
                    outputs: Vec::new(),
                };
            }

            return self.normal_input(event);
        }

        if self.suppressed_until_up.contains(&event.code) {
            if !event.down {
                self.suppressed_until_up.remove(&event.code);
            }
            return RouteResult {
                revision: self.revision,
                consume: true,
                outputs: Vec::new(),
            };
        }

        if !self.listening || self.recording {
            if event.down {
                self.local_held.insert(event.code);
            }
            if !event.down {
                self.local_held.remove(&event.code);
            }
            return RouteResult::pass(PhysicalInput::Edge {
                code: event.code,
                down: event.down,
            });
        }

        if let Some(active) = self.active.get(&event.code).cloned() {
            let mut result = RouteResult {
                revision: self.revision,
                consume: true,
                outputs: Vec::new(),
            };
            if !event.down {
                self.active.remove(&event.code);
                self.finish(&active, &mut result, event.captured);
            }
            return result;
        }

        if !event.down && self.local_held.remove(&event.code) {
            return RouteResult {
                revision: self.revision,
                consume: false,
                outputs: vec![RoutedOutput::Local(PhysicalInput::Edge {
                    code: event.code,
                    down: false,
                })],
            };
        }

        if event.down
            && let Some(index) = self.index.best_match(event.code, &self.physical)
        {
            let binding = self.index.binding(index).expect("index entry").clone();
            return self.activate(binding, event.captured);
        }

        if !event.down && self.pending_modifiers.contains(&event.code) {
            let captured = self.captured_modifiers.remove(&event.code);
            if captured {
                self.pending_modifiers.retain(|code| *code != event.code);
                return RouteResult {
                    revision: self.revision,
                    consume: true,
                    outputs: Vec::new(),
                };
            }
            let mut result = RouteResult {
                revision: self.revision,
                consume: false,
                outputs: Vec::new(),
            };
            self.flush_pending(&mut result);

            result
                .outputs
                .push(RoutedOutput::Local(PhysicalInput::Edge {
                    code: event.code,
                    down: false,
                }));
            return result;
        }

        if event.down
            && matches!(event.code, InputCode::Key(key) if modifier(key))
            && self.index.uses_modifier(event.code)
        {
            if !self.pending_modifiers.contains(&event.code) {
                self.pending_modifiers.push(event.code);
            }
            return RouteResult {
                revision: self.revision,
                consume: true,
                outputs: Vec::new(),
            };
        }

        self.normal_input(event)
    }

    fn normal_input(&mut self, event: InputEvent) -> RouteResult {
        if !self.listening || self.recording {
            return RouteResult::pass(PhysicalInput::Edge {
                code: event.code,
                down: event.down,
            });
        }

        let mut result = RouteResult {
            revision: self.revision,
            consume: false,
            outputs: Vec::new(),
        };
        self.flush_pending(&mut result);

        result.consume = false;
        if event.down {
            self.local_held.insert(event.code);
        }
        result
            .outputs
            .push(RoutedOutput::Local(PhysicalInput::Edge {
                code: event.code,
                down: event.down,
            }));

        result
    }

    fn activate(
        &mut self,
        binding: crate::binding::IndexedBinding,
        created: Instant,
    ) -> RouteResult {
        self.next_token = self.next_token.wrapping_add(1).max(1);
        let token = self.next_token;
        let mut result = RouteResult {
            revision: self.revision,
            consume: true,
            outputs: vec![RoutedOutput::Feedback {
                binding: binding.key,
                action: binding.action,
            }],
        };

        // A merged function cannot know which of its two gestures the user
        // meant yet: the tap is emitted on release, and the hold gesture
        // becomes due at the threshold. A plain tap has nothing to wait for,
        // so it keeps firing on the press edge.
        let deadline = binding.hold_action.map(|_| created + HOLD_THRESHOLD);
        if deadline.is_none() {
            result.outputs.push(RoutedOutput::Function {
                binding: binding.key,
                action: binding.action,
                activation: Activation::Press,
                down: true,
                token,
                revision: self.revision,
                created,
            });
        }
        // A function match consumes the modifier prefix until the associated
        // primary is released. If another ordinary input arrives later,
        // flush_pending will replay the still-held modifiers in order so the
        // user can continue a normal host chord without a timer-based
        // guess.
        self.captured_modifiers
            .extend(self.pending_modifiers.iter().copied());
        self.active.insert(
            binding.shortcut.primary_code(),
            ActiveToken {
                binding: binding.key,
                shortcut: binding.shortcut,
                action: binding.action,
                hold_action: binding.hold_action,
                token,
                revision: self.revision,
                created,
                deadline,
                hold_started: false,
            },
        );
        result
    }

    /// The earliest instant at which [`InputRouter::tick`] would emit an
    /// output. The platform arms its wake-up from this and owns the clock;
    /// the router never reads the current time itself.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.active
            .values()
            .filter_map(|token| token.deadline)
            .min()
    }

    /// Promote every shortcut that has been held past [`HOLD_THRESHOLD`].
    /// Returns no outputs while nothing is due, so a platform timer may call
    /// this freely.
    pub fn tick(&mut self, now: Instant) -> RouteResult {
        let mut result = RouteResult {
            revision: self.revision,
            consume: false,
            outputs: Vec::new(),
        };
        let due: Vec<InputCode> = self
            .active
            .iter()
            .filter(|(_, token)| token.deadline.is_some_and(|deadline| deadline <= now))
            .map(|(&code, _)| code)
            .collect();
        for code in due {
            let Some((binding, action, token, revision)) =
                self.active.get(&code).and_then(|active| {
                    active
                        .hold_action
                        .map(|action| (active.binding, action, active.token, active.revision))
                })
            else {
                continue;
            };
            if let Some(active) = self.active.get_mut(&code) {
                active.deadline = None;
                active.hold_started = true;
            }
            // Two shortcuts of one function share a single held output: the
            // receiver must not see the same key pressed twice.
            let count = self.held_functions.entry(binding.function).or_default();
            if *count == 0 {
                result.outputs.push(RoutedOutput::Function {
                    binding,
                    action,
                    activation: Activation::Hold,
                    down: true,
                    token,
                    revision,
                    created: now,
                });
            }
            *count += 1;
        }
        self.stamp(result)
    }

    /// Resolve a captured shortcut on its release edge. A merged function that
    /// was released before its threshold is a tap; one that already escalated
    /// releases the held seek instead.
    fn finish(&mut self, active: &ActiveToken, result: &mut RouteResult, released: Instant) {
        if active.hold_started {
            self.release_hold(active, result, released);
            return;
        }
        if active.hold_action.is_none() {
            // The tap was already emitted on the press edge.
            return;
        }
        result.outputs.push(RoutedOutput::Function {
            binding: active.binding,
            action: active.action,
            activation: Activation::Press,
            down: true,
            token: active.token,
            revision: active.revision,
            // The intent is fresh at the decision, not at the press.
            // Otherwise a deferred tap would already have outlived the press
            // admission deadline by the time the application queues it.
            created: released,
        });
    }

    fn release_hold(&mut self, active: &ActiveToken, result: &mut RouteResult, created: Instant) {
        let Some(action) = active.hold_action else {
            return;
        };
        if !active.hold_started {
            return;
        }
        if let Some(count) = self.held_functions.get_mut(&active.binding.function) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                self.held_functions.remove(&active.binding.function);
                result.outputs.push(RoutedOutput::Function {
                    binding: active.binding,
                    action,
                    activation: Activation::Hold,
                    down: false,
                    token: active.token,
                    revision: active.revision,
                    created,
                });
            }
        }
    }

    fn flush_pending(&mut self, result: &mut RouteResult) {
        self.flush_pending_with_capture(result, true);
    }

    fn flush_pending_unmatched(&mut self, result: &mut RouteResult) {
        self.flush_pending_with_capture(result, false);
    }

    fn flush_pending_with_capture(&mut self, result: &mut RouteResult, replay_captured: bool) {
        if self.pending_modifiers.is_empty() {
            return;
        }
        for code in self.pending_modifiers.iter().copied() {
            if !replay_captured && self.captured_modifiers.contains(&code) {
                self.suppressed_until_up.insert(code);
                continue;
            }
            let input = PhysicalInput::Edge { code, down: true };

            self.local_held.insert(code);
            result.outputs.push(RoutedOutput::Replay(input));
        }
        self.pending_modifiers.clear();
        self.captured_modifiers.clear();
    }

    fn flush_pending_local(&mut self, result: &mut RouteResult) {
        self.flush_pending_unmatched(result);
    }
}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::{
        command::MediaCommand,
        function::{FunctionConfig, ModifierSet, Shortcut, default_configs},
        input::MouseButton,
    };
    use std::time::Instant;

    fn event_at(code: InputCode, down: bool, captured: Instant) -> InputEvent {
        InputEvent {
            code,
            down,
            captured,
        }
    }

    fn event(code: InputCode, down: bool) -> InputEvent {
        event_at(code, down, Instant::now())
    }

    /// Every function output as `(action, down)`, so a test can assert both the
    /// gesture the router chose and its edge without matching the whole struct.
    fn outputs(result: &RouteResult) -> Vec<(FunctionAction, bool)> {
        result
            .outputs
            .iter()
            .filter_map(|output| match output {
                RoutedOutput::Function { action, down, .. } => Some((*action, *down)),
                _ => None,
            })
            .collect()
    }

    fn media(command: MediaCommand) -> FunctionAction {
        FunctionAction::Media(command)
    }

    fn router_with(shortcut: Shortcut, function: FunctionId) -> InputRouter {
        let mut configs = default_configs();
        configs.insert(
            function,
            FunctionConfig {
                enabled: true,
                shortcuts: vec![shortcut],
            },
        );
        let mut router = InputRouter::new(&configs, 1);
        router.set_listening(true);
        router
    }

    #[test]
    fn a_tap_only_function_still_fires_on_the_press_edge() {
        let mut router = router_with(
            Shortcut::keyboard(ModifierSet::empty(), 0x58),
            FunctionId::MediaPlayPause,
        );
        assert!(router.next_deadline().is_none());
        let pressed = router.route_event(event(InputCode::Key(0x58), true));
        assert_eq!(
            outputs(&pressed),
            vec![(media(MediaCommand::PlayPause), true)]
        );
        // Nothing waits for the release, so the platform never arms a timer.
        assert!(router.next_deadline().is_none());
        let released = router.route_event(event(InputCode::Key(0x58), false));
        assert!(released.consume && outputs(&released).is_empty());
    }

    #[test]
    fn a_quick_release_taps_the_press_gesture_instead_of_seeking() {
        let mut router = router_with(
            Shortcut::keyboard(ModifierSet::empty(), 0x58),
            FunctionId::MediaPrevious,
        );
        let pressed = Instant::now();
        let down = router.route_event(event_at(InputCode::Key(0x58), true, pressed));
        assert!(down.consume);
        assert!(
            outputs(&down).is_empty(),
            "a merged function must not commit to a gesture on the press edge"
        );
        assert!(router.next_deadline().is_some());

        let released = pressed + Duration::from_millis(80);
        let up = router.route_event(event_at(InputCode::Key(0x58), false, released));
        assert_eq!(outputs(&up), vec![(media(MediaCommand::Previous), true)]);
        assert!(router.next_deadline().is_none());
        // The deferred tap is decided on release, so its admission deadline
        // must start there rather than at the press.
        let RoutedOutput::Function { created, .. } = up.outputs.last().expect("tap") else {
            panic!("expected a function output");
        };
        assert_eq!(*created, released);
    }

    #[test]
    fn holding_past_the_threshold_escalates_to_the_hold_gesture() {
        let mut router = router_with(
            Shortcut::keyboard(ModifierSet::empty(), 0x58),
            FunctionId::MediaPrevious,
        );
        let pressed = Instant::now();
        router.route_event(event_at(InputCode::Key(0x58), true, pressed));
        assert!(
            outputs(&router.tick(pressed + HOLD_THRESHOLD - Duration::from_millis(1))).is_empty(),
            "the hold gesture is not due before the threshold"
        );
        let escalated = router.tick(pressed + HOLD_THRESHOLD);
        assert_eq!(
            outputs(&escalated),
            vec![(media(MediaCommand::Rewind), true)]
        );
        assert!(router.next_deadline().is_none());
        // A second tick must not press the held key twice.
        assert!(outputs(&router.tick(pressed + HOLD_THRESHOLD * 2)).is_empty());

        let up = router.route_event(event_at(
            InputCode::Key(0x58),
            false,
            pressed + HOLD_THRESHOLD * 2,
        ));
        assert_eq!(outputs(&up), vec![(media(MediaCommand::Rewind), false)]);
        assert!(
            !outputs(&up).contains(&(media(MediaCommand::Previous), true)),
            "a long press must not also skip a track"
        );
    }

    #[test]
    fn a_held_shortcut_release_after_its_window_closed_still_releases_the_seek() {
        let mut router = router_with(
            Shortcut::keyboard(ModifierSet::empty(), 0x58),
            FunctionId::MediaNext,
        );
        router.route_event(event(InputCode::Key(0x58), true));
        assert_eq!(
            outputs(&router.tick(Instant::now() + HOLD_THRESHOLD)),
            vec![(media(MediaCommand::FastForward), true)]
        );
        assert_eq!(
            outputs(&router.route_event(event(InputCode::Key(0x58), false))),
            vec![(media(MediaCommand::FastForward), false)]
        );
    }

    #[test]
    fn exact_primary_edge_matches_and_repeat_is_consumed_once() {
        let mut router = router_with(
            Shortcut::keyboard(
                ModifierSet {
                    ctrl: true,
                    ..Default::default()
                },
                0x58,
            ),
            FunctionId::MediaPlayPause,
        );
        assert!(
            router
                .route_event(event(InputCode::Key(0xa2), true))
                .consume
        );
        let matched = router.route_event(event(InputCode::Key(0x58), true));
        assert!(matched.consume);
        assert!(
            matched
                .outputs
                .iter()
                .any(|output| matches!(output, RoutedOutput::Function { down: true, .. }))
        );
        let repeat = router.route_event(event(InputCode::Key(0x58), true));
        assert!(repeat.consume);
        assert!(
            !repeat
                .outputs
                .iter()
                .any(|output| matches!(output, RoutedOutput::Function { .. }))
        );
    }

    #[test]
    fn a_modifier_added_after_primary_does_not_reclassify_the_edge() {
        let mut router = router_with(
            Shortcut::keyboard(
                ModifierSet {
                    ctrl: true,
                    shift: true,
                    ..Default::default()
                },
                0x58,
            ),
            FunctionId::MediaPlayPause,
        );
        router.route_event(event(InputCode::Key(0xa2), true));
        let plain = router.route_event(event(InputCode::Key(0x58), true));
        assert!(
            plain
                .outputs
                .iter()
                .any(|output| matches!(output, RoutedOutput::Local(_)))
        );
        router.route_event(event(InputCode::Key(0xa1), true));
        assert!(
            !router
                .route_event(event(InputCode::Key(0x58), true))
                .outputs
                .iter()
                .any(|output| matches!(output, RoutedOutput::Function { .. }))
        );
    }

    #[test]
    fn unmatched_modifier_prefix_is_replayed_in_physical_order() {
        let mut router = router_with(
            Shortcut::keyboard(
                ModifierSet {
                    ctrl: true,
                    shift: true,
                    ..Default::default()
                },
                0x58,
            ),
            FunctionId::MediaNext,
        );
        router.route_event(event(InputCode::Key(0xa2), true));
        router.route_event(event(InputCode::Key(0xa0), true));
        let release = router.route_event(event(InputCode::Key(0xa2), false));
        assert!(matches!(
            release.outputs.as_slice(),
            [
                RoutedOutput::Replay(PhysicalInput::Edge {
                    code: InputCode::Key(0xa2),
                    down: true
                }),
                RoutedOutput::Replay(PhysicalInput::Edge {
                    code: InputCode::Key(0xa0),
                    down: true
                }),
                RoutedOutput::Local(PhysicalInput::Edge {
                    code: InputCode::Key(0xa2),
                    down: false
                }),
            ]
        ));
    }

    #[test]
    fn termination_suppresses_consumed_modifier_tail_up() {
        let mut router = router_with(
            Shortcut::keyboard(
                ModifierSet {
                    ctrl: true,
                    ..Default::default()
                },
                0x58,
            ),
            FunctionId::MediaNext,
        );
        router.set_listening(true);
        router.route_event(event(InputCode::Key(0xa2), true));
        router.route_event(event(InputCode::Key(0x58), true));
        router.terminate(RouterReason::BeginRecording);
        let ctrl_up = router.route_event(event(InputCode::Key(0xa2), false));
        assert!(ctrl_up.consume && ctrl_up.outputs.is_empty());
    }

    #[test]
    fn hold_tokens_release_last_owner_only() {
        let mut configs = default_configs();
        let shortcut_a = Shortcut::mouse(ModifierSet::empty(), MouseButton::Side1);
        let shortcut_b = Shortcut::keyboard(ModifierSet::empty(), 0x58);
        configs.get_mut(&FunctionId::MediaPrevious).unwrap().enabled = true;
        configs
            .get_mut(&FunctionId::MediaPrevious)
            .unwrap()
            .shortcuts = vec![shortcut_a, shortcut_b];
        let mut router = InputRouter::new(&configs, 1);
        router.set_listening(true);
        let first = router.route_event(event(InputCode::Mouse(MouseButton::Side1), true));
        let second = router.route_event(event(InputCode::Key(0x58), true));
        assert!(
            outputs(&first).is_empty() && outputs(&second).is_empty(),
            "both shortcuts are still inside their tap window"
        );

        // Two shortcuts of one function share a single held seek.
        let escalated = router.tick(Instant::now() + HOLD_THRESHOLD);
        assert_eq!(
            outputs(&escalated),
            vec![(media(MediaCommand::Rewind), true)]
        );

        let release_one = router.route_event(event(InputCode::Mouse(MouseButton::Side1), false));
        assert!(
            !release_one
                .outputs
                .iter()
                .any(|o| matches!(o, RoutedOutput::Function { down: false, .. }))
        );
        let release_two = router.route_event(event(InputCode::Key(0x58), false));
        assert!(
            release_two
                .outputs
                .iter()
                .any(|o| matches!(o, RoutedOutput::Function { down: false, .. }))
        );
    }

    #[test]
    fn an_unmatched_modifier_is_replayed_before_its_local_release() {
        let mut router = router_with(
            Shortcut::keyboard(
                ModifierSet {
                    ctrl: true,
                    ..Default::default()
                },
                0x58,
            ),
            FunctionId::MediaNext,
        );
        let down = router.route_event(event(InputCode::Key(0xa2), true));
        assert!(down.consume && down.outputs.is_empty());
        let up = router.route_event(event(InputCode::Key(0xa2), false));
        assert!(!up.consume);
        assert!(matches!(
            up.outputs.as_slice(),
            [
                RoutedOutput::Replay(PhysicalInput::Edge {
                    code: InputCode::Key(0xa2),
                    down: true
                }),
                RoutedOutput::Local(PhysicalInput::Edge {
                    code: InputCode::Key(0xa2),
                    down: false
                }),
            ]
        ));
    }

    #[test]
    fn a_consumed_modifier_stays_hidden_until_a_normal_input_needs_it() {
        let mut router = router_with(
            Shortcut::keyboard(
                ModifierSet {
                    ctrl: true,
                    ..Default::default()
                },
                0x58,
            ),
            FunctionId::MediaNext,
        );
        router.route_event(event(InputCode::Key(0xa2), true));
        router.route_event(event(InputCode::Key(0x58), true));
        router.route_event(event(InputCode::Key(0x58), false));
        let ctrl_up = router.route_event(event(InputCode::Key(0xa2), false));
        assert!(ctrl_up.consume && ctrl_up.outputs.is_empty());
    }

    #[test]
    fn ordinary_input_after_a_function_replays_the_still_held_modifier() {
        let mut router = router_with(
            Shortcut::keyboard(
                ModifierSet {
                    ctrl: true,
                    ..Default::default()
                },
                0x58,
            ),
            FunctionId::MediaNext,
        );
        router.route_event(event(InputCode::Key(0xa2), true));
        router.route_event(event(InputCode::Key(0x58), true));
        router.route_event(event(InputCode::Key(0x58), false));
        let ordinary = router.route_event(event(InputCode::Key(0x43), true));
        assert!(ordinary.outputs.iter().any(|output| matches!(
            output,
            RoutedOutput::Replay(PhysicalInput::Edge {
                code: InputCode::Key(0xa2),
                down: true
            })
        )));
        assert!(ordinary.outputs.iter().any(|output| matches!(
            output,
            RoutedOutput::Local(PhysicalInput::Edge {
                code: InputCode::Key(0x43),
                down: true
            })
        )));
    }

    #[test]
    fn editing_an_active_binding_releases_it_and_old_key_up_stays_consumed() {
        let old = Shortcut::keyboard(ModifierSet::empty(), 0x58);
        let new = Shortcut::keyboard(ModifierSet::empty(), 0x59);
        let mut configs = default_configs();
        configs.get_mut(&FunctionId::MediaPrevious).unwrap().enabled = true;
        configs
            .get_mut(&FunctionId::MediaPrevious)
            .unwrap()
            .shortcuts = vec![old.clone()];
        let mut router = InputRouter::new(&configs, 1);
        router.set_listening(true);
        router.route_event(event(InputCode::Key(0x58), true));
        assert_eq!(
            outputs(&router.tick(Instant::now() + HOLD_THRESHOLD)),
            vec![(media(MediaCommand::Rewind), true)]
        );

        configs
            .get_mut(&FunctionId::MediaPrevious)
            .unwrap()
            .shortcuts = vec![new];
        let cleanup = router.update_config(&configs, 2);
        assert_eq!(
            outputs(&cleanup),
            vec![(media(MediaCommand::Rewind), false)]
        );
        let old_up = router.route_event(event(InputCode::Key(0x58), false));
        assert!(old_up.consume && old_up.outputs.is_empty());
    }

    #[test]
    fn editing_a_pending_tap_window_cancels_it_without_cleanup() {
        let old = Shortcut::keyboard(ModifierSet::empty(), 0x58);
        let new = Shortcut::keyboard(ModifierSet::empty(), 0x59);
        let mut configs = default_configs();
        configs.get_mut(&FunctionId::MediaPrevious).unwrap().enabled = true;
        configs
            .get_mut(&FunctionId::MediaPrevious)
            .unwrap()
            .shortcuts = vec![old];
        let mut router = InputRouter::new(&configs, 1);
        router.set_listening(true);
        router.route_event(event(InputCode::Key(0x58), true));

        configs
            .get_mut(&FunctionId::MediaPrevious)
            .unwrap()
            .shortcuts = vec![new];
        let cleanup = router.update_config(&configs, 2);
        assert!(
            outputs(&cleanup).is_empty(),
            "a tap that never became a hold owns no key to release"
        );
        assert!(router.next_deadline().is_none());
        let old_up = router.route_event(event(InputCode::Key(0x58), false));
        assert!(old_up.consume && old_up.outputs.is_empty());
    }

    #[test]
    fn editing_a_captured_prefix_does_not_replay_its_modifier() {
        let shortcut = Shortcut::keyboard(
            ModifierSet {
                ctrl: true,
                ..Default::default()
            },
            0x58,
        );
        let mut router = router_with(shortcut, FunctionId::MediaPrevious);
        router.route_event(event(InputCode::Key(0xa2), true));
        router.route_event(event(InputCode::Key(0x58), true));

        let mut configs = default_configs();
        configs.get_mut(&FunctionId::MediaPrevious).unwrap().enabled = false;
        let cleanup = router.update_config(&configs, 2);
        assert!(!cleanup.outputs.iter().any(|output| matches!(
            output,
            RoutedOutput::Replay(PhysicalInput::Edge {
                code: InputCode::Key(0xa2),
                down: true
            })
        )));
        let ctrl_up = router.route_event(event(InputCode::Key(0xa2), false));
        assert!(ctrl_up.consume && ctrl_up.outputs.is_empty());
    }

    #[test]
    fn local_window_event_bypasses_function_matching_and_keeps_prefix_order() {
        let mut router = router_with(
            Shortcut::keyboard(
                ModifierSet {
                    ctrl: true,
                    ..Default::default()
                },
                0x58,
            ),
            FunctionId::MediaNext,
        );
        router.route_event(event(InputCode::Key(0xa2), true));
        let down = router.route_local_event(event(InputCode::Key(0x58), true));
        assert!(!down.consume);
        assert!(matches!(
            down.outputs.as_slice(),
            [
                RoutedOutput::Replay(PhysicalInput::Edge {
                    code: InputCode::Key(0xa2),
                    down: true
                }),
                RoutedOutput::Local(PhysicalInput::Edge {
                    code: InputCode::Key(0x58),
                    down: true
                }),
            ]
        ));
        let up = router.route_local_event(event(InputCode::Key(0x58), false));
        assert!(matches!(
            up.outputs.as_slice(),
            [RoutedOutput::Local(PhysicalInput::Edge {
                code: InputCode::Key(0x58),
                down: false
            })]
        ));
        let ctrl_up = router.route_local_event(event(InputCode::Key(0xa2), false));
        assert!(matches!(
            ctrl_up.outputs.as_slice(),
            [RoutedOutput::Local(PhysicalInput::Edge {
                code: InputCode::Key(0xa2),
                down: false
            })]
        ));
    }
}
