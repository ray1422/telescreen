//! Source session rules. Spec IDs are in the test names.
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.

use std::time::{Duration, Instant};

use telescreen_core::ids::{ComputerId, DeviceId, SessionId};
use telescreen_core::input::{Button, InputEvent};
use telescreen_core::keys::Key;
use telescreen_core::pointer::Normalized;
use telescreen_core::session::source::{HandoffResult, SourceAction, SourcePhase, SourceSession};
use telescreen_core::session::{CONFIRM_TIMEOUT, VideoInput};
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

fn awaiting_confirm(now: Instant) -> SourceSession {
    let mut s = granted(now);
    let actions = s.on_handoff_finished(HandoffResult::Done { verified: None });
    assert_eq!(actions, vec![Send(Message::HandoffDone { session: S })]);
    assert_eq!(s.phase(), SourcePhase::AwaitingPrompt);
    assert_eq!(s.video_input(), VideoInput::Unverified);
    assert!(
        s.on_message(&Message::PromptShown { session: S }, now)
            .is_empty()
    );
    assert_eq!(s.phase(), SourcePhase::AwaitingConfirm);
    s
}

fn forwarding(now: Instant) -> SourceSession {
    let mut s = awaiting_confirm(now);
    assert_eq!(
        s.confirm(true),
        Some(vec![Send(Message::Confirm { session: S })])
    );
    assert_eq!(s.phase(), SourcePhase::Starting);
    assert_eq!(
        s.on_message(&Message::Started { session: S }, now),
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
fn flow7_flow10_happy_path_confirms_video_input() {
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
fn verify_true_confirms_video_input() {
    let now = Instant::now();
    let mut s = granted(now);
    let actions = s.on_handoff_finished(HandoffResult::Done {
        verified: Some(true),
    });
    assert_eq!(actions, vec![Send(Message::HandoffDone { session: S })]);
    assert_eq!(s.video_input(), VideoInput::Confirmed);
}

#[test]
fn flow12_ac10_verify_false_ends_without_forwarding() {
    let mut s = granted(Instant::now());
    let actions = s.on_handoff_finished(HandoffResult::Done {
        verified: Some(false),
    });
    assert_eq!(
        actions,
        vec![Send(Message::End {
            session: S,
            reason: EndReason::HandoffFailed
        })]
    );
    assert_eq!(s.phase(), SourcePhase::Ended(EndReason::HandoffFailed));
    assert_eq!(s.video_input(), VideoInput::Unknown);
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
    s.on_handoff_finished(HandoffResult::Failed { completed_steps: 1 });
    assert_eq!(s.phase(), SourcePhase::Ended(EndReason::HandoffFailed));
    assert_eq!(s.video_input(), VideoInput::Unknown);
}

#[test]
fn flow8_ac6_confirm_needs_prompt_and_focus() {
    let now = Instant::now();
    let mut s = granted(now);
    assert_eq!(s.confirm(true), None, "no prompt yet");
    s.on_handoff_finished(HandoffResult::Done { verified: None });
    assert_eq!(s.confirm(true), None, "prompt not shown yet");
    s.on_message(&Message::PromptShown { session: S }, now);
    assert_eq!(s.confirm(false), None, "portal has no focus");
    assert_eq!(s.phase(), SourcePhase::AwaitingConfirm);
    assert!(s.confirm(true).is_some());
    assert_eq!(s.confirm(true), None, "only one confirmation");
}

#[test]
fn flow11_ac7_confirm_timeout() {
    let now = Instant::now();
    let mut s = awaiting_confirm(now);
    assert!(
        s.tick(now + CONFIRM_TIMEOUT - Duration::from_millis(1))
            .is_empty()
    );
    let actions = s.tick(now + CONFIRM_TIMEOUT);
    assert_eq!(
        actions,
        vec![Send(Message::End {
            session: S,
            reason: EndReason::ConfirmTimeout
        })]
    );
    assert_eq!(s.phase(), SourcePhase::Ended(EndReason::ConfirmTimeout));
    assert_eq!(s.video_input(), VideoInput::Unknown);
    assert!(s.tick(now + CONFIRM_TIMEOUT * 2).is_empty());
}

#[test]
fn flow11_timer_starts_when_prompt_shows() {
    let now = Instant::now();
    let mut s = granted(now);
    s.on_handoff_finished(HandoffResult::Done { verified: None });
    let later = now + Duration::from_secs(30);
    s.on_message(&Message::PromptShown { session: S }, later);
    assert!(s.tick(now + CONFIRM_TIMEOUT).is_empty());
    assert!(!s.tick(later + CONFIRM_TIMEOUT).is_empty());
}

#[test]
fn flow13_forwards_input_only_while_forwarding() {
    let now = Instant::now();
    let mut s = awaiting_confirm(now);
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
    let mut s = awaiting_confirm(Instant::now());
    assert!(s.on_input_lost().is_empty());
    assert!(s.on_keyboard(false).is_empty());
}

#[test]
fn flow16_release_shortcut_while_forwarding() {
    let mut s = forwarding(Instant::now());
    s.on_input(key(30, true));
    let actions = s.end(EndReason::ReleaseShortcut);
    assert_eq!(
        actions,
        vec![
            StopForwarding,
            Send(Message::End {
                session: S,
                reason: EndReason::ReleaseShortcut
            })
        ]
    );
    assert_eq!(s.phase(), SourcePhase::Ended(EndReason::ReleaseShortcut));
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
fn flow16_end_before_confirmation_makes_unverified_unknown() {
    let mut s = awaiting_confirm(Instant::now());
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

    let mut s = awaiting_confirm(Instant::now());
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
        s.on_message(&Message::Started { session: old }, now)
            .is_empty()
    );
    assert_eq!(s.phase(), SourcePhase::Forwarding);
}

#[test]
fn unexpected_message_for_phase_is_ignored() {
    let now = Instant::now();
    let mut s = granted(now);
    assert!(
        s.on_message(&Message::Started { session: S }, now)
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
