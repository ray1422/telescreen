//! Input injection on the target (ADR 0006, spike 3).
//!
//! The pointer uses the wlroots virtual pointer with absolute motion on the
//! input-target output, so the pointer cannot leave that output (IN-11). Keys use a
//! uinput keyboard, so the keymap of the compositor applies.

use std::time::Instant;

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, EventType, KeyCode};
use telescreen_core::input::{Button, InputEvent};
use telescreen_core::keys::Key;
use telescreen_core::platform::{DisplayInfo, Injector, PlatformError};
use telescreen_core::pointer::Normalized;
use wayland_client::Proxy;
use wayland_client::protocol::wl_pointer;
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
    zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

use crate::wl::{Globals, ignore_events};

ignore_events!(ZwlrVirtualPointerManagerV1, ZwlrVirtualPointerV1);

/// The extent of absolute motion. A fraction of the output, not pixels (spike 3).
const EXTENT: u32 = 10_000;

pub struct LinuxInjector {
    globals: Globals,
    manager: ZwlrVirtualPointerManagerV1,
    pointer: Option<ZwlrVirtualPointerV1>,
    keyboard: VirtualDevice,
    start: Instant,
}

impl std::fmt::Debug for LinuxInjector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinuxInjector")
            .field("has_pointer", &self.pointer.is_some())
            .finish_non_exhaustive()
    }
}

fn failed(e: impl std::fmt::Display) -> PlatformError {
    PlatformError::Failed(e.to_string())
}

impl LinuxInjector {
    pub fn new() -> Result<Self, PlatformError> {
        let globals = Globals::connect().map_err(PlatformError::Failed)?;
        let manager: ZwlrVirtualPointerManagerV1 = globals
            .list
            .bind(&globals.handle(), 1..=2, ())
            .map_err(|_| PlatformError::Unsupported("no zwlr_virtual_pointer_manager_v1".into()))?;
        // All key codes, so that each forwarded key exists on the device.
        let keys: AttributeSet<KeyCode> = (1..=0x2ff).map(KeyCode::new).collect();
        let keyboard = VirtualDevice::builder()
            .and_then(|b| b.name("Telescreen keyboard").with_keys(&keys))
            .and_then(|b| b.build())
            .map_err(|e| PlatformError::Permission(format!("/dev/uinput: {e}")))?;
        Ok(Self {
            globals,
            manager,
            pointer: None,
            keyboard,
            start: Instant::now(),
        })
    }

    fn time(&self) -> u32 {
        // Milliseconds with wrap-around, as Wayland uses.
        (self.start.elapsed().as_millis() & u128::from(u32::MAX)) as u32
    }

    fn flush(&mut self) -> Result<(), PlatformError> {
        self.globals
            .queue
            .dispatch_pending(&mut self.globals.state)
            .map_err(failed)?;
        self.globals.connection.flush().map_err(failed)
    }

    fn key(&mut self, key: Key, down: bool) -> Result<(), PlatformError> {
        let event = evdev::InputEvent::new(EventType::KEY.0, key.0, i32::from(down));
        self.keyboard.emit(&[event]).map_err(failed)
    }

    fn button(&mut self, button: Button, down: bool) -> Result<(), PlatformError> {
        let Some(pointer) = &self.pointer else {
            return Ok(());
        };
        let code = match button {
            Button::Left => 0x110,
            Button::Right => 0x111,
            Button::Middle => 0x112,
            Button::Back => 0x113,
            Button::Forward => 0x114,
        };
        let state = if down {
            wl_pointer::ButtonState::Pressed
        } else {
            wl_pointer::ButtonState::Released
        };
        pointer.button(self.time(), code, state);
        pointer.frame();
        self.flush()
    }

    fn scroll(&mut self, dx: f32, dy: f32) -> Result<(), PlatformError> {
        let time = self.time();
        let Some(pointer) = &self.pointer else {
            return Ok(());
        };
        pointer.axis_source(wl_pointer::AxisSource::Finger);
        for (axis, value) in [
            (wl_pointer::Axis::VerticalScroll, dy),
            (wl_pointer::Axis::HorizontalScroll, dx),
        ] {
            if value != 0.0 {
                pointer.axis(time, axis, f64::from(value));
            }
        }
        pointer.frame();
        self.flush()
    }
}

impl Injector for LinuxInjector {
    fn set_display(&mut self, display: &DisplayInfo) -> Result<(), PlatformError> {
        self.globals.roundtrip().map_err(PlatformError::Failed)?;
        let output = self
            .globals
            .state
            .by_name(&display.token.0)
            .cloned()
            .ok_or_else(|| PlatformError::Failed(format!("no output {}", display.token)))?;
        if let Some(old) = self.pointer.take() {
            old.destroy();
        }
        let seat = self.globals.seat();
        let qh = self.globals.handle();
        self.pointer = Some(if self.manager.version() >= 2 {
            self.manager
                .create_virtual_pointer_with_output(seat.as_ref(), Some(&output), &qh, ())
        } else {
            return Err(PlatformError::Unsupported(
                "the virtual pointer manager is version 1: no motion on one output".into(),
            ));
        });
        self.flush()
    }

    fn pointer(&mut self, at: Normalized) -> Result<(), PlatformError> {
        let time = self.time();
        let Some(pointer) = &self.pointer else {
            return Err(PlatformError::Failed("no input-target output".into()));
        };
        let x = (f64::from(at.x.clamp(0.0, 1.0)) * f64::from(EXTENT)) as u32;
        let y = (f64::from(at.y.clamp(0.0, 1.0)) * f64::from(EXTENT)) as u32;
        // The extent is exclusive: keep the last position in the output.
        pointer.motion_absolute(time, x.min(EXTENT - 1), y.min(EXTENT - 1), EXTENT, EXTENT);
        pointer.frame();
        self.flush()
    }

    fn input(&mut self, event: &InputEvent) -> Result<(), PlatformError> {
        match *event {
            InputEvent::Key { key, down } => self.key(key, down),
            InputEvent::Button { button, down } => self.button(button, down),
            InputEvent::Scroll { dx, dy } => self.scroll(dx, dy),
        }
    }
}

impl Drop for LinuxInjector {
    fn drop(&mut self) {
        if let Some(pointer) = self.pointer.take() {
            pointer.destroy();
            let _ = self.globals.connection.flush();
        }
    }
}
