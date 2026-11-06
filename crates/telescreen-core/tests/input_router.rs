//! FLOW-13, FLOW-22: pointer input follows the cursor, keys follow the keyboard
//! focus, and a click in the portal asks for the native keyboard focus.

use telescreen_core::input::{Button, InputEvent, ScrollPhase};
use telescreen_core::key_route::{Decision, InputRouter, Route};
use telescreen_core::keys::Key;

const INSIDE: bool = true;
const OUTSIDE: bool = false;

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
        route: Route::Forward,
        focus_wanted: false,
    }
}

fn local() -> Decision {
    Decision {
        route: Route::Local,
        focus_wanted: false,
    }
}

#[test]
fn flow22_keyboard_starts_out_of_the_portal() {
    let mut r = InputRouter::new();
    assert!(!r.keyboard_in_portal());
    assert_eq!(r.route(&key(30, true), INSIDE), local());
}

#[test]
fn flow22_window_focus_moves_the_keyboard() {
    let mut r = InputRouter::new();
    assert_eq!(r.set_window_focus(true), Some(true));
    assert_eq!(r.set_window_focus(true), None, "no change");
    // Keys follow the keyboard focus, not the cursor.
    assert_eq!(r.route(&key(30, true), OUTSIDE), forward());
    assert_eq!(r.route(&key(30, false), OUTSIDE), forward());
    assert_eq!(r.set_window_focus(false), Some(false));
    assert_eq!(r.route(&key(30, true), INSIDE), local());
}

#[test]
fn flow13_pointer_input_follows_the_cursor() {
    let mut r = InputRouter::new();
    r.set_window_focus(true);
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
            route: Route::Forward,
            focus_wanted: true,
        }
    );
    // v0.5.0 set the focus here. Then a click that the OS did not follow with
    // the focus sent all keys to the target until the session ended.
    assert!(
        !r.keyboard_in_portal(),
        "only the OS changes the keyboard focus"
    );
    assert_eq!(r.route(&key(30, true), OUTSIDE), local());
    assert_eq!(r.route(&key(30, false), OUTSIDE), local());
    assert_eq!(r.route(&left(false), INSIDE), forward());

    assert_eq!(r.set_window_focus(true), Some(true));
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
    r.set_window_focus(true);
    assert_eq!(r.route(&key(42, true), OUTSIDE), forward());
    r.set_window_focus(false);
    assert_eq!(r.route(&key(42, false), OUTSIDE), forward());

    assert_eq!(r.route(&key(29, true), OUTSIDE), local());
    r.set_window_focus(true);
    assert_eq!(r.route(&key(29, false), OUTSIDE), local());
}

#[test]
fn in13_clear_forgets_the_routes_but_not_the_focus() {
    let mut r = InputRouter::new();
    r.set_window_focus(true);
    r.route(&key(42, true), OUTSIDE);
    r.clear();
    assert!(r.keyboard_in_portal());
    assert_eq!(r.route(&key(42, false), OUTSIDE), local(), "forgotten down");
}

#[test]
fn in14_a_slow_scroll_is_not_lost() {
    let mut rest = 0.0;
    let units: Vec<i32> = (0..5)
        .map(|_| telescreen_core::input::whole_units(0.4, &mut rest))
        .collect();
    assert_eq!(units.iter().sum::<i32>(), 2, "{units:?}");
}
