//! FLOW-20 and IN-3: each up event goes where its down event went, also when the
//! free cursor crossed the portal edge in between.

use telescreen_core::input::{Button, InputEvent};
use telescreen_core::key_route::{KeyRoutes, Route};
use telescreen_core::keys::Key;

const INSIDE: bool = true;
const OUTSIDE: bool = false;

fn key(code: u16, down: bool) -> InputEvent {
    InputEvent::Key {
        key: Key(code),
        down,
    }
}

fn button(button: Button, down: bool) -> InputEvent {
    InputEvent::Button { button, down }
}

#[test]
fn flow20_down_in_the_portal_goes_to_the_target() {
    let mut r = KeyRoutes::new();
    assert_eq!(r.route(&key(30, true), INSIDE), Route::Forward);
    assert_eq!(r.route(&key(30, false), INSIDE), Route::Forward);
    assert_eq!(r.route(&key(30, true), OUTSIDE), Route::Local);
    assert_eq!(r.route(&key(30, false), OUTSIDE), Route::Local);
}

#[test]
fn in3_up_outside_follows_a_forwarded_down() {
    // Shift is pressed in the portal, the cursor leaves, Shift is released.
    let mut r = KeyRoutes::new();
    assert_eq!(r.route(&key(42, true), INSIDE), Route::Forward);
    assert!(r.any_forwarded());
    assert_eq!(r.route(&key(42, false), OUTSIDE), Route::Forward);
    assert!(!r.any_forwarded());
}

#[test]
fn in3_up_inside_follows_a_local_down() {
    // Shift is pressed on a different display, the cursor enters, Shift is released.
    let mut r = KeyRoutes::new();
    assert_eq!(r.route(&key(42, true), OUTSIDE), Route::Local);
    assert!(!r.any_forwarded());
    assert_eq!(r.route(&key(42, false), INSIDE), Route::Local);
}

#[test]
fn key_repeat_keeps_the_route_of_the_first_down() {
    let mut r = KeyRoutes::new();
    assert_eq!(r.route(&key(30, true), INSIDE), Route::Forward);
    assert_eq!(r.route(&key(30, true), OUTSIDE), Route::Forward);
    assert_eq!(r.route(&key(30, false), OUTSIDE), Route::Forward);

    assert_eq!(r.route(&key(31, true), OUTSIDE), Route::Local);
    assert_eq!(r.route(&key(31, true), INSIDE), Route::Local);
    assert_eq!(r.route(&key(31, false), INSIDE), Route::Local);
}

#[test]
fn buttons_follow_their_down() {
    let mut r = KeyRoutes::new();
    assert_eq!(r.route(&button(Button::Left, true), INSIDE), Route::Forward);
    assert_eq!(r.route(&button(Button::Right, true), OUTSIDE), Route::Local);
    assert_eq!(
        r.route(&button(Button::Left, false), OUTSIDE),
        Route::Forward
    );
    assert_eq!(r.route(&button(Button::Right, false), INSIDE), Route::Local);
    assert!(!r.any_forwarded());
}

#[test]
fn keys_and_buttons_are_separate() {
    let mut r = KeyRoutes::new();
    // Key 0 and the first button must not share a record.
    assert_eq!(r.route(&key(0, true), INSIDE), Route::Forward);
    assert_eq!(r.route(&button(Button::Left, false), INSIDE), Route::Local);
    assert!(r.any_forwarded());
}

#[test]
fn up_with_no_recorded_down_is_local() {
    // The down event came before the capture started.
    let mut r = KeyRoutes::new();
    assert_eq!(r.route(&key(29, false), INSIDE), Route::Local);
    assert_eq!(r.route(&button(Button::Left, false), INSIDE), Route::Local);
}

#[test]
fn full_key_range() {
    let mut r = KeyRoutes::new();
    for code in [0, 63, 64, 767, 1023, u16::MAX] {
        assert_eq!(r.route(&key(code, true), INSIDE), Route::Forward, "{code}");
    }
    for code in [0, 63, 64, 767, 1023, u16::MAX] {
        assert_eq!(
            r.route(&key(code, false), OUTSIDE),
            Route::Forward,
            "{code}"
        );
    }
    assert!(!r.any_forwarded());
}

#[test]
fn scroll_follows_the_cursor() {
    let mut r = KeyRoutes::new();
    let scroll = InputEvent::Scroll { dx: 0.0, dy: 3.0 };
    assert_eq!(r.route(&scroll, INSIDE), Route::Forward);
    assert_eq!(r.route(&scroll, OUTSIDE), Route::Local);
    assert!(!r.any_forwarded());
}

#[test]
fn clear_forgets_all() {
    let mut r = KeyRoutes::new();
    r.route(&key(30, true), INSIDE);
    r.route(&button(Button::Middle, true), INSIDE);
    r.route(&key(31, true), OUTSIDE);
    r.clear();
    assert!(!r.any_forwarded());
    assert_eq!(r.route(&key(30, false), OUTSIDE), Route::Local);
    assert_eq!(
        r.route(&button(Button::Middle, false), OUTSIDE),
        Route::Local
    );
    // Key 31 is not down any more, so a new down takes the current route.
    assert_eq!(r.route(&key(31, true), INSIDE), Route::Forward);
}
