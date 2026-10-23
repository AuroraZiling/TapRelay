//! Stable app key identities retain the existing Windows configuration values.
//! macOS hardware keycodes are translated at this boundary.
use taprelay_core::input::{KeyboardUsage, PhysicalKey};
#[derive(Clone, Copy)]
pub struct NativeKeyCode(pub u16);
pub struct Key {
    pub native: NativeKeyCode,
    pub physical: PhysicalKey,
    pub usage: KeyboardUsage,
}
impl Key {
    const fn new(native: u16, physical: u8, usage: u8) -> Self {
        Self {
            native: NativeKeyCode(native),
            physical: PhysicalKey::from_legacy_code(physical),
            usage: KeyboardUsage::new(usage),
        }
    }
}
pub const KEYS: &[Key] = &[
    Key::new(0, 0x41, 4),
    Key::new(1, 0x53, 22),
    Key::new(2, 0x44, 7),
    Key::new(3, 0x46, 9),
    Key::new(4, 0x48, 11),
    Key::new(5, 0x47, 10),
    Key::new(6, 0x5a, 29),
    Key::new(7, 0x58, 27),
    Key::new(8, 0x43, 6),
    Key::new(9, 0x56, 25),
    Key::new(10, 0xe2, 0x64), // ISO extra key; normalize the ISO swap at the event boundary.
    Key::new(11, 0x42, 5),
    Key::new(12, 0x51, 20),
    Key::new(13, 0x57, 26),
    Key::new(14, 0x45, 8),
    Key::new(15, 0x52, 21),
    Key::new(16, 0x59, 28),
    Key::new(17, 0x54, 23),
    Key::new(18, 0x31, 30),
    Key::new(19, 0x32, 31),
    Key::new(20, 0x33, 32),
    Key::new(21, 0x34, 33),
    Key::new(22, 0x36, 35),
    Key::new(23, 0x35, 34),
    Key::new(24, 0xbb, 46),
    Key::new(25, 0x39, 38),
    Key::new(26, 0x37, 36),
    Key::new(27, 0xbd, 45),
    Key::new(28, 0x38, 37),
    Key::new(29, 0x30, 39),
    Key::new(30, 0xdd, 48),
    Key::new(31, 0x4f, 18),
    Key::new(32, 0x55, 24),
    Key::new(33, 0xdb, 47),
    Key::new(34, 0x49, 12),
    Key::new(35, 0x50, 19),
    Key::new(36, 0x0d, 40),
    Key::new(37, 0x4c, 15),
    Key::new(38, 0x4a, 13),
    Key::new(39, 0xde, 52),
    Key::new(40, 0x4b, 14),
    Key::new(41, 0xba, 51),
    Key::new(42, 0xdc, 49),
    Key::new(43, 0xbc, 54),
    Key::new(44, 0xbf, 56),
    Key::new(45, 0x4e, 17),
    Key::new(46, 0x4d, 16),
    Key::new(47, 0xbe, 55),
    Key::new(48, 0x09, 43),
    Key::new(49, 0x20, 44),
    Key::new(50, 0xc0, 53),
    Key::new(51, 0x08, 42),
    Key::new(53, 0x1b, 41),
    Key::new(54, 0x5c, 0xe7),
    Key::new(55, 0x5b, 0xe3),
    Key::new(56, 0xa0, 0xe1),
    Key::new(57, 0x14, 0x39),
    Key::new(58, 0xa4, 0xe2),
    Key::new(59, 0xa2, 0xe0),
    Key::new(60, 0xa1, 0xe5),
    Key::new(61, 0xa5, 0xe6),
    Key::new(62, 0xa3, 0xe4),
    Key::new(64, 0x80, 0x6c), // F17
    Key::new(65, 0x6e, 0x63),
    Key::new(67, 0x6a, 0x55),
    Key::new(69, 0x6b, 0x57),
    Key::new(71, 0x90, 0x53),
    Key::new(75, 0x6f, 0x54),
    Key::new(76, 0xe0, 0x58),
    Key::new(78, 0x6d, 0x56),
    Key::new(79, 0x81, 0x6d), // F18
    Key::new(80, 0x82, 0x6e), // F19
    Key::new(81, 0x92, 0x67),
    Key::new(82, 0x60, 0x62),
    Key::new(83, 0x61, 0x59),
    Key::new(84, 0x62, 0x5a),
    Key::new(85, 0x63, 0x5b),
    Key::new(86, 0x64, 0x5c),
    Key::new(87, 0x65, 0x5d),
    Key::new(88, 0x66, 0x5e),
    Key::new(89, 0x67, 0x5f),
    Key::new(90, 0x83, 0x6f), // F20
    Key::new(91, 0x68, 0x60),
    Key::new(92, 0x69, 0x61),
    // Distinct physical positions in the legacy byte vocabulary. These two
    // OEM slots were unused by this adapter; existing Windows IDs stay intact.
    Key::new(93, 0xc1, 0x89), // JIS Yen (International3)
    Key::new(94, 0xc2, 0x87), // JIS Ro (International1)
    Key::new(95, 0x6c, 0x85), // JIS keypad comma
    Key::new(96, 0x74, 0x3e),
    Key::new(97, 0x75, 0x3f),
    Key::new(98, 0x76, 0x40),
    Key::new(99, 0x72, 0x3c),
    Key::new(100, 0x77, 0x41),
    Key::new(101, 0x78, 0x42),
    Key::new(102, 0x1a, 0x91), // Eisu / LANG2
    Key::new(103, 0x7a, 0x44),
    Key::new(104, 0x15, 0x90), // Kana / LANG1
    Key::new(105, 0x7c, 0x68),
    Key::new(106, 0x7f, 0x6b),
    Key::new(107, 0x7d, 0x69),
    Key::new(109, 0x79, 0x43),
    Key::new(110, 0x5d, 0x65), // Application menu
    Key::new(111, 0x7b, 0x45),
    Key::new(113, 0x7e, 0x6a),
    Key::new(114, 0x2f, 0x75),
    Key::new(115, 0x24, 0x4a),
    Key::new(116, 0x21, 0x4b),
    Key::new(117, 0x2e, 0x4c),
    Key::new(118, 0x73, 0x3d),
    Key::new(119, 0x23, 0x4d),
    Key::new(120, 0x71, 0x3b),
    Key::new(121, 0x22, 0x4e),
    Key::new(122, 0x70, 0x3a),
    Key::new(123, 0x25, 0x50),
    Key::new(124, 0x27, 0x4f),
    Key::new(125, 0x28, 0x51),
    Key::new(126, 0x26, 0x52),
];
// Compile-time indexes keep native input/replay lookups allocation-free and O(1).
const NATIVE_INDEX: [Option<u8>; 128] = {
    let mut index = [None; 128];
    let mut i = 0;
    while i < KEYS.len() {
        index[KEYS[i].native.0 as usize] = Some(KEYS[i].physical.legacy_code());
        i += 1;
    }
    index
};
const PHYSICAL_INDEX: [Option<(u16, u8)>; 256] = {
    let mut index = [None; 256];
    let mut i = 0;
    while i < KEYS.len() {
        index[KEYS[i].physical.legacy_code() as usize] =
            Some((KEYS[i].native.0, KEYS[i].usage.get()));
        i += 1;
    }
    index
};
pub fn internal(native: u16) -> Option<u8> {
    NATIVE_INDEX.get(native as usize).copied().flatten()
}
pub fn native(key: u8) -> Option<u16> {
    PHYSICAL_INDEX[key as usize].map(|(native, _)| native)
}
pub fn usage(key: u8) -> Option<u8> {
    PHYSICAL_INDEX[key as usize].map(|(_, usage)| usage)
}
/// Apple's ISO driver exchanges the grave and extra-backslash positions.
/// Apply the same involution for native input and local replay.
pub fn normalize_native(native: u16, iso: bool) -> u16 {
    match (iso, native) {
        (true, 10) => 50,
        (true, 50) => 10,
        _ => native,
    }
}
pub fn name(key: u8) -> String {
    match key {
        0xc1 => "JIS Yen".into(),
        0xc2 => "JIS Ro".into(),
        0x6c => "JIS Numpad ,".into(),
        0x1a => "Eisu".into(),
        0x15 => "Kana".into(),
        0x5b => "Left Command".into(),
        0x5c => "Right Command".into(),
        0xa4 => "Left Option".into(),
        0xa5 => "Right Option".into(),
        0xa2 => "Left Control".into(),
        0xa3 => "Right Control".into(),
        _ => taprelay_core::input::key_name(key),
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extended_hardware_keys_and_iso_replay_keep_their_hid_usage() {
        for (native, hid) in [
            (10, 0x64),
            (64, 0x6c),
            (79, 0x6d),
            (80, 0x6e),
            (90, 0x6f),
            (93, 0x89),
            (94, 0x87),
            (95, 0x85),
            (102, 0x91),
            (104, 0x90),
            (110, 0x65),
        ] {
            let key = internal(native).expect("supported hardware key");
            assert_eq!(usage(key), Some(hid));
            assert_eq!(super::native(key), Some(native));
        }
        assert_eq!(
            usage(internal(normalize_native(10, true)).unwrap()),
            Some(0x35)
        );
        assert_eq!(
            usage(internal(normalize_native(50, true)).unwrap()),
            Some(0x64)
        );
        for iso in [false, true] {
            for key in KEYS {
                assert_eq!(
                    normalize_native(normalize_native(key.native.0, iso), iso),
                    key.native.0
                );
            }
        }
        assert_eq!(internal(u16::MAX), None);
    }
    #[test]
    fn identities_are_bijective_and_modifiers_keep_hid_meanings() {
        let mut native = std::collections::BTreeSet::new();
        let mut internal = std::collections::BTreeSet::new();
        for entry in KEYS {
            let n = entry.native.0;
            let k = entry.physical.legacy_code();
            assert!(native.insert(n));
            assert!(internal.insert(k));
            assert_eq!(super::native(k), Some(n));
        }
        assert_eq!(usage(0xa2), Some(0xe0));
        assert_eq!(usage(0x5b), Some(0xe3));
        assert_eq!(super::internal(0), Some(0x41));
    }
}
