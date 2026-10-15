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
    // Accept legacy config/OS spellings, but pass the catalog's exact ID to t!.
    let normalized = id.trim().replace('_', "-");
    locale::all()
        .iter()
        .map(String::as_str)
        .find(|id| id.eq_ignore_ascii_case(&normalized))
}

pub fn system_locale() -> &'static str {
    crate::platform::desktop::system_locale_id()
        .as_deref()
        .and_then(resolve)
        .unwrap_or(locale::SOURCE)
}

pub fn foreground_app_count(locale_id: &str, count: usize) -> String {
    let locale = resolve(locale_id).unwrap_or(locale_id);
    if count == 1 {
        t!("groups.count_one", locale = locale, count = count).into_owned()
    } else {
        t!("groups.count_other", locale = locale, count = count).into_owned()
    }
}

pub fn shortcut_labels(
    locale_id: &str,
    shortcut: &taprelay_core::function::Shortcut,
) -> Vec<String> {
    let mut labels = shortcut.modifiers.labels();
    labels.push(input_label(locale_id, shortcut.primary_code()));
    labels
}

pub fn capture_preview(locale_id: &str, capture: &crate::runtime::CaptureSession) -> String {
    use taprelay_core::{
        function::ModifierSet,
        input::{InputCode, modifier},
    };
    let mut labels = ModifierSet::from_keys(capture.preview.iter().filter_map(|code| match code {
        InputCode::Key(key) => Some(*key),
        InputCode::Mouse(_) => None,
    }))
    .labels();
    labels.extend(
        capture
            .preview
            .iter()
            .filter(|code| !matches!(code, InputCode::Key(key) if modifier(*key)))
            .map(|code| input_label(locale_id, *code)),
    );
    labels.join("+")
}

fn input_label(locale_id: &str, input: taprelay_core::input::InputCode) -> String {
    match input {
        taprelay_core::input::InputCode::Key(key) => crate::platform::key_name(key),
        taprelay_core::input::InputCode::Mouse(button) => mouse_button_label(locale_id, button),
    }
}

pub fn mouse_button_label(locale_id: &str, button: taprelay_core::input::MouseButton) -> String {
    use taprelay_core::input::MouseButton;
    let locale = resolve(locale_id).unwrap_or(locale::SOURCE);
    t!(
        match button {
            MouseButton::Left => keys::INPUT_MOUSE_LEFT,
            MouseButton::Right => keys::INPUT_MOUSE_RIGHT,
            MouseButton::Middle => keys::INPUT_MOUSE_MIDDLE,
            MouseButton::Side1 => keys::INPUT_MOUSE_SIDE1,
            MouseButton::Side2 => keys::INPUT_MOUSE_SIDE2,
        },
        locale = locale
    )
    .into_owned()
}

pub fn language_options(locale_id: &str) -> Vec<crate::gui::presentation::ChoiceOption> {
    option_rows(locale_id)
}

/// The single definition of the selector content, shared by the model and its tests.
fn option_rows(locale_id: &str) -> Vec<crate::gui::presentation::ChoiceOption> {
    let locale_id = resolve(locale_id).unwrap_or(locale::SOURCE);
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

pub fn tray_labels(locale_id: &str) -> [String; 4] {
    let locale = resolve(locale_id).unwrap_or(locale_id);
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
