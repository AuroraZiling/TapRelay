use super::*;

#[test]
fn sidebar_defaults_expanded_for_old_configs_and_round_trips_both_states() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let mut json = serde_json::to_value(Config::default()).unwrap();
    json["options"]
        .as_object_mut()
        .unwrap()
        .remove("sidebar_collapsed");
    let bytes = serde_json::to_vec(&json).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let mut config = Config::load(&path).unwrap();
    assert!(!config.options.sidebar_collapsed);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    for collapsed in [true, false] {
        config.options.sidebar_collapsed = collapsed;
        config.save(&path).unwrap();
        assert_eq!(
            Config::load(&path).unwrap().options.sidebar_collapsed,
            collapsed
        );
    }
}

#[test]
fn overlay_defaults_on_for_old_configs_and_disabled_setting_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let mut json = serde_json::to_value(Config::default()).unwrap();
    json["options"]
        .as_object_mut()
        .unwrap()
        .remove("passthrough_overlay");
    let bytes = serde_json::to_vec(&json).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let mut config = Config::load(&path).unwrap();
    assert!(config.options.passthrough_overlay);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    config.options.passthrough_overlay = false;
    config.save(&path).unwrap();
    assert!(!Config::load(&path).unwrap().options.passthrough_overlay);
}

#[test]
fn old_configs_remain_global_and_groups_round_trip_without_rewriting_input() {
    use taprelay_core::{foreground_app::ForegroundAppGroup, function::FunctionId};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let mut json = serde_json::to_value(Config::default()).unwrap();
    json.as_object_mut().unwrap().remove("foreground_app_rules");
    let bytes = serde_json::to_vec(&json).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let mut config = Config::load(&path).unwrap();
    assert!(config.foreground_app_rules.assignments.is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    config.foreground_app_rules.groups.insert(
        "games".into(),
        ForegroundAppGroup {
            name: "游戏".into(),
            foreground_apps: vec![r"C:\Games\game.exe".into()],
        },
    );
    config
        .foreground_app_rules
        .assignments
        .insert(FunctionId::MediaPlayPause, ["games".into()].into());
    config.save(&path).unwrap();
    let saved = Config::load(&path).unwrap();
    assert_eq!(saved.foreground_app_rules, config.foreground_app_rules);
    config.foreground_app_rules.groups.remove("games");
    assert!(config.save(&path).is_err());
    assert_eq!(
        Config::load(&path).unwrap().foreground_app_rules,
        saved.foreground_app_rules
    );
}

#[test]
fn mouse_report_rate_defaults_for_old_configs_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let mut json = serde_json::to_value(Config::default()).unwrap();
    json["options"]
        .as_object_mut()
        .unwrap()
        .remove("passthrough_mouse_report_rate");
    let bytes = serde_json::to_vec(&json).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let mut config = Config::load(&path).unwrap();
    assert_eq!(config.options.passthrough_mouse_report_rate, 0);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    for hz in taprelay_core::passthrough::MOUSE_REPORT_RATES {
        config.options.passthrough_mouse_report_rate = hz;
        config.save(&path).unwrap();
        assert_eq!(
            Config::load(&path)
                .unwrap()
                .options
                .passthrough_mouse_report_rate,
            hz
        );
    }
    for hz in [1, 124, 1001, u16::MAX] {
        config.options.passthrough_mouse_report_rate = hz;
        assert!(config.validate().is_err());
    }
}

#[test]
fn loading_removes_only_standalone_primary_mouse_bindings() {
    use taprelay_core::{
        function::{FunctionConfig, FunctionId, ModifierSet, Shortcut},
        input::MouseButton,
    };

    for button in [MouseButton::Left, MouseButton::Right] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let mut config = Config::default();
        let retained = Shortcut::mouse(ModifierSet::from_keys([0x11]), button);
        config.functions.insert(
            FunctionId::MediaNext,
            FunctionConfig {
                enabled: true,
                shortcuts: vec![
                    Shortcut::mouse(ModifierSet::empty(), button),
                    retained.clone(),
                ],
            },
        );
        assert!(config.save(&path).is_err());
        let bytes = serde_json::to_vec(&config).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        let loaded = Config::load(&path).unwrap();
        let function = &loaded.functions[&FunctionId::MediaNext];
        assert!(function.enabled);
        assert_eq!(function.shortcuts, vec![retained]);
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        loaded.save(&path).unwrap();
        assert_eq!(Config::load(&path).unwrap().functions, loaded.functions);
    }
}
#[test]
fn invalid_config_is_not_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("config.json");
    std::fs::write(&p, "{\"verified\":true}").unwrap();
    assert!(Config::load(&p).is_err());
    assert_eq!(std::fs::read_to_string(p).unwrap(), "{\"verified\":true}");
}

#[test]
fn theme_and_language_names_match_the_persisted_config() {
    // The settings page sends these names, so they must stay identical to
    // what serde writes; otherwise a click would change nothing on reload.
    for (name, theme) in [
        ("system", Theme::System),
        ("light", Theme::Light),
        ("dark", Theme::Dark),
    ] {
        assert_eq!(Theme::from_name(name), Some(theme));
        assert_eq!(
            serde_json::to_value(theme).unwrap(),
            serde_json::json!(name)
        );
    }
    for (name, language) in [
        ("system", Language::System),
        ("chinese", Language::Chinese),
        ("english", Language::English),
    ] {
        assert_eq!(Language::from_name(name), Some(language.clone()));
        assert_eq!(
            serde_json::to_value(&language).unwrap(),
            serde_json::json!(name)
        );
    }
    assert_eq!(Theme::from_name("Dark"), None);
    assert_eq!(Language::from_name(""), None);
}

#[test]
fn locale_identifiers_select_the_matching_catalog() {
    assert_eq!(Language::from_name("zh-CN"), Some(Language::Chinese));
    assert_eq!(Language::from_name("zh-cn"), Some(Language::Chinese));
    assert_eq!(Language::from_name("ZH_cn"), Some(Language::Chinese));
    let legacy: Language = serde_json::from_str(r#"{"locale":"zh-cn"}"#).unwrap();
    assert_eq!(legacy.locale(), "zh-CN");
    assert_eq!(Language::from_name("en"), Some(Language::English));
    assert_eq!(Language::from_name("de"), None);
}

#[test]
fn unsupported_schema_and_unknown_function_are_rejected_without_writing() {
    let dir = tempfile::tempdir().unwrap();
    let unsupported_schema = dir.path().join("unsupported-schema.json");
    let mut invalid = serde_json::to_value(Config::default()).unwrap();
    invalid["schema"] = serde_json::json!(5);
    let invalid_bytes = serde_json::to_vec(&invalid).unwrap();
    std::fs::write(&unsupported_schema, &invalid_bytes).unwrap();
    assert!(Config::load(&unsupported_schema).is_err());
    assert_eq!(std::fs::read(&unsupported_schema).unwrap(), invalid_bytes);

    let unknown = dir.path().join("unknown-function.json");
    let mut invalid = serde_json::to_value(Config::default()).unwrap();
    invalid["functions"] = serde_json::json!({
        "media.not-a-function": {
            "enabled": true,
            "shortcuts": []
        }
    });
    let invalid_bytes = serde_json::to_vec(&invalid).unwrap();
    std::fs::write(&unknown, &invalid_bytes).unwrap();
    assert!(Config::load(&unknown).is_err());
    assert_eq!(std::fs::read(&unknown).unwrap(), invalid_bytes);
}
#[test]
fn legacy_config_only_gains_the_app_function_and_round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    let mut original = Config::default();
    original
        .functions
        .get_mut(&taprelay_core::function::FunctionId::MediaMute)
        .unwrap()
        .enabled = true;
    let mut json = serde_json::to_value(&original).unwrap();
    json["functions"]
        .as_object_mut()
        .unwrap()
        .remove("app.toggle-listening");
    let bytes = serde_json::to_vec(&json).unwrap();
    std::fs::write(&path, &bytes).unwrap();
    let loaded = Config::load(&path).unwrap();
    assert_eq!(loaded.functions, original.functions);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    loaded.save(&path).unwrap();
    assert_eq!(Config::load(&path).unwrap().functions, original.functions);
    json["functions"]
        .as_object_mut()
        .unwrap()
        .remove("media.mute");
    std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(Config::load(&path).is_err());
}
