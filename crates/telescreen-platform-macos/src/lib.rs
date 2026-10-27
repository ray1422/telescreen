//! macOS backends of Telescreen (`docs/spec.md` section 8.1).
//!
//! - [`capture`]: the event tap that captures and suppresses input on the source,
//!   with the safety exit and the watchdog (IN-7..IN-10, ADR 0003).
//! - [`inject`]: `CGEventPost` injection on the target.
//! - [`displays`]: the display list.
//! - [`window`]: AppKit adjustments of winit windows (portal level, input sources,
//!   prompt overlay).
#![cfg(target_os = "macos")]

pub mod capture;
pub mod displays;
mod ffi;
pub mod focus;
pub mod inject;
pub mod window;

use telescreen_core::platform::{Capability, InputCapabilities};

/// IN-1, IN-12: report each capability. Input Monitoring is needed to listen,
/// Accessibility to suppress and to post events.
pub fn capabilities() -> InputCapabilities {
    // SAFETY: these functions only read the permission state.
    let (listen, trusted, post) = unsafe {
        (
            ffi::CGPreflightListenEventAccess(),
            ffi::AXIsProcessTrusted(),
            ffi::CGPreflightPostEventAccess(),
        )
    };
    let state = |ok: bool, what: &str| {
        if ok {
            Capability::Available
        } else {
            Capability::Missing(format!(
                "grant {what} to Telescreen in System Settings > Privacy & Security"
            ))
        }
    };
    InputCapabilities {
        capture: state(listen, "Input Monitoring"),
        suppress: state(trusted, "Accessibility"),
        inject: state(post, "Accessibility"),
    }
}

/// Asks macOS to show the permission prompts (Input Monitoring, Accessibility).
/// The user grants them; Telescreen never changes the settings.
pub fn request_permissions() {
    // SAFETY: these functions only ask the system to show a prompt.
    unsafe {
        if !ffi::CGPreflightListenEventAccess() {
            ffi::CGRequestListenEventAccess();
        }
        if !ffi::CGPreflightPostEventAccess() {
            ffi::CGRequestPostEventAccess();
        }
    }
}

/// True if Secure Event Input is on: then an event tap does not see keys (IN-12).
pub fn secure_event_input() -> bool {
    // SAFETY: a read-only query.
    unsafe { ffi::IsSecureEventInputEnabled() }
}
