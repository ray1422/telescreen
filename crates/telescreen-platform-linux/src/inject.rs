//! Input injection on the target (ADR 0006, spike 3, IN-15).
//!
//! Keys use a uinput keyboard, so the keymap of the compositor applies.
//!
//! The pointer has two backends:
//!
//! - The wlroots virtual pointer, if the compositor has it (niri). Absolute motion
//!   is on the input-target output, so the pointer cannot leave that output (IN-11).
//! - Else a uinput absolute pointer (KWin). The compositor maps its range to all
//!   outputs together, so this backend computes the position in that area.

use std::time::Instant;

use evdev::uinput::VirtualDevice;
use evdev::{
    AbsInfo, AbsoluteAxisCode, AttributeSet, EventType, KeyCode, RelativeAxisCode, UinputAbsSetup,
};
use telescreen_core::input::{Button, InputEvent, whole_units};
use telescreen_core::keys::Key;
use telescreen_core::platform::{DisplayInfo, Injector, PlatformError};
use telescreen_core::pointer::{Normalized, Rect};
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

/// The largest value of the axes of the uinput absolute pointer.
const AXIS_MAX: i32 = 65_535;

/// High-resolution wheel units for one logical pixel of a scroll. One wheel notch
/// has 120 units and is 60 pixels. The user changes the speed with `scroll-speed`.
const WHEEL_UNITS_PER_PIXEL: f32 = 2.0;

enum PointerBackend {
    Virtual {
        manager: ZwlrVirtualPointerManagerV1,
        pointer: Option<ZwlrVirtualPointerV1>,
    },
    Uinput {
        device: VirtualDevice,
        /// The name of the input-target output.
        output: Option<String>,
        /// The wheel units that the last scrolls did not inject (IN-14).
        rest: (f32, f32),
    },
}

pub struct LinuxInjector {
    globals: Globals,
    backend: PointerBackend,
    keyboard: VirtualDevice,
    start: Instant,
}

impl std::fmt::Debug for LinuxInjector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinuxInjector")
            .field(
                "virtual_pointer",
                &matches!(self.backend, PointerBackend::Virtual { .. }),
            )
            .finish_non_exhaustive()
    }
}

fn failed(e: impl std::fmt::Display) -> PlatformError {
    PlatformError::Failed(e.to_string())
}

impl LinuxInjector {
    pub fn new() -> Result<Self, PlatformError> {
        let globals = Globals::connect().map_err(PlatformError::Failed)?;
        let backend = match globals.list.bind(&globals.handle(), 1..=2, ()) {
            Ok(manager) => PointerBackend::Virtual {
                manager,
                pointer: None,
            },
            Err(_) => PointerBackend::Uinput {
                device: absolute_pointer()
                    .map_err(|e| PlatformError::Permission(format!("/dev/uinput: {e}")))?,
                output: None,
                rest: (0.0, 0.0),
            },
        };
        // All key codes, so that each forwarded key exists on the device.
        let keys: AttributeSet<KeyCode> = (1..=0x2ff).map(KeyCode::new).collect();
        let keyboard = VirtualDevice::builder()
            .and_then(|b| b.name("Telescreen keyboard").with_keys(&keys))
            .and_then(|b| b.build())
            .map_err(|e| PlatformError::Permission(format!("/dev/uinput: {e}")))?;
        Ok(Self {
            globals,
            backend,
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
        let code = match button {
            Button::Left => 0x110,
            Button::Right => 0x111,
            Button::Middle => 0x112,
            Button::Back => 0x113,
            Button::Forward => 0x114,
        };
        let time = self.time();
        match &mut self.backend {
            PointerBackend::Virtual { pointer, .. } => {
                let Some(pointer) = pointer else {
                    return Ok(());
                };
                let state = if down {
                    wl_pointer::ButtonState::Pressed
                } else {
                    wl_pointer::ButtonState::Released
                };
                pointer.button(time, code.into(), state);
                pointer.frame();
                self.flush()
            }
            PointerBackend::Uinput { device, .. } => device
                .emit(&[evdev::InputEvent::new(
                    EventType::KEY.0,
                    code,
                    i32::from(down),
                )])
                .map_err(failed),
        }
    }

    fn scroll(&mut self, dx: f32, dy: f32) -> Result<(), PlatformError> {
        let time = self.time();
        match &mut self.backend {
            PointerBackend::Virtual { pointer, .. } => {
                let Some(pointer) = pointer else {
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
            PointerBackend::Uinput { device, rest, .. } => {
                // evdev: a positive vertical wheel value scrolls up. The wire is the
                // opposite. A positive horizontal value scrolls right, as on the wire.
                let vertical = whole_units(-dy * WHEEL_UNITS_PER_PIXEL, &mut rest.1);
                let horizontal = whole_units(dx * WHEEL_UNITS_PER_PIXEL, &mut rest.0);
                let mut events = Vec::with_capacity(2);
                for (axis, value) in [
                    (RelativeAxisCode::REL_WHEEL_HI_RES, vertical),
                    (RelativeAxisCode::REL_HWHEEL_HI_RES, horizontal),
                ] {
                    if value != 0 {
                        events.push(evdev::InputEvent::new(EventType::RELATIVE.0, axis.0, value));
                    }
                }
                if events.is_empty() {
                    return Ok(());
                }
                device.emit(&events).map_err(failed)
            }
        }
    }
}

/// A pointer with absolute axes and a wheel. libinput sees it as a mouse with
/// absolute motion, as the tablet device of a virtual machine.
fn absolute_pointer() -> std::io::Result<VirtualDevice> {
    let buttons: AttributeSet<KeyCode> = (0x110..=0x114).map(KeyCode::new).collect();
    let wheels: AttributeSet<RelativeAxisCode> = [
        RelativeAxisCode::REL_WHEEL,
        RelativeAxisCode::REL_HWHEEL,
        RelativeAxisCode::REL_WHEEL_HI_RES,
        RelativeAxisCode::REL_HWHEEL_HI_RES,
    ]
    .into_iter()
    .collect();
    let axis = AbsInfo::new(0, 0, AXIS_MAX, 0, 0, 0);
    VirtualDevice::builder()?
        .name("Telescreen pointer")
        .with_keys(&buttons)?
        .with_relative_axes(&wheels)?
        .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_X, axis))?
        .with_absolute_axis(&UinputAbsSetup::new(AbsoluteAxisCode::ABS_Y, axis))?
        .build()
}

/// The axis values for a position on an output, in the area of all outputs.
fn axis_values(at: Normalized, output: Rect, layout: Rect) -> (i32, i32) {
    let axis = |at: f32, start: f64, size: f64, layout_start: f64, layout_size: f64| {
        let position = start + f64::from(at.clamp(0.0, 1.0)) * size;
        (((position - layout_start) / layout_size) * f64::from(AXIS_MAX)) as i32
    };
    (
        axis(at.x, output.x, output.width, layout.x, layout.width),
        axis(at.y, output.y, output.height, layout.y, layout.height),
    )
}

impl Injector for LinuxInjector {
    fn set_display(&mut self, display: &DisplayInfo) -> Result<(), PlatformError> {
        let output = self
            .globals
            .output(&display.token.0)
            .map_err(PlatformError::Failed)?;
        match &mut self.backend {
            PointerBackend::Virtual { manager, pointer } => {
                if let Some(old) = pointer.take() {
                    old.destroy();
                }
                if manager.version() < 2 {
                    return Err(PlatformError::Unsupported(
                        "the virtual pointer manager is version 1: no motion on one output".into(),
                    ));
                }
                let seat = self.globals.seat();
                let qh = self.globals.handle();
                *pointer = Some(manager.create_virtual_pointer_with_output(
                    seat.as_ref(),
                    Some(&output),
                    &qh,
                    (),
                ));
            }
            PointerBackend::Uinput { output, .. } => *output = Some(display.token.0.clone()),
        }
        self.flush()
    }

    fn pointer(&mut self, at: Normalized) -> Result<(), PlatformError> {
        let time = self.time();
        // The outputs can change during a session (IN-15).
        if matches!(self.backend, PointerBackend::Uinput { .. }) {
            self.globals.poll().map_err(PlatformError::Failed)?;
        }
        match &mut self.backend {
            PointerBackend::Virtual { pointer, .. } => {
                let Some(pointer) = pointer else {
                    return Err(PlatformError::Failed("no input-target output".into()));
                };
                let x = (f64::from(at.x.clamp(0.0, 1.0)) * f64::from(EXTENT)) as u32;
                let y = (f64::from(at.y.clamp(0.0, 1.0)) * f64::from(EXTENT)) as u32;
                // The extent is exclusive: keep the last position in the output.
                pointer.motion_absolute(time, x.min(EXTENT - 1), y.min(EXTENT - 1), EXTENT, EXTENT);
                pointer.frame();
                self.flush()
            }
            PointerBackend::Uinput { device, output, .. } => {
                let bounds = output
                    .as_deref()
                    .and_then(|name| self.globals.state.bounds(name));
                let Some((output, layout)) = bounds else {
                    return Err(PlatformError::Failed("no input-target output".into()));
                };
                let (x, y) = axis_values(at, output, layout);
                device
                    .emit(&[
                        evdev::InputEvent::new(EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_X.0, x),
                        evdev::InputEvent::new(EventType::ABSOLUTE.0, AbsoluteAxisCode::ABS_Y.0, y),
                    ])
                    .map_err(failed)
            }
        }
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
        if let PointerBackend::Virtual { pointer, .. } = &mut self.backend
            && let Some(pointer) = pointer.take()
        {
            pointer.destroy();
            let _ = self.globals.connection.flush();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, width: f64, height: f64) -> Rect {
        Rect {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn in15_axis_values_are_in_the_area_of_all_outputs() {
        // One output at the right of another one.
        let layout = rect(0.0, 0.0, 4000.0, 1000.0);
        let output = rect(2000.0, 500.0, 2000.0, 500.0);
        let (x, y) = axis_values(Normalized { x: 0.5, y: 0.0 }, output, layout);
        assert_eq!((x, y), (AXIS_MAX * 3 / 4, AXIS_MAX / 2));
        let (x, y) = axis_values(Normalized { x: 2.0, y: 1.0 }, output, layout);
        assert_eq!(
            (x, y),
            (AXIS_MAX, AXIS_MAX),
            "a position stays in the output"
        );
    }

    #[test]
    fn in15_the_layout_has_all_outputs() {
        let left = rect(-100.0, 0.0, 100.0, 50.0);
        let right = rect(0.0, -20.0, 200.0, 100.0);
        let all = crate::wl::layout(&[left, right]).unwrap();
        assert_eq!(all, rect(-100.0, -20.0, 300.0, 100.0));
        assert_eq!(crate::wl::layout(&[]), None);
    }
}
