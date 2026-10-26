//! Linux (Wayland) backends of Telescreen (`docs/spec.md` section 8.1, ADR 0006).
//!
//! - [`displays`]: the outputs, with their names and logical coordinates.
//! - [`inject`]: the wlroots virtual pointer (absolute motion on the input-target
//!   output) and a uinput keyboard.
//! - [`prompt`]: the layer-shell confirmation prompt.
//! - [`inhibit`]: keyboard shortcut inhibition for the portal window.
//!
//! Tested only on niri 26.04 (`docs/spikes.md`). Do not claim other compositors.
#![cfg(target_os = "linux")]

pub mod displays;
pub mod inhibit;
pub mod inject;
pub mod prompt;
mod text;
mod wl;

use telescreen_core::platform::{Capability, InputCapabilities};

/// IN-1, IN-12: report each capability of this session.
pub fn capabilities() -> InputCapabilities {
    let globals = wl::Globals::connect().ok();
    let has = |name: &str| globals.as_ref().is_some_and(|g| g.has(name));
    let window = |ok: bool, what: &str| {
        if ok {
            Capability::Available
        } else {
            Capability::Missing(format!("the compositor has no {what}"))
        }
    };
    let uinput = std::fs::OpenOptions::new()
        .write(true)
        .open("/dev/uinput")
        .is_ok();
    let inject = match (has("zwlr_virtual_pointer_manager_v1"), uinput) {
        (true, true) => Capability::Available,
        (false, _) => {
            Capability::Missing("the compositor has no zwlr_virtual_pointer_manager_v1".into())
        }
        (_, false) => Capability::Missing(
            "cannot open /dev/uinput for writing (add the user to the group of /dev/uinput)".into(),
        ),
    };
    InputCapabilities {
        capture: window(
            has("zwp_pointer_constraints_v1") && has("zwp_relative_pointer_manager_v1"),
            "pointer constraints or relative pointer protocol",
        ),
        suppress: window(
            has("zwp_keyboard_shortcuts_inhibit_manager_v1"),
            "keyboard shortcuts inhibit protocol",
        ),
        inject,
    }
}
