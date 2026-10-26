//! Physical key identity (IN-5).
//!
//! The wire key identity is the Linux evdev key code (`linux/input-event-codes.h`),
//! for example `KEY_A` = 30. It is a physical key, not a character. The input
//! method and the keyboard layout do not change it.

use serde::{Deserialize, Serialize};

/// A physical key, as a Linux evdev key code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Key(pub u16);

impl Key {
    pub const ESC: Key = Key(1);
    pub const ENTER: Key = Key(28);
    pub const LEFT_CTRL: Key = Key(29);
    pub const LEFT_SHIFT: Key = Key(42);
    pub const RIGHT_SHIFT: Key = Key(54);
    pub const LEFT_ALT: Key = Key(56);
    pub const RIGHT_CTRL: Key = Key(97);
    pub const RIGHT_ALT: Key = Key(100);
    pub const KP_ENTER: Key = Key(96);
    pub const LEFT_META: Key = Key(125);
    pub const RIGHT_META: Key = Key(126);
}

const MAC_EVDEV_CODES: &[(u16, u16)] = &[
    (0x00, 30),  // kVK_ANSI_A, KEY_A
    (0x01, 31),  // kVK_ANSI_S, KEY_S
    (0x02, 32),  // kVK_ANSI_D, KEY_D
    (0x03, 33),  // kVK_ANSI_F, KEY_F
    (0x04, 35),  // kVK_ANSI_H, KEY_H
    (0x05, 34),  // kVK_ANSI_G, KEY_G
    (0x06, 44),  // kVK_ANSI_Z, KEY_Z
    (0x07, 45),  // kVK_ANSI_X, KEY_X
    (0x08, 46),  // kVK_ANSI_C, KEY_C
    (0x09, 47),  // kVK_ANSI_V, KEY_V
    (0x0A, 86),  // kVK_ISO_Section, KEY_102ND
    (0x0B, 48),  // kVK_ANSI_B, KEY_B
    (0x0C, 16),  // kVK_ANSI_Q, KEY_Q
    (0x0D, 17),  // kVK_ANSI_W, KEY_W
    (0x0E, 18),  // kVK_ANSI_E, KEY_E
    (0x0F, 19),  // kVK_ANSI_R, KEY_R
    (0x10, 21),  // kVK_ANSI_Y, KEY_Y
    (0x11, 20),  // kVK_ANSI_T, KEY_T
    (0x12, 2),   // kVK_ANSI_1, KEY_1
    (0x13, 3),   // kVK_ANSI_2, KEY_2
    (0x14, 4),   // kVK_ANSI_3, KEY_3
    (0x15, 5),   // kVK_ANSI_4, KEY_4
    (0x16, 7),   // kVK_ANSI_6, KEY_6
    (0x17, 6),   // kVK_ANSI_5, KEY_5
    (0x18, 13),  // kVK_ANSI_Equal, KEY_EQUAL
    (0x19, 10),  // kVK_ANSI_9, KEY_9
    (0x1A, 8),   // kVK_ANSI_7, KEY_7
    (0x1B, 12),  // kVK_ANSI_Minus, KEY_MINUS
    (0x1C, 9),   // kVK_ANSI_8, KEY_8
    (0x1D, 11),  // kVK_ANSI_0, KEY_0
    (0x1E, 27),  // kVK_ANSI_RightBracket, KEY_RIGHTBRACE
    (0x1F, 24),  // kVK_ANSI_O, KEY_O
    (0x20, 22),  // kVK_ANSI_U, KEY_U
    (0x21, 26),  // kVK_ANSI_LeftBracket, KEY_LEFTBRACE
    (0x22, 23),  // kVK_ANSI_I, KEY_I
    (0x23, 25),  // kVK_ANSI_P, KEY_P
    (0x24, 28),  // kVK_Return, KEY_ENTER
    (0x25, 38),  // kVK_ANSI_L, KEY_L
    (0x26, 36),  // kVK_ANSI_J, KEY_J
    (0x27, 40),  // kVK_ANSI_Quote, KEY_APOSTROPHE
    (0x28, 37),  // kVK_ANSI_K, KEY_K
    (0x29, 39),  // kVK_ANSI_Semicolon, KEY_SEMICOLON
    (0x2A, 43),  // kVK_ANSI_Backslash, KEY_BACKSLASH
    (0x2B, 51),  // kVK_ANSI_Comma, KEY_COMMA
    (0x2C, 53),  // kVK_ANSI_Slash, KEY_SLASH
    (0x2D, 49),  // kVK_ANSI_N, KEY_N
    (0x2E, 50),  // kVK_ANSI_M, KEY_M
    (0x2F, 52),  // kVK_ANSI_Period, KEY_DOT
    (0x30, 15),  // kVK_Tab, KEY_TAB
    (0x31, 57),  // kVK_Space, KEY_SPACE
    (0x32, 41),  // kVK_ANSI_Grave, KEY_GRAVE
    (0x33, 14),  // kVK_Delete, KEY_BACKSPACE
    (0x35, 1),   // kVK_Escape, KEY_ESC
    (0x36, 126), // kVK_RightCommand, KEY_RIGHTMETA
    (0x37, 125), // kVK_Command, KEY_LEFTMETA
    (0x38, 42),  // kVK_Shift, KEY_LEFTSHIFT
    (0x39, 58),  // kVK_CapsLock, KEY_CAPSLOCK
    (0x3A, 56),  // kVK_Option, KEY_LEFTALT
    (0x3B, 29),  // kVK_Control, KEY_LEFTCTRL
    (0x3C, 54),  // kVK_RightShift, KEY_RIGHTSHIFT
    (0x3D, 100), // kVK_RightOption, KEY_RIGHTALT
    (0x3E, 97),  // kVK_RightControl, KEY_RIGHTCTRL
    (0x3F, 464), // kVK_Function, KEY_FN
    (0x40, 187), // kVK_F17, KEY_F17
    (0x41, 83),  // kVK_ANSI_KeypadDecimal, KEY_KPDOT
    (0x43, 55),  // kVK_ANSI_KeypadMultiply, KEY_KPASTERISK
    (0x45, 78),  // kVK_ANSI_KeypadPlus, KEY_KPPLUS
    (0x47, 69),  // kVK_ANSI_KeypadClear, KEY_NUMLOCK
    (0x48, 115), // kVK_VolumeUp, KEY_VOLUMEUP
    (0x49, 114), // kVK_VolumeDown, KEY_VOLUMEDOWN
    (0x4A, 113), // kVK_Mute, KEY_MUTE
    (0x4B, 98),  // kVK_ANSI_KeypadDivide, KEY_KPSLASH
    (0x4C, 96),  // kVK_ANSI_KeypadEnter, KEY_KPENTER
    (0x4E, 74),  // kVK_ANSI_KeypadMinus, KEY_KPMINUS
    (0x4F, 188), // kVK_F18, KEY_F18
    (0x50, 189), // kVK_F19, KEY_F19
    (0x51, 117), // kVK_ANSI_KeypadEquals, KEY_KPEQUAL
    (0x52, 82),  // kVK_ANSI_Keypad0, KEY_KP0
    (0x53, 79),  // kVK_ANSI_Keypad1, KEY_KP1
    (0x54, 80),  // kVK_ANSI_Keypad2, KEY_KP2
    (0x55, 81),  // kVK_ANSI_Keypad3, KEY_KP3
    (0x56, 75),  // kVK_ANSI_Keypad4, KEY_KP4
    (0x57, 76),  // kVK_ANSI_Keypad5, KEY_KP5
    (0x58, 77),  // kVK_ANSI_Keypad6, KEY_KP6
    (0x59, 71),  // kVK_ANSI_Keypad7, KEY_KP7
    (0x5A, 190), // kVK_F20, KEY_F20
    (0x5B, 72),  // kVK_ANSI_Keypad8, KEY_KP8
    (0x5C, 73),  // kVK_ANSI_Keypad9, KEY_KP9
    (0x5D, 124), // kVK_JIS_Yen, KEY_YEN
    (0x5E, 89),  // kVK_JIS_Underscore, KEY_RO
    (0x5F, 95),  // kVK_JIS_KeypadComma, KEY_KPJPCOMMA
    (0x60, 63),  // kVK_F5, KEY_F5
    (0x61, 64),  // kVK_F6, KEY_F6
    (0x62, 65),  // kVK_F7, KEY_F7
    (0x63, 61),  // kVK_F3, KEY_F3
    (0x64, 66),  // kVK_F8, KEY_F8
    (0x65, 67),  // kVK_F9, KEY_F9
    (0x66, 123), // kVK_JIS_Eisu, KEY_HANJA
    (0x67, 87),  // kVK_F11, KEY_F11
    (0x68, 122), // kVK_JIS_Kana, KEY_HANGEUL
    (0x69, 183), // kVK_F13, KEY_F13
    (0x6A, 186), // kVK_F16, KEY_F16
    (0x6B, 184), // kVK_F14, KEY_F14
    (0x6D, 68),  // kVK_F10, KEY_F10
    (0x6F, 88),  // kVK_F12, KEY_F12
    (0x71, 185), // kVK_F15, KEY_F15
    (0x72, 110), // kVK_Help, KEY_INSERT
    (0x73, 102), // kVK_Home, KEY_HOME
    (0x74, 104), // kVK_PageUp, KEY_PAGEUP
    (0x75, 111), // kVK_ForwardDelete, KEY_DELETE
    (0x76, 62),  // kVK_F4, KEY_F4
    (0x77, 107), // kVK_End, KEY_END
    (0x78, 60),  // kVK_F2, KEY_F2
    (0x79, 109), // kVK_PageDown, KEY_PAGEDOWN
    (0x7A, 59),  // kVK_F1, KEY_F1
    (0x7B, 105), // kVK_LeftArrow, KEY_LEFT
    (0x7C, 106), // kVK_RightArrow, KEY_RIGHT
    (0x7D, 108), // kVK_DownArrow, KEY_DOWN
    (0x7E, 103), // kVK_UpArrow, KEY_UP
];

/// Maps a macOS virtual key code (`kVK_*` in `HIToolbox/Events.h`) to a key.
/// Returns `None` for a code that has no evdev key.
pub fn from_mac(code: u16) -> Option<Key> {
    MAC_EVDEV_CODES
        .iter()
        .find(|&&(mac_code, _)| mac_code == code)
        .map(|&(_, evdev_code)| Key(evdev_code))
}

/// Maps a key to a macOS virtual key code. Returns `None` for a key that has no
/// macOS virtual key code.
pub fn to_mac(key: Key) -> Option<u16> {
    MAC_EVDEV_CODES
        .iter()
        .find(|&&(_, evdev_code)| evdev_code == key.0)
        .map(|&(mac_code, _)| mac_code)
}
