//! Platform interfaces. The platform crates implement them; the core and the app
//! use only these types. Prompt and window presentation are not here: they belong
//! to the view layer (`telescreen-view`).

use serde::Serialize;
use serde_json::{Map, Value};

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

/// Injects input on the target (IN-11, FLOW-5).
pub trait Injector {
    /// Selects the input-target display for the next pointer positions.
    fn set_display(&mut self, display: &DisplayInfo) -> Result<(), PlatformError>;
    /// Moves the pointer to a position on the input-target display.
    fn pointer(&mut self, at: Normalized) -> Result<(), PlatformError>;
    fn input(&mut self, event: &InputEvent) -> Result<(), PlatformError>;
}

/// An event from the input capture of the source. The capture callback makes it
/// and pushes it to a bounded queue (IN-2).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CapturedEvent {
    Input(InputEvent),
    /// A relative pointer motion in logical pixels, while the cursor is frozen.
    Motion {
        dx: f64,
        dy: f64,
    },
    /// While armed and not forwarding: the pointer is in the portal (FLOW-14).
    PointerEntered {
        at: Point,
    },
    /// The pointer left the portal by a way that the capture does not control, for
    /// example a focus change that ended the pointer lock (FLOW-14).
    PointerLeft,
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

/// Input capture on the source (FLOW-13, FLOW-14, IN-1, IN-2). The capture sends
/// [`CapturedEvent`]s to the app through a bounded queue.
pub trait InputCapture {
    /// Watches the pointer with no suppression. Reports `PointerEntered` when the
    /// pointer is in `portal`, also at once if it is there now.
    fn arm(&mut self, portal: Rect) -> Result<(), PlatformError>;
    /// Freezes the cursor, suppresses all input, and reports it. Returns the cursor
    /// position, where the virtual pointer starts.
    fn forward(&mut self) -> Result<Point, PlatformError>;
    /// Stops suppression and shows the cursor, at `at` if given. Stays armed.
    fn pause(&mut self, at: Option<Point>);
    /// Stops all capture. Local input and the cursor come back.
    fn disarm(&mut self);
    /// The main loop calls this on each turn. A capture with a watchdog stops
    /// suppression when the heartbeat stops (IN-9).
    fn heartbeat(&mut self) {}
}
