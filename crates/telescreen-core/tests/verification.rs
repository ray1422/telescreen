//! The rules of one verification on the destination (VER-3, VER-4, FLOW-8). The
//! target session and the destination of a hand back share them.
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.

use telescreen_core::verification::{Verification, VerificationAction, VerificationResult};
use telescreen_core::verifier::VerifierEvent;

use VerificationAction::{Ack, AckWanted, Finished, Hide, Show, Stop};

fn show(text: Option<&str>, ack_wanted: bool) -> VerificationAction {
    Show {
        text: text.map(str::to_owned),
        ack_wanted,
    }
}

/// A verifier that shows a text and wants an acknowledgement, in a visible window.
fn ack_wanted() -> Verification {
    let mut v = Verification::new();
    assert_eq!(
        v.on_verifier(VerifierEvent::Show("hint".into())),
        vec![show(Some("hint"), false)]
    );
    assert!(v.on_shown(true).is_empty());
    assert_eq!(
        v.on_verifier(VerifierEvent::WantAck),
        vec![show(Some("hint"), true)]
    );
    assert!(!v.ack_wanted(), "the window is not visible yet");
    assert_eq!(v.on_shown(true), vec![AckWanted]);
    assert!(v.ack_wanted());
    v
}

#[test]
fn ver4_one_ack_for_each_want() {
    let mut v = ack_wanted();
    assert_eq!(v.acknowledge(), vec![Ack]);
    assert!(!v.ack_wanted());
    assert!(v.acknowledge().is_empty());
    assert_eq!(
        v.on_verifier(VerifierEvent::WantAck),
        vec![show(Some("hint"), true)]
    );
    assert_eq!(
        v.on_shown(true),
        vec![AckWanted],
        "a verifier can ask again"
    );
    assert_eq!(v.acknowledge(), vec![Ack]);
}

#[test]
fn ver4_no_ack_before_the_window_is_visible() {
    let mut v = Verification::new();
    assert!(v.acknowledge().is_empty());
    v.on_verifier(VerifierEvent::WantAck);
    assert!(v.acknowledge().is_empty());
    assert_eq!(v.on_shown(true), vec![AckWanted]);
    assert!(
        v.on_shown(true).is_empty(),
        "AckWanted goes once for each want"
    );
}

#[test]
fn ver3_ok_keeps_the_window_until_hide() {
    // The target removes the window after the display check (FLOW-10).
    let mut v = ack_wanted();
    v.acknowledge();
    assert_eq!(
        v.on_verifier(VerifierEvent::Done(true)),
        vec![Finished(VerificationResult::Ok)]
    );
    assert_eq!(v.hide(), vec![Hide]);
    assert!(v.hide().is_empty());
}

#[test]
fn ver3_not_ok_and_failure_remove_the_window() {
    for event in [
        VerifierEvent::Done(false),
        VerifierEvent::Failed("timeout".into()),
    ] {
        let mut v = ack_wanted();
        assert_eq!(
            v.on_verifier(event),
            vec![Hide, Finished(VerificationResult::NotOk)]
        );
    }
    let mut v = Verification::new();
    assert_eq!(
        v.on_verifier(VerifierEvent::Done(false)),
        vec![Finished(VerificationResult::NotOk)],
        "no window to remove"
    );
}

#[test]
fn flow7_a_window_failure_stops_the_verifier() {
    let mut v = Verification::new();
    v.on_verifier(VerifierEvent::WantAck);
    assert_eq!(
        v.on_shown(false),
        vec![Stop, Finished(VerificationResult::WindowFailed)]
    );
    let mut v = ack_wanted();
    v.on_verifier(VerifierEvent::Show("next".into()));
    assert_eq!(
        v.on_shown(false),
        vec![Stop, Hide, Finished(VerificationResult::WindowFailed)]
    );
}

#[test]
fn stop_ends_the_verification() {
    let mut v = ack_wanted();
    assert_eq!(v.stop(), vec![Stop, Hide]);
    assert!(v.stop().is_empty());
    assert!(v.on_verifier(VerifierEvent::Done(true)).is_empty());
    assert!(v.on_verifier(VerifierEvent::WantAck).is_empty());
    assert!(v.acknowledge().is_empty());
    assert!(!v.ack_wanted());

    let mut v = Verification::new();
    assert_eq!(v.stop(), vec![Stop], "no window to remove");
}

#[test]
fn a_finished_verification_ignores_late_events() {
    let mut v = ack_wanted();
    v.on_verifier(VerifierEvent::Done(true));
    assert!(v.on_verifier(VerifierEvent::Show("late".into())).is_empty());
    assert!(v.on_verifier(VerifierEvent::Done(false)).is_empty());
    assert!(v.acknowledge().is_empty());
    assert_eq!(
        v.stop(),
        vec![Hide],
        "the verifier ended; only the window stays"
    );
}
