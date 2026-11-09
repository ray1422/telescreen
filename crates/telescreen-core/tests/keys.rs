//! Physical key identity (IN-5): macOS virtual key codes and evdev key codes.
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.

use telescreen_core::keys::{self, Key, SpecialKeyGroup};

/// (macOS kVK_* code, evdev KEY_* code) pairs from `HIToolbox/Events.h` and
/// `linux/input-event-codes.h`.
const KNOWN: &[(u16, u16, &str)] = &[
    (0x00, 30, "A"),
    (0x01, 31, "S"),
    (0x0C, 16, "Q"),
    (0x06, 44, "Z"),
    (0x12, 2, "1"),
    (0x1D, 11, "0"),
    (0x24, 28, "Return"),
    (0x30, 15, "Tab"),
    (0x31, 57, "Space"),
    (0x33, 14, "Delete (backspace)"),
    (0x35, 1, "Escape"),
    (0x75, 111, "ForwardDelete"),
    (0x32, 41, "Grave"),
    (0x0A, 86, "ISO Section (KEY_102ND)"),
    (0x2A, 43, "Backslash"),
    (0x27, 40, "Quote"),
    (0x37, 125, "Command"),
    (0x36, 126, "RightCommand"),
    (0x3A, 56, "Option"),
    (0x3D, 100, "RightOption"),
    (0x3B, 29, "Control"),
    (0x3E, 97, "RightControl"),
    (0x38, 42, "Shift"),
    (0x3C, 54, "RightShift"),
    (0x39, 58, "CapsLock"),
    (0x3F, 464, "Function (KEY_FN)"),
    (0x7A, 59, "F1"),
    (0x6F, 88, "F12"),
    (0x69, 183, "F13"),
    (0x5A, 190, "F20"),
    (0x7B, 105, "LeftArrow"),
    (0x7C, 106, "RightArrow"),
    (0x7D, 108, "DownArrow"),
    (0x7E, 103, "UpArrow"),
    (0x73, 102, "Home"),
    (0x77, 107, "End"),
    (0x74, 104, "PageUp"),
    (0x79, 109, "PageDown"),
    (0x72, 110, "Help (KEY_INSERT)"),
    (0x4C, 96, "KeypadEnter"),
    (0x52, 82, "Keypad0"),
    (0x5C, 73, "Keypad9"),
    (0x41, 83, "KeypadDecimal"),
    (0x43, 55, "KeypadMultiply"),
    (0x45, 78, "KeypadPlus"),
    (0x4E, 74, "KeypadMinus"),
    (0x4B, 98, "KeypadDivide"),
    (0x51, 117, "KeypadEquals"),
    (0x47, 69, "KeypadClear (KEY_NUMLOCK)"),
    (0x48, 115, "VolumeUp"),
    (0x49, 114, "VolumeDown"),
    (0x4A, 113, "Mute"),
    (0x5D, 124, "JIS Yen"),
    (0x5E, 89, "JIS Underscore (KEY_RO)"),
    (0x5F, 95, "JIS KeypadComma"),
    (0x66, 123, "JIS Eisu (KEY_HANJA / LANG2)"),
    (0x68, 122, "JIS Kana (KEY_HANGEUL / LANG1)"),
];

#[test]
fn known_pairs() {
    for &(mac, evdev, name) in KNOWN {
        assert_eq!(keys::from_mac(mac), Some(Key(evdev)), "from_mac {name}");
        assert_eq!(keys::to_mac(Key(evdev)), Some(mac), "to_mac {name}");
    }
}

#[test]
fn all_letters_digits_and_function_keys_map() {
    // KEY_Q..KEY_P, KEY_A..KEY_L, KEY_Z..KEY_M, KEY_1..KEY_0, KEY_F1..KEY_F10.
    let ranges = [16..=25, 30..=38, 44..=50, 2..=11, 59..=68];
    for code in ranges.into_iter().flatten() {
        let mac = keys::to_mac(Key(code)).unwrap_or_else(|| panic!("evdev {code} has no mac code"));
        assert_eq!(
            keys::from_mac(mac),
            Some(Key(code)),
            "round trip of evdev {code}"
        );
    }
}

#[test]
fn every_mapping_round_trips() {
    for mac in 0..=0x7Fu16 {
        if let Some(key) = keys::from_mac(mac) {
            assert_eq!(keys::to_mac(key), Some(mac), "mac {mac:#x} -> {key:?}");
        }
    }
    for code in 0..=0x2FFu16 {
        if let Some(mac) = keys::to_mac(Key(code)) {
            assert_eq!(
                keys::from_mac(mac),
                Some(Key(code)),
                "evdev {code} -> {mac:#x}"
            );
        }
    }
}

#[test]
fn unknown_codes() {
    assert_eq!(keys::from_mac(0xFFFF), None);
    assert_eq!(keys::from_mac(0x80), None);
    assert_eq!(keys::to_mac(Key(0)), None, "KEY_RESERVED");
    assert_eq!(keys::to_mac(Key(0x2FF)), None);
}

/// IN-20: (`NX_KEYTYPE_*` from `IOKit/hidsystem/ev_keymap.h`, group, evdev KEY_* code).
const SPECIAL: &[(u16, SpecialKeyGroup, u16, &str)] = &[
    (0, SpecialKeyGroup::Volume, 115, "SOUND_UP -> KEY_VOLUMEUP"),
    (
        1,
        SpecialKeyGroup::Volume,
        114,
        "SOUND_DOWN -> KEY_VOLUMEDOWN",
    ),
    (
        2,
        SpecialKeyGroup::Brightness,
        225,
        "BRIGHTNESS_UP -> KEY_BRIGHTNESSUP",
    ),
    (
        3,
        SpecialKeyGroup::Brightness,
        224,
        "BRIGHTNESS_DOWN -> KEY_BRIGHTNESSDOWN",
    ),
    (7, SpecialKeyGroup::Volume, 113, "MUTE -> KEY_MUTE"),
    (16, SpecialKeyGroup::Media, 164, "PLAY -> KEY_PLAYPAUSE"),
    (17, SpecialKeyGroup::Media, 163, "NEXT -> KEY_NEXTSONG"),
    (
        18,
        SpecialKeyGroup::Media,
        165,
        "PREVIOUS -> KEY_PREVIOUSSONG",
    ),
    (
        19,
        SpecialKeyGroup::Media,
        163,
        "FAST (Apple F9) -> KEY_NEXTSONG",
    ),
    (
        20,
        SpecialKeyGroup::Media,
        165,
        "REWIND (Apple F7) -> KEY_PREVIOUSSONG",
    ),
    (
        21,
        SpecialKeyGroup::KeyboardLight,
        230,
        "ILLUMINATION_UP -> KEY_KBDILLUMUP",
    ),
    (
        22,
        SpecialKeyGroup::KeyboardLight,
        229,
        "ILLUMINATION_DOWN -> KEY_KBDILLUMDOWN",
    ),
    (
        23,
        SpecialKeyGroup::KeyboardLight,
        228,
        "ILLUMINATION_TOGGLE -> KEY_KBDILLUMTOGGLE",
    ),
];

#[test]
fn in20_special_keys_map_to_their_group_and_evdev_key() {
    for &(key_type, group, code, name) in SPECIAL {
        assert_eq!(
            keys::from_mac_special(key_type),
            Some((group, Key(code))),
            "{name}"
        );
    }
}

#[test]
fn in20_other_special_keys_have_no_group() {
    for key_type in 0..=64u16 {
        if SPECIAL.iter().any(|&(t, ..)| t == key_type) {
            continue;
        }
        // For example CAPS_LOCK (4), POWER_KEY (6), NUM_LOCK (10), EJECT (14).
        assert_eq!(
            keys::from_mac_special(key_type),
            None,
            "NX_KEYTYPE {key_type}"
        );
    }
}
