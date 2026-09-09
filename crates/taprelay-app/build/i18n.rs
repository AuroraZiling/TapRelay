use std::{collections::BTreeMap, path::PathBuf};

pub fn generate() -> PathBuf {
    let read = |name: &str| -> BTreeMap<String, String> {
        let path = format!("i18n/{name}.json");
        println!("cargo:rerun-if-changed={path}");
        serde_json::from_str(&std::fs::read_to_string(&path).expect("Read translation catalog"))
            .unwrap_or_else(|e| panic!("Invalid translation catalog {path}: {e}"))
    };
    let english = read("en");
    let chinese = read("zh-CN");
    assert!(!english.is_empty(), "English catalog is empty");
    for key in chinese.keys() {
        assert!(
            english.contains_key(key),
            "Unknown Chinese translation key: {key}"
        );
    }
    let mut slint = String::from(
        "// Generated from i18n/en.json. Edit the JSON catalogs.\nexport struct TranslationText {\n",
    );
    let mut constants = String::from("pub mod keys {\n");
    let mut values = String::from(
        "fn translation_values(chinese: bool) -> crate::TranslationText {\n    crate::TranslationText {\n",
    );
    let mut defaults = String::new();
    for (key, value) in &english {
        assert!(
            key.split('.').all(|part| !part.is_empty()
                && part
                    .bytes()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())),
            "Invalid translation key: {key}"
        );
        assert!(!value.is_empty(), "Empty English translation: {key}");
        let property = key.replace('.', "-");
        let field = key.replace('.', "_");
        let constant = field.to_uppercase();
        slint.push_str(&format!("    {property}: string,\n"));
        defaults.push_str(&format!(
            "        {property}: {},\n",
            serde_json::to_string(value).unwrap()
        ));
        constants.push_str(&format!("    pub const {constant}: &str = {key:?};\n"));
        values.push_str(&format!(
            "        {field}: text(chinese, keys::{constant}).into(),\n"
        ));
    }
    slint.push_str("}\nexport global I18n {\n    in-out property <string> locale: \"en\";\n    in-out property <TranslationText> text: {\n");
    slint.push_str(&defaults);
    slint.push_str("    };\n}\n");
    constants.push_str("}\n");
    values.push_str("    }\n}\n");
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    std::fs::write(out.join("i18n.slint"), slint).unwrap();
    std::fs::write(out.join("i18n_bindings.rs"), constants + &values).unwrap();
    out.join("i18n.slint")
}
