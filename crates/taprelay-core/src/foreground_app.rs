//! Foreground-app allowlists and the media functions that use them.
use crate::function::{CategoryId, FunctionConfigs, FunctionId, function_definition};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForegroundAppGroup {
    pub name: String,
    #[serde(alias = "applications")]
    pub foreground_apps: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ForegroundAppRules {
    pub groups: BTreeMap<String, ForegroundAppGroup>,
    /// Absence means all applications. Control functions must never be scoped.
    #[serde(deserialize_with = "deserialize_assignments")]
    pub assignments: BTreeMap<FunctionId, BTreeSet<String>>,
}

/// Revision-scoped lookup for the input hook. Paths are normalized once when
/// publishing rules, so routing never scans groups or allocates per candidate.
#[derive(Default)]
pub(crate) struct ScopeIndex(BTreeMap<FunctionId, BTreeSet<String>>);

impl ScopeIndex {
    pub fn new(rules: &ForegroundAppRules) -> Self {
        Self(
            rules
                .assignments
                .iter()
                .map(|(&id, groups)| {
                    let paths = groups
                        .iter()
                        .filter_map(|group| rules.groups.get(group))
                        .flat_map(|group| &group.foreground_apps)
                        .map(|path| executable_identity(path))
                        .collect();
                    (id, paths)
                })
                .collect(),
        )
    }

    /// `foreground` is already normalized by the router. Missing groups produce
    /// an empty set, not a global scope; application control shortcuts stay global.
    pub fn allows(&self, id: FunctionId, foreground: Option<&str>) -> bool {
        function_definition(id).category == CategoryId::App
            || self
                .0
                .get(&id)
                .is_none_or(|paths| foreground.is_some_and(|path| paths.contains(path)))
    }
}

fn deserialize_assignments<'de, D>(
    deserializer: D,
) -> Result<BTreeMap<FunctionId, BTreeSet<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Selection {
        Legacy(String),
        Multiple(BTreeSet<String>),
    }
    let values = BTreeMap::<FunctionId, Selection>::deserialize(deserializer)?;
    Ok(values
        .into_iter()
        .map(|(id, selection)| {
            let groups = match selection {
                Selection::Legacy(group) => BTreeSet::from([group]),
                Selection::Multiple(groups) => groups,
            };
            (id, groups)
        })
        .collect())
}

/// Both native discovery and file selection supply absolute Win32 paths.
pub fn executable_identity(path: &str) -> String {
    let path = path.replace('/', "\\").to_lowercase();
    if let Some(path) = path.strip_prefix(r"\\?\unc\") {
        format!(r"\\{path}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
    }
}

pub fn valid_executable(path: &str) -> bool {
    let path = executable_identity(path);
    let bytes = path.as_bytes();
    !path.contains('\0')
        && path.ends_with(".exe")
        && ((bytes.len() > 3 && bytes[0].is_ascii_alphabetic() && &bytes[1..3] == b":\\")
            || path.starts_with(r"\\"))
        && !path.split('\\').any(|part| matches!(part, "." | ".."))
}

impl ForegroundAppRules {
    pub fn valid(&self) -> bool {
        let mut names = BTreeSet::new();
        self.groups.iter().all(|(id, group)| {
            let mut paths = BTreeSet::new();
            !id.is_empty()
                && !group.name.trim().is_empty()
                && group.name.chars().count() <= 80
                && !group.name.chars().any(char::is_control)
                && names.insert(group.name.trim().to_lowercase())
                && group
                    .foreground_apps
                    .iter()
                    .all(|path| valid_executable(path) && paths.insert(executable_identity(path)))
        }) && self.assignments.iter().all(|(id, groups)| {
            function_definition(*id).category == CategoryId::Media
                && !groups.is_empty()
                && groups.iter().all(|group| self.groups.contains_key(group))
        })
    }

    pub fn allows(&self, id: FunctionId, foreground: Option<&str>) -> bool {
        if function_definition(id).category == CategoryId::App {
            return true;
        }
        let Some(groups) = self.assignments.get(&id) else {
            return true;
        };
        let Some(path) = foreground else { return false };
        let path = executable_identity(path);
        groups
            .iter()
            .filter_map(|id| self.groups.get(id))
            .any(|group| {
                group
                    .foreground_apps
                    .iter()
                    .any(|candidate| executable_identity(candidate) == path)
            })
    }

    pub fn overlaps(&self, first: FunctionId, second: FunctionId) -> bool {
        if first == second {
            return true;
        }
        let (Some(a), Some(b)) = (self.assignments.get(&first), self.assignments.get(&second))
        else {
            // Global bindings reserve their shortcut even against an empty group.
            return true;
        };
        if !a.is_disjoint(b) {
            return true;
        }
        if a.is_empty() || b.is_empty() || a.union(b).any(|id| !self.groups.contains_key(id)) {
            return true;
        }
        a.iter()
            .flat_map(|id| &self.groups[id].foreground_apps)
            .any(|path| {
                b.iter()
                    .flat_map(|id| &self.groups[id].foreground_apps)
                    .any(|other| executable_identity(path) == executable_identity(other))
            })
    }

    pub fn conflict(&self, configs: &FunctionConfigs) -> Option<(FunctionId, FunctionId)> {
        let mut seen = Vec::new();
        for (&id, config) in configs {
            for shortcut in &config.shortcuts {
                if let Some((other, _)) = seen
                    .iter()
                    .find(|(other, candidate)| *candidate == shortcut && self.overlaps(id, *other))
                {
                    return Some((*other, id));
                }
                seen.push((id, shortcut));
            }
        }
        None
    }

    pub fn valid_bindings(&self, configs: &FunctionConfigs) -> bool {
        self.valid()
            && configs.len() == crate::function::FUNCTION_CATALOG.len()
            && crate::binding::valid_structure(configs)
            && self.conflict(configs).is_none()
    }

    pub fn remove_group(&mut self, id: &str) {
        self.groups.remove(id);
        self.assignments.retain(|_, groups| {
            groups.remove(id);
            !groups.is_empty()
        });
    }

    /// The empty choice selects all applications; toggling the last group off restores it.
    pub fn toggle_scope(&mut self, function: FunctionId, group: &str) {
        if group.is_empty() {
            self.assignments.remove(&function);
            return;
        }
        let groups = self.assignments.entry(function).or_default();
        if !groups.remove(group) {
            groups.insert(group.into());
        }
        if groups.is_empty() {
            self.assignments.remove(&function);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::function::{ModifierSet, Shortcut, default_configs};

    pub fn rules() -> ForegroundAppRules {
        ForegroundAppRules {
            groups: BTreeMap::from([
                (
                    "games".into(),
                    ForegroundAppGroup {
                        name: "Games".into(),
                        foreground_apps: vec![r"C:\Games\a.exe".into(), r"C:\Games\b.exe".into()],
                    },
                ),
                (
                    "work".into(),
                    ForegroundAppGroup {
                        name: "Work".into(),
                        foreground_apps: vec![r"C:\Work\c.exe".into()],
                    },
                ),
            ]),
            assignments: BTreeMap::from([
                (FunctionId::MediaVolumeUp, ["games".into()].into()),
                (FunctionId::MediaMute, ["work".into()].into()),
            ]),
        }
    }

    #[test]
    fn compiled_scopes_match_union_rules_and_fail_closed_for_invalid_references() {
        let mut rules = rules();
        rules.toggle_scope(FunctionId::MediaVolumeUp, "work");
        rules
            .assignments
            .insert(FunctionId::MediaNext, ["missing".into()].into());
        let index = ScopeIndex::new(&rules);
        for id in crate::function::function_ids() {
            for path in [
                None,
                Some("c:/GAMES/A.EXE"),
                Some(r"C:\Work\c.exe"),
                Some(r"C:\outside.exe"),
            ] {
                let identity = path.map(executable_identity);
                assert_eq!(
                    index.allows(id, identity.as_deref()),
                    rules.allows(id, path)
                );
            }
        }
        rules.groups.clear();
        let index = ScopeIndex::new(&rules);
        assert!(!index.allows(FunctionId::MediaVolumeUp, Some(r"c:\games\a.exe")));
        assert!(index.allows(FunctionId::AppToggleListening, None));
    }

    #[test]
    fn multiple_groups_form_a_union_and_all_foreground_apps_is_exclusive() {
        let mut rules = rules();
        let id = FunctionId::MediaVolumeUp;
        rules.toggle_scope(id, "work");
        assert_eq!(rules.assignments[&id].len(), 2);
        assert!(rules.valid());
        assert!(rules.allows(id, Some(r"C:\Games\a.exe")));
        assert!(rules.allows(id, Some(r"C:\Work\c.exe")));
        assert!(!rules.allows(id, Some(r"C:\Elsewhere\a.exe")));
        assert!(!rules.allows(id, None));
        rules.toggle_scope(id, "games");
        assert!(!rules.allows(id, Some(r"C:\Games\a.exe")));
        assert!(rules.allows(id, Some(r"C:\Work\c.exe")));
        rules.toggle_scope(id, "work");
        assert!(!rules.assignments.contains_key(&id));
        assert!(rules.allows(id, None));
        rules.toggle_scope(id, "games");
        rules.toggle_scope(id, "work");
        rules.toggle_scope(id, "");
        assert!(!rules.assignments.contains_key(&id));
        rules.toggle_scope(id, "work");
        assert_eq!(rules.assignments[&id], BTreeSet::from(["work".into()]));
    }

    #[test]
    fn deleting_one_selected_group_preserves_the_remaining_scope() {
        let mut rules = rules();
        let id = FunctionId::MediaVolumeUp;
        rules.toggle_scope(id, "work");
        rules.remove_group("games");
        assert_eq!(rules.assignments[&id], BTreeSet::from(["work".into()]));
        assert!(!rules.allows(id, Some(r"C:\Games\a.exe")));
        rules.remove_group("work");
        assert!(rules.assignments.is_empty());
        assert!(rules.valid());
    }

    #[test]
    fn conflicts_consider_every_selected_group_and_its_paths() {
        let mut rules = rules();
        let mut configs = default_configs();
        let shortcut = Shortcut::keyboard(ModifierSet::empty(), 0x58);
        for id in [FunctionId::MediaVolumeUp, FunctionId::MediaMute] {
            configs
                .get_mut(&id)
                .unwrap()
                .shortcuts
                .push(shortcut.clone());
        }
        assert!(rules.conflict(&configs).is_none());
        rules.toggle_scope(FunctionId::MediaVolumeUp, "work");
        assert!(rules.conflict(&configs).is_some());
        rules.toggle_scope(FunctionId::MediaVolumeUp, "work");
        rules.groups.insert(
            "other".into(),
            ForegroundAppGroup {
                name: "Other".into(),
                foreground_apps: vec!["c:/WORK/C.EXE".into()],
            },
        );
        rules.toggle_scope(FunctionId::MediaVolumeUp, "other");
        assert!(rules.conflict(&configs).is_some());
        rules
            .assignments
            .get_mut(&FunctionId::MediaVolumeUp)
            .unwrap()
            .clear();
        assert!(!rules.valid());
    }

    #[test]
    fn identity_is_full_path_and_unknown_foregrounds_fail_closed() {
        let rules = rules();
        assert!(rules.valid());
        assert!(rules.allows(FunctionId::MediaVolumeUp, Some("c:/GAMES/A.EXE")));
        assert!(!rules.allows(FunctionId::MediaVolumeUp, Some(r"D:\Games\a.exe")));
        assert!(!rules.allows(FunctionId::MediaVolumeUp, None));
        assert!(rules.allows(FunctionId::AppToggleListening, None));
        assert!(rules.allows(FunctionId::MediaNext, None));
        assert_eq!(
            executable_identity(r"\\?\C:\Games\A.exe"),
            executable_identity("c:/games/a.exe")
        );
        assert_eq!(
            executable_identity(r"\\?\UNC\Server\Share\A.exe"),
            executable_identity(r"\\server\share\a.exe")
        );
    }

    #[test]
    fn reuse_conflicts_are_checked_for_edits_deletion_and_disabled_functions() {
        let mut rules = rules();
        let mut configs = default_configs();
        let shortcut = Shortcut::keyboard(ModifierSet::empty(), 0x58);
        for id in [FunctionId::MediaVolumeUp, FunctionId::MediaMute] {
            configs
                .get_mut(&id)
                .unwrap()
                .shortcuts
                .push(shortcut.clone());
        }
        assert!(rules.valid_bindings(&configs));
        rules
            .groups
            .get_mut("work")
            .unwrap()
            .foreground_apps
            .push(r"c:\games\A.exe".into());
        assert!(rules.conflict(&configs).is_some());
        rules.groups.get_mut("work").unwrap().foreground_apps.pop();
        rules.remove_group("work");
        assert!(rules.conflict(&configs).is_some());
    }

    #[test]
    fn missing_groups_duplicate_paths_and_scoped_control_functions_are_invalid() {
        let mut rules = rules();
        rules
            .groups
            .get_mut("games")
            .unwrap()
            .foreground_apps
            .clear();
        assert!(rules.valid());
        assert!(!rules.allows(FunctionId::MediaVolumeUp, Some(r"C:\Games\a.exe")));
        rules.groups.remove("games");
        assert!(!rules.valid());
        assert!(!rules.allows(FunctionId::MediaVolumeUp, Some(r"C:\Games\a.exe")));
        let mut rules = self::rules();
        rules
            .assignments
            .insert(FunctionId::AppTogglePassthrough, ["games".into()].into());
        assert!(!rules.valid());
        let mut rules = self::rules();
        rules
            .groups
            .get_mut("games")
            .unwrap()
            .foreground_apps
            .push("c:/games/A.EXE".into());
        assert!(!rules.valid());
    }
}
