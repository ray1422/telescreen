//! FLOW-13, FLOW-22: pointer input follows the cursor, keys follow the keyboard
//! focus, and a click in the portal asks for the native keyboard focus.

use telescreen_core::ids::PortalId;
use telescreen_core::input::{Button, InputEvent, ScrollPhase};
use telescreen_core::key_route::{Decision, FocusChange, InputRouter, Route};
use telescreen_core::keys::Key;

const A: PortalId = PortalId(1);
const B: PortalId = PortalId(2);
const INSIDE: Option<PortalId> = Some(A);
const OUTSIDE: Option<PortalId> = None;

fn entered(portal: PortalId) -> FocusChange {
    FocusChange {
        left: None,
        entered: Some(portal),
    }
}

fn left_focus(portal: PortalId) -> FocusChange {
    FocusChange {
        left: Some(portal),
        entered: None,
    }
}

fn key(code: u16, down: bool) -> InputEvent {
    InputEvent::Key {
        key: Key(code),
        down,
    }
}

fn left(down: bool) -> InputEvent {
    InputEvent::Button {
        button: Button::Left,
        down,
    }
}

fn forward() -> Decision {
    Decision {
        route: Route::Forward(A),
        focus_wanted: None,
    }
}

fn local() -> Decision {
    Decision {
        route: Route::Local,
        focus_wanted: None,
    }
}

#[test]
fn flow22_keyboard_starts_out_of_the_portal() {
    let mut r = InputRouter::new();
    assert_eq!(r.keyboard(), None);
    assert_eq!(r.route(&key(30, true), INSIDE), local());
}

#[test]
fn flow22_window_focus_moves_the_keyboard() {
    let mut r = InputRouter::new();
    assert_eq!(r.set_window_focus(A, true), entered(A));
    assert!(r.set_window_focus(A, true).is_none(), "no change");
    // Keys follow the keyboard focus, not the cursor.
    assert_eq!(r.route(&key(30, true), OUTSIDE), forward());
    assert_eq!(r.route(&key(30, false), OUTSIDE), forward());
    assert_eq!(r.set_window_focus(A, false), left_focus(A));
    assert_eq!(r.route(&key(30, true), INSIDE), local());
}

#[test]
fn flow13_pointer_input_follows_the_cursor() {
    let mut r = InputRouter::new();
    r.set_window_focus(A, true);
    assert_eq!(r.route(&left(true), OUTSIDE), local());
    assert_eq!(r.route(&left(false), OUTSIDE), local());
    let scroll = InputEvent::Scroll {
        dx: 0.0,
        dy: 2.0,
        phase: ScrollPhase::None,
    };
    assert_eq!(r.route(&scroll, INSIDE), forward());
    assert_eq!(r.route(&scroll, OUTSIDE), local());
}

#[test]
fn flow22_click_in_the_portal_asks_for_the_native_focus() {
    let mut r = InputRouter::new();
    assert_eq!(
        r.route(&left(true), INSIDE),
        Decision {
            route: Route::Forward(A),
            focus_wanted: Some(A),
        }
    );
    // v0.5.0 set the focus here. Then a click that the OS did not follow with
    // the focus sent all keys to the target until the session ended.
    assert!(
        r.keyboard().is_none(),
        "only the OS changes the keyboard focus"
    );
    assert_eq!(r.route(&key(30, true), OUTSIDE), local());
    assert_eq!(r.route(&key(30, false), OUTSIDE), local());
    assert_eq!(r.route(&left(false), INSIDE), forward());

    assert_eq!(r.set_window_focus(A, true), entered(A));
    assert_eq!(r.route(&key(30, true), OUTSIDE), forward());
    // With the focus, a click asks for nothing.
    assert_eq!(r.route(&left(true), INSIDE), forward());
}

#[test]
fn flow22_only_a_down_asks_for_the_keyboard() {
    let mut r = InputRouter::new();
    assert_eq!(r.route(&left(false), INSIDE), local(), "no recorded down");
    let scroll = InputEvent::Scroll {
        dx: 0.0,
        dy: 2.0,
        phase: ScrollPhase::None,
    };
    assert_eq!(
        r.route(&scroll, INSIDE),
        forward(),
        "scroll asks for nothing"
    );
}

#[test]
fn in3_up_follows_the_down_across_a_focus_change() {
    let mut r = InputRouter::new();
    r.set_window_focus(A, true);
    assert_eq!(r.route(&key(42, true), OUTSIDE), forward());
    r.set_window_focus(A, false);
    assert_eq!(r.route(&key(42, false), OUTSIDE), forward());

    assert_eq!(r.route(&key(29, true), OUTSIDE), local());
    r.set_window_focus(A, true);
    assert_eq!(r.route(&key(29, false), OUTSIDE), local());
}

#[test]
fn in13_clear_forgets_the_routes_but_not_the_focus() {
    let mut r = InputRouter::new();
    r.set_window_focus(A, true);
    r.route(&key(42, true), OUTSIDE);
    r.clear();
    assert_eq!(r.keyboard(), Some(A));
    assert_eq!(r.route(&key(42, false), OUTSIDE), local(), "forgotten down");
}

#[test]
fn flow25_flow22_keys_follow_the_focused_portal() {
    let mut r = InputRouter::new();
    r.set_window_focus(A, true);
    assert_eq!(r.route(&key(30, true), OUTSIDE).route, Route::Forward(A));
    assert_eq!(r.route(&key(30, false), Some(B)).route, Route::Forward(A));
    // The OS reports the new window before the late loss of the old one.
    assert_eq!(
        r.set_window_focus(B, true),
        FocusChange {
            left: Some(A),
            entered: Some(B),
        }
    );
    assert!(r.set_window_focus(A, false).is_none(), "late report");
    assert_eq!(r.keyboard(), Some(B));
    assert_eq!(r.route(&key(30, true), INSIDE).route, Route::Forward(B));
}

#[test]
fn flow25_flow20_buttons_follow_the_portal_under_the_cursor() {
    let mut r = InputRouter::new();
    r.set_window_focus(A, true);
    assert_eq!(
        r.route(&left(true), Some(B)),
        Decision {
            route: Route::Forward(B),
            focus_wanted: Some(B),
        },
        "a click in B asks for the focus of B"
    );
    assert_eq!(r.route(&left(false), Some(B)).route, Route::Forward(B));
    assert_eq!(
        r.route(&left(true), Some(A)),
        Decision {
            route: Route::Forward(A),
            focus_wanted: None,
        }
    );
}

#[test]
fn flow25_in3_up_goes_to_the_portal_of_its_down() {
    let mut r = InputRouter::new();
    r.set_window_focus(A, true);
    assert_eq!(r.route(&key(42, true), OUTSIDE).route, Route::Forward(A));
    r.set_window_focus(B, true);
    assert_eq!(r.route(&key(42, false), OUTSIDE).route, Route::Forward(A));

    assert_eq!(r.route(&left(true), Some(A)).route, Route::Forward(A));
    assert_eq!(r.route(&left(false), Some(B)).route, Route::Forward(A));
}

#[test]
fn flow25_scroll_outside_all_portals_is_local() {
    let mut r = InputRouter::new();
    r.set_window_focus(A, true);
    let scroll = InputEvent::Scroll {
        dx: 0.0,
        dy: 2.0,
        phase: ScrollPhase::None,
    };
    assert_eq!(r.route(&scroll, OUTSIDE).route, Route::Local);
    assert_eq!(r.route(&scroll, Some(B)).route, Route::Forward(B));
}

#[test]
fn flow25_forget_drops_only_one_portal() {
    let mut r = InputRouter::new();
    r.set_window_focus(A, true);
    r.route(&key(42, true), OUTSIDE);
    r.route(&left(true), Some(B));
    assert!(r.forwarded_to(A) && r.forwarded_to(B));
    r.forget(A);
    assert_eq!(r.keyboard(), None, "A had the keyboard");
    assert!(!r.forwarded_to(A));
    assert_eq!(r.route(&key(42, false), OUTSIDE).route, Route::Local);
    assert_eq!(r.route(&left(false), OUTSIDE).route, Route::Forward(B));
}

#[test]
fn in14_a_slow_scroll_is_not_lost() {
    let mut rest = 0.0;
    let units: Vec<i32> = (0..5)
        .map(|_| telescreen_core::input::whole_units(0.4, &mut rest))
        .collect();
    assert_eq!(units.iter().sum::<i32>(), 2, "{units:?}");
}
