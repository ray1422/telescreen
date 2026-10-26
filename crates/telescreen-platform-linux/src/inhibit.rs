//! Keyboard shortcut inhibition for the portal window (ADR 0006, UI-6).
//!
//! winit gives pointer lock and relative motion on Wayland, but not
//! `zwp_keyboard_shortcuts_inhibit_manager_v1`. This module opens the protocol on
//! the Wayland connection of winit (a foreign display) for the surface of the
//! portal window. The compositor removes the inhibitor with the window, so a crash
//! cannot leave the shortcuts inhibited. Not tested on hardware yet.

use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use telescreen_core::platform::PlatformError;
use wayland_backend::client::{Backend, ObjectId};
use wayland_client::globals::registry_queue_init;
use wayland_client::protocol::wl_surface::WlSurface;
use wayland_client::{Connection, EventQueue, Proxy};
use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::{
    zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1,
    zwp_keyboard_shortcuts_inhibitor_v1::ZwpKeyboardShortcutsInhibitorV1,
};

use crate::wl::{State, ignore_events};

ignore_events!(
    ZwpKeyboardShortcutsInhibitManagerV1,
    ZwpKeyboardShortcutsInhibitorV1
);

/// An active request to inhibit compositor shortcuts while the portal has the
/// keyboard focus. Drop it to give the shortcuts back.
pub struct ShortcutInhibitor {
    connection: Connection,
    _queue: EventQueue<State>,
    inhibitor: ZwpKeyboardShortcutsInhibitorV1,
}

impl std::fmt::Debug for ShortcutInhibitor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShortcutInhibitor").finish_non_exhaustive()
    }
}

impl ShortcutInhibitor {
    /// The window must be a winit window on Wayland and must live longer than the
    /// inhibitor.
    pub fn new(window: &(impl HasWindowHandle + HasDisplayHandle)) -> Result<Self, PlatformError> {
        let unsupported = |what: &str| PlatformError::Unsupported(what.into());
        let display = window
            .display_handle()
            .map_err(|e| PlatformError::Failed(e.to_string()))?;
        let RawDisplayHandle::Wayland(display) = display.as_raw() else {
            return Err(unsupported("the window is not a Wayland window"));
        };
        let handle = window
            .window_handle()
            .map_err(|e| PlatformError::Failed(e.to_string()))?;
        let RawWindowHandle::Wayland(handle) = handle.as_raw() else {
            return Err(unsupported("the window is not a Wayland window"));
        };
        // SAFETY: winit owns the wl_display and keeps it open while the window lives.
        let backend = unsafe { Backend::from_foreign_display(display.display.as_ptr().cast()) };
        let connection = Connection::from_backend(backend);
        // SAFETY: the surface pointer comes from winit and is valid while the window lives.
        let id =
            unsafe { ObjectId::from_ptr(WlSurface::interface(), handle.surface.as_ptr().cast()) }
                .map_err(|e| PlatformError::Failed(format!("wl_surface: {e}")))?;
        let surface = WlSurface::from_id(&connection, id)
            .map_err(|e| PlatformError::Failed(e.to_string()))?;
        let (globals, mut queue) = registry_queue_init::<State>(&connection)
            .map_err(|e| PlatformError::Failed(e.to_string()))?;
        let qh = queue.handle();
        let manager: ZwpKeyboardShortcutsInhibitManagerV1 =
            globals.bind(&qh, 1..=1, ()).map_err(|_| {
                unsupported("the compositor has no keyboard shortcuts inhibit protocol")
            })?;
        let seat: wayland_client::protocol::wl_seat::WlSeat = globals
            .bind(&qh, 1..=7, ())
            .map_err(|_| unsupported("no wl_seat"))?;
        let inhibitor = manager.inhibit_shortcuts(&surface, &seat, &qh, ());
        queue
            .roundtrip(&mut State::default())
            .map_err(|e| PlatformError::Failed(e.to_string()))?;
        Ok(Self {
            connection,
            _queue: queue,
            inhibitor,
        })
    }
}

impl Drop for ShortcutInhibitor {
    fn drop(&mut self) {
        self.inhibitor.destroy();
        let _ = self.connection.flush();
    }
}
