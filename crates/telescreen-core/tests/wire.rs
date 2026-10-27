//! NET-1, NET-2 wire encoding.
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.

use telescreen_core::ids::{ComputerId, DeviceId, SessionId};
use telescreen_core::input::{Button, InputEvent};
use telescreen_core::keys::Key;
use telescreen_core::pointer::Normalized;
use telescreen_core::wire::*;

#[test]
fn messages_round_trip() {
    let s = SessionId(1_800_000_000_000);
    let messages = [
        Message::Hello {
            protocol: PROTOCOL_VERSION,
            computer: ComputerId::from("mac"),
        },
        Message::Heartbeat,
        Message::RequestSession {
            device: DeviceId::from("shared-lg"),
        },
        Message::SessionGranted {
            session: s,
            device: DeviceId::from("shared-lg"),
        },
        Message::SessionRefused {
            device: DeviceId::from("x"),
            reason: RefuseReason::Busy,
        },
        Message::HandoffDone { session: s },
        Message::PromptShown { session: s },
        Message::Confirm { session: s },
        Message::Started { session: s },
        Message::Input {
            session: s,
            event: InputEvent::Key {
                key: Key::LEFT_META,
                down: true,
            },
        },
        Message::Input {
            session: s,
            event: InputEvent::Button {
                button: Button::Right,
                down: false,
            },
        },
        Message::Input {
            session: s,
            event: InputEvent::Scroll { dx: -1.5, dy: 12.0 },
        },
        Message::ReleaseAll { session: s },
        Message::KeyboardAway { session: s },
        Message::KeyboardBack { session: s },
        Message::ClipboardText {
            session: s,
            text: "héllo\nwörld".into(),
            concealed: true,
        },
        Message::ClipboardClear { session: s },
        Message::SessionRefused {
            device: telescreen_core::ids::DeviceId::from("lg"),
            reason: telescreen_core::wire::RefuseReason::NoFocusHold,
        },
        Message::End {
            session: s,
            reason: EndReason::SafetyExit,
        },
    ];
    for message in messages {
        let bytes = encode_message(&message).unwrap();
        assert_eq!(decode_message(&bytes).unwrap(), message);
    }
}

#[test]
fn garbage_is_an_error() {
    assert!(decode_message(&[0xFF, 0xFF, 0xFF]).is_err());
    assert!(decode_message(&vec![0; MAX_FRAME + 1]).is_err());
    assert!(decode_pointer(&[1]).is_err());
}

#[test]
fn net2_pointer_round_trip_and_order() {
    let d = |session, seq| PointerDatagram {
        session: SessionId(session),
        seq,
        position: Normalized { x: 0.25, y: 1.0 },
    };
    let bytes = encode_pointer(&d(3, 9)).unwrap();
    assert!(bytes.len() < 32, "small datagram: {} bytes", bytes.len());
    assert_eq!(decode_pointer(&bytes).unwrap(), d(3, 9));

    let mut order = PointerOrder::default();
    assert!(order.accept(&d(3, 2)));
    assert!(!order.accept(&d(3, 1)));
    assert!(!order.accept(&d(3, 2)));
    assert!(order.accept(&d(3, 3)));
    assert!(!order.accept(&d(2, 50)), "older session");
    assert!(
        order.accept(&d(4, 0)),
        "newer session restarts the sequence"
    );
}

#[test]
fn protocol_version_4_has_the_concealed_clipboard_messages() {
    // CLIP-7 changed the messages, so an older peer is refused.
    assert_eq!(telescreen_core::wire::PROTOCOL_VERSION, 4);
}

#[test]
fn clip6_the_largest_clipboard_text_fits_in_a_frame() {
    let message = Message::ClipboardText {
        session: telescreen_core::ids::SessionId(1),
        text: "\u{10348}".repeat(telescreen_core::clipboard::MAX_TEXT / 4),
        concealed: true,
    };
    let bytes = encode_message(&message).unwrap();
    assert!(bytes.len() <= telescreen_core::wire::MAX_FRAME);
}
