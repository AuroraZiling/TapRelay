use super::*;

#[test]
fn mouse_shortcuts_and_in_progress_capture_use_the_same_localized_labels() {
    use crate::runtime::{CaptureKind, CapturePhase, CaptureSession};
    use taprelay_core::{
        function::{ModifierSet, Shortcut},
        input::{InputCode, MouseButton},
    };
    for (button, zh, en) in [
        (MouseButton::Left, "左键", "Left button"),
        (MouseButton::Right, "右键", "Right button"),
        (MouseButton::Middle, "中键", "Middle button"),
        (MouseButton::Side1, "侧键 1", "Side button 1"),
        (MouseButton::Side2, "侧键 2", "Side button 2"),
    ] {
        let shortcut = Shortcut::mouse(ModifierSet::empty(), button);
        let capture = CaptureSession {
            kind: CaptureKind::MappingOutput,
            phase: CapturePhase::Recording,
            preview: vec![InputCode::Mouse(button)],
            error: None,
        };
        for (locale, expected) in [("zh-CN", zh), ("en", en)] {
            assert_eq!(
                capture_preview(locale, &capture),
                expected,
                "Recording must be localized before release"
            );
            assert_eq!(shortcut_labels(locale, &shortcut), vec![expected]);
        }
    }
}

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
fn explicit_locales_keep_tray_and_messages_independent() {
    for _ in 0..2 {
        assert_eq!(tray_labels("en")[0], t!("tray.open", locale = "en"));
        assert_eq!(tray_labels("ZH_cn")[0], t!("tray.open", locale = "zh-CN"));
        assert_ne!(tray_labels("en"), tray_labels("zh-cn"));
        assert_eq!(foreground_app_count("zh-CN", 2), "共 2 个应用");
        assert_eq!(foreground_app_count("en", 2), "2 applications");
    }
}

#[test]
fn foreground_app_totals_use_localized_wording() {
    for count in [0, 1, 2, 100] {
        assert_eq!(
            foreground_app_count("zh-cn", count),
            format!("共 {count} 个应用")
        );
    }
    assert_eq!(foreground_app_count("en", 0), "0 applications");
    assert_eq!(foreground_app_count("en", 1), "1 application");
    assert_eq!(foreground_app_count("en", 2), "2 applications");
}

#[test]
fn locale_identifiers_are_normalized() {
    assert_eq!(resolve("zh-CN"), Some("zh-CN"));
    assert_eq!(resolve("zh-cn"), Some("zh-CN"));
    assert_eq!(resolve("ZH_cn"), Some("zh-CN"));
    assert_eq!(resolve(" en "), Some("en"));
    assert_eq!(resolve("de"), None);
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
    let options = option_rows("zh-cn");
    assert_eq!(options.len(), locale::all().len() + 1);
    assert_eq!(options[0].value, "system");
    assert_eq!(options[0].label, "系统语言");
    assert_eq!(options[1].value, "en");
    assert_eq!(options[1].label, "English");
    assert_eq!(options[2].value, "zh-CN");
    assert_eq!(options[2].label, "简体中文");
    let english = option_rows("en");
    assert_eq!(english[0].label, "System language");
    assert_eq!(english[2].label, "简体中文");
}
