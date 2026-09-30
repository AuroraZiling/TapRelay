use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

type Catalog = BTreeMap<String, String>;

fn read_catalog(path: &Path) -> Catalog {
    let mut data: BTreeMap<String, serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(path).expect("Read translations"))
            .unwrap_or_else(|e| panic!("Invalid catalog {}: {e}", path.display()));
    assert_eq!(
        data.remove("_version"),
        Some(1.into()),
        "Expected version 1: {}",
        path.display()
    );
    data.into_iter()
        .map(|(key, value)| {
            let value = value
                .as_str()
                .unwrap_or_else(|| panic!("Expected text: {}/{key}", path.display()));
            (key, value.to_owned())
        })
        .collect()
}

pub fn generate() -> PathBuf {
    let directory = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap()).join("locales");
    println!("cargo:rerun-if-changed={}", directory.display());
    let english = read_catalog(&directory.join("en.json"));
    assert!(!english.is_empty(), "Empty source catalog");
    for (key, value) in &english {
        assert!(
            key.split('.').all(|part| !part.is_empty()
                && !part.starts_with('_')
                && !part.ends_with('_')
                && part
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_')),
            "Invalid translation key: {key}"
        );
        assert!(!value.is_empty(), "Empty source translation: {key}");
    }
    let mut locales = std::collections::BTreeSet::new();
    for entry in std::fs::read_dir(&directory).expect("Read translation directory") {
        let path = entry.expect("Read translation entry").path();
        if path.extension().is_none_or(|ext| ext != "json") {
            continue;
        }
        let id = path.file_stem().unwrap().to_str().expect("UTF-8 locale");
        let normalized = id.to_ascii_lowercase().replace('_', "-");
        assert!(
            normalized.split('-').all(|part| !part.is_empty()
                && part
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())),
            "Invalid locale: {id}"
        );
        assert!(locales.insert(normalized.clone()), "Duplicate locale: {id}");
        let catalog = read_catalog(&path);
        assert!(
            english.keys().eq(catalog.keys()),
            "Translation keys differ: {id}"
        );
        for (key, value) in &catalog {
            assert!(!value.is_empty(), "Empty translation: {id}/{key}");
        }
        assert!(
            catalog.contains_key(&format!("language.{}", normalized.replace('-', "_"))),
            "Missing language label: {id}"
        );
        // The native overlay elides only the device name, so it needs one split point.
        for key in [
            "overlay.controlling",
            "overlay.stopped",
            "overlay.disconnected",
        ] {
            assert_eq!(
                catalog[key].matches("%{device}").count(),
                1,
                "Invalid device placeholder: {id}/{key}"
            );
        }
        for key in ["groups.count_one", "groups.count_other"] {
            assert_eq!(
                catalog[key].matches("%{count}").count(),
                1,
                "Invalid count placeholder: {id}/{key}"
            );
        }
    }

    let mut rust = String::from("pub mod keys {\n");
    let mut slint = String::from(
        "// Generated typed boundary for rust-i18n catalogs.\nexport struct TranslationText {\n",
    );
    let mut defaults = String::new();
    let mut values = String::new();
    for (key, value) in &english {
        let field = key.replace('.', "_");
        let constant = field.to_uppercase();
        let property = key.replace('.', "-");
        rust.push_str(&format!("    pub const {constant}: &str = {key:?};\n"));
        slint.push_str(&format!("    {property}: string,\n"));
        defaults.push_str(&format!(
            "        {property}: {},\n",
            serde_json::to_string(value).unwrap()
        ));
        values.push_str(&format!(
            "        {field}: rust_i18n::t!(keys::{constant}, locale = locale).into_owned().into(),\n"
        ));
    }
    rust.push_str("}\nfn translation_values(locale: &str) -> crate::TranslationText {\n    crate::TranslationText {\n");
    rust.push_str(&values);
    rust.push_str("    }\n}\n");
    slint.push_str("}\nexport global I18n {\n    in-out property <string> locale: \"en\";\n    in-out property <TranslationText> text: {\n");
    slint.push_str(&defaults);
    slint.push_str("    };\n}\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(out.join("i18n_bindings.rs"), rust).expect("Write Rust translation bindings");
    std::fs::write(out.join("i18n.slint"), slint).expect("Write Slint translation bindings");
    out.join("i18n.slint")
}
