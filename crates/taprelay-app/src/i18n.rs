use rust_i18n::t;
use std::sync::LazyLock;

pub mod locale {
    use super::*;
    pub const SOURCE: &str = "en";

    pub fn all() -> &'static [String] {
        static LOCALES: LazyLock<Vec<String>> = LazyLock::new(|| {
            let mut locales: Vec<_> = rust_i18n::available_locales!()
                .into_iter()
                .map(|id| id.into_owned())
                .collect();
            locales.sort();
            locales
        });
        &LOCALES
    }
}

include!(concat!(env!("OUT_DIR"), "/i18n_bindings.rs"));

pub fn resolve(id: &str) -> Option<&'static str> {
    locale::all()
        .iter()
        .map(String::as_str)
        .find(|locale| *locale == id)
}

pub fn system_locale() -> &'static str {
    crate::platform::desktop::system_locale_id()
        .as_deref()
        .and_then(resolve)
        .unwrap_or(locale::SOURCE)
}

pub fn foreground_app_count(locale: &str, count: usize) -> String {
    if count == 1 {
        t!("groups.count_one", locale = locale, count = count).into_owned()
    } else {
        t!("groups.count_other", locale = locale, count = count).into_owned()
    }
}

pub fn language_options(locale_id: &str) -> Vec<crate::gui::presentation::ChoiceOption> {
    let mut options = vec![crate::gui::presentation::ChoiceOption {
        label: t!("language.system", locale = locale_id).into_owned(),
        value: "system".into(),
    }];
    options.extend(locale::all().iter().map(|locale| {
        crate::gui::presentation::ChoiceOption {
            label: t!(
                format!("language.{}", locale.to_ascii_lowercase().replace('-', "_")),
                locale = locale
            )
            .into_owned(),
            value: locale.as_str().into(),
        }
    }));
    options
}

pub fn tray_labels(locale: &str) -> [String; 4] {
    [
        keys::TRAY_OPEN,
        keys::LISTENING_START,
        keys::LISTENING_STOP,
        keys::COMMON_QUIT,
    ]
    .map(|key| t!(key, locale = locale).into_owned())
}

#[cfg(test)]
mod tests;
