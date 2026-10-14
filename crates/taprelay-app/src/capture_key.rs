pub fn virtual_key(text: &str) -> Option<u8> {
    let named = match text {
        "backspace" => Some(0x08),
        "tab" => Some(0x09),
        "enter" => Some(0x0d),
        "escape" => Some(0x1b),
        "space" => Some(0x20),
        "left" => Some(0x25),
        "up" => Some(0x26),
        "right" => Some(0x27),
        "down" => Some(0x28),
        "delete" => Some(0x2e),
        "insert" => Some(0x2d),
        "home" => Some(0x24),
        "end" => Some(0x23),
        "pageup" => Some(0x21),
        "pagedown" => Some(0x22),
        _ => None,
    };
    if named.is_some() {
        return named;
    }
    if let Some(number) = text
        .strip_prefix('f')
        .and_then(|n| n.parse::<u8>().ok())
        .filter(|n| (1..=24).contains(n))
    {
        return Some(0x6f + number);
    }
    let mut chars = text.chars();
    let ch = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    crate::platform::desktop::virtual_key_for_character(ch)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn named_keys_do_not_depend_on_text_input_or_ime() {
        assert_eq!(virtual_key("enter"), Some(0x0d));
        assert_eq!(virtual_key("f24"), Some(0x87));
        assert_eq!(virtual_key("pageup"), Some(0x21));
        assert_eq!(virtual_key("f25"), None);
        assert_eq!(virtual_key("你好"), None);
    }
}
