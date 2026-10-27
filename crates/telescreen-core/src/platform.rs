//! Platform interfaces. The platform crates implement them; the core and the app
//! use only these types. Prompt and window presentation are not here: they belong
//! to the view layer (`telescreen-view`).

use serde::Serialize;
use serde_json::{Map, Value};

use crate::clipboard::ClipboardItem;
use crate::ids::DisplayToken;
use crate::input::InputEvent;
use crate::pointer::{Normalized, Point, Rect};

/// One display from the current OS enumeration (`docs/config.md`, driver request
/// `displays`).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DisplayInfo {
    pub token: DisplayToken,
    /// Local desktop logical coordinates, or `None` if the platform cannot get
    /// global coordinates. Do not make up coordinates.
    pub bounds: Option<Rect>,
    /// Logical pixels to physical pixels.
    pub scale: f64,
    /// Platform-dependent data for driver helpers, for example the vendor and model
    /// on macOS, or the output name on Linux.
    pub metadata: Map<String, Value>,
}

#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    /// A permission is missing (IN-12). The text tells the user what to grant.
    #[error("permission missing: {0}")]
    Permission(String),
    /// The OS or the compositor does not support the operation.
    #[error("not supported: {0}")]
    Unsupported(String),
    #[error("{0}")]
    Failed(String),
}

/// Enumerates the local displays.
pub trait Displays {
    fn displays(&self) -> Result<Vec<DisplayInfo>, PlatformError>;
}

/// Provides the local text clipboard capability (CLIP-8).
/// The app calls it outside input callbacks (IN-2).
pub trait Clipboard {
    /// Returns the latest item if the clipboard changed since the last call.
    /// Returns `None` if it did not change or does not contain text.
    fn changed(&mut self) -> Option<ClipboardItem>;
    /// Replaces the clipboard with plain text (CLIP-8).
    fn set_text(&mut self, text: &str) -> Result<(), PlatformError>;
}

/// Injects input on the target (IN-11, FLOW-5).
pub trait Injector {
    /// Selects the input-target display for the next pointer positions.
    fn set_display(&mut self, display: &DisplayInfo) -> Result<(), PlatformError>;
    /// Moves the pointer to a position on the input-target display.
    fn pointer(&mut self, at: Normalized) -> Result<(), PlatformError>;
    fn input(&mut self, event: &InputEvent) -> Result<(), PlatformError>;
}

/// Holds the keyboard focus away from target applications (UI-7, FLOW-23).
/// The focus holder shows nothing and takes no pointer input.
pub trait FocusHolder {
    /// Takes the keyboard focus on the input-target display (UI-7).
    fn hold(&mut self, display: &DisplayInfo) -> Result<(), PlatformError>;
    /// Removes the focus holder and restores the previous keyboard focus (UI-7).
    /// Does nothing if no hold is active.
    fn release(&mut self);
}

/// An event from the input capture of the source. The capture callback makes it
/// and pushes it to a bounded queue (IN-2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CapturedEvent {
    /// A key, button, or scroll event that the capture suppressed (its route is
    /// `Forward`, see [`crate::key_route`]).
    Input(InputEvent),
    /// While forwarding: the latest cursor position in the portal, in local
    /// desktop logical coordinates. The capture merges motions, so that a fast
    /// motion does not fill the queue.
    PointerMoved { at: Point },
    /// The cursor came into the portal, at this position (FLOW-14).
    /// The session stays in forwarding.
    PointerEntered { at: Point },
    /// The cursor left the portal (FLOW-14).
    /// The session stays in forwarding.
    PointerLeft,
    /// The portal got the keyboard focus (FLOW-22).
    KeyboardEntered,
    /// The portal lost the keyboard focus (FLOW-22).
    KeyboardLeft,
    /// The capture can no longer see forwarded up events (IN-13).
    InputLost,
    /// The release shortcut (IN-7). It is never forwarded.
    ReleaseShortcut,
    /// The safety exit (IN-8). The capture already stopped suppression.
    SafetyExit,
    /// The queue was full and an event was lost (IN-3).
    Overflow,
}

/// The state of one input capability (IN-1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Capability {
    Available,
    /// Not available, with the reason for the user.
    Missing(String),
}

/// The four input capabilities of IN-1. Forward is the transport, so it is not here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputCapabilities {
    pub capture: Capability,
    pub suppress: Capability,
    pub inject: Capability,
}

/// Input capture on the source (FLOW-13, FLOW-14, FLOW-20, IN-1, IN-2). The
/// capture sends [`CapturedEvent`]s to the app through a bounded queue.
///
/// The cursor is always free: the capture never freezes, locks, or hides it, and
/// it never suppresses pointer motion (FLOW-20).
pub trait InputCapture {
    /// Watches the cursor with no suppression, for the portal area `portal`.
    /// Reports `PointerEntered` and `PointerLeft` at each crossing of the portal
    /// edge, and `PointerEntered` at once if the cursor is in the portal now. A
    /// second call replaces the portal area (AC-8).
    fn arm(&mut self, portal: Rect) -> Result<(), PlatformError>;
    /// Starts forwarding (FLOW-13).
    /// Keys follow the keyboard focus (FLOW-22).
    /// Pointer input follows the cursor in the portal (FLOW-20).
    /// The capture uses [`crate::key_route::InputRouter`] for paired events (IN-3).
    /// It reports keyboard focus changes before the input that caused them (FLOW-23).
    /// Forwarding continues across each leave and entry until `disarm`.
    fn forward(&mut self) -> Result<(), PlatformError>;
    /// The app calls this when the portal's native keyboard focus changes (FLOW-22).
    /// The capture reports `KeyboardEntered` or `KeyboardLeft` if it changed.
    fn set_portal_focus(&mut self, _focused: bool) {}
    /// FLOW-21: while the source waits for the confirmation, keeps the portal fixed
    /// and focused. With `on`, the capture suppresses the gestures and shortcuts that
    /// switch Spaces or applications, and the clicks out of the portal. It lets the
    /// confirmation key and the clicks in the portal through, and it reports nothing.
    /// `guard(false)`, `forward`, and `disarm` end it. It needs `arm` first.
    fn guard(&mut self, _on: bool) -> Result<(), PlatformError> {
        Ok(())
    }
    /// Stops all capture. Local input comes back.
    fn disarm(&mut self);
    /// The main loop calls this on each turn. A capture with a watchdog stops
    /// suppression when the heartbeat stops (IN-9).
    fn heartbeat(&mut self) {}
}
