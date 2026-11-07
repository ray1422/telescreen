//! IN-8, IN-13: the capture rules of the portals that the macOS
//! event tap and the Linux portal windows share.

use std::time::{Duration, Instant};

use telescreen_core::ids::PortalId;
use telescreen_core::input::{Button, InputEvent};
use telescreen_core::key_route::{InputRouter, Route};
use telescreen_core::keys::Key;
use telescreen_core::platform::CapturedEvent;
use telescreen_core::portal_capture::{
    FocusLoss, SAFETY_PRESSES, SAFETY_WINDOW, SafetyExit, window_focus,
};

const A: PortalId = PortalId(1);
const B: PortalId = PortalId(2);

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Presses Escape `count` times, `step` apart, from `start`. Returns the
/// results of the presses.
fn presses(exit: &mut SafetyExit, start: Instant, step: Duration, count: usize) -> Vec<bool> {
    (0..count)
        .map(|i| exit.press(start + step * i as u32))
        .collect()
}

#[test]
fn in8_the_tenth_press_in_five_seconds_fires() {
    assert_eq!(SAFETY_PRESSES, 10);
    assert_eq!(SAFETY_WINDOW, Duration::from_secs(5));
    let mut exit = SafetyExit::new();
    let results = presses(&mut exit, Instant::now(), ms(100), 10);
    assert_eq!(results, [vec![false; 9], vec![true]].concat());
}

#[test]
fn in8_nine_presses_do_not_fire() {
    let mut exit = SafetyExit::new();
    let results = presses(&mut exit, Instant::now(), ms(100), 9);
    assert!(results.iter().all(|fired| !fired));
}

#[test]
fn in8_presses_older_than_the_window_do_not_count() {
    let mut exit = SafetyExit::new();
    let start = Instant::now();
    // 600 ms apart: ten presses take 5.4 s, so the oldest press is too old.
    let results = presses(&mut exit, start, ms(600), 20);
    assert!(results.iter().all(|fired| !fired));
}

#[test]
fn in8_the_window_slides() {
    let mut exit = SafetyExit::new();
    let start = Instant::now();
    // Presses 1 to 9 at 0 s to 0.8 s.
    presses(&mut exit, start, ms(100), 9);
    // At 5.001 s, the press at 0 s is too old: 9 presses count.
    assert!(!exit.press(start + ms(5001)));
    // At 5.1 s, presses at 0.1 s to 0.8 s and the two new presses count.
    assert!(exit.press(start + ms(5100)));
}

#[test]
fn in8_a_press_exactly_at_the_window_edge_counts() {
    let mut exit = SafetyExit::new();
    let start = Instant::now();
    let mut results = presses(&mut exit, start, ms(100), 9);
    // The first press is exactly 5 s old: it still counts.
    results.push(exit.press(start + SAFETY_WINDOW));
    assert_eq!(results.last(), Some(&true));
}

#[test]
fn in8_the_count_starts_again_after_the_exit_fires() {
    let mut exit = SafetyExit::new();
    let start = Instant::now();
    assert!(presses(&mut exit, start, ms(10), 10)[9]);
    // The next press does not fire again: ten new presses are necessary.
    let again = presses(&mut exit, start + ms(200), ms(10), 10);
    assert_eq!(again, [vec![false; 9], vec![true]].concat());
}

#[test]
fn in8_reset_forgets_the_presses() {
    let mut exit = SafetyExit::new();
    let start = Instant::now();
    presses(&mut exit, start, ms(10), 9);
    exit.reset();
    let results = presses(&mut exit, start + ms(100), ms(10), 9);
    assert!(results.iter().all(|fired| !fired));
    assert!(exit.press(start + ms(200)));
}

#[test]
fn in8_default_is_new() {
    let mut exit = SafetyExit::default();
    let results = presses(&mut exit, Instant::now(), ms(1), 10);
    assert!(results[9]);
}

/// Gives `count` Escape key downs to [`SafetyExit::escape_down`], `step`
/// apart, from `start`. Returns the results.
fn escape_downs(
    exit: &mut SafetyExit,
    start: Instant,
    step: Duration,
    count: usize,
    waits_or_forwards: bool,
    repeat: bool,
) -> Vec<bool> {
    (0..count)
        .map(|i| exit.escape_down(start + step * i as u32, waits_or_forwards, repeat))
        .collect()
}

#[test]
fn in8_escape_down_counts_while_a_portal_waits_or_forwards() {
    let mut exit = SafetyExit::new();
    let results = escape_downs(&mut exit, Instant::now(), ms(100), 10, true, false);
    assert_eq!(results, [vec![false; 9], vec![true]].concat());
}

#[test]
fn in8_ac57_escape_down_does_not_count_while_only_armed() {
    let mut exit = SafetyExit::new();
    let start = Instant::now();
    // No portal waits for the confirmation or forwards.
    let results = escape_downs(&mut exit, start, ms(100), 20, false, false);
    assert!(results.iter().all(|fired| !fired));
    // The presses while armed are not recorded: nine presses later do not fire.
    let results = escape_downs(&mut exit, start + ms(2100), ms(10), 9, true, false);
    assert!(results.iter().all(|fired| !fired));
}

#[test]
fn in8_ac57_nine_presses_and_one_press_while_armed_do_not_fire() {
    let mut exit = SafetyExit::new();
    let start = Instant::now();
    escape_downs(&mut exit, start, ms(100), 9, true, false);
    assert!(!exit.escape_down(start + ms(1000), false, false));
    // The next counted press is the tenth.
    assert!(exit.escape_down(start + ms(1100), true, false));
}

#[test]
fn in8_ac57_held_escape_does_not_fire() {
    let mut exit = SafetyExit::new();
    let start = Instant::now();
    // Hold Escape for 5 s: one key down, then the auto-repeated key downs.
    assert!(!exit.escape_down(start, true, false));
    let results = escape_downs(&mut exit, start + ms(500), ms(33), 136, true, true);
    assert!(results.iter().all(|fired| !fired));
}

#[test]
fn in8_ac57_auto_repeat_does_not_add_to_the_presses() {
    let mut exit = SafetyExit::new();
    let start = Instant::now();
    escape_downs(&mut exit, start, ms(100), 9, true, false);
    let results = escape_downs(&mut exit, start + ms(1000), ms(30), 5, true, true);
    assert!(results.iter().all(|fired| !fired));
    // The next press that is not a repeat is the tenth.
    assert!(exit.escape_down(start + ms(1200), true, false));
}

fn key(code: u16, down: bool) -> InputEvent {
    InputEvent::Key {
        key: Key(code),
        down,
    }
}

fn focus(
    router: &mut InputRouter,
    portal: PortalId,
    focused: bool,
    forwarding: &[PortalId],
    loss: FocusLoss,
) -> Vec<CapturedEvent> {
    window_focus(router, portal, focused, |p| forwarding.contains(&p), loss).collect()
}

#[test]
fn flow23_focus_reports_the_change_in_order() {
    let mut router = InputRouter::new();
    assert_eq!(
        focus(&mut router, A, true, &[A], FocusLoss::UpEventsSeen),
        [CapturedEvent::KeyboardEntered { portal: A }]
    );
    assert_eq!(router.keyboard(), Some(A));
    // B takes the keyboard from A: A leaves before B enters.
    assert_eq!(
        focus(&mut router, B, true, &[A, B], FocusLoss::UpEventsSeen),
        [
            CapturedEvent::KeyboardLeft { portal: A },
            CapturedEvent::KeyboardEntered { portal: B },
        ]
    );
    // A late loss report of A changes nothing.
    assert!(focus(&mut router, A, false, &[A, B], FocusLoss::UpEventsSeen).is_empty());
    assert_eq!(
        focus(&mut router, B, false, &[A, B], FocusLoss::UpEventsSeen),
        [CapturedEvent::KeyboardLeft { portal: B }]
    );
    assert_eq!(router.keyboard(), None);
}

#[test]
fn in13_a_focus_loss_that_hides_up_events_loses_the_input() {
    // UI-6: on Linux the other windows get the up events.
    let mut router = InputRouter::new();
    focus(&mut router, A, true, &[A], FocusLoss::UpEventsLost);
    assert_eq!(router.route(&key(42, true), None).route, Route::Forward(A));
    assert_eq!(
        focus(&mut router, A, false, &[A], FocusLoss::UpEventsLost),
        [
            CapturedEvent::KeyboardLeft { portal: A },
            CapturedEvent::InputLost,
        ]
    );
    assert!(!router.forwarded_to(A), "the routes are forgotten");
}

#[test]
fn in13_input_lost_comes_before_the_new_keyboard_portal() {
    let mut router = InputRouter::new();
    focus(&mut router, A, true, &[A, B], FocusLoss::UpEventsLost);
    assert_eq!(
        focus(&mut router, B, true, &[A, B], FocusLoss::UpEventsLost),
        [
            CapturedEvent::KeyboardLeft { portal: A },
            CapturedEvent::InputLost,
            CapturedEvent::KeyboardEntered { portal: B },
        ]
    );
}

#[test]
fn in13_a_portal_that_does_not_forward_loses_no_input() {
    // The portal waits for the confirmation (FLOW-21): no key went to a target.
    let mut router = InputRouter::new();
    focus(&mut router, A, true, &[], FocusLoss::UpEventsLost);
    assert_eq!(
        router
            .route(
                &InputEvent::Button {
                    button: Button::Left,
                    down: true
                },
                Some(B)
            )
            .route,
        Route::Forward(B)
    );
    assert_eq!(
        focus(&mut router, A, false, &[], FocusLoss::UpEventsLost),
        [CapturedEvent::KeyboardLeft { portal: A }]
    );
    assert!(router.forwarded_to(B), "the routes stay");
}

#[test]
fn in13_the_event_tap_sees_up_events_after_a_focus_loss() {
    // macOS: the event tap still sees the up events, so no input is lost.
    let mut router = InputRouter::new();
    focus(&mut router, A, true, &[A], FocusLoss::UpEventsSeen);
    assert_eq!(router.route(&key(42, true), None).route, Route::Forward(A));
    assert_eq!(
        focus(&mut router, A, false, &[A], FocusLoss::UpEventsSeen),
        [CapturedEvent::KeyboardLeft { portal: A }]
    );
    assert!(router.forwarded_to(A), "the up event goes to A");
}
