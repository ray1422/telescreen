//! AppKit adjustments of winit windows. The app makes the windows with winit; these
//! functions change only what winit does not offer (spike 1, ADR 0003, UI-1, UI-2).
//! Call them on the main thread.

use objc2::MainThreadMarker;
use objc2::rc::Retained;
use objc2_app_kit::{
    NSAllRomanInputSourcesLocaleIdentifier, NSApplication, NSScreen, NSView, NSWindow,
    NSWindowCollectionBehavior,
};
use objc2_foundation::{NSArray, NSNumber, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

/// UI-1: above the menu bar (`NSMainMenuWindowLevel` is 24). Spike 1 measured 26.
const PORTAL_LEVEL: isize = 26;
/// The prompt shows above fullscreen apps and the portal (`kCGScreenSaverWindowLevel`).
const PROMPT_LEVEL: isize = 1000;

fn ns_view(window: &impl HasWindowHandle) -> Option<Retained<NSView>> {
    let handle = window.window_handle().ok()?;
    let RawWindowHandle::AppKit(handle) = handle.as_raw() else {
        return None;
    };
    // SAFETY: winit gives a valid NSView pointer for the life of the window. The
    // retain keeps it valid for the caller.
    unsafe { Retained::retain(handle.ns_view.as_ptr().cast::<NSView>()) }
}

fn ns_window(window: &impl HasWindowHandle) -> Option<Retained<NSWindow>> {
    ns_view(window)?.window()
}

/// UI-1: after the portal entered fullscreen and has the focus, put it above the
/// menu bar, so that the menu bar and the title bar do not show at the top edge.
pub fn raise_portal(window: &impl HasWindowHandle) {
    if let Some(window) = ns_window(window) {
        window.setLevel(PORTAL_LEVEL);
    }
}

/// UI-2: while the portal has the focus, permit only Roman input sources. macOS
/// restores the previous input source when the focus goes away.
pub fn allow_only_roman_input(window: &impl HasWindowHandle) {
    let Some(context) = ns_view(window).and_then(|v| v.inputContext()) else {
        return;
    };
    // SAFETY: an AppKit constant string.
    let roman = unsafe { NSAllRomanInputSourcesLocaleIdentifier };
    context.setAllowedInputSourceLocales(Some(&NSArray::from_slice(&[roman])));
}

/// UI-5: the prompt is an overlay that shows on all Spaces, also over a fullscreen
/// app, and does not take mouse events.
pub fn make_overlay(window: &impl HasWindowHandle) {
    let Some(window) = ns_window(window) else {
        return;
    };
    window.setLevel(PROMPT_LEVEL);
    window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    window.setIgnoresMouseEvents(true);
}

/// ADR 0003: while forwarding, make Telescreen the active app again after a Space
/// change that the event tap could not stop.
pub fn activate_app() {
    if let Some(mtm) = MainThreadMarker::new() {
        #[allow(deprecated)] // `activate` needs macOS 14; this works on all versions.
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    }
}

/// The localized name of a display (for example "LG HDR WFHD"), for the UI and
/// the driver metadata. `None` if this is not the main thread.
pub fn screen_name(display: u32) -> Option<String> {
    let mtm = MainThreadMarker::new()?;
    let key = NSString::from_str("NSScreenNumber");
    NSScreen::screens(mtm).iter().find_map(|screen| {
        let number = screen.deviceDescription().objectForKey(&key)?;
        let number = number.downcast::<NSNumber>().ok()?;
        (number.unsignedIntValue() == display).then(|| screen.localizedName().to_string())
    })
}
