use std::{collections::BTreeMap, path::PathBuf};

const SOURCE_LOCALE: &str = "en";
const LANGUAGE_LABEL_PREFIX: &str = "language.";

type Catalog = BTreeMap<String, String>;

struct LocaleCatalog {
    id: String,
    file: &'static str,
    english_name: String,
    text: Catalog,
}

pub fn generate() -> PathBuf {
    let manifest =
        PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory"));
    let directory = manifest.join("i18n");
    println!("cargo:rerun-if-changed={}", directory.display());
    let locales = scan(&directory);
    let english = &locales
        .iter()
        .find(|locale| locale.id == SOURCE_LOCALE)
        .expect("i18n/en.json is required as the source of truth")
        .text;
    validate(&locales, english);
    let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo output directory"));
    let staged = out.join("i18n");
    std::fs::create_dir_all(&staged).expect("Create staged translation directory");
    for locale in &locales {
        std::fs::copy(directory.join(locale.file), staged.join(locale.file))
            .expect("Stage translation catalog");
    }
    std::fs::write(out.join("i18n.slint"), slint_module(english))
        .expect("Write Slint translations");
    std::fs::write(out.join("i18n_bindings.rs"), rust_module(&locales, english))
        .expect("Write Rust translation bindings");
    out.join("i18n.slint")
}

fn scan(directory: &std::path::Path) -> Vec<LocaleCatalog> {
    let mut paths = std::fs::read_dir(directory)
        .unwrap_or_else(|e| panic!("Read translation directory {}: {e}", directory.display()))
        .map(|entry| entry.expect("Read translation directory entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    paths.sort();
    assert!(!paths.is_empty(), "No translation catalogs found");
    let mut locales = paths
        .into_iter()
        .map(|path| {
            let file_name = path
                .file_name()
                .expect("Catalog file name")
                .to_string_lossy()
                .into_owned();
            let id = file_name
                .strip_suffix(".json")
                .expect("Catalog file extension")
                .to_ascii_lowercase()
                .replace('_', "-");
            assert!(
                id.split('-').all(|part| !part.is_empty()
                    && part
                        .bytes()
                        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())),
                "Invalid translation locale: {file_name}"
            );
            let text = serde_json::from_str::<Catalog>(
                &std::fs::read_to_string(&path)
                    .unwrap_or_else(|e| panic!("Read translation catalog {file_name}: {e}")),
            )
            .unwrap_or_else(|e| panic!("Invalid translation catalog {file_name}: {e}"));
            assert!(
                !text.is_empty(),
                "Translation catalog is empty: {file_name}"
            );
            // Leaked so generated sources can name the catalog file without borrowing.
            let file: &'static str = Box::leak(file_name.into_boxed_str());
            let label_key = format!("{LANGUAGE_LABEL_PREFIX}{}", id.replace('-', "_"));
            LocaleCatalog {
                english_name: text.get(&label_key).cloned().unwrap_or_else(|| id.clone()),
                id,
                file,
                text,
            }
        })
        .collect::<Vec<_>>();
    locales.sort_by(|a, b| a.id.cmp(&b.id));
    assert_eq!(
        locales
            .iter()
            .filter(|locale| locale.id == SOURCE_LOCALE)
            .count(),
        1,
        "Exactly one {SOURCE_LOCALE}.json catalog is required"
    );
    locales
}

fn validate(locales: &[LocaleCatalog], english: &Catalog) {
    for (key, value) in english {
        assert!(valid_key(key), "Invalid translation key: {key} in en.json");
        assert!(!value.is_empty(), "Empty English translation: {key}");
    }
    for locale in locales {
        let unknown = locale
            .text
            .keys()
            .filter(|key| !english.contains_key(*key))
            .cloned()
            .collect::<Vec<_>>();
        assert!(
            unknown.is_empty(),
            "Translation keys missing from en.json ({}): {}",
            locale.id,
            unknown.join(", ")
        );
        for (key, value) in &locale.text {
            assert!(
                valid_key(key),
                "Invalid translation key: {key} in {}.json",
                locale.id
            );
            assert!(
                !value.is_empty() || locale.id == SOURCE_LOCALE,
                "Empty translation: {key} in {}.json",
                locale.id
            );
        }
    }
}

fn valid_key(key: &str) -> bool {
    key.split('.').all(|part| {
        !part.is_empty()
            && !part.starts_with('_')
            && !part.ends_with('_')
            && part
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')
    })
}

fn slint_property(key: &str) -> String {
    key.replace('.', "-")
}

fn rust_field(key: &str) -> String {
    key.replace('.', "_")
}

fn rust_constant(key: &str) -> String {
    rust_field(key).to_uppercase()
}

fn slint_module(english: &Catalog) -> String {
    let mut slint = String::from(
        "// Generated from i18n/*.json. Edit the JSON catalogs.\nexport struct TranslationText {\n",
    );
    let mut defaults = String::new();
    for (key, value) in english {
        slint.push_str(&format!("    {}: string,\n", slint_property(key)));
        defaults.push_str(&format!(
            "        {}: {},\n",
            slint_property(key),
            serde_json::to_string(value).expect("Serialize English translation")
        ));
    }
    slint.push_str("}\nexport global I18n {\n    in-out property <string> locale: ");
    slint.push_str(&serde_json::to_string(SOURCE_LOCALE).expect("Serialize source locale"));
    slint.push_str(";\n    in-out property <TranslationText> text: {\n");
    slint.push_str(&defaults);
    slint.push_str("    };\n}\n");
    slint
}

fn rust_module(locales: &[LocaleCatalog], english: &Catalog) -> String {
    let mut keys = String::from("pub mod keys {\n");
    for key in english.keys() {
        keys.push_str(&format!(
            "    pub const {}: &str = {key:?};\n",
            rust_constant(key)
        ));
    }
    keys.push_str("}\n");

    let mut registry = String::from("pub mod locale {\n    use super::Locale;\n");
    registry.push_str(&format!(
        "    pub const SOURCE: &str = {SOURCE_LOCALE:?};\n"
    ));
    registry.push_str(
        "    pub fn all() -> &'static [Locale] {\n        static ALL: std::sync::LazyLock<Vec<Locale>> =\n            std::sync::LazyLock::new(|| {\n                vec![\n",
    );
    for locale in locales {
        registry.push_str(&format!(
            "                    Locale::new({:?}, {:?}),\n",
            locale.id, locale.english_name
        ));
    }
    registry.push_str("                ]\n            });\n        &ALL\n    }\n");
    registry.push_str(
        "    #[allow(clippy::unwrap_used)]\n    pub fn default_locale() -> &'static Locale {\n        all()\n            .iter()\n            .find(|locale| locale.id() == SOURCE)\n            .unwrap()\n    }\n}\n",
    );

    let mut sources = String::from(
        "fn catalog_source(locale_id: &str) -> &'static str {\n    match locale_id {\n",
    );
    for locale in locales {
        sources.push_str(&format!(
            "        {:?} => include_str!(concat!(env!(\"OUT_DIR\"), \"/i18n/{}\")),\n",
            locale.id, locale.file
        ));
    }
    sources.push_str(
        "        other => unreachable!(\"No compiled catalog for {other}\"),\n    }\n}\n",
    );

    let mut values = String::new();
    for key in english.keys() {
        values.push_str(&format!(
            "        {}: crate::i18n::text(locale, crate::i18n::keys::{}).into(),\n",
            rust_field(key),
            rust_constant(key)
        ));
    }
    let signature = format!(
        "fn translation_values(locale: &str) -> crate::TranslationText {{\n    crate::TranslationText {{\n{}    }}\n}}\n",
        values
    );
    keys + &registry + &sources + &signature
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog(entries: &[(&str, &str)]) -> Catalog {
        entries
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect()
    }

    #[test]
    fn generated_names_follow_the_reserved_mapping() {
        assert_eq!(slint_property("nav.overview"), "nav-overview");
        assert_eq!(rust_field("language.zh_cn"), "language_zh_cn");
        assert_eq!(rust_constant("language.zh_cn"), "LANGUAGE_ZH_CN");
    }

    #[test]
    fn locale_identifiers_become_an_interior_underscore_segment() {
        assert_eq!(
            format!("language.{}", "zh-cn".replace('-', "_")),
            "language.zh_cn"
        );
        assert!(valid_key("language.zh_cn"));
    }

    #[test]
    fn keys_reject_segments_that_cannot_become_identifiers() {
        assert!(valid_key("nav.overview"));
        assert!(!valid_key("nav."));
        assert!(!valid_key("nav..overview"));
        assert!(!valid_key("Nav.Overview"));
        assert!(!valid_key("nav-overview"));
        assert!(!valid_key("nav._overview"));
        assert!(!valid_key("nav.overview_"));
    }

    #[test]
    fn a_locale_may_not_introduce_or_empty_keys() {
        let english = catalog(&[("nav.overview", "Overview")]);
        let extra = LocaleCatalog {
            id: "de".into(),
            english_name: "de".into(),
            text: catalog(&[("nav.overview", "Übersicht"), ("nav.extra", "Extra")]),
        };
        assert!(std::panic::catch_unwind(|| validate(&[extra], &english)).is_err());
        let blank = LocaleCatalog {
            id: "de".into(),
            english_name: "de".into(),
            text: catalog(&[("nav.overview", "")]),
        };
        assert!(std::panic::catch_unwind(|| validate(&[blank], &english)).is_err());
    }
}
