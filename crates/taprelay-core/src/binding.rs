//! Shortcut indexing and global binding invariants.

use crate::{
    function::{FunctionAction, FunctionConfigs, FunctionId, Shortcut, function_definition},
    input::{InputCode, InputState},
};
use std::collections::HashSet;

pub const MAX_SHORTCUTS_PER_FUNCTION: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BindingKey {
    pub function: FunctionId,
    pub slot: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IndexedBinding {
    pub key: BindingKey,
    pub shortcut: Shortcut,
    pub action: FunctionAction,
}

/// Disabled entries remain in the config and therefore remain in this global
/// uniqueness check. They are absent from the runtime index below.
pub fn valid(configs: &FunctionConfigs) -> bool {
    let mut seen = HashSet::new();
    configs.values().all(|config| {
        config.shortcuts.len() <= MAX_SHORTCUTS_PER_FUNCTION
            && config
                .shortcuts
                .iter()
                .all(|shortcut| shortcut.valid() && seen.insert(shortcut.clone()))
    })
}

/// Runtime index keyed by the only edge that can start a shortcut: its primary.
/// Matching is exact on the normalized modifier set and deliberately ignores
/// unrelated ordinary keys held by the user.
pub struct BindingIndex {
    candidates: [Vec<usize>; 261],
    bindings: Vec<IndexedBinding>,
}

impl BindingIndex {
    pub fn new(configs: &FunctionConfigs) -> Self {
        let mut index = Self {
            candidates: std::array::from_fn(|_| Vec::new()),
            bindings: Vec::new(),
        };
        for id in crate::function::function_ids() {
            let Some(config) = configs.get(&id) else {
                continue;
            };
            if !config.enabled {
                continue;
            }
            let definition = function_definition(id);
            for (slot, shortcut) in config.shortcuts.iter().cloned().enumerate() {
                let binding_index = index.bindings.len();
                index.candidates[shortcut.primary_code().index()].push(binding_index);
                index.bindings.push(IndexedBinding {
                    key: BindingKey { function: id, slot },
                    shortcut,
                    action: definition.action,
                });
            }
        }
        index
    }

    pub fn binding(&self, index: usize) -> Option<&IndexedBinding> {
        self.bindings.get(index)
    }

    pub fn iter(&self) -> impl Iterator<Item = &IndexedBinding> {
        self.bindings.iter()
    }

    pub fn best_match(&self, code: InputCode, state: &InputState) -> Option<usize> {
        self.candidates[code.index()]
            .iter()
            .copied()
            .find(|&index| {
                let binding = &self.bindings[index];
                binding.shortcut.primary_code() == code
                    && state.logical_modifiers() == binding.shortcut.modifiers
            })
    }

    pub fn find(&self, key: BindingKey) -> Option<usize> {
        self.bindings.iter().position(|binding| binding.key == key)
    }

    pub fn uses_modifier(&self, code: InputCode) -> bool {
        let InputCode::Key(key) = code else {
            return false;
        };
        let logical = crate::function::ModifierSet::from_keys([key]);
        self.bindings
            .iter()
            .any(|binding| logical.is_subset_of(binding.shortcut.modifiers))
    }
}

pub fn all_shortcuts(configs: &FunctionConfigs) -> impl Iterator<Item = (BindingKey, &Shortcut)> {
    configs.iter().flat_map(|(&function, config)| {
        config
            .shortcuts
            .iter()
            .enumerate()
            .map(move |(slot, shortcut)| (BindingKey { function, slot }, shortcut))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        function::{FunctionConfig, ModifierSet, PrimaryInput, default_configs},
        input::{InputEvent, MouseButton},
    };
    use std::time::Instant;

    fn shortcut(modifiers: ModifierSet, key: u8) -> Shortcut {
        Shortcut::new(modifiers, PrimaryInput::keyboard(key))
    }

    #[test]
    fn index_matches_only_primary_and_exact_logical_modifiers() {
        let mut configs = default_configs();
        configs.insert(
            FunctionId::MediaNext,
            FunctionConfig {
                enabled: true,
                shortcuts: vec![shortcut(ModifierSet::empty(), 0x58)],
            },
        );
        configs.insert(
            FunctionId::MediaPrevious,
            FunctionConfig {
                enabled: true,
                shortcuts: vec![shortcut(
                    ModifierSet {
                        ctrl: true,
                        ..Default::default()
                    },
                    0x58,
                )],
            },
        );
        let index = BindingIndex::new(&configs);
        let mut state = InputState::default();
        state.update(InputEvent {
            code: InputCode::Key(0x41),
            down: true,
            captured: Instant::now(),
        });
        state.update(InputEvent {
            code: InputCode::Key(0x58),
            down: true,
            captured: Instant::now(),
        });
        assert_eq!(index.best_match(InputCode::Key(0x58), &state), Some(1));
        state.update(InputEvent {
            code: InputCode::Key(0x11),
            down: true,
            captured: Instant::now(),
        });
        assert_eq!(index.best_match(InputCode::Key(0x58), &state), Some(0));
    }

    #[test]
    fn disabled_shortcuts_are_not_indexed_but_still_occupy_the_config() {
        let mut configs = default_configs();
        let shortcut = Shortcut::mouse(ModifierSet::empty(), MouseButton::Left);
        configs
            .get_mut(&FunctionId::MediaNext)
            .unwrap()
            .shortcuts
            .push(shortcut.clone());
        configs
            .get_mut(&FunctionId::MediaPrevious)
            .unwrap()
            .shortcuts
            .push(shortcut);
        assert!(!valid(&configs));
        let index = BindingIndex::new(&configs);
        assert_eq!(index.iter().count(), 0);
    }
}
