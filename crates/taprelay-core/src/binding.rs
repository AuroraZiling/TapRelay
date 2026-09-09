use crate::{
    command::MediaCommand,
    input::{CompiledTrigger, InputCode, InputState, Trigger},
};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Binding {
    pub tap: Trigger,
    // Keep schema 2 fields for existing files. The GUI currently exposes only
    // PlayPause and normalizes all listed bindings to enabled at startup.
    pub relay: MediaCommand,
    pub enabled: bool,
}
/// No arbitrary UI limit; duplicate physical triggers remain invalid.
pub fn valid(bindings: &[Binding]) -> bool {
    // Small lists avoid a hash-table allocation; the quadratic branch is capped.
    if bindings.len() <= 16 {
        return bindings.iter().enumerate().all(|(i, b)| {
            b.tap.valid() && !bindings[..i].iter().any(|previous| previous.tap == b.tap)
        });
    }
    let mut seen = std::collections::HashSet::with_capacity(bindings.len());
    bindings
        .iter()
        .all(|b| b.tap.valid() && seen.insert(&b.tap))
}

/// Candidate lookup avoids scanning unrelated bindings on every physical edge.
pub struct BindingIndex {
    candidates: [Vec<usize>; 261],
    triggers: Vec<CompiledTrigger>,
}
impl BindingIndex {
    pub fn new(bindings: &[Binding]) -> Self {
        let mut index = Self {
            candidates: std::array::from_fn(|_| Vec::new()),
            triggers: bindings
                .iter()
                .map(|b| CompiledTrigger::new(&b.tap))
                .collect(),
        };
        for (i, binding) in bindings.iter().enumerate().filter(|(_, b)| b.enabled) {
            match &binding.tap {
                Trigger::Keyboard { keys } => {
                    for &key in keys {
                        index.candidates[key as usize].push(i);
                    }
                }
                Trigger::Mouse { button } | Trigger::Mixed { button, .. } => {
                    index.candidates[InputCode::Mouse(*button).index()].push(i);
                }
            }
        }
        index
    }
    pub fn best_match(&self, code: InputCode, state: &InputState) -> Option<usize> {
        self.candidates[code.index()]
            .iter()
            .copied()
            .filter(|&i| self.triggers[i].matches(state))
            .max_by_key(|&i| self.triggers[i].specificity)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::{InputEvent, MouseButton};
    #[test]
    fn indexed_matching_agrees_with_reference_for_every_edge() {
        let bindings = vec![
            Binding {
                tap: Trigger::Keyboard { keys: vec![0x77] },
                relay: MediaCommand::PlayPause,
                enabled: true,
            },
            Binding {
                tap: Trigger::Keyboard {
                    keys: vec![0x77, 0xa2],
                },
                relay: MediaCommand::PlayPause,
                enabled: true,
            },
            Binding {
                tap: Trigger::Mouse {
                    button: MouseButton::Side1,
                },
                relay: MediaCommand::PlayPause,
                enabled: true,
            },
            Binding {
                tap: Trigger::Mixed {
                    modifiers: vec![0xa2],
                    button: MouseButton::Side1,
                },
                relay: MediaCommand::PlayPause,
                enabled: true,
            },
        ];
        let index = BindingIndex::new(&bindings);
        let mut state = InputState::default();
        let mut seed = 42u64;
        for _ in 0..4096 {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1);
            let code = [
                InputCode::Key(0x77),
                InputCode::Key(0xa2),
                InputCode::Key(0x41),
                InputCode::Mouse(MouseButton::Side1),
            ][(seed >> 32) as usize % 4];
            state.update(InputEvent {
                code,
                down: seed & 0x100 != 0,
                captured: std::time::Instant::now(),
            });
            let expected = bindings
                .iter()
                .enumerate()
                .filter(|(_, b)| b.enabled && b.tap.contains(code) && state.matches(&b.tap))
                .max_by_key(|(_, b)| b.tap.specificity())
                .map(|(i, _)| i);
            assert_eq!(index.best_match(code, &state), expected);
        }
        assert!(valid(&bindings));
        let mut duplicate = bindings.clone();
        duplicate.push(bindings[0].clone());
        assert!(!valid(&duplicate));
    }
}
