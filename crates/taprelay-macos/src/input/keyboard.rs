//! Quartz keyboard quirks belong at the native boundary
use super::*;
use std::sync::OnceLock;

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn KBGetLayoutType(keyboard_type: i16) -> u32;
}
static ISO_LAYOUTS: OnceLock<[bool; 256]> = OnceLock::new();

pub(crate) fn initialize() -> Result<(), BackendError> {
    if ISO_LAYOUTS.get().is_none() {
        // Carbon documents this query as non-thread-safe. Resolve all byte-sized
        // hardware types on the UI thread; callbacks only read the immutable map.
        let _main = objc2::MainThreadMarker::new().ok_or_else(|| {
            BackendError::Unavailable("Initialize keyboard layouts on the main thread".into())
        })?;
        ISO_LAYOUTS.get_or_init(|| {
            std::array::from_fn(|kind| unsafe {
                KBGetLayoutType(kind as i16) == u32::from_be_bytes(*b"ISO ")
            })
        });
    }
    Ok(())
}

pub(super) unsafe fn normalize(native: u16, event: NativeEvent) -> u16 {
    if !matches!(native, 10 | 50) {
        return native;
    }
    let kind =
        unsafe { CGEventGetIntegerValueField(event, CGEventField::KeyboardEventKeyboardType) };
    let iso = usize::try_from(kind)
        .ok()
        .and_then(|kind| ISO_LAYOUTS.get()?.get(kind))
        .copied()
        .unwrap_or(false);
    keys::normalize_native(native, iso)
}

/// Device-specific NX flags distinguish both sides. Synthetic/remote events
/// may supply only public CG flags; fall back when the two disagree, as SDL does.
pub(super) fn modifier_down(native: u16, flags: CGEventFlags) -> Option<bool> {
    let (left_key, left, right, either) = match native {
        56 | 60 => (56, 0x02, 0x04, CGEventFlags::MaskShift),
        59 | 62 => (59, 0x01, 0x2000, CGEventFlags::MaskControl),
        58 | 61 => (58, 0x20, 0x40, CGEventFlags::MaskAlternate),
        55 | 54 => (55, 0x08, 0x10, CGEventFlags::MaskCommand),
        _ => return None,
    };
    let either_pressed = flags.contains(either);
    let sided = flags.bits() & (left | right);
    let target = if native == left_key { left } else { right };
    Some(if either_pressed != (sided != 0) {
        either_pressed
    } else {
        sided & target != 0
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn public_and_sided_modifier_flags_preserve_releases_and_chords() {
        for (left, right, left_flag, right_flag, public) in [
            (56, 60, 0x02, 0x04, CGEventFlags::MaskShift),
            (59, 62, 0x01, 0x2000, CGEventFlags::MaskControl),
            (58, 61, 0x20, 0x40, CGEventFlags::MaskAlternate),
            (55, 54, 0x08, 0x10, CGEventFlags::MaskCommand),
        ] {
            for key in [left, right] {
                assert_eq!(modifier_down(key, public), Some(true));
                assert_eq!(modifier_down(key, CGEventFlags::empty()), Some(false));
                assert_eq!(
                    modifier_down(key, public | CGEventFlags(left_flag | right_flag)),
                    Some(true)
                );
            }
            assert_eq!(
                modifier_down(left, public | CGEventFlags(right_flag)),
                Some(false)
            );
            assert_eq!(
                modifier_down(right, public | CGEventFlags(right_flag)),
                Some(true)
            );
            assert_eq!(
                modifier_down(left, CGEventFlags(left_flag)),
                Some(false),
                "public release overrides stale side flags"
            );
        }
        assert_eq!(modifier_down(0, CGEventFlags::MaskShift), None);
    }
}
