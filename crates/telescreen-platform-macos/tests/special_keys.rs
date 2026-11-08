//! IN-20: decode the special key of a system-defined event (`NSEventTypeSystemDefined`).
#![cfg(target_os = "macos")]

use telescreen_platform_macos::capture::{AuxKey, aux_key};

/// The subtype of a system-defined event for the special keys (`NX_SUBTYPE_AUX_CONTROL_BUTTONS`).
const AUX_CONTROL_BUTTONS: i16 = 8;

/// `data1`: the key type in bits 16..32, the key state in bits 8..16 (0x0A down,
/// 0x0B up), and the repeat flag in bit 0. The values are from the MacBook of
/// the user (2026-11-09).
#[test]
fn in20_decodes_down_repeat_and_up() {
    let rewind = 20;
    assert_eq!(
        aux_key(AUX_CONTROL_BUTTONS, (rewind << 16) | 0x0A00),
        Some(AuxKey {
            key_type: 20,
            down: true,
            repeat: false
        })
    );
    assert_eq!(
        aux_key(AUX_CONTROL_BUTTONS, (rewind << 16) | 0x0A01),
        Some(AuxKey {
            key_type: 20,
            down: true,
            repeat: true
        })
    );
    assert_eq!(
        aux_key(AUX_CONTROL_BUTTONS, (rewind << 16) | 0x0B00),
        Some(AuxKey {
            key_type: 20,
            down: false,
            repeat: false
        })
    );
    assert_eq!(
        aux_key(AUX_CONTROL_BUTTONS, 0x0A00),
        Some(AuxKey {
            key_type: 0,
            down: true,
            repeat: false
        }),
        "SOUND_UP is key type 0"
    );
}

#[test]
fn in20_ignores_other_subtypes_and_states() {
    let brightness_up = 2 << 16;
    assert_eq!(aux_key(7, brightness_up | 0x0A00), None, "another subtype");
    assert_eq!(aux_key(0, brightness_up | 0x0A00), None, "another subtype");
    assert_eq!(
        aux_key(AUX_CONTROL_BUTTONS, brightness_up | 0x0C00),
        None,
        "an unknown key state"
    );
}
