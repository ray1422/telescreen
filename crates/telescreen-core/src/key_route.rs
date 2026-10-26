//! The route of each key and button: local or forwarded (FLOW-20, IN-3).
//!
//! The cursor is free, so it can cross the portal edge while a key or button is
//! down. The capture decides the route of a down event from the state at that
//! time. The matching up event, and each repeat of the down event, take the same
//! route. The input callback calls this inline (IN-2): it does not allocate and
//! does not lock.

use crate::input::{Button, InputEvent};

const KEY_WORDS: usize = (u16::MAX as usize + 1) / u64::BITS as usize;

/// Where an input event goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// The source handles the event. The capture lets it through.
    Local,
    /// The capture suppresses the event and reports it for the target.
    Forward,
}

/// The route of each key and button that is down. Keys use the full `Key` range.
#[derive(Debug, Clone)]
pub struct KeyRoutes {
    forwarded_keys: [u64; KEY_WORDS],
    local_keys: [u64; KEY_WORDS],
    forwarded_buttons: u64,
    local_buttons: u64,
}

impl KeyRoutes {
    /// No key or button is down.
    pub fn new() -> Self {
        Self {
            forwarded_keys: [0; KEY_WORDS],
            local_keys: [0; KEY_WORDS],
            forwarded_buttons: 0,
            local_buttons: 0,
        }
    }

    /// Returns the route of one event and records it.
    ///
    /// `forward_now` is true while forwarding is on and the cursor is in the
    /// portal.
    ///
    /// - A down event of a key or button that is not down takes `Forward` if
    ///   `forward_now`, else `Local`.
    /// - A down event of a key or button that is down (a key repeat) takes the
    ///   route of the first down event.
    /// - An up event takes the route of its down event, and the key or button is
    ///   then not down. An up event with no recorded down event takes `Local`: the
    ///   source saw the down event before the capture started.
    /// - A scroll event takes `Forward` if `forward_now`, else `Local`.
    pub fn route(&mut self, event: &InputEvent, forward_now: bool) -> Route {
        let (forwarded, local, mask, down) = match *event {
            InputEvent::Key { key, down } => {
                let code = usize::from(key.0);
                let word = code / u64::BITS as usize;
                let mask = 1_u64 << (code % u64::BITS as usize);
                (
                    &mut self.forwarded_keys[word],
                    &mut self.local_keys[word],
                    mask,
                    down,
                )
            }
            InputEvent::Button { button, down } => {
                let mask = match button {
                    Button::Left => 1,
                    Button::Right => 1 << 1,
                    Button::Middle => 1 << 2,
                    Button::Back => 1 << 3,
                    Button::Forward => 1 << 4,
                };
                (
                    &mut self.forwarded_buttons,
                    &mut self.local_buttons,
                    mask,
                    down,
                )
            }
            InputEvent::Scroll { .. } => {
                return if forward_now {
                    Route::Forward
                } else {
                    Route::Local
                };
            }
        };

        let route = if *forwarded & mask != 0 {
            Route::Forward
        } else if *local & mask != 0 || !down || !forward_now {
            Route::Local
        } else {
            Route::Forward
        };

        if down {
            match route {
                Route::Forward => *forwarded |= mask,
                Route::Local => *local |= mask,
            }
        } else {
            *forwarded &= !mask;
            *local &= !mask;
        }
        route
    }

    /// True if a key or button with the route `Forward` is down.
    pub fn any_forwarded(&self) -> bool {
        self.forwarded_buttons != 0 || self.forwarded_keys.iter().any(|&word| word != 0)
    }

    /// Forgets all keys and buttons, for example when the capture stops.
    pub fn clear(&mut self) {
        self.forwarded_keys.fill(0);
        self.local_keys.fill(0);
        self.forwarded_buttons = 0;
        self.local_buttons = 0;
    }
}

impl Default for KeyRoutes {
    fn default() -> Self {
        Self::new()
    }
}
