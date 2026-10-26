//! The C functions of CoreGraphics, ApplicationServices, and Carbon that this crate
//! uses. They are declared here, not taken from a binding crate, so that each
//! signature is visible and matches the spikes (`docs/spikes.md`).
#![allow(non_snake_case, non_upper_case_globals)] // C names.

use std::ffi::c_void;

pub type CGEventRef = *mut c_void;
pub type CGEventSourceRef = *mut c_void;
pub type CFMachPortRef = *mut c_void;
pub type CGDisplayModeRef = *mut c_void;
pub type CGDirectDisplayID = u32;
pub type TapCallback = extern "C" fn(*mut c_void, u32, CGEventRef, *mut c_void) -> CGEventRef;

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CGPoint {
    pub x: f64,
    pub y: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CGSize {
    pub width: f64,
    pub height: f64,
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Default)]
pub struct CGRect {
    pub origin: CGPoint,
    pub size: CGSize,
}

// Event types (CGEventType).
pub const LEFT_DOWN: u32 = 1;
pub const LEFT_UP: u32 = 2;
pub const RIGHT_DOWN: u32 = 3;
pub const RIGHT_UP: u32 = 4;
pub const MOUSE_MOVED: u32 = 5;
pub const LEFT_DRAGGED: u32 = 6;
pub const RIGHT_DRAGGED: u32 = 7;
pub const KEY_DOWN: u32 = 10;
pub const KEY_UP: u32 = 11;
pub const FLAGS_CHANGED: u32 = 12;
pub const SCROLL_WHEEL: u32 = 22;
pub const OTHER_DOWN: u32 = 25;
pub const OTHER_UP: u32 = 26;
pub const OTHER_DRAGGED: u32 = 27;
pub const TAP_DISABLED_BY_TIMEOUT: u32 = 0xFFFF_FFFE;
pub const TAP_DISABLED_BY_USER: u32 = 0xFFFF_FFFF;

// Event fields (CGEventField).
pub const FIELD_CLICK_STATE: u32 = 1;
pub const FIELD_BUTTON_NUMBER: u32 = 3;
pub const FIELD_DELTA_X: u32 = 4;
pub const FIELD_DELTA_Y: u32 = 5;
pub const FIELD_KEYCODE: u32 = 9;
pub const FIELD_SCROLL_POINT_DELTA_1: u32 = 96;
pub const FIELD_SCROLL_POINT_DELTA_2: u32 = 97;

// Event flags (CGEventFlags) and the device-dependent bits of IOLLEvent.h.
pub const FLAG_SHIFT: u64 = 0x0002_0000;
pub const FLAG_CONTROL: u64 = 0x0004_0000;
pub const FLAG_ALTERNATE: u64 = 0x0008_0000;
pub const FLAG_COMMAND: u64 = 0x0010_0000;
pub const FLAG_SECONDARY_FN: u64 = 0x0080_0000;
pub const DEVICE_LCTL: u64 = 0x0000_0001;
pub const DEVICE_LSHIFT: u64 = 0x0000_0002;
pub const DEVICE_RSHIFT: u64 = 0x0000_0004;
pub const DEVICE_LCMD: u64 = 0x0000_0008;
pub const DEVICE_RCMD: u64 = 0x0000_0010;
pub const DEVICE_LALT: u64 = 0x0000_0020;
pub const DEVICE_RALT: u64 = 0x0000_0040;
pub const DEVICE_RCTL: u64 = 0x0000_2000;

pub const HID_EVENT_TAP: u32 = 0;
pub const HEAD_INSERT: u32 = 0;
pub const TAP_DEFAULT: u32 = 0;
pub const EVENT_SOURCE_HID_STATE: i32 = 1;
pub const SCROLL_UNIT_PIXEL: u32 = 0;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    pub fn CGEventTapCreate(
        tap: u32,
        place: u32,
        options: u32,
        mask: u64,
        callback: TapCallback,
        info: *mut c_void,
    ) -> CFMachPortRef;
    pub fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    pub fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
    pub fn CGEventSetIntegerValueField(event: CGEventRef, field: u32, value: i64);
    pub fn CGEventGetFlags(event: CGEventRef) -> u64;
    pub fn CGEventSetFlags(event: CGEventRef, flags: u64);
    pub fn CGEventGetLocation(event: CGEventRef) -> CGPoint;
    pub fn CGEventCreate(source: CGEventSourceRef) -> CGEventRef;
    pub fn CGEventSourceCreate(state: i32) -> CGEventSourceRef;
    pub fn CGEventCreateKeyboardEvent(
        source: CGEventSourceRef,
        keycode: u16,
        down: bool,
    ) -> CGEventRef;
    pub fn CGEventCreateMouseEvent(
        source: CGEventSourceRef,
        kind: u32,
        location: CGPoint,
        button: u32,
    ) -> CGEventRef;
    pub fn CGEventCreateScrollWheelEvent2(
        source: CGEventSourceRef,
        units: u32,
        wheel_count: u32,
        wheel1: i32,
        wheel2: i32,
        wheel3: i32,
    ) -> CGEventRef;
    pub fn CGEventPost(tap: u32, event: CGEventRef);
    pub fn CGAssociateMouseAndMouseCursorPosition(connected: bool) -> i32;
    pub fn CGWarpMouseCursorPosition(point: CGPoint) -> i32;
    pub fn CGMainDisplayID() -> CGDirectDisplayID;
    pub fn CGDisplayHideCursor(display: CGDirectDisplayID) -> i32;
    pub fn CGDisplayShowCursor(display: CGDirectDisplayID) -> i32;
    pub fn CGGetActiveDisplayList(
        max: u32,
        displays: *mut CGDirectDisplayID,
        count: *mut u32,
    ) -> i32;
    pub fn CGDisplayBounds(display: CGDirectDisplayID) -> CGRect;
    pub fn CGDisplayCopyDisplayMode(display: CGDirectDisplayID) -> CGDisplayModeRef;
    pub fn CGDisplayModeGetPixelWidth(mode: CGDisplayModeRef) -> usize;
    pub fn CGDisplayModeGetWidth(mode: CGDisplayModeRef) -> usize;
    pub fn CGDisplayModeRelease(mode: CGDisplayModeRef);
    pub fn CGDisplayVendorNumber(display: CGDirectDisplayID) -> u32;
    pub fn CGDisplayModelNumber(display: CGDirectDisplayID) -> u32;
    pub fn CGDisplaySerialNumber(display: CGDirectDisplayID) -> u32;
    pub fn CGDisplayIsBuiltin(display: CGDirectDisplayID) -> u32;
    pub fn CGDisplayRotation(display: CGDirectDisplayID) -> f64;
    pub fn CGPreflightListenEventAccess() -> bool;
    pub fn CGRequestListenEventAccess() -> bool;
    pub fn CGPreflightPostEventAccess() -> bool;
    pub fn CGRequestPostEventAccess() -> bool;
}

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    pub fn AXIsProcessTrusted() -> bool;
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    pub fn IsSecureEventInputEnabled() -> bool;
}

unsafe extern "C" {
    pub fn CFMachPortCreateRunLoopSource(
        allocator: *const c_void,
        port: CFMachPortRef,
        order: isize,
    ) -> *mut c_void;
}

/// Releases a CoreFoundation object. Null is permitted.
pub fn release(object: *mut c_void) {
    if !object.is_null() {
        // SAFETY: the caller owns one reference to the object.
        unsafe { core_foundation_sys::base::CFRelease(object as _) };
    }
}

/// The current cursor position in global display coordinates.
pub fn cursor_position() -> CGPoint {
    // SAFETY: CGEventCreate(NULL) makes an event at the cursor; it is released below.
    unsafe {
        let event = CGEventCreate(std::ptr::null_mut());
        if event.is_null() {
            return CGPoint::default();
        }
        let point = CGEventGetLocation(event);
        release(event);
        point
    }
}
