//! Input injection on the target with `CGEventPost` (ADR 0006). Needs Accessibility.

use std::time::{Duration, Instant};

use telescreen_core::input::{Button, InputEvent, whole_units};
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
    /// Caps Lock is on. A posted key event does not change the lock state of the
    /// system, so the injector keeps it and sets it on each event.
    caps_lock: bool,
    last_click: Option<(Button, Instant, Point, i64)>,
    /// The parts of a pixel that the last scrolls did not inject (IN-14).
    scroll_rest: (f32, f32),
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
            // SAFETY: reads the current flags of the hardware state.
            caps_lock: unsafe { ffi::CGEventSourceFlagsState(ffi::EVENT_SOURCE_HID_STATE) }
                & ffi::FLAG_ALPHA_SHIFT
                != 0,
            last_click: None,
            scroll_rest: (0.0, 0.0),
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
        if key == Key::CAPS_LOCK {
            // macOS reports Caps Lock as one flags-changed event that toggles
            // the lock. The release has no event.
            if !down {
                return Ok(());
            }
            self.caps_lock = !self.caps_lock;
        } else if let Some(flag) = modifier_flag(key) {
            if down {
                self.flags |= flag;
            } else {
                self.flags &= !flag;
            }
        }
        // SAFETY: the source is valid; the event is released by post.
        let event = unsafe { ffi::CGEventCreateKeyboardEvent(self.source, code, down) };
        if !event.is_null() {
            let flags = self.event_flags() | key_flags(code);
            // SAFETY: the event is valid.
            unsafe { ffi::CGEventSetFlags(event, flags) };
        }
        self.post(event)
    }

    /// The flags of the held modifier keys and of Caps Lock.
    fn event_flags(&self) -> u64 {
        if self.caps_lock {
            self.flags | ffi::FLAG_ALPHA_SHIFT
        } else {
            self.flags
        }
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
                ffi::CGEventSetFlags(event, self.event_flags());
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

    fn scroll(&mut self, dx: f32, dy: f32) -> Result<(), PlatformError> {
        // macOS: a positive wheel value scrolls up or left. The wire is the opposite.
        let wheel1 = -whole_units(dy, &mut self.scroll_rest.1);
        let wheel2 = -whole_units(dx, &mut self.scroll_rest.0);
        if wheel1 == 0 && wheel2 == 0 {
            return Ok(());
        }
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

/// The flags that a hardware keyboard sets on a key, by macOS key code. Input
/// methods use them, for example to select a candidate with an arrow key.
fn key_flags(code: u16) -> u64 {
    match code {
        // The arrow keys.
        0x7B..=0x7E => ffi::FLAG_SECONDARY_FN | ffi::FLAG_NUMERIC_PAD,
        // Help, Home, Page Up, Forward Delete, End, Page Down, and F1 to F20.
        0x72..=0x7A
        | 0x60..=0x65
        | 0x67
        | 0x69
        | 0x6A
        | 0x6B
        | 0x6D
        | 0x6F
        | 0x71
        | 0x40
        | 0x4F
        | 0x50
        | 0x5A => ffi::FLAG_SECONDARY_FN,
        // The keypad.
        0x41 | 0x43 | 0x45 | 0x47 | 0x4B | 0x4C | 0x4E | 0x51..=0x59 | 0x5B | 0x5C => {
            ffi::FLAG_NUMERIC_PAD
        }
        _ => 0,
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
            unsafe { ffi::CGEventSetFlags(event, self.event_flags()) };
        }
        self.post(event)
    }

    fn input(&mut self, event: &InputEvent) -> Result<(), PlatformError> {
        match *event {
            InputEvent::Key { key, down } => self.key(key, down),
            InputEvent::Button { button, down } => self.button(button, down),
            InputEvent::Scroll { dx, dy, .. } => self.scroll(dx, dy),
        }
    }
}

impl Drop for MacInjector {
    fn drop(&mut self) {
        ffi::release(self.source);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hardware_flags_of_keys() {
        let both = ffi::FLAG_SECONDARY_FN | ffi::FLAG_NUMERIC_PAD;
        for arrow in 0x7B..=0x7E {
            assert_eq!(key_flags(arrow), both);
        }
        assert_eq!(key_flags(0x73), ffi::FLAG_SECONDARY_FN, "Home");
        assert_eq!(key_flags(0x7A), ffi::FLAG_SECONDARY_FN, "F1");
        assert_eq!(key_flags(0x6F), ffi::FLAG_SECONDARY_FN, "F12");
        assert_eq!(key_flags(0x52), ffi::FLAG_NUMERIC_PAD, "keypad 0");
        assert_eq!(key_flags(0x4C), ffi::FLAG_NUMERIC_PAD, "keypad Enter");
        assert_eq!(key_flags(0x00), 0, "A");
        assert_eq!(key_flags(0x24), 0, "Return");
        assert_eq!(key_flags(0x39), 0, "Caps Lock");
    }
}
