//! Shared resource catalogs for pages, application messages and the system tray.
use slint::ComponentHandle;
use std::{collections::BTreeMap, sync::LazyLock};
type Catalog = BTreeMap<String, String>;
static ENGLISH: LazyLock<Catalog> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../i18n/en.json")).expect("Build-validated English catalog")
});
static CHINESE: LazyLock<Catalog> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../i18n/zh-CN.json"))
        .expect("Build-validated Chinese catalog")
});

fn lookup<'a>(primary: &'a Catalog, english: &'a Catalog, key: &'a str) -> &'a str {
    primary
        .get(key)
        .filter(|s| !s.is_empty())
        .or_else(|| english.get(key))
        .map(String::as_str)
        .unwrap_or(key)
}
pub fn text(chinese: bool, key: &str) -> &str {
    lookup(if chinese { &CHINESE } else { &ENGLISH }, &ENGLISH, key)
}
include!(concat!(env!("OUT_DIR"), "/i18n_bindings.rs"));

pub fn apply(ui: &crate::AppWindow, chinese: bool) {
    let locale = if chinese { "zh-CN" } else { "en" };
    let global = ui.global::<crate::I18n>();
    if global.get_locale() != locale {
        global.set_text(translation_values(chinese));
        global.set_locale(locale.into());
    }
}
pub fn tray_labels(chinese: bool) -> [String; 4] {
    [
        keys::TRAY_OPEN,
        keys::LISTENING_START,
        keys::LISTENING_STOP,
        keys::COMMON_QUIT,
    ]
    .map(|key| text(chinese, key).to_owned())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn both_catalogs_cover_the_same_keys() {
        assert_eq!(
            ENGLISH.keys().collect::<Vec<_>>(),
            CHINESE.keys().collect::<Vec<_>>()
        );
        assert_eq!(text(false, keys::NAV_OVERVIEW), "Overview");
        assert_eq!(text(true, keys::NAV_OVERVIEW), "概览");
    }
    #[test]
    fn missing_or_empty_translation_falls_back_to_english() {
        let english = Catalog::from([("key".into(), "Fallback".into())]);
        assert_eq!(lookup(&Catalog::new(), &english, "key"), "Fallback");
        assert_eq!(
            lookup(&Catalog::from([("key".into(), "".into())]), &english, "key"),
            "Fallback"
        );
        assert_eq!(lookup(&Catalog::new(), &english, "unknown"), "unknown");
    }
}
