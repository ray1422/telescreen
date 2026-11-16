//! The keyboard-only uinput device of the login stage (STG-2).

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, EventType, KeyCode};
use telescreen_core::keys::Key;
use telescreen_core::platform::PlatformError;

pub struct UinputKeyboard {
    device: VirtualDevice,
}

impl std::fmt::Debug for UinputKeyboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("UinputKeyboard").finish_non_exhaustive()
    }
}

impl UinputKeyboard {
    pub fn new() -> Result<Self, PlatformError> {
        // Button codes begin at 0x100; this device supports only keyboard keys.
        let keys: AttributeSet<KeyCode> = (1..=0xff).map(KeyCode::new).collect();
        let device = VirtualDevice::builder()
            .and_then(|b| b.name("Telescreen login keyboard").with_keys(&keys))
            .and_then(|b| b.build())
            .map_err(|e| PlatformError::Permission(format!("/dev/uinput: {e}")))?;
        Ok(Self { device })
    }

    pub fn key(&mut self, key: Key, down: bool) -> Result<(), PlatformError> {
        if key.0 > 0xff {
            return Ok(());
        }
        let event = evdev::InputEvent::new(EventType::KEY.0, key.0, i32::from(down));
        self.device
            .emit(&[event])
            .map_err(|e| PlatformError::Failed(e.to_string()))
    }
}
