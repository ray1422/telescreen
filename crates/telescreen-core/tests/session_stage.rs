//! The session of a target in the login stage (STG-5). Spec IDs are in the
//! test names.
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.

use telescreen_core::ids::{ComputerId, DeviceId, SessionId};
use telescreen_core::input::{Button, InputEvent, ScrollPhase};
use telescreen_core::keys::Key;
use telescreen_core::pointer::Normalized;
use telescreen_core::session::stage_target::{
    StageTargetAction, StageTargetPhase, StageTargetSession,
};
use telescreen_core::wire::{EndReason, Message};

use StageTargetAction::{Inject, Send};

const S: SessionId = SessionId(3);
const A: Key = Key(30);
const B: Key = Key(48);

fn device() -> DeviceId {
    DeviceId::from("shared-lg")
}

fn key(key: Key, down: bool) -> InputEvent {
    InputEvent::Key { key, down }
}

fn input(event: InputEvent) -> Message {
    Message::Input { session: S, event }
}

fn granted() -> StageTargetSession {
    let (session, actions) = StageTargetSession::new(S, ComputerId::from("mac"), device());
    assert_eq!(
        actions,
        vec![Send(Message::SessionGranted {
            session: S,
            device: device(),
        })]
    );
    assert_eq!(session.phase(), StageTargetPhase::AwaitingHandoff);
    assert_eq!(session.session(), S);
    assert_eq!(session.source(), &ComputerId::from("mac"));
    assert_eq!(session.device(), &device());
    assert!(!session.is_finished());
    session
}

fn forwarding() -> StageTargetSession {
    let mut session = granted();
    let actions = session.on_message(&Message::HandoffDone { session: S });
    assert_eq!(
        actions,
        vec![Send(Message::Started {
            session: S,
            verified: false,
        })],
        "STG-5: no verify; started, not verified"
    );
    assert_eq!(session.phase(), StageTargetPhase::Forwarding);
    session
}

#[test]
fn stg5_a_stage_session_starts_with_no_verification() {
    forwarding();
}

#[test]
fn stg5_keys_are_injected_only_while_forwarding() {
    let mut session = granted();
    assert!(session.on_message(&input(key(A, true))).is_empty());
    assert!(
        session.end(EndReason::TargetDisconnected).len() == 1,
        "no held key"
    );

    let mut session = forwarding();
    assert_eq!(
        session.on_message(&input(key(A, true))),
        vec![Inject(key(A, true))]
    );
    // A repeated down is a key repeat.
    assert_eq!(
        session.on_message(&input(key(A, true))),
        vec![Inject(key(A, true))]
    );
    assert_eq!(
        session.on_message(&input(key(A, false))),
        vec![Inject(key(A, false))]
    );
    // An up for a key that is not down is not injected (IN-4).
    assert!(session.on_message(&input(key(A, false))).is_empty());
}

#[test]
fn stg5_buttons_and_scrolls_are_not_injected_and_not_held() {
    let mut session = forwarding();
    let button = InputEvent::Button {
        button: Button::Left,
        down: true,
    };
    let scroll = InputEvent::Scroll {
        dx: 0.0,
        dy: 12.0,
        phase: ScrollPhase::None,
    };
    assert!(session.on_message(&input(button)).is_empty());
    assert!(session.on_message(&input(scroll)).is_empty());
    // The button is not held: the end releases nothing.
    assert_eq!(
        session.end(EndReason::LoginComplete),
        vec![Send(Message::End {
            session: S,
            reason: EndReason::LoginComplete,
        })]
    );
}

#[test]
fn stg5_the_other_messages_of_a_session_have_no_effect() {
    let mut session = forwarding();
    let ignored = [
        Message::Ack { session: S },
        Message::HandoffDone { session: S },
        Message::HandBackRequest { session: S },
        Message::KeyboardAway { session: S },
        Message::KeyboardBack { session: S },
        Message::PortalHidden { session: S },
        Message::PortalShown { session: S },
        Message::PortalCursor {
            session: S,
            at: Some(Normalized { x: 0.5, y: 0.5 }),
        },
        Message::ClipboardText {
            session: S,
            text: "secret".into(),
            concealed: false,
        },
        Message::ClipboardClear { session: S },
        Message::RequestSession { device: device() },
        Message::DisplaySwitched { device: device() },
        Message::ControlRequest { device: device() },
        Message::Heartbeat,
    ];
    for message in ignored {
        assert!(session.on_message(&message).is_empty(), "{message:?}");
        assert_eq!(session.phase(), StageTargetPhase::Forwarding, "{message:?}");
    }
}

#[test]
fn stg5_a_message_of_a_different_session_has_no_effect() {
    let mut session = forwarding();
    let other = SessionId(9);
    let stale = [
        Message::Input {
            session: other,
            event: key(A, true),
        },
        Message::ReleaseAll { session: other },
        Message::End {
            session: other,
            reason: EndReason::SourceClosed,
        },
    ];
    for message in stale {
        assert!(session.on_message(&message).is_empty(), "{message:?}");
        assert_eq!(session.phase(), StageTargetPhase::Forwarding, "{message:?}");
    }
}

#[test]
fn in4_release_all_releases_each_held_key() {
    let mut session = forwarding();
    session.on_message(&input(key(B, true)));
    session.on_message(&input(key(A, true)));
    assert_eq!(
        session.on_message(&Message::ReleaseAll { session: S }),
        vec![Inject(key(A, false)), Inject(key(B, false))]
    );
    assert!(
        session
            .on_message(&Message::ReleaseAll { session: S })
            .is_empty()
    );
    assert_eq!(session.phase(), StageTargetPhase::Forwarding);
}

#[test]
fn stg4_stg9_an_end_on_the_target_releases_the_keys_then_tells_the_source() {
    let mut session = forwarding();
    session.on_message(&input(key(A, true)));
    assert_eq!(
        session.end(EndReason::LoginComplete),
        vec![
            Inject(key(A, false)),
            Send(Message::End {
                session: S,
                reason: EndReason::LoginComplete,
            }),
        ]
    );
    assert_eq!(
        session.phase(),
        StageTargetPhase::Ended(EndReason::LoginComplete)
    );
    assert!(session.is_finished());
    // No effect after the end.
    assert!(session.end(EndReason::TargetDisconnected).is_empty());
    assert!(session.on_message(&input(key(A, true))).is_empty());
    assert!(session.on_peer_lost().is_empty());
    assert_eq!(
        session.phase(),
        StageTargetPhase::Ended(EndReason::LoginComplete)
    );
}

#[test]
fn stg10_an_end_from_the_source_releases_the_keys_and_sends_nothing() {
    for phase in [granted as fn() -> StageTargetSession, forwarding] {
        let mut session = phase();
        let held = session.phase() == StageTargetPhase::Forwarding;
        if held {
            session.on_message(&input(key(A, true)));
        }
        let actions = session.on_message(&Message::End {
            session: S,
            reason: EndReason::SourceClosed,
        });
        let expected = if held {
            vec![Inject(key(A, false))]
        } else {
            Vec::new()
        };
        assert_eq!(actions, expected);
        assert_eq!(
            session.phase(),
            StageTargetPhase::Ended(EndReason::SourceClosed)
        );
    }
}

#[test]
fn stg10_a_lost_source_releases_the_keys_and_sends_nothing() {
    let mut session = forwarding();
    session.on_message(&input(key(A, true)));
    session.on_message(&input(key(B, true)));
    assert_eq!(
        session.on_peer_lost(),
        vec![Inject(key(A, false)), Inject(key(B, false))]
    );
    assert_eq!(
        session.phase(),
        StageTargetPhase::Ended(EndReason::PeerLost)
    );
}
