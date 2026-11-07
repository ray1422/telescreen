//! Target session rules. Spec IDs are in the test names.
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.

use telescreen_core::ids::{ComputerId, DeviceId, DisplayToken, SessionId};
use telescreen_core::input::{Button, InputEvent};
use telescreen_core::keys::Key;
use telescreen_core::pointer::Normalized;
use telescreen_core::session::target::{TargetAction, TargetPhase, TargetSession};
use telescreen_core::verifier::VerifierEvent;
use telescreen_core::wire::{EndReason, Message, PointerDatagram};

use TargetAction::{
    AckVerifier, CheckDisplay, HideVerifier, HoldKeyboard, Inject, MovePointer, ReleaseKeyboard,
    ReleaseLease, RunHandBack, Send, ShowVerifier, StartVerifier, StopVerifier,
};

const S: SessionId = SessionId(42);

fn device() -> DeviceId {
    DeviceId::from("shared-lg")
}

fn new() -> TargetSession {
    let (t, actions) = TargetSession::new(
        S,
        ComputerId::from("mac"),
        device(),
        DisplayToken::from("HDMI-A-2"),
    );
    assert_eq!(
        actions,
        vec![Send(Message::SessionGranted {
            session: S,
            device: device()
        })]
    );
    assert_eq!(t.phase(), TargetPhase::AwaitingHandoff);
    assert_eq!(t.display(), &DisplayToken::from("HDMI-A-2"));
    t
}

fn window(text: Option<&str>, ack_wanted: bool) -> TargetAction {
    ShowVerifier {
        text: text.map(str::to_owned),
        ack_wanted,
    }
}

/// The source finished the steps; this computer verifies as the destination (VER-1).
fn verifying() -> TargetSession {
    let mut t = new();
    assert_eq!(
        t.on_message(&Message::HandoffDone { session: S }),
        vec![StartVerifier]
    );
    assert_eq!(t.phase(), TargetPhase::Verifying);
    t
}

/// The `prompt` verifier: a text, then a request for an acknowledgement (VER-5).
fn ack_wanted() -> TargetSession {
    let mut t = verifying();
    assert_eq!(
        t.on_verifier(VerifierEvent::Show("hint".into())),
        vec![window(Some("hint"), false)]
    );
    assert!(t.on_verifier_shown(true).is_empty());
    assert_eq!(
        t.on_verifier(VerifierEvent::WantAck),
        vec![window(Some("hint"), true)]
    );
    // AckWanted goes to the source only when the window is visible (FLOW-8).
    assert_eq!(
        t.on_verifier_shown(true),
        vec![Send(Message::AckWanted { session: S })]
    );
    assert_eq!(t.phase(), TargetPhase::Verifying);
    t
}

fn checking_display() -> TargetSession {
    let mut t = ack_wanted();
    assert_eq!(
        t.on_message(&Message::Ack { session: S }),
        vec![AckVerifier]
    );
    assert_eq!(t.on_verifier(VerifierEvent::Done(true)), vec![CheckDisplay]);
    assert_eq!(t.phase(), TargetPhase::CheckingDisplay);
    t
}

fn forwarding() -> TargetSession {
    let mut t = checking_display();
    assert_eq!(
        t.on_display_checked(true),
        vec![
            HideVerifier,
            Send(Message::Started {
                session: S,
                verified: true
            })
        ]
    );
    assert_eq!(t.phase(), TargetPhase::Forwarding);
    t
}

fn input(event: InputEvent) -> Message {
    Message::Input { session: S, event }
}

fn key(code: u16, down: bool) -> InputEvent {
    InputEvent::Key {
        key: Key(code),
        down,
    }
}

fn datagram(session: SessionId, seq: u64, x: f32) -> PointerDatagram {
    PointerDatagram {
        session,
        seq,
        sent_us: seq * 8000,
        position: Normalized { x, y: 0.5 },
    }
}

#[test]
fn ver2_ac39_no_verify_starts_unverified() {
    let mut t = verifying();
    assert_eq!(t.on_no_verify(), vec![CheckDisplay]);
    assert_eq!(
        t.on_display_checked(true),
        vec![Send(Message::Started {
            session: S,
            verified: false
        })],
        "no window to hide"
    );
    assert_eq!(t.phase(), TargetPhase::Forwarding);
}

#[test]
fn ver3_a_new_show_replaces_the_text_and_ack_wanted_goes_once() {
    let mut t = ack_wanted();
    assert_eq!(
        t.on_verifier(VerifierEvent::Show("next".into())),
        vec![window(Some("next"), true)]
    );
    assert!(t.on_verifier_shown(true).is_empty(), "AckWanted was sent");
}

#[test]
fn ver3_want_with_no_show_opens_the_window() {
    let mut t = verifying();
    assert_eq!(
        t.on_verifier(VerifierEvent::WantAck),
        vec![window(None, true)]
    );
    assert_eq!(
        t.on_verifier_shown(true),
        vec![Send(Message::AckWanted { session: S })]
    );
}

#[test]
fn ver4_ack_only_after_ack_wanted() {
    let mut t = verifying();
    assert!(t.on_message(&Message::Ack { session: S }).is_empty());
    t.on_verifier(VerifierEvent::WantAck);
    assert!(
        t.on_message(&Message::Ack { session: S }).is_empty(),
        "the window is not visible yet"
    );
    t.on_verifier_shown(true);
    assert_eq!(
        t.on_message(&Message::Ack { session: S }),
        vec![AckVerifier]
    );
    assert!(
        t.on_message(&Message::Ack { session: S }).is_empty(),
        "one ack for each want"
    );
}

#[test]
fn ver3_verifier_events_only_while_verifying() {
    let mut t = new();
    assert!(t.on_verifier(VerifierEvent::WantAck).is_empty());
    assert!(t.on_no_verify().is_empty());
    let mut t = forwarding();
    assert!(t.on_verifier(VerifierEvent::Show("x".into())).is_empty());
    assert!(t.on_verifier(VerifierEvent::Done(false)).is_empty());
    assert_eq!(t.phase(), TargetPhase::Forwarding);
}

#[test]
fn flow11_ac7_ac10_a_failed_verification_ends() {
    for event in [
        VerifierEvent::Done(false),
        VerifierEvent::Failed("timeout".into()),
    ] {
        let mut t = ack_wanted();
        assert_eq!(
            t.on_verifier(event),
            vec![
                HideVerifier,
                Send(Message::End {
                    session: S,
                    reason: EndReason::VerifyFailed
                }),
                ReleaseLease
            ]
        );
        assert_eq!(t.phase(), TargetPhase::Ended(EndReason::VerifyFailed));
    }
    // With no window, there is nothing to hide.
    let mut t = verifying();
    assert_eq!(
        t.on_verifier(VerifierEvent::Done(false)),
        vec![
            Send(Message::End {
                session: S,
                reason: EndReason::VerifyFailed
            }),
            ReleaseLease
        ]
    );
}

#[test]
fn ver5_flow11_window_failure_fails_the_verification_and_releases_lease() {
    let mut t = verifying();
    t.on_verifier(VerifierEvent::WantAck);
    let actions = t.on_verifier_shown(false);
    assert_eq!(
        actions,
        vec![
            StopVerifier,
            Send(Message::End {
                session: S,
                reason: EndReason::VerifyFailed
            }),
            ReleaseLease
        ]
    );
    assert_eq!(t.phase(), TargetPhase::Ended(EndReason::VerifyFailed));
}

#[test]
fn flow10_invalid_display_ends() {
    let mut t = checking_display();
    let actions = t.on_display_checked(false);
    assert_eq!(
        actions,
        vec![
            HideVerifier,
            Send(Message::End {
                session: S,
                reason: EndReason::DisplayGone
            }),
            ReleaseLease
        ]
    );
    assert!(t.is_finished());
}

#[test]
fn flow10_no_injection_before_started() {
    let mut t = ack_wanted();
    assert!(t.on_message(&input(key(30, true))).is_empty());
    assert!(t.on_pointer(&datagram(S, 1, 0.5)).is_none());
}

#[test]
fn flow13_injects_input() {
    let mut t = forwarding();
    assert_eq!(
        t.on_message(&input(key(30, true))),
        vec![Inject(key(30, true))]
    );
    assert_eq!(
        t.on_message(&input(key(30, true))),
        vec![Inject(key(30, true))],
        "key repeat"
    );
    assert_eq!(
        t.on_message(&input(key(30, false))),
        vec![Inject(key(30, false))]
    );
    assert!(
        t.on_message(&input(key(30, false))).is_empty(),
        "unpaired up"
    );
}

#[test]
fn net2_pointer_drops_old_datagrams() {
    let mut t = forwarding();
    assert_eq!(
        t.on_pointer(&datagram(S, 5, 0.1)),
        Some(MovePointer(Normalized { x: 0.1, y: 0.5 }))
    );
    assert_eq!(t.on_pointer(&datagram(S, 4, 0.2)), None, "older");
    assert_eq!(t.on_pointer(&datagram(S, 5, 0.2)), None, "same");
    assert!(t.on_pointer(&datagram(S, 6, 0.3)).is_some());
    assert_eq!(
        t.on_pointer(&datagram(SessionId(41), 100, 0.3)),
        None,
        "other session"
    );
}

#[test]
fn in13_release_all_keeps_the_session() {
    let mut t = forwarding();
    t.on_message(&input(key(42, true)));
    t.on_message(&input(InputEvent::Button {
        button: Button::Left,
        down: true,
    }));
    let actions = t.on_message(&Message::ReleaseAll { session: S });
    assert_eq!(
        actions,
        vec![
            Inject(InputEvent::Button {
                button: Button::Left,
                down: false
            }),
            Inject(key(42, false))
        ]
    );
    assert_eq!(t.phase(), TargetPhase::Forwarding, "the session stays");
    assert!(t.on_message(&Message::ReleaseAll { session: S }).is_empty());
}

#[test]
fn flow23_keyboard_away_holds_and_back_releases() {
    let mut t = forwarding();
    assert_eq!(
        t.on_message(&Message::KeyboardAway { session: S }),
        vec![HoldKeyboard]
    );
    assert!(
        t.on_message(&Message::KeyboardAway { session: S })
            .is_empty(),
        "already held"
    );
    assert_eq!(
        t.on_message(&Message::KeyboardBack { session: S }),
        vec![ReleaseKeyboard]
    );
    assert!(
        t.on_message(&Message::KeyboardBack { session: S })
            .is_empty(),
        "not held"
    );
    assert_eq!(t.phase(), TargetPhase::Forwarding);
}

#[test]
fn flow23_keyboard_messages_need_forwarding_and_the_session() {
    let mut t = ack_wanted();
    assert!(
        t.on_message(&Message::KeyboardAway { session: S })
            .is_empty()
    );
    let mut t = forwarding();
    assert!(
        t.on_message(&Message::KeyboardAway {
            session: SessionId(41)
        })
        .is_empty()
    );
}

#[test]
fn flow23_end_removes_the_focus_holder() {
    let mut t = forwarding();
    t.on_message(&Message::KeyboardAway { session: S });
    t.on_message(&input(key(56, true)));
    let actions = t.on_message(&Message::End {
        session: S,
        reason: EndReason::SafetyExit,
    });
    assert_eq!(
        actions,
        vec![Inject(key(56, false)), ReleaseKeyboard, ReleaseLease]
    );

    let mut t = forwarding();
    t.on_message(&Message::KeyboardAway { session: S });
    assert_eq!(t.on_peer_lost(), vec![ReleaseKeyboard, ReleaseLease]);

    let mut t = forwarding();
    t.on_message(&Message::KeyboardAway { session: S });
    assert_eq!(
        t.end(EndReason::TargetDisconnected),
        vec![
            ReleaseKeyboard,
            Send(Message::End {
                session: S,
                reason: EndReason::TargetDisconnected
            }),
            ReleaseLease
        ]
    );
}

#[test]
fn in4_ac18_peer_lost_releases_held_input() {
    let mut t = forwarding();
    t.on_message(&input(key(29, true)));
    let actions = t.on_peer_lost();
    assert_eq!(actions, vec![Inject(key(29, false)), ReleaseLease]);
    assert_eq!(t.phase(), TargetPhase::Ended(EndReason::PeerLost));
    assert!(t.on_message(&input(key(30, true))).is_empty());
    assert!(t.on_peer_lost().is_empty());
}

#[test]
fn flow16_source_end_while_verifying_stops_the_verifier() {
    let mut t = ack_wanted();
    let actions = t.on_message(&Message::End {
        session: S,
        reason: EndReason::SourceClosed,
    });
    assert_eq!(actions, vec![StopVerifier, HideVerifier, ReleaseLease]);
    assert_eq!(t.phase(), TargetPhase::Ended(EndReason::SourceClosed));
    assert!(t.on_verifier(VerifierEvent::Done(true)).is_empty());
}

#[test]
fn flow16_source_end_while_forwarding_releases() {
    let mut t = forwarding();
    t.on_message(&input(key(56, true)));
    let actions = t.on_message(&Message::End {
        session: S,
        reason: EndReason::SafetyExit,
    });
    assert_eq!(actions, vec![Inject(key(56, false)), ReleaseLease]);
}

#[test]
fn flow16_ac23_target_disconnect() {
    let mut t = forwarding();
    t.on_message(&input(key(30, true)));
    let actions = t.end(EndReason::TargetDisconnected);
    assert_eq!(
        actions,
        vec![
            Inject(key(30, false)),
            Send(Message::End {
                session: S,
                reason: EndReason::TargetDisconnected
            }),
            ReleaseLease
        ]
    );
    assert!(t.end(EndReason::TargetDisconnected).is_empty());
}

#[test]
fn flow17_request_control_while_verifying() {
    let mut t = ack_wanted();
    let actions = t.end(EndReason::TargetRequestedControl);
    assert_eq!(
        actions,
        vec![
            StopVerifier,
            HideVerifier,
            Send(Message::End {
                session: S,
                reason: EndReason::TargetRequestedControl
            }),
            ReleaseLease
        ]
    );
}

#[test]
fn net6_stale_and_unexpected_messages_have_no_effect() {
    let mut t = new();
    let old = SessionId(41);
    assert!(
        t.on_message(&Message::HandoffDone { session: old })
            .is_empty()
    );
    assert!(
        t.on_message(&Message::End {
            session: old,
            reason: EndReason::PeerLost
        })
        .is_empty()
    );
    assert!(
        t.on_message(&Message::Ack { session: S }).is_empty(),
        "no verifier yet"
    );
    assert_eq!(t.phase(), TargetPhase::AwaitingHandoff);
}

#[test]
fn hb1_hb2_hand_back_ends_then_runs_the_route() {
    let mut t = forwarding();
    t.on_message(&Message::KeyboardAway { session: S });
    t.on_message(&input(key(56, true)));
    assert!(t.can_hand_back());
    // The end comes first: no key stays held across the switch.
    assert_eq!(
        t.hand_back(),
        vec![
            Inject(key(56, false)),
            ReleaseKeyboard,
            Send(Message::End {
                session: S,
                reason: EndReason::HandBack
            }),
            ReleaseLease,
            RunHandBack
        ]
    );
    assert_eq!(t.phase(), TargetPhase::Ended(EndReason::HandBack));
    assert!(t.hand_back().is_empty());
    assert!(!t.can_hand_back());
}

#[test]
fn hb1_hand_back_while_verifying_stops_the_verifier() {
    let mut t = ack_wanted();
    assert_eq!(
        t.hand_back(),
        vec![
            StopVerifier,
            HideVerifier,
            Send(Message::End {
                session: S,
                reason: EndReason::HandBack
            }),
            ReleaseLease,
            RunHandBack
        ]
    );
    let mut t = checking_display();
    assert!(t.can_hand_back());
    assert_eq!(*t.hand_back().last().unwrap(), RunHandBack);
}

#[test]
fn hb1_no_hand_back_before_handoff_done() {
    // The source can still run its steps.
    let mut t = new();
    assert!(!t.can_hand_back());
    assert!(t.hand_back().is_empty());
    assert!(
        t.on_message(&Message::HandBackRequest { session: S })
            .is_empty()
    );
    assert_eq!(t.phase(), TargetPhase::AwaitingHandoff);
}

#[test]
fn hb3_a_request_of_the_source_runs_the_route() {
    let mut t = forwarding();
    t.on_message(&input(key(56, true)));
    assert_eq!(
        t.on_message(&Message::HandBackRequest { session: S }),
        vec![Inject(key(56, false)), ReleaseLease, RunHandBack],
        "the source already ended the session: no End"
    );
    assert_eq!(t.phase(), TargetPhase::Ended(EndReason::HandBack));

    let mut t = ack_wanted();
    assert_eq!(
        t.on_message(&Message::HandBackRequest { session: S }),
        vec![StopVerifier, HideVerifier, ReleaseLease, RunHandBack]
    );
}

#[test]
fn hb3_a_request_for_another_session_has_no_effect() {
    let mut t = forwarding();
    assert!(
        t.on_message(&Message::HandBackRequest {
            session: SessionId(41)
        })
        .is_empty()
    );
    assert_eq!(t.phase(), TargetPhase::Forwarding);
}

fn portal_cursor(x: f32) -> Message {
    Message::PortalCursor {
        session: S,
        at: Some(Normalized { x, y: 0.5 }),
    }
}

#[test]
fn flow7_ac56_the_prompt_shows_the_cursor_while_verifying() {
    let mut t = ack_wanted();
    assert_eq!(
        t.on_message(&portal_cursor(0.25)),
        vec![TargetAction::PortalCursor(Some(Normalized {
            x: 0.25,
            y: 0.5
        }))]
    );
    assert_eq!(
        t.on_message(&Message::PortalCursor {
            session: S,
            at: None
        }),
        vec![TargetAction::PortalCursor(None)]
    );
}

#[test]
fn flow7_flow10_the_cursor_on_the_prompt_injects_nothing() {
    for mut t in [new(), checking_display(), forwarding()] {
        assert!(
            t.on_message(&portal_cursor(0.25)).is_empty(),
            "no prompt and no warning: {:?}",
            t.phase()
        );
    }
}
