//! The native macOS UI (ADR 0012): the status item and the menu panel (UI-9,
//! UI-10). Rust makes the windows. Swift makes the SwiftUI hosting views. The
//! crate is empty on other platforms.

#[cfg(target_os = "macos")]
mod mac;
#[cfg(target_os = "macos")]
pub use mac::*;
