//! Smooths a forwarded finger scroll on the target (IN-19).
//!
//! The network can delay scroll events and then give several at the same time.
//! The smoother keeps the distance that it did not inject yet, and injects it in
//! steps of [`STEP`], at the speed of the last scroll events. It injects only
//! the distance that it received, so a scroll never goes past the end of the
//! fingers on the source. It uses no time stamps from the source.

use std::time::{Duration, Instant};

/// The time between two steps while distance waits.
pub const STEP: Duration = Duration::from_millis(8);
/// Distance that waits is injected in this time at least, also if the speed of
/// the scroll goes down. Thus a group of late events adds this delay at most.
const CATCH_UP: Duration = Duration::from_millis(50);
/// A time between two scroll events that is longer than this starts a new
/// scroll: the first event of a scroll goes at once.
const NEW_SCROLL: Duration = Duration::from_millis(100);
/// Events closer than this came together from the network: they do not change
/// the speed.
const TOGETHER: Duration = Duration::from_millis(2);
/// The weight of the newest speed in the average.
const WEIGHT: f32 = 0.5;

#[derive(Debug, Default, Clone)]
pub struct ScrollSmoother {
    /// The distance in logical pixels that waits.
    pending: (f32, f32),
    /// The average speed of the scroll events, in pixels for each second.
    speed: (f32, f32),
    last_event: Option<Instant>,
    last_step: Option<Instant>,
}

impl ScrollSmoother {
    /// Adds a scroll event. Returns the distance to inject now: the first event
    /// of a scroll goes at once, the next events wait for [`ScrollSmoother::step`].
    pub fn push(&mut self, dx: f32, dy: f32, now: Instant) -> (f32, f32) {
        let interval = self.last_event.map(|last| now.duration_since(last));
        self.last_event = Some(now);
        match interval {
            Some(interval) if interval < TOGETHER => {
                self.pending.0 += dx;
                self.pending.1 += dy;
                (0.0, 0.0)
            }
            Some(interval) if interval < NEW_SCROLL => {
                let seconds = interval.as_secs_f32();
                let average = |old: f32, new: f32| {
                    if old == 0.0 {
                        new
                    } else {
                        old + WEIGHT * (new - old)
                    }
                };
                self.speed = (
                    average(self.speed.0, dx.abs() / seconds),
                    average(self.speed.1, dy.abs() / seconds),
                );
                self.pending.0 += dx;
                self.pending.1 += dy;
                (0.0, 0.0)
            }
            _ => {
                // A new scroll: what waits goes first, with this event.
                let out = (self.pending.0 + dx, self.pending.1 + dy);
                *self = Self {
                    last_event: Some(now),
                    last_step: Some(now),
                    ..Self::default()
                };
                out
            }
        }
    }

    /// The distance to inject at this time. Call it at [`ScrollSmoother::next_step`].
    pub fn step(&mut self, now: Instant) -> (f32, f32) {
        if !self.waiting() {
            self.last_step = Some(now);
            return (0.0, 0.0);
        }
        let elapsed = self.last_step.map_or(STEP, |last| now.duration_since(last));
        self.last_step = Some(now);
        // A late step does not inject more than two steps at the speed.
        let moving = elapsed.min(STEP * 2).as_secs_f32();
        let catch_up = (elapsed.as_secs_f32() / CATCH_UP.as_secs_f32()).min(1.0);
        let take = |pending: &mut f32, speed: f32| {
            let size = (speed * moving).max(pending.abs() * catch_up);
            let out = pending.signum() * size.min(pending.abs());
            *pending -= out;
            out
        };
        (
            take(&mut self.pending.0, self.speed.0),
            take(&mut self.pending.1, self.speed.1),
        )
    }

    /// The time of the next step, while distance waits.
    pub fn next_step(&self) -> Option<Instant> {
        self.waiting()
            .then(|| self.last_step.map(|last| last + STEP))
            .flatten()
    }

    /// True while distance waits.
    pub fn waiting(&self) -> bool {
        self.pending.0.abs() >= 0.01 || self.pending.1.abs() >= 0.01
    }

    /// Returns all distance that waits, and forgets the scroll.
    pub fn flush(&mut self) -> (f32, f32) {
        let out = self.pending;
        *self = Self::default();
        out
    }
}
