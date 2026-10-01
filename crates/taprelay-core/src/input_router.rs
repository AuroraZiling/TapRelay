//! A single, platform-neutral input routing state machine.
//!
//! The Windows adapter supplies physical edges and consumes the boolean
//! decision in [`RouteResult`]. Output delivery is intentionally represented
//! as values so the adapter can enqueue it without making the hook wait for
//! Bluetooth, disk, or UI work.

use crate::{
    binding::{BindingIndex, BindingKey},
    function::{FunctionAction, FunctionConfigs, FunctionId, function_definition},
    input::{InputCode, InputEvent, InputState, MouseButton},
};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

/// How long a merged press/hold shortcut must stay down before its long-press
/// gesture takes over. Time is the only thing that separates a tapped
/// "previous" from a held "rewind", so the router must be able to wait.
pub const HOLD_THRESHOLD: Duration = Duration::from_millis(400);
pub const VOLUME_REPEAT_INTERVAL: Duration = Duration::from_millis(100);

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
    Mapping {
        id: u64,
        output: crate::mapping::MappingOutput,
        down: bool,
        token: u64,
        created: Instant,
    },
    Local(PhysicalInput),
    Remote(PhysicalInput),
    EndPassthrough,
    Replay(PhysicalInput),
    Function {
        binding: BindingKey,
        action: FunctionAction,
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
    token: u64,
    revision: u64,
    created: Instant,
    /// Next hold promotion or volume repeat; `None` when nothing is due.
    deadline: Option<Instant>,
    /// The hold gesture was emitted and owns one held output.
    hold_started: bool,
}

/// Owns physical state, exact shortcut matching, captured lifetimes, and the
/// function lifetimes. Callers publish a new config at a revision
/// boundary and then feed subsequent physical events through `route`.
pub struct InputRouter {
    mappings: Vec<crate::mapping::CustomMapping>,
    mapping_candidates: BTreeMap<InputCode, Vec<usize>>,
    mapping_paths: BTreeMap<u64, BTreeSet<String>>,
    mapping_active: BTreeMap<InputCode, (crate::mapping::CustomMapping, u64)>,
    mapping_rules: crate::foreground_app::ForegroundAppRules,
    foreground_app_scopes: crate::foreground_app::ScopeIndex,
    foreground: Option<String>,
    index: BindingIndex,
    revision: u64,
    listening: bool,
    passthrough: bool,
    recording: bool,
    physical: InputState,
    active: BTreeMap<InputCode, ActiveToken>,
    held_functions: BTreeMap<FunctionId, usize>,
    local_held: BTreeSet<InputCode>,
    suppressed_until_up: BTreeSet<InputCode>,
    next_token: u64,
}

impl InputRouter {
    pub fn new(configs: &FunctionConfigs, revision: u64) -> Self {
        Self {
            mappings: Vec::new(),
            mapping_candidates: Default::default(),
            mapping_paths: Default::default(),
            mapping_active: BTreeMap::new(),
            mapping_rules: Default::default(),
            foreground_app_scopes: Default::default(),
            foreground: None,
            index: BindingIndex::new(configs),
            revision,
            listening: false,
            passthrough: false,
            recording: false,
            physical: InputState::default(),
            active: BTreeMap::new(),
            held_functions: BTreeMap::new(),
            local_held: BTreeSet::new(),
            suppressed_until_up: BTreeSet::new(),
            next_token: 0,
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn set_foreground_app_rules(
        &mut self,
        rules: &crate::foreground_app::ForegroundAppRules,
    ) -> RouteResult {
        self.mapping_rules = rules.clone();
        self.index_mappings();
        self.foreground_app_scopes = crate::foreground_app::ScopeIndex::new(rules);
        self.cancel_out_of_scope()
    }

    pub fn set_foreground(&mut self, executable: Option<&str>) -> RouteResult {
        // Native discovery caches canonical identities; the common unchanged
        // foreground path needs neither allocation nor another scope lookup.
        if self.foreground.as_deref() == executable {
            return self.stamp(RouteResult::default());
        }
        let foreground = executable.map(crate::foreground_app::executable_identity);
        if self.foreground == foreground {
            return self.stamp(RouteResult::default());
        }
        self.foreground = foreground;
        self.cancel_out_of_scope()
    }

    fn cancel_out_of_scope(&mut self) -> RouteResult {
        let codes: Vec<_> = self
            .active
            .iter()
            .filter_map(|(&code, active)| {
                (!self
                    .foreground_app_scopes
                    .allows(active.binding.function, self.foreground.as_deref()))
                .then_some(code)
            })
            .collect();
        let mut result = RouteResult::default();
        let mapped: Vec<_> = self
            .mapping_active
            .iter()
            .filter_map(|(&code, (mapping, _))| (!self.mapping_allows(mapping)).then_some(code))
            .collect();
        for code in mapped {
            self.release_mapping(code, &mut result, Instant::now());
            self.suppressed_until_up.insert(code);
        }
        for code in codes {
            let active = self.active.remove(&code).expect("active token");
            if active.hold_started {
                self.release_hold(&active, &mut result, Instant::now());
            }
            self.suppressed_until_up.insert(code);
        }
        self.stamp(result)
    }

    pub fn passthrough(&self) -> bool {
        self.passthrough
    }

    pub fn set_passthrough(&mut self, enabled: bool) -> RouteResult {
        if self.passthrough == enabled {
            return self.stamp(RouteResult::default());
        }
        let mut result = self.terminate(RouterReason::SessionChanged);
        for code in std::mem::take(&mut self.local_held) {
            result
                .outputs
                .push(RoutedOutput::Replay(PhysicalInput::Edge {
                    code,
                    down: false,
                }));
        }
        self.isolate_held();
        self.passthrough = enabled;
        result
    }

    fn isolate_held(&mut self) {
        for key in 0..=255 {
            let code = InputCode::Key(key);
            if self.physical.is_down(code) {
                self.suppressed_until_up.insert(code);
            }
        }
        for button in MouseButton::ALL {
            let code = InputCode::Mouse(button);
            if self.physical.is_down(code) {
                self.suppressed_until_up.insert(code);
            }
        }
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
        let next_index = BindingIndex::new(configs);
        let active_codes: Vec<_> = self
            .active
            .iter()
            .filter_map(|(&code, token)| {
                let replacement = next_index
                    .find(token.binding)
                    .and_then(|index| next_index.binding(index))
                    .is_none_or(|binding| binding.shortcut != token.shortcut);
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
        if self.passthrough
            && !configs
                .get(&FunctionId::AppTogglePassthrough)
                .is_some_and(|config| config.enabled && !config.shortcuts.is_empty())
        {
            cleanup.outputs.extend(self.set_passthrough(false).outputs);
        }
        self.stamp(cleanup)
    }

    pub fn update_mappings(&mut self, mappings: &[crate::mapping::CustomMapping]) -> RouteResult {
        let mut result = RouteResult::default();
        let changed: Vec<_> = self
            .mapping_active
            .iter()
            .filter_map(|(&code, (active, _))| {
                mappings
                    .iter()
                    .find(|mapping| mapping.id == active.id)
                    .is_none_or(|mapping| {
                        !mapping.enabled
                            || mapping.output != active.output
                            || mapping.shortcuts != active.shortcuts
                            || mapping.groups != active.groups
                    })
                    .then_some(code)
            })
            .collect();
        for code in changed {
            self.release_mapping(code, &mut result, Instant::now());
            self.suppressed_until_up.insert(code);
        }
        self.mappings = mappings.to_vec();
        self.index_mappings();
        self.stamp(result)
    }

    fn index_mappings(&mut self) {
        self.mapping_candidates.clear();
        self.mapping_paths.clear();
        for (index, mapping) in self.mappings.iter().enumerate() {
            if mapping.enabled {
                for shortcut in &mapping.shortcuts {
                    self.mapping_candidates
                        .entry(shortcut.primary_code())
                        .or_default()
                        .push(index);
                }
            }
            if !mapping.groups.is_empty() {
                self.mapping_paths.insert(
                    mapping.id,
                    crate::mapping::scope_paths(&self.mapping_rules, &mapping.groups),
                );
            }
        }
    }

    fn mapping_allows(&self, mapping: &crate::mapping::CustomMapping) -> bool {
        mapping.groups.is_empty()
            || self.foreground.as_ref().is_some_and(|path| {
                self.mapping_paths
                    .get(&mapping.id)
                    .is_some_and(|paths| paths.contains(path))
            })
    }

    fn release_mapping(&mut self, code: InputCode, result: &mut RouteResult, created: Instant) {
        if let Some((mapping, token)) = self.mapping_active.remove(&code) {
            result.outputs.push(RoutedOutput::Mapping {
                id: mapping.id,
                output: mapping.output,
                down: false,
                token,
                created,
            });
        }
    }

    pub fn terminate(&mut self, _reason: RouterReason) -> RouteResult {
        let mut result = RouteResult {
            revision: self.revision,
            consume: true,
            outputs: Vec::new(),
        };
        let mapped: Vec<_> = self.mapping_active.keys().copied().collect();
        for code in mapped {
            self.release_mapping(code, &mut result, Instant::now());
            self.suppressed_until_up.insert(code);
        }
        if self.passthrough {
            self.passthrough = false;
            self.isolate_held();
            result.outputs.push(RoutedOutput::EndPassthrough);
        }
        for active in self.active.values() {
            // The corresponding down was consumed by the hook. Keep its
            // trailing physical up consumed even if the session ends before
            // the application has applied the cleanup outputs.
            self.suppressed_until_up
                .insert(active.shortcut.primary_code());
            if active.hold_started
                && let Some(action) = function_definition(active.binding.function).hold_action
            {
                result.outputs.push(RoutedOutput::Function {
                    binding: active.binding,
                    action,
                    down: false,
                    token: active.token,
                    revision: active.revision,
                    created: active.created,
                });
            }
        }
        self.active.clear();
        self.held_functions.clear();
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
    /// App control toggles can start here; active passthrough owns
    /// the window's input just like every other window.
    pub fn route_local_event(&mut self, event: InputEvent) -> RouteResult {
        if self.passthrough {
            return self.route_event(event);
        }
        if event.down
            && !self.recording
            && self
                .index
                .best_match_where(event.code, &self.physical, |id| {
                    matches!(
                        id,
                        FunctionId::AppToggleListening | FunctionId::AppTogglePassthrough
                    )
                })
                .is_some()
        {
            return self.route_event(event);
        }
        if self.mapping_active.contains_key(&event.code) {
            self.physical.update(event);
            let mut result = RouteResult {
                consume: true,
                ..Default::default()
            };
            if !event.down {
                self.release_mapping(event.code, &mut result, event.captured);
            }
            return self.stamp(result);
        }
        let changed = self.physical.update(event);
        self.cancel_unmatched_repeats();
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
        let result = self.normal_input(event);
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
        if self.passthrough {
            return RouteResult {
                consume: true,
                outputs: vec![RoutedOutput::Remote(input)],
                revision: self.revision,
            };
        }
        RouteResult::pass(input)
    }

    fn route_edge(&mut self, event: InputEvent) -> RouteResult {
        let changed = self.physical.update(event);
        self.cancel_unmatched_repeats();
        if !changed {
            if self.passthrough {
                return self.stamp(RouteResult {
                    consume: true,
                    ..Default::default()
                });
            }
            if self.mapping_active.contains_key(&event.code)
                || self.active.contains_key(&event.code)
                || self.suppressed_until_up.contains(&event.code)
            {
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

        if self.recording {
            return self.normal_input(event);
        }

        if self.mapping_active.contains_key(&event.code) {
            let mut result = RouteResult {
                consume: true,
                ..Default::default()
            };
            if !event.down {
                self.release_mapping(event.code, &mut result, event.captured);
            }
            return result;
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
            && let Some(index) = self
                .index
                .best_match_where(event.code, &self.physical, |id| {
                    self.foreground_app_scopes
                        .allows(id, self.foreground.as_deref())
                        && (self.listening
                            || self.passthrough
                            || matches!(
                                id,
                                FunctionId::AppToggleListening | FunctionId::AppTogglePassthrough
                            ))
                })
        {
            let binding = self.index.binding(index).expect("index entry").clone();
            return self.activate(binding, event.captured);
        }

        if event.down && self.listening {
            let mapping = self
                .mapping_candidates
                .get(&event.code)
                .into_iter()
                .flatten()
                .map(|index| &self.mappings[*index])
                .find(|mapping| {
                    self.mapping_allows(mapping)
                        && mapping.shortcuts.iter().any(|shortcut| {
                            shortcut.primary_code() == event.code
                                && shortcut.modifiers == self.physical.logical_modifiers()
                        })
                })
                .cloned();
            if let Some(mapping) = mapping {
                self.next_token = self.next_token.wrapping_add(1).max(1);
                let token = self.next_token;
                let output = RoutedOutput::Mapping {
                    id: mapping.id,
                    output: mapping.output,
                    down: true,
                    token,
                    created: event.captured,
                };
                self.mapping_active.insert(event.code, (mapping, token));
                return RouteResult {
                    consume: true,
                    outputs: vec![output],
                    ..Default::default()
                };
            }
        }
        self.normal_input(event)
    }

    fn cancel_unmatched_repeats(&mut self) {
        let modifiers = self.physical.logical_modifiers();
        self.active.retain(|code, active| {
            if function_definition(active.binding.function).repeats()
                && active.shortcut.modifiers != modifiers
            {
                self.suppressed_until_up.insert(*code);
                false
            } else {
                true
            }
        });
    }

    fn normal_input(&mut self, event: InputEvent) -> RouteResult {
        if self.passthrough {
            return RouteResult {
                revision: self.revision,
                consume: true,
                outputs: vec![RoutedOutput::Remote(PhysicalInput::Edge {
                    code: event.code,
                    down: event.down,
                })],
            };
        }
        if event.down {
            self.local_held.insert(event.code);
        } else {
            self.local_held.remove(&event.code);
        }
        RouteResult::pass(PhysicalInput::Edge {
            code: event.code,
            down: event.down,
        })
    }

    fn activate(
        &mut self,
        binding: crate::binding::IndexedBinding,
        created: Instant,
    ) -> RouteResult {
        self.next_token = self.next_token.wrapping_add(1).max(1);
        let token = self.next_token;
        let definition = function_definition(binding.key.function);
        let mut result = RouteResult {
            revision: self.revision,
            consume: true,
            outputs: vec![RoutedOutput::Feedback {
                binding: binding.key,
                action: definition.tap_action,
            }],
        };

        // A merged function cannot know which of its two gestures the user
        // meant yet: the tap is emitted on release, and the hold gesture
        // becomes due at the threshold. A plain tap has nothing to wait for,
        // so it keeps firing on the press edge.
        let deadline = (definition.hold_action.is_some() || definition.repeats())
            .then_some(created + HOLD_THRESHOLD);
        if definition.hold_action.is_none() {
            result.outputs.push(RoutedOutput::Function {
                binding: binding.key,
                action: definition.tap_action,
                down: true,
                token,
                revision: self.revision,
                created,
            });
        }
        self.active.insert(
            binding.shortcut.primary_code(),
            ActiveToken {
                binding: binding.key,
                shortcut: binding.shortcut,
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
    /// output. The platform arms its wake-up from this and supplies tick times.
    pub fn next_deadline(&self) -> Option<Instant> {
        self.active
            .values()
            .filter_map(|token| token.deadline)
            .min()
    }

    /// Promote held gestures and emit due volume repeats without catch-up bursts.
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
            if let Some(active) = self.active.get_mut(&code)
                && function_definition(active.binding.function).repeats()
            {
                active.deadline = Some(now + VOLUME_REPEAT_INTERVAL);
                result.outputs.push(RoutedOutput::Function {
                    binding: active.binding,
                    action: function_definition(active.binding.function).tap_action,
                    down: true,
                    token: active.token,
                    revision: active.revision,
                    created: now,
                });
                continue;
            }
            let Some((binding, action, token, revision)) =
                self.active.get(&code).and_then(|active| {
                    function_definition(active.binding.function)
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
        let definition = function_definition(active.binding.function);
        if definition.hold_action.is_none() {
            // The tap was already emitted on the press edge.
            return;
        }
        result.outputs.push(RoutedOutput::Function {
            binding: active.binding,
            action: definition.tap_action,
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
        let Some(action) = function_definition(active.binding.function).hold_action else {
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
                    down: false,
                    token: active.token,
                    revision: active.revision,
                    created,
                });
            }
        }
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

    fn custom_router() -> (InputRouter, crate::mapping::CustomMapping) {
        let mapping = crate::mapping::CustomMapping {
            id: 1,
            name: "Speed".into(),
            enabled: true,
            shortcuts: vec![Shortcut::mouse(ModifierSet::empty(), MouseButton::Side1)],
            output: crate::mapping::MappingOutput::Keyboard {
                usage: 0x4f,
                modifiers: 0,
            },
            groups: Default::default(),
        };
        let mut router = InputRouter::new(&default_configs(), 1);
        router.update_mappings(std::slice::from_ref(&mapping));
        router.set_listening(true);
        (router, mapping)
    }

    fn mapped(result: &RouteResult) -> Vec<(bool, u64)> {
        result
            .outputs
            .iter()
            .filter_map(|output| match output {
                RoutedOutput::Mapping { down, token, .. } => Some((*down, *token)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn mapping_starts_immediately_ignores_repeat_and_releases_in_local_window() {
        let (mut router, _) = custom_router();
        let code = InputCode::Mouse(MouseButton::Side1);
        let press = router.route_event(event(code, true));
        assert!(press.consume);
        let token = mapped(&press)[0].1;
        assert_eq!(mapped(&press), vec![(true, token)]);
        assert!(mapped(&router.route_event(event(code, true))).is_empty());
        assert!(router.next_deadline().is_none());
        assert!(!router.route_motion(20, 20).consume);
        assert_eq!(
            mapped(&router.route_local_event(event(code, false))),
            vec![(false, token)]
        );
        assert!(mapped(&router.route_local_event(event(code, true))).is_empty());
    }

    #[test]
    fn editing_a_held_mapping_releases_old_output_and_consumes_its_tail() {
        let (mut router, mut mapping) = custom_router();
        let code = InputCode::Mouse(MouseButton::Side1);
        router.route_event(event(code, true));
        mapping.output = crate::mapping::MappingOutput::Keyboard {
            usage: 0x50,
            modifiers: 0,
        };
        assert_eq!(mapped(&router.update_mappings(&[mapping])).len(), 1);
        let tail = router.route_event(event(code, false));
        assert!(tail.consume && mapped(&tail).is_empty());
        assert!(mapped(&router.route_event(event(code, true)))[0].0);
        assert!(!mapped(&router.set_recording(true))[0].0);
    }

    #[test]
    fn mapping_scope_exit_releases_and_requires_a_fresh_press() {
        let (mut router, mut mapping) = custom_router();
        let rules = scoped_rules(FunctionId::MediaPlayPause);
        mapping.groups.insert("games".into());
        router.update_mappings(&[mapping]);
        router.set_foreground_app_rules(&rules);
        router.set_foreground(Some("C:\\a.exe"));
        let code = InputCode::Mouse(MouseButton::Side1);
        router.route_event(event(code, true));
        assert!(!mapped(&router.set_foreground(Some("C:\\other.exe")))[0].0);
        router.set_foreground(Some("C:\\a.exe"));
        assert!(mapped(&router.route_event(event(code, true))).is_empty());
        router.route_event(event(code, false));
        assert!(mapped(&router.route_event(event(code, true)))[0].0);
        assert!(!mapped(&router.set_listening(false))[0].0);
    }

    #[test]
    fn mapped_input_takes_precedence_over_passthrough_without_duplicating_raw_input() {
        let (mut router, _) = custom_router();
        router.set_passthrough(true);
        let result = router.route_event(event(InputCode::Mouse(MouseButton::Side1), true));
        assert_eq!(mapped(&result).len(), 1);
        assert!(
            !result
                .outputs
                .iter()
                .any(|output| matches!(output, RoutedOutput::Remote(_)))
        );
        assert!(router.route_motion(10, 0).consume);
    }

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

    fn scoped_rules(id: FunctionId) -> crate::foreground_app::ForegroundAppRules {
        use crate::foreground_app::{ForegroundAppGroup, ForegroundAppRules};
        ForegroundAppRules {
            groups: BTreeMap::from([(
                "games".into(),
                ForegroundAppGroup {
                    name: "Games".into(),
                    foreground_apps: vec![r"C:\a.exe".into(), r"C:\b.exe".into()],
                },
            )]),
            assignments: BTreeMap::from([(id, ["games".into()].into())]),
        }
    }

    #[test]
    fn foreground_app_scope_preserves_same_group_volume_hold_and_stops_on_exit() {
        let code = InputCode::Mouse(MouseButton::Side1);
        let mut router = router_with(
            Shortcut::from_physical(&[], code).unwrap(),
            FunctionId::MediaVolumeUp,
        );
        router.set_foreground_app_rules(&scoped_rules(FunctionId::MediaVolumeUp));
        assert!(!router.route_event(event(code, true)).consume);
        router.route_event(event(code, false));
        router.set_foreground(Some(r"C:\a.exe"));
        let now = Instant::now();
        assert!(router.route_event(event_at(code, true, now)).consume);
        router.set_foreground(Some(r"C:\b.exe"));
        assert_eq!(
            outputs(&router.tick(now + HOLD_THRESHOLD)),
            [(media(MediaCommand::VolumeUp), true)]
        );
        router.set_foreground(None);
        assert!(outputs(&router.tick(now + HOLD_THRESHOLD * 2)).is_empty());
        router.set_foreground(Some(r"C:\a.exe"));
        assert!(outputs(&router.route_event(event(code, true))).is_empty());
        assert!(router.route_event(event(code, false)).consume);
        assert_eq!(
            outputs(&router.route_event(event(code, true))),
            [(media(MediaCommand::VolumeUp), true)]
        );
    }

    #[test]
    fn scope_exit_cancels_pending_tap_and_releases_active_seek_once() {
        let code = InputCode::Key(0x58);
        for held in [false, true] {
            let mut router = router_with(
                Shortcut::from_physical(&[], code).unwrap(),
                FunctionId::MediaNext,
            );
            let mut rules = scoped_rules(FunctionId::MediaNext);
            router.set_foreground_app_rules(&rules);
            router.set_foreground(Some(r"C:\a.exe"));
            let now = Instant::now();
            router.route_event(event_at(code, true, now));
            if held {
                router.tick(now + HOLD_THRESHOLD);
            }
            rules
                .groups
                .get_mut("games")
                .unwrap()
                .foreground_apps
                .clear();
            let cleanup = router.set_foreground_app_rules(&rules);
            assert_eq!(
                outputs(&cleanup),
                if held {
                    vec![(media(MediaCommand::FastForward), false)]
                } else {
                    vec![]
                }
            );
            assert!(outputs(&router.route_event(event(code, false))).is_empty());
            assert!(outputs(&router.set_foreground(None)).is_empty());
            assert!(!router.route_event(event(code, true)).consume);
        }
    }

    #[test]
    fn disjoint_scopes_route_shared_shortcut_and_keep_controls_global() {
        let code = InputCode::Key(0x58);
        let mut configs = default_configs();
        for id in [FunctionId::MediaPlayPause, FunctionId::MediaMute] {
            configs.get_mut(&id).unwrap().enabled = true;
            configs
                .get_mut(&id)
                .unwrap()
                .shortcuts
                .push(Shortcut::from_physical(&[], code).unwrap());
        }
        let mut rules = scoped_rules(FunctionId::MediaPlayPause);
        rules.groups.insert(
            "work".into(),
            crate::foreground_app::ForegroundAppGroup {
                name: "Work".into(),
                foreground_apps: vec![r"C:\c.exe".into()],
            },
        );
        rules
            .assignments
            .insert(FunctionId::MediaMute, ["work".into()].into());
        assert!(rules.valid_bindings(&configs));
        let mut router = InputRouter::new(&configs, 1);
        router.set_listening(true);
        router.set_foreground_app_rules(&rules);
        for (path, command) in [
            (r"C:\a.exe", MediaCommand::PlayPause),
            (r"C:\c.exe", MediaCommand::Mute),
        ] {
            router.set_foreground(Some(path));
            assert_eq!(
                outputs(&router.route_event(event(code, true))),
                [(media(command), true)]
            );
            router.route_event(event(code, false));
        }
        let toggle = FunctionId::AppToggleListening;
        let mut router = router_with(Shortcut::from_physical(&[], code).unwrap(), toggle);
        router.set_foreground_app_rules(&rules);
        router.set_foreground(None);
        router.set_listening(false);
        assert!(!outputs(&router.route_event(event(code, true))).is_empty());
    }

    #[test]
    fn volume_press_repeats_on_timer_without_os_repeat_or_catch_up() {
        for code in [
            InputCode::Key(0x58),
            InputCode::Mouse(crate::input::MouseButton::Side1),
        ] {
            let shortcut = Shortcut::from_physical(&[], code).unwrap();
            let mut router = router_with(shortcut, FunctionId::MediaVolumeUp);
            let now = Instant::now();
            let expected = [(media(MediaCommand::VolumeUp), true)];
            assert_eq!(
                outputs(&router.route_event(event_at(code, true, now))),
                expected
            );
            assert!(outputs(&router.route_event(event_at(code, true, now))).is_empty());
            assert!(
                outputs(&router.tick(now + HOLD_THRESHOLD - Duration::from_millis(1))).is_empty()
            );
            assert_eq!(outputs(&router.tick(now + HOLD_THRESHOLD)), expected);
            let later = now + Duration::from_secs(5);
            assert_eq!(outputs(&router.tick(later)), expected);
            assert_eq!(router.next_deadline(), Some(later + VOLUME_REPEAT_INTERVAL));
            assert!(router.route_event(event_at(code, false, later)).consume);
            assert!(router.next_deadline().is_none());
            assert!(outputs(&router.tick(later + Duration::from_secs(1))).is_empty());
        }
    }

    #[test]
    fn volume_repeat_stops_on_modifier_release_and_lifecycle_changes() {
        for reason in [
            RouterReason::ListenerStopped,
            RouterReason::TransportLost,
            RouterReason::SessionChanged,
            RouterReason::BeginRecording,
            RouterReason::ApplicationExit,
        ] {
            let mut router = router_with(
                Shortcut::keyboard(ModifierSet::empty(), 0x58),
                FunctionId::MediaVolumeDown,
            );
            let now = Instant::now();
            router.route_event(event_at(InputCode::Key(0x58), true, now));
            router.terminate(reason);
            assert!(outputs(&router.tick(now + HOLD_THRESHOLD)).is_empty());
            assert!(
                router
                    .route_event(event(InputCode::Key(0x58), false))
                    .consume
            );
        }
        let mut router = router_with(
            Shortcut::keyboard(ModifierSet::from_keys([0x11]), 0x58),
            FunctionId::MediaVolumeDown,
        );
        router.route_event(event(InputCode::Key(0x11), true));
        router.route_event(event(InputCode::Key(0x58), true));
        assert!(
            !router
                .route_event(event(InputCode::Key(0x11), false))
                .consume
        );
        assert!(router.next_deadline().is_none());
        assert!(
            router
                .route_event(event(InputCode::Key(0x58), false))
                .consume
        );
    }

    #[test]
    fn volume_repeat_stops_when_binding_is_disabled_or_edited() {
        for disable in [true, false] {
            let mut router = router_with(
                Shortcut::keyboard(ModifierSet::empty(), 0x58),
                FunctionId::MediaVolumeUp,
            );
            router.route_event(event(InputCode::Key(0x58), true));
            let mut configs = default_configs();
            configs.insert(
                FunctionId::MediaVolumeUp,
                FunctionConfig {
                    enabled: !disable,
                    shortcuts: vec![Shortcut::keyboard(ModifierSet::empty(), 0x59)],
                },
            );
            router.update_config(&configs, 2);
            assert!(router.next_deadline().is_none());
            assert!(outputs(&router.tick(Instant::now() + HOLD_THRESHOLD)).is_empty());
        }
    }

    fn observe_host(host: &mut BTreeSet<InputCode>, result: RouteResult) {
        for output in result.outputs {
            match output {
                RoutedOutput::Replay(PhysicalInput::Edge { code, down })
                | RoutedOutput::Local(PhysicalInput::Edge { code, down }) => {
                    if down {
                        host.insert(code);
                    } else {
                        host.remove(&code);
                    }
                }
                _ => {}
            }
        }
    }

    #[test]
    fn repeated_modifier_taps_never_leave_a_host_key_down() {
        for key in [0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0x5b, 0x5c] {
            let mut router = router_with(
                Shortcut::mouse(ModifierSet::from_keys([key]), MouseButton::Side2),
                FunctionId::MediaPlayPause,
            );
            let mut host = BTreeSet::new();
            for _ in 0..2 {
                for down in [true, false] {
                    observe_host(
                        &mut host,
                        router.route_event(event(InputCode::Key(key), down)),
                    );
                }
            }
            observe_host(&mut host, router.route_motion(1, 0));
            observe_host(&mut host, router.route_wheel(120, 0));
            assert!(host.is_empty(), "released modifier {key:#x} remained down");
        }
    }

    #[test]
    fn sprint_modifier_reaches_host_immediately_with_movement_key_held() {
        let mut router = router_with(
            Shortcut::mouse(ModifierSet::from_keys([0xa0]), MouseButton::Side2),
            FunctionId::MediaPlayPause,
        );
        let mut host = BTreeSet::new();
        observe_host(
            &mut host,
            router.route_event(event(InputCode::Key(0x57), true)),
        );
        for _ in 0..2 {
            let down = router.route_event(event(InputCode::Key(0xa0), true));
            assert!(
                !down.consume,
                "Shift must not wait for mouse movement or a timer"
            );
            observe_host(&mut host, down);
            assert!(host.contains(&InputCode::Key(0xa0)));
            observe_host(
                &mut host,
                router.route_event(event(InputCode::Key(0xa0), false)),
            );
            assert!(!host.contains(&InputCode::Key(0xa0)));
        }
        assert_eq!(host, BTreeSet::from([InputCode::Key(0x57)]));
    }

    #[test]
    fn stopping_after_modifier_taps_never_injects_a_new_press() {
        for reason in [RouterReason::ApplicationExit, RouterReason::ListenerStopped] {
            let mut router = router_with(
                Shortcut::mouse(ModifierSet::from_keys([0xa0]), MouseButton::Side2),
                FunctionId::MediaPlayPause,
            );
            let mut host = BTreeSet::new();
            for _ in 0..2 {
                for down in [true, false] {
                    observe_host(
                        &mut host,
                        router.route_event(event(InputCode::Key(0xa0), down)),
                    );
                }
            }
            observe_host(&mut host, router.terminate(reason));
            observe_host(&mut host, router.terminate(RouterReason::ListenerStopped));
            assert!(host.is_empty(), "shutdown left a released key down");
        }
    }

    #[test]
    fn held_modifiers_keep_their_release_across_lifecycle_changes() {
        for key in [0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0x5b, 0x5c] {
            for reason in [
                RouterReason::BeginRecording,
                RouterReason::ListenerStopped,
                RouterReason::TransportLost,
                RouterReason::SessionChanged,
                RouterReason::ApplicationExit,
            ] {
                for local_release in [false, true] {
                    let mut router = router_with(
                        Shortcut::mouse(ModifierSet::from_keys([key]), MouseButton::Side2),
                        FunctionId::MediaPlayPause,
                    );
                    let code = InputCode::Key(key);
                    let mut host = BTreeSet::new();
                    observe_host(&mut host, router.route_event(event(code, true)));
                    assert!(host.contains(&code));
                    assert!(router.terminate(reason).outputs.is_empty());
                    let up = if local_release {
                        router.route_local_event(event(code, false))
                    } else {
                        router.route_event(event(code, false))
                    };
                    assert!(!up.consume);
                    observe_host(&mut host, up);
                    observe_host(&mut host, router.route_motion(1, 0));
                    assert!(host.is_empty());
                }
            }
        }
    }

    #[test]
    fn passthrough_releases_host_modifiers_and_isolates_their_physical_tail() {
        for key in [0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0x5b, 0x5c] {
            let mut router = router_with(
                Shortcut::mouse(ModifierSet::from_keys([key]), MouseButton::Side2),
                FunctionId::MediaPlayPause,
            );
            let code = InputCode::Key(key);
            let mut host = BTreeSet::new();
            observe_host(&mut host, router.route_event(event(code, true)));
            let start = router.set_passthrough(true);
            assert_eq!(
                start.outputs,
                [RoutedOutput::Replay(PhysicalInput::Edge {
                    code,
                    down: false
                })]
            );
            observe_host(&mut host, start);
            assert!(host.is_empty());
            router.set_passthrough(false);
            let up = router.route_event(event(code, false));
            assert!(up.consume && up.outputs.is_empty());
            for down in [true, false] {
                let result = router.route_event(event(code, down));
                assert!(!result.consume);
                observe_host(&mut host, result);
            }
            assert!(host.is_empty());
        }
    }

    #[test]
    fn standalone_primary_mouse_bindings_do_not_consume_clicks() {
        for button in [MouseButton::Left, MouseButton::Right] {
            for function in crate::function::function_ids() {
                let mut router =
                    router_with(Shortcut::mouse(ModifierSet::empty(), button), function);
                for down in [true, false] {
                    let result = router.route_event(event(InputCode::Mouse(button), down));
                    assert!(!result.consume);
                    assert!(outputs(&result).is_empty());
                }
            }
        }
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
            !router
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
    fn modifier_edges_pass_in_physical_order_without_replay() {
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
        for (key, down) in [(0xa2, true), (0xa0, true), (0xa2, false), (0xa0, false)] {
            let code = InputCode::Key(key);
            let result = router.route_event(event(code, down));
            assert!(!result.consume);
            assert_eq!(
                result.outputs,
                [RoutedOutput::Local(PhysicalInput::Edge { code, down })]
            );
        }
    }

    #[test]
    fn termination_releases_local_modifier_but_suppresses_captured_primary() {
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
        assert!(
            !router
                .route_event(event(InputCode::Key(0xa2), true))
                .consume
        );
        router.route_event(event(InputCode::Key(0x58), true));
        router.terminate(RouterReason::BeginRecording);
        let ctrl_up = router.route_event(event(InputCode::Key(0xa2), false));
        assert!(!ctrl_up.consume);
        assert_eq!(
            ctrl_up.outputs,
            [RoutedOutput::Local(PhysicalInput::Edge {
                code: InputCode::Key(0xa2),
                down: false
            })]
        );
        let primary_up = router.route_event(event(InputCode::Key(0x58), false));
        assert!(primary_up.consume && primary_up.outputs.is_empty());
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
    fn an_unmatched_modifier_passes_without_synthetic_input() {
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
        assert!(!down.consume);
        assert_eq!(
            down.outputs,
            [RoutedOutput::Local(PhysicalInput::Edge {
                code: InputCode::Key(0xa2),
                down: true
            })]
        );
        let up = router.route_event(event(InputCode::Key(0xa2), false));
        assert!(!up.consume);
        assert!(matches!(
            up.outputs.as_slice(),
            [RoutedOutput::Local(PhysicalInput::Edge {
                code: InputCode::Key(0xa2),
                down: false
            }),]
        ));
    }

    #[test]
    fn matching_a_shortcut_preserves_the_local_modifier_release() {
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
        assert!(
            !router
                .route_event(event(InputCode::Key(0xa2), true))
                .consume
        );
        assert!(
            router
                .route_event(event(InputCode::Key(0x58), true))
                .consume
        );
        assert!(
            router
                .route_event(event(InputCode::Key(0x58), false))
                .consume
        );
        let ctrl_up = router.route_event(event(InputCode::Key(0xa2), false));
        assert!(!ctrl_up.consume);
        assert_eq!(
            ctrl_up.outputs,
            [RoutedOutput::Local(PhysicalInput::Edge {
                code: InputCode::Key(0xa2),
                down: false
            })]
        );
    }

    #[test]
    fn ordinary_input_after_a_function_does_not_replay_the_modifier() {
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
        assert!(!ordinary.outputs.iter().any(|output| matches!(
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
    fn editing_a_shortcut_preserves_modifier_release_without_replay() {
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
        assert!(!ctrl_up.consume);
        assert_eq!(
            ctrl_up.outputs,
            [RoutedOutput::Local(PhysicalInput::Edge {
                code: InputCode::Key(0xa2),
                down: false
            })]
        );
    }

    #[test]
    fn local_window_event_bypasses_matching_and_preserves_modifier_release() {
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
            [RoutedOutput::Local(PhysicalInput::Edge {
                code: InputCode::Key(0x58),
                down: true
            }),]
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
    #[test]
    fn app_toggle_survives_pause_without_repeating_or_leaking_edges() {
        for shortcut in [
            Shortcut::keyboard(ModifierSet::empty(), 0x78),
            Shortcut::mouse(ModifierSet::empty(), crate::input::MouseButton::Side1),
            Shortcut::keyboard(
                ModifierSet {
                    ctrl: true,
                    ..Default::default()
                },
                0x78,
            ),
        ] {
            let mut router = router_with(shortcut.clone(), FunctionId::AppToggleListening);
            let action = FunctionAction::App(crate::function::AppCommand::ToggleListening);
            for listening in [false, true, false] {
                router.set_listening(listening);
                if shortcut.modifiers.ctrl {
                    assert!(
                        !router
                            .route_event(event(InputCode::Key(0x11), true))
                            .consume
                    );
                }
                let code = shortcut.primary_code();
                assert_eq!(
                    outputs(&router.route_event(event(code, true))),
                    [(action, true)]
                );
                router.set_listening(!listening);
                let repeat = router.route_event(event(code, true));
                assert!(repeat.consume);
                assert!(outputs(&repeat).is_empty());
                assert!(outputs(&router.tick(Instant::now() + HOLD_THRESHOLD)).is_empty());
                let release = router.route_event(event(code, false));
                assert!(release.consume);
                assert!(outputs(&release).is_empty());
                if shortcut.modifiers.ctrl {
                    assert!(
                        !router
                            .route_event(event(InputCode::Key(0x11), false))
                            .consume
                    );
                }
            }
        }
    }

    #[test]
    fn paused_media_modifiers_pass_through_and_recording_disables_app_shortcuts() {
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
        router.set_listening(false);
        for (code, down) in [(0x11, true), (0x58, true), (0x58, false), (0x11, false)] {
            let result = router.route_event(event(InputCode::Key(code), down));
            assert!(!result.consume);
            assert!(outputs(&result).is_empty());
        }
        let mut router = router_with(
            Shortcut::keyboard(ModifierSet::empty(), 0x78),
            FunctionId::AppToggleListening,
        );
        router.set_recording(true);
        for down in [true, false] {
            let result = router.route_event(event(InputCode::Key(0x78), down));
            assert!(!result.consume);
            assert!(outputs(&result).is_empty());
        }
    }
    #[test]
    fn app_pause_releases_media_hold_and_suppresses_its_remaining_edges() {
        let mut configs = default_configs();
        for (id, key) in [
            (FunctionId::MediaNext, 0x58),
            (FunctionId::AppToggleListening, 0x78),
        ] {
            configs.insert(
                id,
                FunctionConfig {
                    enabled: true,
                    shortcuts: vec![Shortcut::keyboard(ModifierSet::empty(), key)],
                },
            );
        }
        let mut router = InputRouter::new(&configs, 1);
        router.set_listening(true);
        let now = Instant::now();
        router.route_event(event_at(InputCode::Key(0x58), true, now));
        assert_eq!(
            outputs(&router.tick(now + HOLD_THRESHOLD)),
            [(media(MediaCommand::FastForward), true)]
        );
        assert_eq!(
            outputs(&router.route_event(event(InputCode::Key(0x78), true))),
            [(
                FunctionAction::App(crate::function::AppCommand::ToggleListening),
                true
            )]
        );
        assert_eq!(
            outputs(&router.set_listening(false)),
            [(media(MediaCommand::FastForward), false)]
        );
        assert!(outputs(&router.tick(now + HOLD_THRESHOLD * 2)).is_empty());
        for code in [0x58, 0x78] {
            let result = router.route_event(event(InputCode::Key(code), false));
            assert!(result.consume);
            assert!(outputs(&result).is_empty());
        }
        assert!(
            !router
                .route_event(event(InputCode::Key(0x58), true))
                .consume
        );
        assert_eq!(
            outputs(&router.route_event(event(InputCode::Key(0x78), true))).len(),
            1
        );
    }
}
