//! The native Linux UI of Telescreen (ADR 0012, M20 step g): the main window in
//! QML through cxx-qt. The Plasma applet is a separate QML package in `applet/`;
//! it reads the menu view model over D-Bus (UI-11, UI-18).
//!
//! The UI only draws view models and returns intents. Rust gives the newest
//! [`MainScreen`] with [`push_main`]; the QML gets it as JSON. Each user action
//! comes back as a [`UiEvent`] on the channel of [`set_events`].
//!
//! Not connected to `telescreen-app` yet: the app still runs the winit loop.
//! `examples/preview.rs` shows the window with a fixture.

use telescreen_view::Intent;

#[cfg(target_os = "linux")]
mod bridge;

#[cfg(target_os = "linux")]
pub use bridge::{push_main, run, set_events};

/// What the user did in the Linux UI.
#[derive(Debug, Clone, PartialEq)]
pub enum UiEvent {
    Intent(Intent),
    /// "Configure Telescreen…" (UI-14).
    OpenSettings,
    /// "Quit Telescreen" (UI-15).
    Quit,
}
