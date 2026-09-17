//! Shared resource catalogs for pages, application messages and the system tray.
use slint::{ComponentHandle, ModelRc, SharedString, VecModel};
use std::{
    collections::{BTreeMap, HashMap},
    sync::LazyLock,
};
type Catalog = HashMap<String, String>;

pub struct Locale {
    id: &'static str,
    english_name: &'static str,
}
impl Locale {
    fn new(id: &'static str, english_name: &'static str) -> Self {
        Self { id, english_name }
    }
    pub const fn id(&self) -> &'static str {
        self.id
    }
    fn label_key(&self) -> String {
        format!("language.{}", self.id.replace('-', "_"))
    }
}

include!(concat!(env!("OUT_DIR"), "/i18n_bindings.rs"));

static CATALOGS: LazyLock<BTreeMap<&'static str, Catalog>> = LazyLock::new(|| {
    locale::all()
        .iter()
        .map(|locale| (locale.id(), read(locale.id())))
        .collect()
});

fn read(locale_id: &str) -> Catalog {
    serde_json::from_str(catalog_source(locale_id)).expect("Build-validated translation catalog")
}

/// Resolves a locale identifier the way the build normalized catalog file names.
pub fn resolve(id: &str) -> Option<&'static str> {
    let normalized = id.trim().to_ascii_lowercase().replace('_', "-");
    locale::all()
        .iter()
        .map(Locale::id)
        .find(|candidate| *candidate == normalized)
}

/// The source-of-truth locale identifier, checked against the compiled registry.
fn source() -> &'static str {
    let source = locale::default_locale().id();
    debug_assert_eq!(source, locale::SOURCE);
    source
}

/// Resolves the locale the operating system asks for, falling back to the source.
pub fn system_locale() -> &'static str {
    crate::platform::desktop::system_locale_id()
        .as_deref()
        .and_then(resolve)
        .unwrap_or_else(source)
}

pub fn text(locale_id: &str, key: &str) -> String {
    let locale_id = resolve(locale_id).unwrap_or_else(source);
    let english = catalog(source());
    let localized = catalog(locale_id);
    // An unknown key is a programming error: the key constants exist for that.
    assert!(
        english.contains_key(key),
        "Unknown translation key: {key} in {locale_id}"
    );
    if let Some(value) = localized.get(key).filter(|value| !value.is_empty()) {
        return value.clone();
    }
    // Development builds refuse to ship an untranslated fallback.
    debug_assert_eq!(
        locale_id,
        source(),
        "Missing {locale_id} translation: {key}"
    );
    english.get(key).cloned().unwrap_or_else(|| key.to_owned())
}

/// Borrows one compiled catalog. The caller passes an identifier it resolved already.
fn catalog(locale_id: &str) -> &'static Catalog {
    CATALOGS
        .get(locale_id)
        .unwrap_or_else(|| panic!("Unresolved locale: {locale_id}"))
}

/// The language's own name from the reserved `language.<id>` key, falling back to
/// English and finally to the bare identifier.
fn language_name(locale: &Locale) -> String {
    let key = locale.label_key();
    let localized = catalog(locale.id());
    let english = catalog(source());
    localized
        .get(&key)
        .filter(|value| !value.is_empty())
        .or_else(|| english.get(&key))
        .cloned()
        .unwrap_or_else(|| locale.english_name.to_owned())
}

pub fn apply(ui: &crate::AppWindow, locale_id: &str) {
    let locale_id = resolve(locale_id).unwrap_or_else(source);
    let global = ui.global::<crate::I18n>();
    if global.get_locale() == locale_id {
        return;
    }
    global.set_text(translation_values(locale_id));
    global.set_locale(locale_id.into());
}

pub fn language_options(locale_id: &str) -> ModelRc<crate::ChoiceOption> {
    ModelRc::new(VecModel::from(option_rows(locale_id)))
}

/// The single definition of the selector content, shared by the model and its tests.
fn option_rows(locale_id: &str) -> Vec<crate::ChoiceOption> {
    let locale_id = resolve(locale_id).unwrap_or_else(source);
    let mut options = vec![crate::ChoiceOption {
        label: text(locale_id, keys::LANGUAGE_SYSTEM).into(),
        value: "system".into(),
    }];
    options.extend(locale::all().iter().map(|locale| crate::ChoiceOption {
        label: language_name(locale).into(),
        value: SharedString::from(locale.id()),
    }));
    options
}

pub fn tray_labels(locale_id: &str) -> [String; 4] {
    [
        keys::TRAY_OPEN,
        keys::LISTENING_START,
        keys::LISTENING_STOP,
        keys::COMMON_QUIT,
    ]
    .map(|key| text(locale_id, key))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_locale_defines_a_language_name_key() {
        for locale in locale::all() {
            assert!(
                catalog(source()).contains_key(&locale.label_key()),
                "Missing language name key: {}",
                locale.label_key()
            );
        }
    }

    #[test]
    fn every_locale_translates_every_compiled_key() {
        for (locale_id, entries) in CATALOGS.iter() {
            for key in catalog(source()).keys() {
                assert!(
                    entries.get(key).is_some_and(|value| !value.is_empty()),
                    "Missing translation: {key} in {locale_id}"
                );
            }
        }
    }

    #[test]
    fn no_catalog_can_introduce_a_key_the_slint_struct_lacks() {
        for (locale_id, entries) in CATALOGS.iter() {
            for key in entries.keys() {
                assert!(
                    catalog(source()).contains_key(key),
                    "Key not present in en.json: {key} in {locale_id}"
                );
            }
        }
    }

    #[test]
    fn the_compiled_registry_lists_every_catalog_in_order() {
        let ids = locale::all().iter().map(Locale::id).collect::<Vec<_>>();
        assert_eq!(ids, ["en", "zh-cn"]);
        assert_eq!(source(), "en");
    }

    #[test]
    fn locale_identifiers_are_normalized() {
        assert_eq!(resolve("zh-CN"), Some("zh-cn"));
        assert_eq!(resolve("ZH_cn"), Some("zh-cn"));
        assert_eq!(resolve(" en "), Some("en"));
        assert_eq!(resolve("de"), None);
    }

    #[test]
    fn an_unknown_locale_falls_back_to_the_source_catalog() {
        assert_eq!(
            text("de", keys::NAV_OVERVIEW),
            text("en", keys::NAV_OVERVIEW)
        );
    }

    #[test]
    fn an_unknown_key_is_a_bug_in_every_build() {
        assert!(std::panic::catch_unwind(|| text("en", "nav.missing")).is_err());
    }

    #[test]
    fn catalogs_resolve_their_own_and_the_english_text() {
        assert_eq!(text("zh-cn", keys::NAV_OVERVIEW), "概览");
        assert_eq!(text("en", keys::NAV_OVERVIEW), "Overview");
        assert_ne!(
            text("zh-cn", keys::NAV_OVERVIEW),
            text("en", keys::NAV_OVERVIEW)
        );
    }

    #[test]
    fn language_options_cover_the_system_choice_and_every_locale() {
        let options = option_rows("zh-cn");
        assert_eq!(options.len(), locale::all().len() + 1);
        assert_eq!(options[0].value, "system");
        assert_eq!(options[0].label, "系统语言");
        assert_eq!(options[1].value, "en");
        assert_eq!(options[1].label, "English");
        assert_eq!(options[2].value, "zh-cn");
        assert_eq!(options[2].label, "简体中文");
        let english = option_rows("en");
        assert_eq!(english[0].label, "System language");
        assert_eq!(english[2].label, "简体中文");
    }
}
