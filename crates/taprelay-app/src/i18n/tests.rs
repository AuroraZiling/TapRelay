use super::*;

#[test]
fn every_catalog_key_is_loaded_by_the_shared_backend() {
    for (id, source) in [
        (
            "en",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/locales/en.json")),
        ),
        (
            "zh-CN",
            include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/locales/zh-CN.json")),
        ),
    ] {
        let mut catalog: std::collections::BTreeMap<String, serde_json::Value> =
            serde_json::from_str(source).unwrap();
        assert_eq!(catalog.remove("_version"), Some(1.into()));
        // Core definitions supply keys dynamically rather than through generated bindings.
        for function in taprelay_core::function::FUNCTION_CATALOG {
            for key in [
                Some(function.name_key),
                Some(function.tap_action.name_key()),
                function.hold_action.map(|action| action.name_key()),
            ]
            .into_iter()
            .flatten()
            {
                assert!(
                    catalog.contains_key(key),
                    "Missing function translation: {id}/{key}"
                );
            }
        }
        for (key, value) in catalog {
            assert_eq!(t!(&key, locale = id), value.as_str().unwrap(), "{id}/{key}");
        }
    }
}

#[test]
fn explicit_locales_keep_tray_labels_independent() {
    for _ in 0..2 {
        assert_eq!(tray_labels("en")[0], t!("tray.open", locale = "en"));
        assert_eq!(tray_labels("zh-CN")[0], t!("tray.open", locale = "zh-CN"));
        assert_ne!(tray_labels("en"), tray_labels("zh-CN"));
    }
}

#[test]
fn only_exact_catalog_identifiers_are_resolved() {
    assert_eq!(resolve("zh-CN"), Some("zh-CN"));
    assert_eq!(resolve("en"), Some("en"));
    for id in ["zh-cn", "ZH_cn", " en ", "de"] {
        assert_eq!(resolve(id), None, "{id}");
    }
}

#[test]
fn an_unknown_locale_falls_back_to_the_source_catalog() {
    assert_eq!(
        t!("nav.overview", locale = "de"),
        t!("nav.overview", locale = "en")
    );
}

#[test]
fn missing_keys_follow_the_library_default() {
    assert_eq!(t!("nav.missing", locale = "en"), "nav.missing");
}

#[test]
fn language_options_cover_the_system_choice_and_every_locale() {
    let options = language_options("zh-CN");
    assert_eq!(options.len(), locale::all().len() + 1);
    assert_eq!(options[0].value, "system");
    assert_eq!(options[0].label, "系统语言");
    assert_eq!(options[1].value, "en");
    assert_eq!(options[1].label, "English");
    assert_eq!(options[2].value, "zh-CN");
    assert_eq!(options[2].label, "简体中文");
    let english = language_options("en");
    assert_eq!(english[0].label, "System language");
    assert_eq!(english[2].label, "简体中文");
}
