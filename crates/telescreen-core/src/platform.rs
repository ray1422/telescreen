//! Platform interfaces. The platform crates implement them; the core and the app
//! use only these types. Prompt and window presentation are not here: they belong
//! to the view layer (`telescreen-view`).

use std::time::Instant;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::clipboard::ClipboardChange;
use crate::ids::{DisplayToken, PortalId};
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
/// The app calls it outside input callbacks (IN-2). The app calls `changed`,
/// `set_text`, and `clear_if_owned` only between `open` and `close`.
pub trait Clipboard {
    /// Starts to watch the clipboard. The current content is the baseline:
    /// `changed` does not report it (CLIP-3). On Linux, this opens the
    /// data-control connection.
    fn open(&mut self) -> Result<(), PlatformError>;
    /// Stops to watch the clipboard. On Linux, this closes the data-control
    /// connection. The app calls `clear_if_owned` first (CLIP-11).
    fn close(&mut self);
    /// Returns the latest change since the last call, or `None` if there is no
    /// change. A change that `set_text` or `clear_if_owned` made is also reported.
    fn changed(&mut self) -> Option<ClipboardChange>;
    /// Replaces the clipboard with plain text (CLIP-8). With `concealed`, also
    /// sets the concealed mark of the platform (CLIP-7).
    fn set_text(&mut self, text: &str, concealed: bool) -> Result<(), PlatformError>;
    /// Removes the clipboard content if it is still the last text from
    /// `set_text`. Else does nothing, for example after a copy by the user
    /// (CLIP-7, CLIP-11).
    fn clear_if_owned(&mut self);
}

/// Injects input on the target (IN-11, FLOW-5).
pub trait Injector {
    /// Selects the input-target display for the next pointer positions.
    fn set_display(&mut self, display: &DisplayInfo) -> Result<(), PlatformError>;
    /// Moves the pointer to a position on the input-target display.
    fn pointer(&mut self, at: Normalized) -> Result<(), PlatformError>;
    fn input(&mut self, event: &InputEvent) -> Result<(), PlatformError>;
    /// Runs the timers of the injector, for example the end of a scroll (IN-18).
    /// The main loop calls this often.
    fn tick(&mut self, _now: Instant) -> Result<(), PlatformError> {
        Ok(())
    }
    /// The session of the injector ended or was lost (IN-18).
    fn end_session(&mut self) {}
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
/// and pushes it to a bounded queue (IN-2). An event of one portal names the
/// portal (FLOW-25).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CapturedEvent {
    /// A key, button, or scroll event that the capture suppressed (its route is
    /// `Forward(portal)`, see [`crate::key_route`]).
    Input { portal: PortalId, event: InputEvent },
    /// While forwarding: the latest cursor position in the portal, in local
    /// desktop logical coordinates. The capture merges motions, so that a fast
    /// motion does not fill the queue.
    PointerMoved { portal: PortalId, at: Point },
    /// The cursor came into the portal, at this position (FLOW-14).
    /// The session stays in forwarding.
    PointerEntered { portal: PortalId, at: Point },
    /// The cursor left the portal (FLOW-14).
    /// The session stays in forwarding.
    PointerLeft { portal: PortalId },
    /// A button went to the target of the portal while the portal has no
    /// keyboard focus. The window layer asks the OS for the native focus of the
    /// portal window (FLOW-22).
    FocusWanted { portal: PortalId },
    /// The portal got the keyboard focus (FLOW-22).
    KeyboardEntered { portal: PortalId },
    /// The portal lost the keyboard focus (FLOW-22).
    KeyboardLeft { portal: PortalId },
    /// The capture can no longer see forwarded up events (IN-13). It applies to
    /// all portals.
    InputLost,
    /// The safety exit (IN-8). The capture already stopped suppression for all
    /// portals.
    SafetyExit,
    /// The queue was full and an event was lost (IN-3). The queue is shared, so
    /// it applies to all portals.
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
    /// Watches the cursor with no suppression, for the area `area` of the portal
    /// `portal`. Reports `PointerEntered` and `PointerLeft` at each crossing of
    /// the portal edge, and `PointerEntered` at once if the cursor is in the
    /// portal now. A second call for the same portal replaces its area (AC-8).
    /// A call for a different portal adds that portal; the areas of two portals
    /// do not overlap, because each portal is on a different display (FLOW-25).
    /// A capture that supports fewer portals returns an error for one more.
    fn arm(&mut self, portal: PortalId, area: Rect) -> Result<(), PlatformError>;
    /// Starts forwarding for the portal (FLOW-13).
    /// Keys follow the keyboard focus (FLOW-22).
    /// Pointer input follows the cursor in the portal (FLOW-20).
    /// The capture uses one [`crate::key_route::InputRouter`] for all portals
    /// (IN-3). It reports keyboard focus changes before the input that caused
    /// them (FLOW-23).
    /// Forwarding continues across each leave and entry until `disarm`.
    fn forward(&mut self, portal: PortalId) -> Result<(), PlatformError>;
    /// The app calls this when the native keyboard focus of the portal window
    /// changes (FLOW-22). The capture reports `KeyboardEntered` or
    /// `KeyboardLeft` for each portal whose keyboard focus changed.
    fn set_portal_focus(&mut self, _portal: PortalId, _focused: bool) {}
    /// The window layer reports whether the portal is on screen (FLOW-26). While
    /// it is not, the area of that portal does not count as the portal.
    fn set_portal_visible(&mut self, _portal: PortalId, _visible: bool) {}
    /// FLOW-21: while the source waits for the confirmation in this portal,
    /// keeps the portal fixed and focused. With `on`, the capture suppresses the
    /// gestures and shortcuts that switch Spaces or applications, and the clicks
    /// out of all portals. It lets the confirmation key and the clicks in this
    /// portal through, and it reports nothing for this portal. A click in a
    /// different portal that forwards goes to that portal (FLOW-25).
    /// `guard(portal, false)`, `forward(portal)`, and `disarm(portal)` end it.
    /// It needs `arm` of the portal first.
    fn guard(&mut self, _portal: PortalId, _on: bool) -> Result<(), PlatformError> {
        Ok(())
    }
    /// Stops the capture of one portal and forgets its routes. Local input comes
    /// back in its area. The other portals keep their capture.
    fn disarm(&mut self, portal: PortalId);
    /// The main loop calls this on each turn. A capture with a watchdog stops
    /// suppression when the heartbeat stops (IN-9).
    fn heartbeat(&mut self) {}
}

/// The mDNS service type of Telescreen (NET-14).
pub const DISCOVERY_SERVICE: &str = "_telescreen._udp";

/// A record of a search (NET-15). It is only a hint for an address (NET-13).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundPeer {
    /// The text of the TXT key `fp`.
    pub fingerprint: String,
    pub addresses: Vec<std::net::IpAddr>,
    pub port: u16,
}

/// A report of the discovery backend. It can come from any thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryEvent {
    Found(FoundPeer),
    /// A running search or record stopped because of an error.
    Failed(String),
}

/// Receives the reports of a discovery backend.
pub type DiscoverySink = std::sync::Arc<dyn Fn(DiscoveryEvent) + Send + Sync>;

/// Finds peers through the mDNS responder of the system (NET-12). A method must
/// not block for long: the app calls it on the main thread.
pub trait Discovery {
    /// Registers this computer until `stop_announce` (NET-14).
    fn announce(&mut self, port: u16, fingerprint: &str) -> Result<(), PlatformError>;
    fn stop_announce(&mut self);
    /// Searches until `stop_search`, and reports each record (NET-15).
    fn search(&mut self) -> Result<(), PlatformError>;
    fn stop_search(&mut self);
}

/// A random instance name for a record (NET-14). It is not a secret.
pub fn discovery_instance_name() -> String {
    use std::hash::{BuildHasher, Hasher};
    let value = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    format!("telescreen-{value:016x}")
}

/// The value of the TXT key `fp` in one TXT entry.
pub fn fingerprint_of_txt(entry: &[u8]) -> Option<String> {
    let value = entry.strip_prefix(b"fp=")?;
    String::from_utf8(value.to_vec()).ok()
}
