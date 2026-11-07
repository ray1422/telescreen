//! AppKit adjustments of winit windows. The app makes the windows with winit; these
//! functions change only what winit does not offer (spike 1, ADR 0003, UI-1, UI-2).
//! Call them on the main thread.

use std::cell::OnceCell;
use std::ffi::{CStr, c_void};
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};

use objc2::rc::Retained;
use objc2::runtime::NSObject;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAllRomanInputSourcesLocaleIdentifier, NSApplicationActivationOptions, NSRunningApplication,
    NSScreen, NSView, NSWindow, NSWindowCollectionBehavior, NSWorkspace,
    NSWorkspaceActiveSpaceDidChangeNotification,
};
use objc2_foundation::{NSArray, NSNotification, NSNumber, NSString};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::ffi;

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

/// VER-4: the prompt of a hand back takes the acknowledgement on this computer,
/// so it takes mouse events and the keyboard focus. The portal of the session
/// can be closed at this time (ACT-5), so no other window takes them. The
/// warning of FLOW-26 takes them too, for the safety exit (IN-8).
pub fn set_prompt_input(window: &impl HasWindowHandle, on: bool) {
    if let Some(window) = ns_window(window) {
        window.setIgnoresMouseEvents(!on);
    }
}

/// An app that was frontmost before the warning of FLOW-26 took the focus.
#[derive(Debug)]
pub struct FrontmostApp(Retained<NSRunningApplication>);

/// The frontmost app, if it is not Telescreen (FLOW-26).
pub fn frontmost_other_app() -> Option<FrontmostApp> {
    NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .filter(|app| app.processIdentifier() != std::process::id() as i32)
        .map(FrontmostApp)
}

/// Makes `app` the frontmost app again, if it still runs (FLOW-26).
pub fn activate(app: &FrontmostApp) {
    if app.0.isTerminated() {
        return;
    }
    if !app
        .0
        .activateWithOptions(NSApplicationActivationOptions::empty())
    {
        eprintln!("telescreen: cannot activate the app that was frontmost");
    }
}

/// A Space changed since the last [`take_space_change`] (FLOW-26).
static SPACE_CHANGED: AtomicBool = AtomicBool::new(false);

define_class!(
    // SAFETY: a main-thread object with no ivars.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "TelescreenSpaceObserver"]
    struct SpaceObserver;

    impl SpaceObserver {
        // SAFETY: a notification method with one notification argument.
        #[unsafe(method(activeSpaceDidChange:))]
        fn active_space_did_change(&self, _notification: &NSNotification) {
            eprintln!("telescreen: {}: the active Space changed", crate::clock());
            SPACE_CHANGED.store(true, SeqCst);
        }
    }
);

thread_local! {
    static SPACE_OBSERVER: OnceCell<Retained<SpaceObserver>> = const { OnceCell::new() };
}

/// FLOW-26: records each change of the active Space, from now on. Opening
/// Mission Control is not a Space change.
pub fn watch_space_changes() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    SPACE_OBSERVER.with(|cell| {
        cell.get_or_init(|| {
            // SAFETY: `init` of NSObject.
            let observer: Retained<SpaceObserver> =
                unsafe { msg_send![SpaceObserver::alloc(mtm), init] };
            let center = NSWorkspace::sharedWorkspace().notificationCenter();
            // SAFETY: the selector has the signature of a notification method,
            // and the thread-local cell keeps the observer alive.
            unsafe {
                center.addObserver_selector_name_object(
                    &observer,
                    sel!(activeSpaceDidChange:),
                    Some(NSWorkspaceActiveSpaceDidChangeNotification),
                    None,
                );
            }
            observer
        });
    });
}

/// True once after each change of the active Space.
pub fn take_space_change() -> bool {
    SPACE_CHANGED.swap(false, SeqCst)
}

/// True if Telescreen is the frontmost app (FLOW-26).
pub fn is_frontmost() -> bool {
    NSWorkspace::sharedWorkspace()
        .frontmostApplication()
        .is_some_and(|app| app.processIdentifier() == std::process::id() as i32)
}

/// FLOW-22: makes Telescreen the frontmost app through Accessibility, then
/// makes `window` the key window on the main thread. When only the app becomes
/// frontmost, macOS gives the focus to one of its windows on screen, which can
/// be the main window (FLOW-26). macOS can refuse `activate` for an app in the
/// background. The Accessibility request runs on a different thread, and this
/// call does not wait. AppKit windows change only on the main thread: an
/// in-process `AXMain` request on a different thread stopped the app.
pub fn make_frontmost(window: Option<&impl HasWindowHandle>) {
    // One request at a time: the Accessibility request waits for the main
    // thread, so a request for each focus event made thousands of threads, and
    // the focus changed in a loop (hardware test, 2026-11-07). The caller made
    // the window the key window already; a later call does it again.
    static IN_FLIGHT: AtomicBool = AtomicBool::new(false);
    if IN_FLIGHT.swap(true, SeqCst) {
        eprintln!("telescreen: frontmost: a request is running; skip this one");
        return;
    }
    let window = window
        .and_then(ns_window)
        .map(|w| MainWindow(Retained::into_raw(w)));
    let spawned = std::thread::Builder::new()
        .name("frontmost".into())
        .spawn(move || {
            let window = window;
            // SAFETY: the element is valid until the release below.
            let error = unsafe {
                let app = ffi::AXUIElementCreateApplication(std::process::id() as i32);
                if app.is_null() {
                    -1
                } else {
                    let error = ax_set_true(app, c"AXFrontmost");
                    ffi::release(app);
                    error
                }
            };
            if error != 0 {
                eprintln!("telescreen: Accessibility: frontmost failed ({error})");
            }
            IN_FLIGHT.store(false, SeqCst);
            if let Some(MainWindow(window)) = window {
                // SAFETY: the main queue runs `focus_on_main` once, which takes
                // the reference of `into_raw`.
                unsafe {
                    ffi::dispatch_async_f(
                        &raw const ffi::_dispatch_main_q,
                        window.cast(),
                        focus_on_main,
                    );
                }
            }
        });
    if let Err(e) = spawned {
        IN_FLIGHT.store(false, SeqCst);
        eprintln!("telescreen: frontmost: {e}");
    }
}

/// The title of the key window of this app, for the focus log.
pub fn key_window_title() -> Option<String> {
    let mtm = MainThreadMarker::new()?;
    let window = objc2_app_kit::NSApplication::sharedApplication(mtm).keyWindow()?;
    Some(window.title().to_string())
}

/// A retained window that only the main thread uses.
struct MainWindow(*mut NSWindow);
// SAFETY: the other thread only gives the pointer to the main queue.
unsafe impl Send for MainWindow {}

extern "C" fn focus_on_main(context: *mut c_void) {
    // SAFETY: the reference from `Retained::into_raw` in `make_frontmost`.
    if let Some(window) = unsafe { Retained::from_raw(context.cast::<NSWindow>()) } {
        window.makeKeyAndOrderFront(None);
    }
}

/// Sets a boolean attribute of an AX element to true. Returns the AX error.
///
/// # Safety
/// `element` must be a valid AX element.
unsafe fn ax_set_true(element: *mut c_void, attribute: &CStr) -> i32 {
    // SAFETY: a valid C string; we own the result.
    let attribute = unsafe {
        core_foundation_sys::string::CFStringCreateWithCString(
            std::ptr::null(),
            attribute.as_ptr(),
            core_foundation_sys::string::kCFStringEncodingUTF8,
        ) as *mut c_void
    };
    if attribute.is_null() {
        return -1;
    }
    // SAFETY: valid element, attribute, and constant value.
    let error = unsafe {
        ffi::AXUIElementSetAttributeValue(
            element,
            attribute,
            core_foundation_sys::number::kCFBooleanTrue.cast(),
        )
    };
    ffi::release(attribute);
    error
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
