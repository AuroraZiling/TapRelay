//! User-defined shortcuts and persistent, device-independent HID outputs.
use crate::{
    foreground_app::{ForegroundAppRules, executable_identity},
    function::{FunctionConfigs, FunctionId, ModifierSet, PrimaryInput, Shortcut},
    input::MouseButton,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum MappingOutput {
    Keyboard { usage: u8, modifiers: u8 },
    Mouse { button: MouseButton },
}

impl MappingOutput {
    pub fn valid(self) -> bool {
        match self {
            Self::Keyboard { usage, .. } => output_keys().iter().any(|(_, u)| *u == usage),
            Self::Mouse { .. } => true,
        }
    }

    pub fn from_shortcut(shortcut: &Shortcut) -> Option<Self> {
        match shortcut.primary {
            PrimaryInput::Keyboard { key } => Some(Self::Keyboard {
                usage: virtual_key_usage(key)?,
                modifiers: u8::from(shortcut.modifiers.ctrl)
                    | (u8::from(shortcut.modifiers.shift) << 1)
                    | (u8::from(shortcut.modifiers.alt) << 2)
                    | (u8::from(shortcut.modifiers.win) << 3),
            }),
            PrimaryInput::Mouse { button } if shortcut.modifiers == ModifierSet::empty() => {
                Some(Self::Mouse { button })
            }
            _ => None,
        }
    }
}

/// Virtual-key labels are only used by the Windows editor. Config stores HID usages.
pub fn output_keys() -> Vec<(u8, u8)> {
    let mut keys = vec![
        (0x08, 0x2a),
        (0x09, 0x2b),
        (0x0d, 0x28),
        (0x1b, 0x29),
        (0x20, 0x2c),
        (0x21, 0x4b),
        (0x22, 0x4e),
        (0x23, 0x4d),
        (0x24, 0x4a),
        (0x25, 0x50),
        (0x26, 0x52),
        (0x27, 0x4f),
        (0x28, 0x51),
        (0x2d, 0x49),
        (0x2e, 0x4c),
        (0x14, 0x39),
        (0x2c, 0x46),
        (0x90, 0x53),
        (0x91, 0x47),
        (0x13, 0x48),
        (0xe0, 0x58),
        (0xba, 0x33),
        (0xbb, 0x2e),
        (0xbc, 0x36),
        (0xbd, 0x2d),
        (0xbe, 0x37),
        (0xbf, 0x38),
        (0xc0, 0x35),
        (0xdb, 0x2f),
        (0xdc, 0x31),
        (0xdd, 0x30),
        (0xde, 0x34),
        (0xe2, 0x64),
        (0x6a, 0x55),
        (0x6b, 0x57),
        (0x6d, 0x56),
        (0x6e, 0x63),
        (0x6f, 0x54),
    ];
    keys.extend((0x41..=0x5a).map(|vk| (vk, 4 + vk - 0x41)));
    keys.extend((0x31..=0x39).map(|vk| (vk, 0x1e + vk - 0x31)));
    keys.push((0x30, 0x27));
    keys.extend((0x70..=0x7b).map(|vk| (vk, 0x3a + vk - 0x70)));
    keys.extend((0x7c..=0x87).map(|vk| (vk, 0x68 + vk - 0x7c)));
    keys.extend((0x61..=0x69).map(|vk| (vk, 0x59 + vk - 0x61)));
    keys.push((0x60, 0x62));
    keys
}

pub fn virtual_key_usage(key: u8) -> Option<u8> {
    output_keys()
        .into_iter()
        .find_map(|(vk, usage)| (vk == key).then_some(usage))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomMapping {
    pub id: u64,
    pub name: String,
    pub enabled: bool,
    pub shortcuts: Vec<Shortcut>,
    pub output: MappingOutput,
    /// Empty means all computer foreground applications.
    #[serde(default)]
    pub groups: BTreeSet<String>,
}

pub fn scopes_overlap(
    rules: &ForegroundAppRules,
    a: &BTreeSet<String>,
    b: &BTreeSet<String>,
) -> bool {
    a.is_empty()
        || b.is_empty()
        || !a.is_disjoint(b)
        || !scope_paths(rules, a).is_disjoint(&scope_paths(rules, b))
}

pub(crate) fn scope_paths(
    rules: &ForegroundAppRules,
    groups: &BTreeSet<String>,
) -> BTreeSet<String> {
    groups
        .iter()
        .filter_map(|id| rules.groups.get(id))
        .flat_map(|group| &group.foreground_apps)
        .map(|path| executable_identity(path))
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuleId {
    Function(FunctionId),
    Mapping(u64),
}

pub fn conflict(
    functions: &FunctionConfigs,
    rules: &ForegroundAppRules,
    mappings: &[CustomMapping],
) -> Option<(RuleId, RuleId)> {
    let mut entries = Vec::new();
    for (&id, config) in functions {
        let groups = rules.assignments.get(&id).cloned().unwrap_or_default();
        for shortcut in &config.shortcuts {
            entries.push((RuleId::Function(id), shortcut, groups.clone()));
        }
    }
    for mapping in mappings {
        for shortcut in &mapping.shortcuts {
            entries.push((
                RuleId::Mapping(mapping.id),
                shortcut,
                mapping.groups.clone(),
            ));
        }
    }
    for (index, (id, shortcut, groups)) in entries.iter().enumerate() {
        for (other, candidate, other_groups) in &entries[..index] {
            if shortcut == candidate && scopes_overlap(rules, groups, other_groups) {
                return Some((*other, *id));
            }
        }
    }
    None
}

pub fn valid(
    functions: &FunctionConfigs,
    rules: &ForegroundAppRules,
    mappings: &[CustomMapping],
) -> bool {
    let mut ids = BTreeSet::new();
    let mut names = BTreeSet::new();
    rules.valid_bindings(functions)
        && mappings.iter().all(|mapping| {
            mapping.id != 0
                && ids.insert(mapping.id)
                && !mapping.name.trim().is_empty()
                && mapping.name.chars().count() <= 80
                && !mapping.name.chars().any(char::is_control)
                && names.insert(mapping.name.trim().to_lowercase())
                && (1..=2).contains(&mapping.shortcuts.len())
                && mapping.shortcuts.iter().all(Shortcut::valid)
                && mapping.output.valid()
                && mapping
                    .groups
                    .iter()
                    .all(|id| rules.groups.contains_key(id))
        })
        && conflict(functions, rules, mappings).is_none()
}

/// Check an unsaved input slot without requiring a complete mapping or name.
pub fn shortcut_conflict(
    functions: &FunctionConfigs,
    rules: &ForegroundAppRules,
    mappings: &[CustomMapping],
    draft: &CustomMapping,
    slot: usize,
    shortcut: &Shortcut,
) -> Option<RuleId> {
    if draft
        .shortcuts
        .iter()
        .enumerate()
        .any(|(index, other)| index != slot && other == shortcut)
    {
        return Some(RuleId::Mapping(draft.id));
    }
    for (&id, config) in functions {
        if config.shortcuts.contains(shortcut)
            && scopes_overlap(
                rules,
                &draft.groups,
                &rules.assignments.get(&id).cloned().unwrap_or_default(),
            )
        {
            return Some(RuleId::Function(id));
        }
    }
    mappings
        .iter()
        .find(|mapping| {
            mapping.id != draft.id
                && mapping.shortcuts.contains(shortcut)
                && scopes_overlap(rules, &draft.groups, &mapping.groups)
        })
        .map(|mapping| RuleId::Mapping(mapping.id))
}

/// Each physical/mapped source owns its keys, so releasing one does not release another.
#[derive(Default)]
pub struct OutputOwners(BTreeMap<u64, MappingOutput>);

impl OutputOwners {
    pub fn clear_physical(&mut self) {
        self.0.retain(|owner, _| owner & (1 << 63) != 0);
    }
    pub fn clear_mappings(&mut self) {
        self.0.retain(|owner, _| owner & (1 << 63) == 0);
    }
    pub fn update(&mut self, owner: u64, output: MappingOutput, down: bool) -> ([bool; 256], u8) {
        if down {
            self.0.insert(owner, output);
        } else {
            self.0.remove(&owner);
        }
        self.state()
    }
    pub fn state(&self) -> ([bool; 256], u8) {
        let mut keys = [false; 256];
        let mut buttons = 0;
        for output in self.0.values() {
            match *output {
                MappingOutput::Keyboard { usage, modifiers } => {
                    keys[usage as usize] = true;
                    for bit in 0..8 {
                        if modifiers & (1 << bit) != 0 {
                            keys[0xe0 + bit] = true;
                        }
                    }
                }
                MappingOutput::Mouse { button } => buttons |= 1 << button as u8,
            }
        }
        (keys, buttons)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::function::{FunctionConfig, default_configs};

    fn mapping(id: u64, key: u8) -> CustomMapping {
        CustomMapping {
            id,
            name: format!("Mapping {id}"),
            enabled: true,
            shortcuts: vec![Shortcut::keyboard(ModifierSet::empty(), key)],
            output: MappingOutput::Keyboard {
                usage: 0x4f,
                modifiers: 0,
            },
            groups: Default::default(),
        }
    }

    #[test]
    fn disabled_custom_rules_reserve_their_input_against_builtin_rules() {
        let mut functions = default_configs();
        functions.insert(
            FunctionId::MediaPlayPause,
            FunctionConfig {
                enabled: true,
                shortcuts: vec![Shortcut::keyboard(ModifierSet::empty(), 0x58)],
            },
        );
        let mut custom = mapping(1, 0x58);
        custom.enabled = false;
        assert_eq!(
            conflict(&functions, &Default::default(), &[custom]),
            Some((
                RuleId::Function(FunctionId::MediaPlayPause),
                RuleId::Mapping(1)
            ))
        );
    }

    #[test]
    fn scopes_compare_paths_across_distinct_groups_and_allow_disjoint_apps() {
        use crate::foreground_app::ForegroundAppGroup;
        let mut rules = ForegroundAppRules::default();
        for (id, path) in [
            ("a", "C:\\Game.exe"),
            ("b", "c:\\game.exe"),
            ("c", "C:\\Other.exe"),
        ] {
            rules.groups.insert(
                id.into(),
                ForegroundAppGroup {
                    name: id.into(),
                    foreground_apps: vec![path.into()],
                },
            );
        }
        let mut first = mapping(1, 0x58);
        first.groups.insert("a".into());
        let mut second = mapping(2, 0x58);
        second.groups.insert("b".into());
        assert!(conflict(&default_configs(), &rules, &[first.clone(), second.clone()]).is_some());
        second.groups = BTreeSet::from(["c".into()]);
        assert!(valid(&default_configs(), &rules, &[first, second]));
    }

    #[test]
    fn output_validation_differs_from_input_validation() {
        let left = Shortcut::mouse(ModifierSet::empty(), MouseButton::Left);
        assert!(!left.valid());
        assert_eq!(
            MappingOutput::from_shortcut(&left),
            Some(MappingOutput::Mouse {
                button: MouseButton::Left
            })
        );
        assert_eq!(virtual_key_usage(0x27), Some(0x4f));
        assert_eq!(virtual_key_usage(0xe0), Some(0x58));
        assert_eq!(virtual_key_usage(0xff), None);
        assert!(
            !MappingOutput::Keyboard {
                usage: 0,
                modifiers: 0
            }
            .valid()
        );
    }

    #[test]
    fn draft_recording_checks_unsaved_slots_and_reserves_disabled_rules_without_matching_the_old_slot()
     {
        let functions = default_configs();
        let rules = ForegroundAppRules::default();
        let draft = mapping(1, 0x58);
        let shortcut = draft.shortcuts[0].clone();
        assert_eq!(
            shortcut_conflict(
                &functions,
                &rules,
                std::slice::from_ref(&draft),
                &draft,
                0,
                &shortcut
            ),
            None
        );
        assert_eq!(
            shortcut_conflict(&functions, &rules, &[], &draft, 1, &shortcut),
            Some(RuleId::Mapping(1))
        );
        let mut other = mapping(2, 0x58);
        other.enabled = false;
        assert_eq!(
            shortcut_conflict(&functions, &rules, &[other], &draft, 0, &shortcut),
            Some(RuleId::Mapping(2))
        );
    }

    #[test]
    fn shared_keys_modifiers_and_buttons_release_only_the_last_owner() {
        let mut owners = OutputOwners::default();
        let ctrl_x = MappingOutput::Keyboard {
            usage: 0x1b,
            modifiers: 1,
        };
        owners.update(1 << 63, ctrl_x, true);
        let state = owners.update(
            27,
            MappingOutput::Keyboard {
                usage: 0x1b,
                modifiers: 0,
            },
            true,
        );
        assert!(state.0[0x1b] && state.0[0xe0]);
        let state = owners.update(1 << 63, ctrl_x, false);
        assert!(state.0[0x1b] && !state.0[0xe0]);
        owners.update(
            (1 << 63) + 1,
            MappingOutput::Mouse {
                button: MouseButton::Left,
            },
            true,
        );
        owners.update(
            0x1000,
            MappingOutput::Mouse {
                button: MouseButton::Left,
            },
            true,
        );
        owners.clear_mappings();
        assert_eq!(owners.state().1, 1);
        assert_eq!(
            owners
                .update(
                    0x1000,
                    MappingOutput::Mouse {
                        button: MouseButton::Left
                    },
                    false
                )
                .1,
            0
        );
        owners.update((1 << 63) + 2, ctrl_x, true);
        owners.clear_physical();
        assert!(owners.state().0[0x1b] && owners.state().0[0xe0]);
    }
}
