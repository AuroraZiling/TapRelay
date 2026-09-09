use slint::platform::Key;

pub fn virtual_key(text: &str) -> Option<u8> {
    let mut chars = text.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    let named = [
        (Key::Backspace, 0x08),
        (Key::Tab, 0x09),
        (Key::Backtab, 0x09),
        (Key::Return, 0x0d),
        (Key::Escape, 0x1b),
        (Key::Space, 0x20),
        (Key::Shift, 0xa0),
        (Key::ShiftR, 0xa1),
        (Key::Control, 0xa2),
        (Key::ControlR, 0xa3),
        (Key::Alt, 0xa4),
        (Key::AltGr, 0xa5),
        (Key::Meta, 0x5b),
        (Key::MetaR, 0x5c),
        (Key::CapsLock, 0x14),
        (Key::LeftArrow, 0x25),
        (Key::UpArrow, 0x26),
        (Key::RightArrow, 0x27),
        (Key::DownArrow, 0x28),
        (Key::Delete, 0x2e),
        (Key::Insert, 0x2d),
        (Key::Home, 0x24),
        (Key::End, 0x23),
        (Key::PageUp, 0x21),
        (Key::PageDown, 0x22),
    ];
    if let Some((_, vk)) = named.into_iter().find(|(key, _)| char::from(*key) == ch) {
        return Some(vk);
    }
    let first = char::from(Key::F1) as u32;
    if (first..=char::from(Key::F24) as u32).contains(&(ch as u32)) {
        return Some((0x70 + ch as u32 - first) as u8);
    }
    crate::platform::desktop::virtual_key_for_character(ch)
}
