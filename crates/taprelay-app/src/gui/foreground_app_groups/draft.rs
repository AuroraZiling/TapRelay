use taprelay_core::foreground_app::{ForegroundAppGroup, ForegroundAppRules, executable_identity};

pub(crate) struct GroupDraft {
    pub id: Option<String>,
    pub group: ForegroundAppGroup,
}

impl GroupDraft {
    pub fn new(rules: &ForegroundAppRules, default_name: &str) -> Self {
        let names = rules
            .groups
            .values()
            .map(|group| group.name.trim().to_lowercase())
            .collect::<std::collections::BTreeSet<_>>();
        let name = std::iter::once(default_name.to_owned())
            .chain((2..).map(|n| format!("{default_name} {n}")))
            .find(|name| !names.contains(&name.to_lowercase()))
            .unwrap();
        Self {
            id: None,
            group: ForegroundAppGroup {
                name,
                ..ForegroundAppGroup::default()
            },
        }
    }

    pub fn edit(rules: &ForegroundAppRules, id: &str) -> Option<Self> {
        Some(Self {
            id: Some(id.into()),
            group: rules.groups.get(id)?.clone(),
        })
    }

    pub fn add(&mut self, path: String) {
        if !self
            .group
            .foreground_apps
            .iter()
            .any(|other| executable_identity(other) == executable_identity(&path))
        {
            self.group.foreground_apps.push(path);
        }
    }

    pub fn candidate(&self, rules: &ForegroundAppRules, name: &str) -> ForegroundAppRules {
        let mut result = rules.clone();
        let id = self.id.clone().unwrap_or_else(|| {
            (1..)
                .map(|n| format!("group-{n}"))
                .find(|id| !result.groups.contains_key(id))
                .unwrap()
        });
        let mut group = self.group.clone();
        group.name = name.trim().into();
        result.groups.insert(id, group);
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn create_and_cancel_never_change_saved_groups() {
        let rules = ForegroundAppRules::default();
        let mut draft = GroupDraft::new(&rules, "New group");
        draft.add(r"C:\Games\game.exe".into());
        draft.add(r"c:\games\GAME.exe".into());
        assert_eq!(draft.group.foreground_apps.len(), 1);
        let candidate = draft.candidate(&rules, " Games ");
        assert!(candidate.valid());
        assert_eq!(candidate.groups["group-1"].name, "Games");
        drop(draft);
        assert!(rules.groups.is_empty());
    }

    #[test]
    fn new_groups_can_be_saved_with_the_localized_default_name() {
        for locale in ["en", "zh-cn"] {
            let default_name = crate::i18n::text(locale, "groups.default_name");
            let rules = ForegroundAppRules::default();
            let draft = GroupDraft::new(&rules, &default_name);
            assert_eq!(draft.group.name, default_name);
            let saved = draft.candidate(&rules, &draft.group.name);
            assert!(saved.valid());
            assert_eq!(saved.groups["group-1"].name, default_name);

            let next = GroupDraft::new(&saved, &default_name);
            assert_eq!(next.group.name, format!("{default_name} 2"));
            assert!(next.candidate(&saved, &next.group.name).valid());
        }
    }

    #[test]
    fn default_name_skips_existing_names_ignoring_case_and_whitespace() {
        let mut rules = ForegroundAppRules::default();
        for (id, name) in [
            ("a", " NEW GROUP "),
            ("b", "new group 2"),
            ("c", "New group 4"),
        ] {
            rules.groups.insert(
                id.into(),
                ForegroundAppGroup {
                    name: name.into(),
                    ..ForegroundAppGroup::default()
                },
            );
        }
        let draft = GroupDraft::new(&rules, "New group");
        assert_eq!(draft.group.name, "New group 3");
        assert!(draft.candidate(&rules, &draft.group.name).valid());
        assert_eq!(
            draft.candidate(&rules, "Custom name").groups["group-1"].name,
            "Custom name"
        );
    }
    #[test]
    fn editing_retains_identity_and_assignments_and_save_preserves_other_groups() {
        use taprelay_core::function::FunctionId;
        let mut rules = ForegroundAppRules::default();
        rules.groups.insert(
            "games".into(),
            ForegroundAppGroup {
                name: "Games".into(),
                foreground_apps: vec![r"C:\game.exe".into()],
            },
        );
        rules
            .assignments
            .insert(FunctionId::MediaMute, ["games".into()].into());
        let mut draft = GroupDraft::edit(&rules, "games").unwrap();
        draft.group.foreground_apps.clear();
        assert_eq!(rules.groups["games"].foreground_apps.len(), 1);
        rules.groups.insert(
            "work".into(),
            ForegroundAppGroup {
                name: "Work".into(),
                foreground_apps: vec![],
            },
        );
        let saved = draft.candidate(&rules, "Renamed");
        assert_eq!(saved.assignments, rules.assignments);
        assert_eq!(saved.groups["work"], rules.groups["work"]);
        assert!(saved.groups["games"].foreground_apps.is_empty());
        assert_eq!(rules.groups["games"].name, "Games");
    }
}
