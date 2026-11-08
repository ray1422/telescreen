//! The route of each key and button: local or forwarded (FLOW-20, IN-3).
//!
//! The cursor is free, so it can cross the portal edge while a key or button is
//! down. The capture decides the route of a down event from the state at that
//! time. The matching up event, and each repeat of the down event, take the same
//! route. The input callback calls this inline (IN-2): it does not allocate and
//! does not lock.

use crate::ids::PortalId;
use crate::input::{Button, InputEvent};
use crate::keys::Key;

const KEY_COUNT: usize = u16::MAX as usize + 1;
const KEY_WORDS: usize = KEY_COUNT / u64::BITS as usize;
const BUTTON_COUNT: usize = 5;

/// Where an input event goes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Route {
    /// The source handles the event. The capture lets it through.
    Local,
    /// The capture suppresses the event and reports it for the target of this
    /// portal (FLOW-25).
    Forward(PortalId),
}

/// The route of each key and button that is down. Keys use the full `Key` range.
/// Each forwarded key and button records its portal, so that its up event goes
/// to the same portal (IN-3, FLOW-25).
#[derive(Debug, Clone)]
pub struct KeyRoutes {
    /// The portal of each forwarded key that is down. 0 is no portal.
    forwarded_keys: Box<[u64]>,
    local_keys: [u64; KEY_WORDS],
    /// The portal of each forwarded button that is down. 0 is no portal.
    forwarded_buttons: [u64; BUTTON_COUNT],
    local_buttons: u64,
}

fn button_index(button: Button) -> usize {
    match button {
        Button::Left => 0,
        Button::Right => 1,
        Button::Middle => 2,
        Button::Back => 3,
        Button::Forward => 4,
    }
}

// The tables store `PortalId.0 + 1`, so that 0 is free for "no portal".
fn slot(portal: PortalId) -> u64 {
    portal.0.wrapping_add(1)
}

fn portal_of(slot: u64) -> Option<PortalId> {
    (slot != 0).then(|| PortalId(slot.wrapping_sub(1)))
}

impl KeyRoutes {
    /// No key or button is down. This allocates once; `route` does not allocate.
    pub fn new() -> Self {
        Self {
            forwarded_keys: vec![0; KEY_COUNT].into_boxed_slice(),
            local_keys: [0; KEY_WORDS],
            forwarded_buttons: [0; BUTTON_COUNT],
            local_buttons: 0,
        }
    }

    /// Returns the route of one event and records it.
    ///
    /// `forward_to` is the portal that gets the event now: the portal with the
    /// keyboard focus for a key, the portal under the cursor for a button or a
    /// scroll, if that portal forwards. `None` is local.
    ///
    /// - A down event of a key or button that is not down takes
    ///   `Forward(portal)` if `forward_to` is `Some(portal)`, else `Local`.
    /// - A down event of a key or button that is down (a key repeat) takes the
    ///   route of the first down event.
    /// - An up event takes the route of its down event, and the key or button is
    ///   then not down. An up event with no recorded down event takes `Local`: the
    ///   source saw the down event before the capture started.
    /// - A scroll event takes `Forward(portal)` if `forward_to` is `Some(portal)`,
    ///   else `Local`.
    pub fn route(&mut self, event: &InputEvent, forward_to: Option<PortalId>) -> Route {
        let (forwarded, local, mask, down) = match *event {
            InputEvent::Key { key, down } => {
                let code = usize::from(key.0);
                let word = code / u64::BITS as usize;
                let mask = 1_u64 << (code % u64::BITS as usize);
                (
                    &mut self.forwarded_keys[code],
                    &mut self.local_keys[word],
                    mask,
                    down,
                )
            }
            InputEvent::Button { button, down } => {
                let index = button_index(button);
                (
                    &mut self.forwarded_buttons[index],
                    &mut self.local_buttons,
                    1_u64 << index,
                    down,
                )
            }
            InputEvent::Scroll { .. } => {
                return forward_to.map_or(Route::Local, Route::Forward);
            }
        };

        let route = if let Some(portal) = portal_of(*forwarded) {
            Route::Forward(portal)
        } else if *local & mask != 0 || !down {
            Route::Local
        } else {
            forward_to.map_or(Route::Local, Route::Forward)
        };

        if down {
            match route {
                Route::Forward(portal) => *forwarded = slot(portal),
                Route::Local => *local |= mask,
            }
        } else {
            *forwarded = 0;
            *local &= !mask;
        }
        route
    }

    /// True if a key or button with a route `Forward` is down.
    pub fn any_forwarded(&self) -> bool {
        self.forwarded_buttons.iter().any(|&s| s != 0)
            || self.forwarded_keys.iter().any(|&s| s != 0)
    }

    /// The portal of a key that is down with the route `Forward`, else `None`.
    pub fn key_forwarded(&self, key: Key) -> Option<PortalId> {
        self.forwarded_keys
            .get(usize::from(key.0))
            .copied()
            .and_then(portal_of)
    }

    /// True if a key or button with the route `Forward(portal)` is down.
    pub fn forwarded_to(&self, portal: PortalId) -> bool {
        let s = slot(portal);
        self.forwarded_buttons.contains(&s) || self.forwarded_keys.contains(&s)
    }

    /// Forgets the keys and buttons of one portal, for example when its capture
    /// stops. Their up events then take `Local`. The other portals keep theirs.
    pub fn forget(&mut self, portal: PortalId) {
        let s = slot(portal);
        for entry in self
            .forwarded_keys
            .iter_mut()
            .chain(self.forwarded_buttons.iter_mut())
        {
            if *entry == s {
                *entry = 0;
            }
        }
    }

    /// Forgets all keys and buttons, for example when the capture stops.
    pub fn clear(&mut self) {
        self.forwarded_keys.fill(0);
        self.local_keys.fill(0);
        self.forwarded_buttons = [0; BUTTON_COUNT];
        self.local_buttons = 0;
    }
}

impl Default for KeyRoutes {
    fn default() -> Self {
        Self::new()
    }
}

/// The route of one event, and whether a portal must ask for the native
/// keyboard focus (FLOW-22).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Decision {
    pub route: Route,
    /// A button went down in this portal while it has no keyboard focus. The
    /// window layer asks the OS for the native focus of that portal window. The
    /// keyboard focus changes only when the OS reports it
    /// ([`InputRouter::set_window_focus`]).
    pub focus_wanted: Option<PortalId>,
}

/// A change of the keyboard focus between portals (FLOW-22, FLOW-25).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct FocusChange {
    /// The portal that lost the keyboard focus.
    pub left: Option<PortalId>,
    /// The portal that got the keyboard focus.
    pub entered: Option<PortalId>,
}

impl FocusChange {
    /// True if no portal got or lost the keyboard focus.
    pub fn is_none(&self) -> bool {
        self.left.is_none() && self.entered.is_none()
    }
}

/// Routes pointer input by the cursor and keys by keyboard focus (FLOW-13).
/// With more than one portal, at most one portal has the keyboard focus, because
/// the native focus is one window (FLOW-25).
#[derive(Debug, Clone)]
pub struct InputRouter {
    routes: KeyRoutes,
    keyboard: Option<PortalId>,
}

impl InputRouter {
    /// Starts with no recorded routes and the keyboard outside all portals
    /// (FLOW-22).
    pub fn new() -> Self {
        Self {
            routes: KeyRoutes::new(),
            keyboard: None,
        }
    }

    /// Routes one event and records its down route (IN-3).
    ///
    /// `cursor_in` is the forwarding portal under the cursor, if any. Keys go
    /// to the portal with the keyboard focus; the caller reports only the focus
    /// of a forwarding portal. A button down in a portal with no keyboard focus
    /// asks for the focus of that portal (FLOW-22).
    pub fn route(&mut self, event: &InputEvent, cursor_in: Option<PortalId>) -> Decision {
        let focus_wanted = match (event, cursor_in) {
            (InputEvent::Button { down: true, .. }, Some(portal))
                if self.keyboard != Some(portal) =>
            {
                Some(portal)
            }
            _ => None,
        };
        let forward_to = match event {
            InputEvent::Key { .. } => self.keyboard,
            InputEvent::Button { .. } | InputEvent::Scroll { .. } => cursor_in,
        };
        Decision {
            route: self.routes.route(event, forward_to),
            focus_wanted,
        }
    }

    /// Records a change of the native focus of one portal window, and returns
    /// the change of the keyboard focus. This is the only change of the
    /// keyboard focus (FLOW-22).
    ///
    /// - `focused` for a portal that does not have the keyboard: it gets the
    ///   keyboard, and the portal that had it loses it.
    /// - Not `focused` for the portal that has the keyboard: no portal has it.
    /// - Each other report changes nothing, for example the late loss report
    ///   of a portal that already lost the keyboard to a different portal.
    pub fn set_window_focus(&mut self, portal: PortalId, focused: bool) -> FocusChange {
        match (focused, self.keyboard) {
            (true, current) if current != Some(portal) => {
                self.keyboard = Some(portal);
                FocusChange {
                    left: current,
                    entered: Some(portal),
                }
            }
            (false, Some(current)) if current == portal => {
                self.keyboard = None;
                FocusChange {
                    left: Some(portal),
                    entered: None,
                }
            }
            _ => FocusChange::default(),
        }
    }

    /// The portal with the keyboard focus (FLOW-22).
    pub fn keyboard(&self) -> Option<PortalId> {
        self.keyboard
    }

    /// The portal of a key that is down with the route `Forward`, else `None`.
    pub fn key_forwarded(&self, key: Key) -> Option<PortalId> {
        self.routes.key_forwarded(key)
    }

    /// True if a key or button with the route `Forward(portal)` is down.
    pub fn forwarded_to(&self, portal: PortalId) -> bool {
        self.routes.forwarded_to(portal)
    }

    /// Forgets one portal when its capture stops: its routes, and its keyboard
    /// focus. The other portals keep theirs.
    pub fn forget(&mut self, portal: PortalId) {
        self.routes.forget(portal);
        if self.keyboard == Some(portal) {
            self.keyboard = None;
        }
    }

    /// Forgets routes after input is lost without changing keyboard focus (IN-13).
    pub fn clear(&mut self) {
        self.routes.clear();
    }
}

impl Default for InputRouter {
    fn default() -> Self {
        Self::new()
    }
}
