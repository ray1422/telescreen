//! Target session rules. Spec IDs are in the test names.
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.

use telescreen_core::ids::{ComputerId, DeviceId, DisplayToken, SessionId};
use telescreen_core::input::{Button, InputEvent};
use telescreen_core::keys::Key;
use telescreen_core::pointer::Normalized;
use telescreen_core::session::target::{TargetAction, TargetPhase, TargetSession};
use telescreen_core::wire::{EndReason, Message, PointerDatagram};

use TargetAction::{
    CheckDisplay, HidePrompt, HoldKeyboard, Inject, MovePointer, ReleaseKeyboard, ReleaseLease,
    Send, ShowPrompt,
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

fn prompting() -> TargetSession {
    let mut t = new();
    assert_eq!(
        t.on_message(&Message::HandoffDone { session: S }),
        vec![ShowPrompt]
    );
    assert_eq!(t.phase(), TargetPhase::ShowingPrompt);
    assert_eq!(
        t.on_prompt_shown(true),
        vec![Send(Message::PromptShown { session: S })]
    );
    assert_eq!(t.phase(), TargetPhase::Prompting);
    t
}

fn forwarding() -> TargetSession {
    let mut t = prompting();
    assert_eq!(
        t.on_message(&Message::Confirm { session: S }),
        vec![CheckDisplay]
    );
    assert_eq!(t.phase(), TargetPhase::CheckingDisplay);
    assert_eq!(
        t.on_display_checked(true),
        vec![HidePrompt, Send(Message::Started { session: S })]
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
        position: Normalized { x, y: 0.5 },
    }
}

#[test]
fn flow7_prompt_failure_ends_and_releases_lease() {
    let mut t = new();
    t.on_message(&Message::HandoffDone { session: S });
    let actions = t.on_prompt_shown(false);
    assert_eq!(
        actions,
        vec![
            Send(Message::End {
                session: S,
                reason: EndReason::PromptFailed
            }),
            ReleaseLease
        ]
    );
    assert_eq!(t.phase(), TargetPhase::Ended(EndReason::PromptFailed));
}

#[test]
fn flow10_invalid_display_ends() {
    let mut t = prompting();
    t.on_message(&Message::Confirm { session: S });
    let actions = t.on_display_checked(false);
    assert_eq!(
        actions,
        vec![
            HidePrompt,
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
    let mut t = prompting();
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
    let mut t = prompting();
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
        reason: EndReason::ReleaseShortcut,
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
fn flow11_source_end_while_prompting_hides_prompt() {
    let mut t = prompting();
    let actions = t.on_message(&Message::End {
        session: S,
        reason: EndReason::ConfirmTimeout,
    });
    assert_eq!(actions, vec![HidePrompt, ReleaseLease]);
    assert_eq!(t.phase(), TargetPhase::Ended(EndReason::ConfirmTimeout));
}

#[test]
fn flow16_source_end_while_forwarding_releases() {
    let mut t = forwarding();
    t.on_message(&input(key(56, true)));
    let actions = t.on_message(&Message::End {
        session: S,
        reason: EndReason::ReleaseShortcut,
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
fn flow17_request_control_while_prompting() {
    let mut t = prompting();
    let actions = t.end(EndReason::TargetRequestedControl);
    assert_eq!(
        actions,
        vec![
            HidePrompt,
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
        t.on_message(&Message::Confirm { session: S }).is_empty(),
        "no prompt yet"
    );
    assert_eq!(t.phase(), TargetPhase::AwaitingHandoff);
}
