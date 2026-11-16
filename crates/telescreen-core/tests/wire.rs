//! NET-1, NET-2 wire encoding.
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.

use telescreen_core::ids::{ComputerId, DeviceId, SessionId};
use telescreen_core::input::{Button, InputEvent, ScrollPhase};
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
            stage: None,
        },
        Message::Hello {
            protocol: PROTOCOL_VERSION,
            computer: ComputerId::from("pc1"),
            stage: Some(Stage::Login),
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
        Message::AckWanted { session: s },
        Message::Ack { session: s },
        Message::Started {
            session: s,
            verified: true,
        },
        Message::HandBackRequest { session: s },
        Message::HandBackResult {
            device: DeviceId::from("shared-lg"),
            outcome: HandBackOutcome::Done,
        },
        Message::HandBackResult {
            device: DeviceId::from("shared-lg"),
            outcome: HandBackOutcome::NoRoute,
        },
        Message::HandBackResult {
            device: DeviceId::from("shared-lg"),
            outcome: HandBackOutcome::Failed { partial: true },
        },
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
            event: InputEvent::Scroll {
                dx: -1.5,
                dy: 12.0,
                phase: ScrollPhase::None,
            },
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
        // ACT-6.
        Message::DisplaySwitched {
            device: DeviceId::from("shared-lg"),
        },
        Message::ControlRequest {
            device: DeviceId::from("shared-lg"),
        },
        Message::ControlRefused {
            device: DeviceId::from("shared-lg"),
            reason: ControlRefuseReason::AmbiguousDisplay,
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
        sent_us: 123_456,
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
fn stg5_protocol_version_7_has_the_stage_in_the_hello() {
    // ACT-6 (version 6) added DisplaySwitched, ControlRequest, and
    // ControlRefused. STG-5 (version 7) adds the stage to Hello and the end
    // reason LoginComplete, so an older peer is refused.
    assert_eq!(telescreen_core::wire::PROTOCOL_VERSION, 7);
}

#[test]
fn stg5_the_stage_has_the_name_of_the_config_key() {
    assert_eq!(Stage::Login.name(), "login");
    assert_eq!(Stage::from_name("login"), Some(Stage::Login));
    assert_eq!(Stage::from_name("boot"), None);
    assert_eq!(Stage::from_name(""), None);
}

#[test]
fn stg9_the_login_complete_end_reason_round_trips() {
    let message = Message::End {
        session: SessionId(4),
        reason: EndReason::LoginComplete,
    };
    let bytes = encode_message(&message).unwrap();
    assert_eq!(decode_message(&bytes).unwrap(), message);
}

#[test]
fn act6_each_control_refuse_reason_round_trips() {
    use ControlRefuseReason::*;
    for reason in [
        UnknownDevice,
        NoRoute,
        NoDisplay,
        AmbiguousDisplay,
        MatchFailed,
        NoCapture,
        Busy,
    ] {
        let message = Message::ControlRefused {
            device: DeviceId::from("d"),
            reason,
        };
        let bytes = encode_message(&message).unwrap();
        assert_eq!(decode_message(&bytes).unwrap(), message);
    }
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
