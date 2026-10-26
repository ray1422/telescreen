//! Input injection on the target with `CGEventPost` (ADR 0006). Needs Accessibility.

use std::time::{Duration, Instant};

use telescreen_core::input::{Button, InputEvent};
use telescreen_core::keys::{self, Key};
use telescreen_core::platform::{DisplayInfo, Injector, PlatformError};
use telescreen_core::pointer::{self, Normalized, Point, Rect};

use crate::ffi::{self, CGPoint};

/// The macOS default double-click interval.
const DOUBLE_CLICK: Duration = Duration::from_millis(500);
const DOUBLE_CLICK_DISTANCE: f64 = 4.0;

#[derive(Debug)]
pub struct MacInjector {
    source: ffi::CGEventSourceRef,
    display: Option<Rect>,
    position: Point,
    buttons: Vec<Button>,
    /// The modifier keys that are down, as event flags.
    flags: u64,
    last_click: Option<(Button, Instant, Point, i64)>,
}

impl MacInjector {
    pub fn new() -> Result<Self, PlatformError> {
        if !crate::capabilities()
            .inject
            .eq(&telescreen_core::platform::Capability::Available)
        {
            return Err(PlatformError::Permission(
                "grant Accessibility to Telescreen".into(),
            ));
        }
        // SAFETY: makes an event source that this injector owns and releases on drop.
        let source = unsafe { ffi::CGEventSourceCreate(ffi::EVENT_SOURCE_HID_STATE) };
        if source.is_null() {
            return Err(PlatformError::Failed("CGEventSourceCreate failed".into()));
        }
        let at = ffi::cursor_position();
        Ok(Self {
            source,
            display: None,
            position: Point { x: at.x, y: at.y },
            buttons: Vec::new(),
            flags: 0,
            last_click: None,
        })
    }

    fn post(&self, event: ffi::CGEventRef) -> Result<(), PlatformError> {
        if event.is_null() {
            return Err(PlatformError::Failed("cannot make the event".into()));
        }
        // SAFETY: the event is valid; it is released after the post.
        unsafe { ffi::CGEventPost(ffi::HID_EVENT_TAP, event) };
        ffi::release(event);
        Ok(())
    }

    fn mouse(&self, kind: u32, button: u32) -> ffi::CGEventRef {
        let at = CGPoint {
            x: self.position.x,
            y: self.position.y,
        };
        // SAFETY: the source is valid for the life of self.
        unsafe { ffi::CGEventCreateMouseEvent(self.source, kind, at, button) }
    }

    fn key(&mut self, key: Key, down: bool) -> Result<(), PlatformError> {
        let Some(code) = keys::to_mac(key) else {
            return Ok(()); // A key that macOS does not have.
        };
        if let Some(flag) = modifier_flag(key) {
            if down {
                self.flags |= flag;
            } else {
                self.flags &= !flag;
            }
        }
        // SAFETY: the source is valid; the event is released by post.
        let event = unsafe { ffi::CGEventCreateKeyboardEvent(self.source, code, down) };
        if !event.is_null() {
            // SAFETY: the event is valid.
            unsafe { ffi::CGEventSetFlags(event, self.flags) };
        }
        self.post(event)
    }

    fn button(&mut self, button: Button, down: bool) -> Result<(), PlatformError> {
        let (kind, number) = match (button, down) {
            (Button::Left, true) => (ffi::LEFT_DOWN, 0),
            (Button::Left, false) => (ffi::LEFT_UP, 0),
            (Button::Right, true) => (ffi::RIGHT_DOWN, 1),
            (Button::Right, false) => (ffi::RIGHT_UP, 1),
            (Button::Middle, true) => (ffi::OTHER_DOWN, 2),
            (Button::Middle, false) => (ffi::OTHER_UP, 2),
            (Button::Back, true) => (ffi::OTHER_DOWN, 3),
            (Button::Back, false) => (ffi::OTHER_UP, 3),
            (Button::Forward, true) => (ffi::OTHER_DOWN, 4),
            (Button::Forward, false) => (ffi::OTHER_UP, 4),
        };
        if down {
            self.buttons.push(button);
        } else {
            self.buttons.retain(|b| *b != button);
        }
        let clicks = self.click_count(button, down);
        let event = self.mouse(kind, number);
        if !event.is_null() {
            // SAFETY: the event is valid.
            unsafe {
                ffi::CGEventSetIntegerValueField(event, ffi::FIELD_CLICK_STATE, clicks);
                ffi::CGEventSetIntegerValueField(
                    event,
                    ffi::FIELD_BUTTON_NUMBER,
                    i64::from(number),
                );
                ffi::CGEventSetFlags(event, self.flags);
            }
        }
        self.post(event)
    }

    /// macOS apps read double clicks from the click state field, not from timing.
    fn click_count(&mut self, button: Button, down: bool) -> i64 {
        if !down {
            return self.last_click.filter(|c| c.0 == button).map_or(1, |c| c.3);
        }
        let now = Instant::now();
        let at = self.position;
        let count = match self.last_click {
            Some((b, time, p, count))
                if b == button
                    && now.duration_since(time) < DOUBLE_CLICK
                    && (p.x - at.x).abs() < DOUBLE_CLICK_DISTANCE
                    && (p.y - at.y).abs() < DOUBLE_CLICK_DISTANCE =>
            {
                count + 1
            }
            _ => 1,
        };
        self.last_click = Some((button, now, at, count));
        count
    }

    fn scroll(&self, dx: f32, dy: f32) -> Result<(), PlatformError> {
        // macOS: a positive wheel value scrolls up or left. The wire is the opposite.
        let wheel1 = -(dy.round() as i32);
        let wheel2 = -(dx.round() as i32);
        // SAFETY: the source is valid; the event is released by post.
        let event = unsafe {
            ffi::CGEventCreateScrollWheelEvent2(
                self.source,
                ffi::SCROLL_UNIT_PIXEL,
                2,
                wheel1,
                wheel2,
                0,
            )
        };
        self.post(event)
    }
}

/// The event flag of a modifier key, with its device-dependent bit.
fn modifier_flag(key: Key) -> Option<u64> {
    Some(match key {
        Key::LEFT_SHIFT => ffi::FLAG_SHIFT | ffi::DEVICE_LSHIFT,
        Key::RIGHT_SHIFT => ffi::FLAG_SHIFT | ffi::DEVICE_RSHIFT,
        Key::LEFT_CTRL => ffi::FLAG_CONTROL | ffi::DEVICE_LCTL,
        Key::RIGHT_CTRL => ffi::FLAG_CONTROL | ffi::DEVICE_RCTL,
        Key::LEFT_ALT => ffi::FLAG_ALTERNATE | ffi::DEVICE_LALT,
        Key::RIGHT_ALT => ffi::FLAG_ALTERNATE | ffi::DEVICE_RALT,
        Key::LEFT_META => ffi::FLAG_COMMAND | ffi::DEVICE_LCMD,
        Key::RIGHT_META => ffi::FLAG_COMMAND | ffi::DEVICE_RCMD,
        _ => return None,
    })
}

impl Injector for MacInjector {
    fn set_display(&mut self, display: &DisplayInfo) -> Result<(), PlatformError> {
        let bounds = display
            .bounds
            .ok_or_else(|| PlatformError::Unsupported("the display has no coordinates".into()))?;
        self.display = Some(bounds);
        Ok(())
    }

    fn pointer(&mut self, at: Normalized) -> Result<(), PlatformError> {
        let display = self
            .display
            .ok_or_else(|| PlatformError::Failed("no display".into()))?;
        self.position = pointer::denormalize(display, at);
        // A move with a held button is a drag; apps need the drag type.
        let (kind, number) = match self.buttons.last() {
            Some(Button::Left) => (ffi::LEFT_DRAGGED, 0),
            Some(Button::Right) => (ffi::RIGHT_DRAGGED, 1),
            Some(_) => (ffi::OTHER_DRAGGED, 2),
            None => (ffi::MOUSE_MOVED, 0),
        };
        let event = self.mouse(kind, number);
        if !event.is_null() {
            // SAFETY: the event is valid.
            unsafe { ffi::CGEventSetFlags(event, self.flags) };
        }
        self.post(event)
    }

    fn input(&mut self, event: &InputEvent) -> Result<(), PlatformError> {
        match *event {
            InputEvent::Key { key, down } => self.key(key, down),
            InputEvent::Button { button, down } => self.button(button, down),
            InputEvent::Scroll { dx, dy } => self.scroll(dx, dy),
        }
    }
}

impl Drop for MacInjector {
    fn drop(&mut self) {
        ffi::release(self.source);
    }
}
