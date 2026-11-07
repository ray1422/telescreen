//! IN-19: the smoothing of a forwarded finger scroll on the target.
#![allow(clippy::unwrap_used)] // Tests may panic on a failed setup.

use std::time::{Duration, Instant};

use telescreen_core::scroll_smooth::{STEP, ScrollSmoother};

fn ms(n: u64) -> Duration {
    Duration::from_millis(n)
}

/// Runs the steps until `until`, and returns the total distance.
fn steps(s: &mut ScrollSmoother, until: Instant) -> (f32, f32) {
    let mut total = (0.0, 0.0);
    while let Some(at) = s.next_step().filter(|at| *at <= until) {
        let (dx, dy) = s.step(at);
        total.0 += dx;
        total.1 += dy;
    }
    total
}

#[test]
fn in19_the_first_event_of_a_scroll_goes_at_once() {
    let now = Instant::now();
    let mut s = ScrollSmoother::default();
    assert_eq!(s.push(0.0, 12.0, now), (0.0, 12.0));
    assert!(!s.waiting());
    assert_eq!(s.next_step(), None);
}

#[test]
fn in19_a_steady_scroll_goes_one_step_later() {
    let t0 = Instant::now();
    let mut s = ScrollSmoother::default();
    s.push(0.0, 10.0, t0);
    for i in 1..=10 {
        let at = t0 + STEP * i;
        assert_eq!(s.push(0.0, 10.0, at), (0.0, 0.0));
        assert_eq!(s.next_step(), Some(at), "a step is due");
        let (_, dy) = s.step(at);
        assert!((dy - 10.0).abs() < 0.01, "step {i}: {dy}");
    }
}

#[test]
fn in19_a_group_of_late_events_is_spread_and_not_lost() {
    let t0 = Instant::now();
    let mut s = ScrollSmoother::default();
    s.push(0.0, 10.0, t0);
    s.push(0.0, 10.0, t0 + ms(8));
    s.step(t0 + ms(8));
    // The network holds 5 events; they come together 48 ms later.
    let late = t0 + ms(56);
    for _ in 0..5 {
        s.push(0.0, 10.0, late);
    }
    let (_, first) = s.step(late);
    assert!(first < 50.0, "not all at once: {first}");
    let (_, rest) = steps(&mut s, late + ms(200));
    assert!(
        (first + rest - 50.0).abs() < 0.1,
        "all distance goes: {}",
        first + rest
    );
    assert!(!s.waiting());
}

#[test]
fn in19_the_distance_waits_at_most_about_the_catch_up_time() {
    let t0 = Instant::now();
    let mut s = ScrollSmoother::default();
    s.push(0.0, 1.0, t0);
    // A slow scroll, then a large late group.
    s.push(0.0, 1.0, t0 + ms(50));
    s.push(0.0, 500.0, t0 + ms(50));
    let (_, moved) = steps(&mut s, t0 + ms(50) + ms(200));
    assert!(moved > 495.0, "{moved}");
}

#[test]
fn in19_a_pause_starts_a_new_scroll_with_what_waits() {
    let t0 = Instant::now();
    let mut s = ScrollSmoother::default();
    s.push(0.0, 10.0, t0);
    s.push(0.0, 10.0, t0 + ms(8));
    let out = s.push(0.0, -5.0, t0 + ms(500));
    assert_eq!(out, (0.0, 5.0), "the 10 that waits, and the new -5");
    assert!(!s.waiting());
}

#[test]
fn in19_flush_returns_what_waits() {
    let t0 = Instant::now();
    let mut s = ScrollSmoother::default();
    s.push(3.0, 10.0, t0);
    s.push(3.0, 10.0, t0 + ms(8));
    assert_eq!(s.flush(), (3.0, 10.0));
    assert!(!s.waiting());
    assert_eq!(s.flush(), (0.0, 0.0));
}
