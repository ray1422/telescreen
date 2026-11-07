//! The capture rules of the portals on the source that all platforms share:
//! the safety exit (IN-8) and a change of the native keyboard focus
//! (FLOW-22, FLOW-23, IN-13).
//!
//! The input callback calls these inline (IN-2): they do not allocate, do not
//! lock, and do not read a clock. The caller gives the time.

use std::time::{Duration, Instant};

use crate::ids::PortalId;
use crate::key_route::{FocusChange, InputRouter};
use crate::platform::CapturedEvent;

/// IN-8: Escape this many times in [`SAFETY_WINDOW`].
pub const SAFETY_PRESSES: usize = 10;
/// IN-8: the time for [`SAFETY_PRESSES`].
pub const SAFETY_WINDOW: Duration = Duration::from_secs(5);

/// Counts the Escape presses of the safety exit (IN-8). Give each Escape key
/// down to [`SafetyExit::escape_down`]: it applies the rules of IN-8.
#[derive(Debug, Clone)]
pub struct SafetyExit {
    /// The times of the last presses, as a ring.
    presses: [Option<Instant>; SAFETY_PRESSES],
    next: usize,
}

impl SafetyExit {
    /// No press is recorded.
    pub const fn new() -> Self {
        Self {
            presses: [None; SAFETY_PRESSES],
            next: 0,
        }
    }

    /// Records one press at `now`. Returns true if this is the
    /// [`SAFETY_PRESSES`]th press in [`SAFETY_WINDOW`]: then the count starts
    /// again. A press exactly [`SAFETY_WINDOW`] old still counts.
    pub fn press(&mut self, now: Instant) -> bool {
        self.presses[self.next] = Some(now);
        self.next = (self.next + 1) % SAFETY_PRESSES;
        let recent = self
            .presses
            .iter()
            .flatten()
            .filter(|t| now.saturating_duration_since(**t) <= SAFETY_WINDOW)
            .count();
        if recent >= SAFETY_PRESSES {
            self.reset();
            return true;
        }
        false
    }

    /// Records one Escape key down at `now` by the counting rules of IN-8.
    /// Returns true if the safety exit fires. All capture code (the source
    /// and the warning of the destination) uses this function.
    ///
    /// - `waits_or_forwards`: a portal waits for the confirmation (FLOW-21)
    ///   or forwards. Only then does the source suppress input. An armed
    ///   capture with no such portal does not count.
    /// - `repeat`: the key down is an auto-repeat of a held key. It does not
    ///   count, because a held key is not ten presses.
    pub fn escape_down(&mut self, now: Instant, waits_or_forwards: bool, repeat: bool) -> bool {
        waits_or_forwards && !repeat && self.press(now)
    }

    /// Forgets all presses, for example when forwarding starts.
    pub fn reset(&mut self) {
        *self = Self::new();
    }
}

impl Default for SafetyExit {
    fn default() -> Self {
        Self::new()
    }
}

/// What the capture can see after a portal window loses the native keyboard
/// focus (IN-13).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusLoss {
    /// The capture still sees the up events of the forwarded keys (the macOS
    /// event tap).
    UpEventsSeen,
    /// Other windows get the up events (the Linux portal window, UI-6).
    UpEventsLost,
}

/// The events of a change of the keyboard focus, in order. It holds no
/// borrow, and it does not allocate.
pub type FocusEvents = std::iter::Flatten<std::array::IntoIter<Option<CapturedEvent>, 3>>;

/// Records a change of the native focus of one portal window, and returns
/// the events to report, in order (FLOW-23): `KeyboardLeft`, then `InputLost`,
/// then `KeyboardEntered`.
///
/// With [`FocusLoss::UpEventsLost`], a portal that forwards and loses the
/// keyboard focus loses the input (IN-13): the router forgets all routes, and
/// `InputLost` follows. `forwarding` tells if a portal forwards.
pub fn window_focus(
    router: &mut InputRouter,
    portal: PortalId,
    focused: bool,
    forwarding: impl Fn(PortalId) -> bool,
    loss: FocusLoss,
) -> FocusEvents {
    let FocusChange { left, entered } = router.set_window_focus(portal, focused);
    let lost = loss == FocusLoss::UpEventsLost && left.is_some_and(&forwarding);
    if lost {
        router.clear();
    }
    [
        left.map(|portal| CapturedEvent::KeyboardLeft { portal }),
        lost.then_some(CapturedEvent::InputLost),
        entered.map(|portal| CapturedEvent::KeyboardEntered { portal }),
    ]
    .into_iter()
    .flatten()
}
