//! Input events that go from the source to the target, and the held-input tracker (IN-3, IN-4).

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::keys::Key;

/// A pointer button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Button {
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

/// An input event on the reliable channel (NET-1). Pointer positions use the
/// pointer datagram instead (`wire::PointerDatagram`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum InputEvent {
    Key {
        key: Key,
        down: bool,
    },
    Button {
        button: Button,
        down: bool,
    },
    /// Scroll in logical pixels. A positive `dy` moves the view toward the end of
    /// the document (down). A positive `dx` moves it to the right.
    Scroll {
        dx: f32,
        dy: f32,
    },
}

/// The keys and buttons that are down. Both peers use it to release all held
/// state at a pause or a session end (IN-3, IN-4).
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HeldInputs {
    keys: BTreeSet<Key>,
    buttons: BTreeSet<Button>,
}

impl HeldInputs {
    /// Records an event and returns true if the event must be sent or injected.
    /// A key down is always sent, because a repeated down is a key repeat. A button
    /// down for a button that is down, and an up for a key or button that is not
    /// down, return false.
    pub fn record(&mut self, event: &InputEvent) -> bool {
        match *event {
            InputEvent::Key { key, down: true } => {
                self.keys.insert(key);
                true
            }
            InputEvent::Key { key, down: false } => self.keys.remove(&key),
            InputEvent::Button { button, down: true } => self.buttons.insert(button),
            InputEvent::Button {
                button,
                down: false,
            } => self.buttons.remove(&button),
            InputEvent::Scroll { .. } => true,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.keys.is_empty() && self.buttons.is_empty()
    }

    /// Removes all held state and returns the up events that release it:
    /// buttons first, then keys.
    pub fn release_all(&mut self) -> Vec<InputEvent> {
        let buttons = std::mem::take(&mut self.buttons);
        let keys = std::mem::take(&mut self.keys);
        buttons
            .into_iter()
            .map(|button| InputEvent::Button {
                button,
                down: false,
            })
            .chain(
                keys.into_iter()
                    .map(|key| InputEvent::Key { key, down: false }),
            )
            .collect()
    }
}
