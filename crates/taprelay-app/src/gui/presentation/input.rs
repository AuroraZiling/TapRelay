use crate::{i18n::keys, platform, runtime::CaptureSession};
use taprelay_core::{
    function::{ModifierSet, Shortcut},
    input::{InputCode, MouseButton, modifier},
};

/// Key names belong to presentation; persisted modifier bits remain portable.
pub fn modifier_labels(modifiers: ModifierSet) -> Vec<String> {
    let names = if cfg!(target_os = "macos") {
        ["Control", "Shift", "Option", "Command"]
    } else {
        ["Ctrl", "Shift", "Alt", "Win"]
    };
    [
        modifiers.ctrl,
        modifiers.shift,
        modifiers.alt,
        modifiers.win,
    ]
    .into_iter()
    .zip(names)
    .filter(|(held, _)| *held)
    .map(|(_, label)| label.to_owned())
    .collect()
}

pub fn shortcut_labels(locale: &str, shortcut: &Shortcut) -> Vec<String> {
    let mut labels = modifier_labels(shortcut.modifiers);
    labels.push(input_label(locale, shortcut.primary_code()));
    labels
}

pub fn capture_preview(locale: &str, capture: &CaptureSession) -> String {
    let mut labels = modifier_labels(ModifierSet::from_keys(capture.preview.iter().filter_map(
        |code| match code {
            InputCode::Key(key) => Some(*key),
            InputCode::Mouse(_) => None,
        },
    )));
    labels.extend(
        capture
            .preview
            .iter()
            .filter(|code| !matches!(code, InputCode::Key(key) if modifier(*key)))
            .map(|code| input_label(locale, *code)),
    );
    labels.join("+")
}

fn input_label(locale: &str, input: InputCode) -> String {
    match input {
        InputCode::Key(key) => platform::key_name(key),
        InputCode::Mouse(button) => mouse_button_label(locale, button),
    }
}

pub fn mouse_button_label(locale: &str, button: MouseButton) -> String {
    rust_i18n::t!(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{CaptureKind, CapturePhase};

    #[test]
    fn mouse_shortcuts_and_in_progress_capture_use_the_same_localized_labels() {
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
}
