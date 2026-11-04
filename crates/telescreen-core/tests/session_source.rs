//! Source session rules. Spec IDs are in the test names.
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.

use std::time::Instant;

use telescreen_core::ids::{ComputerId, DeviceId, SessionId};
use telescreen_core::input::{Button, InputEvent};
use telescreen_core::keys::Key;
use telescreen_core::pointer::Normalized;
use telescreen_core::session::VideoInput;
use telescreen_core::session::source::{HandoffResult, SourceAction, SourcePhase, SourceSession};
use telescreen_core::wire::{EndReason, Message, RefuseReason};

use SourceAction::{RunHandoff, Send, StartForwarding, StopForwarding};

const S: SessionId = SessionId(7);

fn device() -> DeviceId {
    DeviceId::from("shared-lg")
}

fn start() -> SourceSession {
    let (session, actions) = SourceSession::start(ComputerId::from("pc1"), device());
    assert_eq!(
        actions,
        vec![Send(Message::RequestSession { device: device() })]
    );
    assert_eq!(session.phase(), SourcePhase::Requesting);
    assert_eq!(session.video_input(), VideoInput::Unchanged);
    assert_eq!(session.session(), None);
    session
}

fn granted(now: Instant) -> SourceSession {
    let mut s = start();
    let actions = s.on_message(
        &Message::SessionGranted {
            session: S,
            device: device(),
        },
        now,
    );
    assert_eq!(actions, vec![RunHandoff]);
    assert_eq!(s.phase(), SourcePhase::HandingOff);
    assert_eq!(s.session(), Some(S));
    s
}

/// The steps completed; the target verifies as the destination (VER-1).
fn verifying(now: Instant) -> SourceSession {
    let mut s = granted(now);
    let actions = s.on_handoff_finished(HandoffResult::Done);
    assert_eq!(actions, vec![Send(Message::HandoffDone { session: S })]);
    assert_eq!(s.phase(), SourcePhase::Verifying);
    assert_eq!(s.video_input(), VideoInput::Unverified);
    assert!(!s.ack_wanted());
    s
}

/// The verifier of the target asks for an acknowledgement (FLOW-8).
fn ack_wanted(now: Instant) -> SourceSession {
    let mut s = verifying(now);
    assert!(
        s.on_message(&Message::AckWanted { session: S }, now)
            .is_empty()
    );
    assert_eq!(s.phase(), SourcePhase::Verifying);
    assert!(s.ack_wanted());
    s
}

fn forwarding(now: Instant) -> SourceSession {
    let mut s = ack_wanted(now);
    assert_eq!(
        s.acknowledge(true),
        Some(vec![Send(Message::Ack { session: S })])
    );
    assert!(!s.ack_wanted());
    assert_eq!(s.phase(), SourcePhase::Verifying, "the verifier answers");
    assert_eq!(
        s.on_message(
            &Message::Started {
                session: S,
                verified: true
            },
            now
        ),
        vec![StartForwarding]
    );
    assert_eq!(s.phase(), SourcePhase::Forwarding);
    s
}

fn key(code: u16, down: bool) -> InputEvent {
    InputEvent::Key {
        key: Key(code),
        down,
    }
}

#[test]
fn flow5_refused() {
    let mut s = start();
    let now = Instant::now();
    let msg = Message::SessionRefused {
        device: device(),
        reason: RefuseReason::Busy,
    };
    assert!(s.on_message(&msg, now).is_empty());
    assert_eq!(s.phase(), SourcePhase::Refused(RefuseReason::Busy));
    assert!(s.is_finished());
    assert_eq!(s.video_input(), VideoInput::Unchanged);
}

#[test]
fn flow_refusal_for_other_device_is_ignored() {
    let mut s = start();
    let msg = Message::SessionRefused {
        device: DeviceId::from("other"),
        reason: RefuseReason::Busy,
    };
    assert!(s.on_message(&msg, Instant::now()).is_empty());
    assert_eq!(s.phase(), SourcePhase::Requesting);
}

#[test]
fn flow10_ver6_a_verified_handoff_is_confirmed() {
    let s = forwarding(Instant::now());
    assert_eq!(s.video_input(), VideoInput::Confirmed);
}

#[test]
fn flow14_no_pause_state() {
    // The session goes from Started to Forwarding and stays there until the end.
    let mut s = forwarding(Instant::now());
    assert!(s.on_keyboard(false).len() == 1);
    assert_eq!(s.phase(), SourcePhase::Forwarding);
    assert!(s.on_input_lost().len() == 1);
    assert_eq!(s.phase(), SourcePhase::Forwarding);
}

#[test]
fn ver2_ac39_no_verify_starts_unverified() {
    let now = Instant::now();
    let mut s = verifying(now);
    assert_eq!(
        s.on_message(
            &Message::Started {
                session: S,
                verified: false
            },
            now
        ),
        vec![StartForwarding]
    );
    assert_eq!(s.phase(), SourcePhase::Forwarding);
    assert_eq!(s.video_input(), VideoInput::Unverified);
}

#[test]
fn flow11_ac10_a_failed_verification_ends_without_forwarding() {
    let now = Instant::now();
    let mut s = ack_wanted(now);
    let end = Message::End {
        session: S,
        reason: EndReason::VerifyFailed,
    };
    assert!(s.on_message(&end, now).is_empty());
    assert_eq!(s.phase(), SourcePhase::Ended(EndReason::VerifyFailed));
    assert_eq!(s.video_input(), VideoInput::Unknown);
    assert_eq!(s.acknowledge(true), None);
}

#[test]
fn flow8_ac6_ack_needs_ack_wanted_and_focus() {
    let now = Instant::now();
    let mut s = granted(now);
    assert_eq!(s.acknowledge(true), None, "no handoff yet");
    s.on_handoff_finished(HandoffResult::Done);
    assert_eq!(s.acknowledge(true), None, "the verifier wants no ack yet");
    s.on_message(&Message::AckWanted { session: S }, now);
    assert_eq!(s.acknowledge(false), None, "portal has no focus");
    assert!(s.acknowledge(true).is_some());
    assert_eq!(s.acknowledge(true), None, "one ack for each AckWanted");
    s.on_message(&Message::AckWanted { session: S }, now);
    assert!(s.acknowledge(true).is_some(), "a verifier can ask again");
}

#[test]
fn flow8_ack_wanted_only_while_verifying() {
    let now = Instant::now();
    let mut s = granted(now);
    assert!(
        s.on_message(&Message::AckWanted { session: S }, now)
            .is_empty()
    );
    assert!(!s.ack_wanted());
    let mut s = forwarding(now);
    s.on_message(&Message::AckWanted { session: S }, now);
    assert!(!s.ack_wanted());
    assert_eq!(s.acknowledge(true), None);
}

#[test]
fn flow12_first_step_failed_leaves_video_input_unchanged() {
    let mut s = granted(Instant::now());
    let actions = s.on_handoff_finished(HandoffResult::Failed { completed_steps: 0 });
    assert_eq!(
        actions,
        vec![Send(Message::End {
            session: S,
            reason: EndReason::HandoffFailed
        })]
    );
    assert_eq!(s.video_input(), VideoInput::Unchanged);
}

#[test]
fn flow12_partial_success_makes_video_input_unknown() {
    let mut s = granted(Instant::now());
    // The target sets its device state to unknown (VER-6).
    assert_eq!(
        s.on_handoff_finished(HandoffResult::Failed { completed_steps: 1 }),
        vec![Send(Message::End {
            session: S,
            reason: EndReason::HandoffPartial
        })]
    );
    assert_eq!(s.phase(), SourcePhase::Ended(EndReason::HandoffPartial));
    assert_eq!(s.video_input(), VideoInput::Unknown);
}

#[test]
fn flow13_forwards_input_only_while_forwarding() {
    let now = Instant::now();
    let mut s = ack_wanted(now);
    assert!(s.on_input(key(30, true)).is_empty());
    assert!(s.pointer(Normalized { x: 0.5, y: 0.5 }).is_none());

    let mut s = forwarding(now);
    let event = key(30, true);
    assert_eq!(
        s.on_input(event),
        vec![Send(Message::Input { session: S, event })]
    );
    let scroll = InputEvent::Scroll { dx: 0.0, dy: 3.0 };
    assert_eq!(
        s.on_input(scroll),
        vec![Send(Message::Input {
            session: S,
            event: scroll
        })]
    );
}

#[test]
fn in3_unpaired_up_is_not_forwarded() {
    let mut s = forwarding(Instant::now());
    assert!(s.on_input(key(30, false)).is_empty());
    let up = InputEvent::Button {
        button: Button::Left,
        down: false,
    };
    assert!(s.on_input(up).is_empty());
}

#[test]
fn net2_pointer_sequence_increases() {
    let mut s = forwarding(Instant::now());
    let a = s.pointer(Normalized { x: 0.1, y: 0.2 }).unwrap();
    let b = s.pointer(Normalized { x: 0.3, y: 0.4 }).unwrap();
    assert_eq!(a.session, S);
    assert!(b.seq > a.seq);
    assert_eq!(b.position, Normalized { x: 0.3, y: 0.4 });
}

#[test]
fn net2_the_same_position_sends_no_datagram() {
    let mut s = forwarding(Instant::now());
    let p = Normalized { x: 0.1, y: 0.2 };
    assert!(s.pointer(p).is_some());
    assert!(s.pointer(p).is_none(), "no motion, no datagram");
    assert!(s.pointer(Normalized { x: 0.3, y: 0.2 }).is_some());
}

#[test]
fn net2_forget_pointer_sends_the_same_position_again() {
    let mut s = forwarding(Instant::now());
    let p = Normalized { x: 0.1, y: 0.2 };
    assert!(s.pointer(p).is_some());
    s.forget_pointer();
    assert!(s.pointer(p).is_some());
}

#[test]
fn flow23_keyboard_away_and_back() {
    let mut s = forwarding(Instant::now());
    // The confirmation needs the portal focus, so the keyboard starts in the portal.
    assert!(s.on_keyboard(true).is_empty());
    assert_eq!(
        s.on_keyboard(false),
        vec![Send(Message::KeyboardAway { session: S })]
    );
    assert!(s.on_keyboard(false).is_empty(), "no change, no message");
    assert_eq!(
        s.on_keyboard(true),
        vec![Send(Message::KeyboardBack { session: S })]
    );
}

#[test]
fn flow22_in3_held_keys_stay_paired_when_the_keyboard_leaves() {
    // macOS: the tap still sees the up event, so it goes to the target.
    let mut s = forwarding(Instant::now());
    s.on_input(key(42, true));
    s.on_keyboard(false);
    assert_eq!(
        s.on_input(key(42, false)),
        vec![Send(Message::Input {
            session: S,
            event: key(42, false)
        })]
    );
}

#[test]
fn in13_ac28_input_lost_releases_held_state() {
    let mut s = forwarding(Instant::now());
    s.on_input(key(42, true));
    s.on_input(InputEvent::Button {
        button: Button::Left,
        down: true,
    });
    assert_eq!(
        s.on_input_lost(),
        vec![Send(Message::ReleaseAll { session: S })]
    );
    assert_eq!(s.phase(), SourcePhase::Forwarding);
    // The held state was released, so a late up is not forwarded.
    assert!(s.on_input(key(42, false)).is_empty());
    // New input still goes to the target.
    assert_eq!(s.on_input(key(30, true)).len(), 1);
}

#[test]
fn in13_input_lost_outside_forwarding_does_nothing() {
    let mut s = ack_wanted(Instant::now());
    assert!(s.on_input_lost().is_empty());
    assert!(s.on_keyboard(false).is_empty());
}

#[test]
fn flow16_in8_the_safety_exit_while_forwarding() {
    let mut s = forwarding(Instant::now());
    s.on_input(key(30, true));
    let actions = s.end(EndReason::SafetyExit);
    assert_eq!(
        actions,
        vec![
            StopForwarding,
            Send(Message::End {
                session: S,
                reason: EndReason::SafetyExit
            })
        ]
    );
    assert_eq!(s.phase(), SourcePhase::Ended(EndReason::SafetyExit));
    assert_eq!(
        s.video_input(),
        VideoInput::Confirmed,
        "a confirmed input stays confirmed"
    );
    assert!(
        s.end(EndReason::SourceClosed).is_empty(),
        "end twice has no effect"
    );
}

#[test]
fn flow16_end_while_verifying_makes_unverified_unknown() {
    let mut s = ack_wanted(Instant::now());
    let actions = s.end(EndReason::SourceClosed);
    assert_eq!(
        actions,
        vec![Send(Message::End {
            session: S,
            reason: EndReason::SourceClosed
        })]
    );
    assert_eq!(s.video_input(), VideoInput::Unknown);
}

#[test]
fn flow16_end_while_requesting_sends_nothing_then_releases_a_late_grant() {
    let mut s = start();
    assert!(s.end(EndReason::SourceClosed).is_empty());
    assert_eq!(s.phase(), SourcePhase::Ended(EndReason::SourceClosed));
    let late = Message::SessionGranted {
        session: S,
        device: device(),
    };
    assert_eq!(
        s.on_message(&late, Instant::now()),
        vec![Send(Message::End {
            session: S,
            reason: EndReason::SourceClosed
        })]
    );
    assert_eq!(s.phase(), SourcePhase::Ended(EndReason::SourceClosed));
}

#[test]
fn flow16_ac23_target_ends_session() {
    let mut s = forwarding(Instant::now());
    let msg = Message::End {
        session: S,
        reason: EndReason::TargetRequestedControl,
    };
    assert_eq!(s.on_message(&msg, Instant::now()), vec![StopForwarding]);
    assert_eq!(
        s.phase(),
        SourcePhase::Ended(EndReason::TargetRequestedControl)
    );
}

#[test]
fn flow16_ac18_peer_lost() {
    let mut s = forwarding(Instant::now());
    assert_eq!(s.on_peer_lost(), vec![StopForwarding]);
    assert_eq!(s.phase(), SourcePhase::Ended(EndReason::PeerLost));

    let mut s = verifying(Instant::now());
    assert!(s.on_peer_lost().is_empty());
    assert_eq!(s.video_input(), VideoInput::Unknown);
}

#[test]
fn net6_stale_session_messages_are_ignored() {
    let now = Instant::now();
    let mut s = forwarding(now);
    let old = SessionId(6);
    assert!(
        s.on_message(
            &Message::End {
                session: old,
                reason: EndReason::PeerLost
            },
            now
        )
        .is_empty()
    );
    assert!(
        s.on_message(
            &Message::Started {
                session: old,
                verified: true
            },
            now
        )
        .is_empty()
    );
    assert_eq!(s.phase(), SourcePhase::Forwarding);
}

#[test]
fn unexpected_message_for_phase_is_ignored() {
    let now = Instant::now();
    let mut s = granted(now);
    assert!(
        s.on_message(
            &Message::Started {
                session: S,
                verified: true
            },
            now
        )
        .is_empty()
    );
    assert_eq!(s.phase(), SourcePhase::HandingOff);
}

#[test]
fn flow16_end_before_handoff_leaves_video_input_unchanged() {
    let mut s = start();
    assert!(s.on_peer_lost().is_empty());
    assert_eq!(s.video_input(), VideoInput::Unchanged);

    let mut s = granted(Instant::now());
    s.end(EndReason::SourceClosed);
    assert_eq!(s.video_input(), VideoInput::Unchanged);
}

#[test]
fn hb3_in7_the_shortcut_requests_a_hand_back_while_forwarding() {
    let mut s = forwarding(Instant::now());
    s.on_input(key(30, true));
    assert_eq!(
        s.request_hand_back(),
        vec![
            StopForwarding,
            Send(Message::HandBackRequest { session: S })
        ],
        "no End: the target ends the session for the hand back"
    );
    assert_eq!(s.phase(), SourcePhase::Ended(EndReason::HandBack));
    assert!(s.request_hand_back().is_empty(), "only once");
}

#[test]
fn hb3_a_hand_back_request_while_verifying() {
    let mut s = ack_wanted(Instant::now());
    assert_eq!(
        s.request_hand_back(),
        vec![Send(Message::HandBackRequest { session: S })]
    );
    assert_eq!(s.phase(), SourcePhase::Ended(EndReason::HandBack));
    assert!(!s.ack_wanted());
}

#[test]
fn hb3_no_hand_back_before_the_steps_complete() {
    // The steps of the source can still run: a route back now would race them.
    let mut s = granted(Instant::now());
    assert_eq!(
        s.request_hand_back(),
        vec![Send(Message::End {
            session: S,
            reason: EndReason::SourceClosed
        })],
        "the shortcut only ends the session"
    );
    assert_eq!(s.phase(), SourcePhase::Ended(EndReason::SourceClosed));

    let mut s = start();
    assert!(s.request_hand_back().is_empty());
    assert_eq!(s.phase(), SourcePhase::Ended(EndReason::SourceClosed));
}

#[test]
fn hb1_an_end_for_a_hand_back_from_the_target() {
    let now = Instant::now();
    let mut s = forwarding(now);
    let end = Message::End {
        session: S,
        reason: EndReason::HandBack,
    };
    assert_eq!(s.on_message(&end, now), vec![StopForwarding]);
    assert_eq!(s.phase(), SourcePhase::Ended(EndReason::HandBack));
}
