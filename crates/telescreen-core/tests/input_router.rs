//! FLOW-13, FLOW-22: pointer input follows the cursor, keys follow the keyboard
//! focus, and a click in the portal gives the portal the keyboard focus.

use telescreen_core::input::{Button, InputEvent};
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
        keyboard: None,
    }
}

fn local() -> Decision {
    Decision {
        route: Route::Local,
        keyboard: None,
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
    let scroll = InputEvent::Scroll { dx: 0.0, dy: 2.0 };
    assert_eq!(r.route(&scroll, INSIDE), forward());
    assert_eq!(r.route(&scroll, OUTSIDE), local());
}

#[test]
fn flow22_flow23_click_in_the_portal_takes_the_keyboard_first() {
    let mut r = InputRouter::new();
    // The change comes with the click, so the app can send KeyboardBack before it.
    assert_eq!(
        r.route(&left(true), INSIDE),
        Decision {
            route: Route::Forward,
            keyboard: Some(true),
        }
    );
    assert!(r.keyboard_in_portal());
    // The native focus event comes later and changes nothing.
    assert_eq!(r.set_window_focus(true), None);
    assert_eq!(r.route(&left(false), INSIDE), forward());
    assert_eq!(r.route(&key(30, true), OUTSIDE), forward());
}

#[test]
fn flow22_only_a_down_takes_the_keyboard() {
    let mut r = InputRouter::new();
    assert_eq!(r.route(&left(false), INSIDE), local(), "no recorded down");
    assert!(!r.keyboard_in_portal());
    let scroll = InputEvent::Scroll { dx: 0.0, dy: 2.0 };
    r.route(&scroll, INSIDE);
    assert!(!r.keyboard_in_portal(), "scroll does not take the keyboard");
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
