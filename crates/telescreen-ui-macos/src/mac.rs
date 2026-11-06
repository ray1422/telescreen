//! The status item and the menu panel on macOS (UI-9, UI-10, UI-17).
//!
//! All functions run on the main thread. Swift calls `on_event` during the
//! event dispatch of AppKit. That callback must not reach the controller: it
//! gives each event to the sink, and the sink wakes the main loop.

use std::cell::{Cell, RefCell};
use std::ffi::{CStr, c_char};
use std::marker::PhantomData;
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject, NSObjectProtocol};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy, NSScreen, NSWindow};
use objc2_app_kit::{
    NSBackingStoreType, NSColor, NSImage, NSPanel, NSStatusBar, NSStatusItem,
    NSVariableStatusItemLength, NSView, NSWindowCollectionBehavior, NSWindowDelegate,
    NSWindowStyleMask,
};
use objc2_foundation::{NSNotification, NSNumber, NSPoint, NSRect, NSSize, NSString};
use telescreen_view::{Glyph, Intent, MainScreen, MenuScreen};

unsafe extern "C" {
    fn ts_set_event_callback(callback: extern "C" fn(*const c_char));
    fn ts_make_menu_view() -> *mut NSView;
    fn ts_push_menu(bytes: *const u8, len: usize) -> bool;
    fn ts_make_main_view() -> *mut NSView;
    fn ts_push_main(bytes: *const u8, len: usize) -> bool;
}

/// The Apple Event of a second start and of a click on the Dock icon:
/// `kCoreEventClass` ('aevt') and `kAEReopenApplication` ('rapp').
const CORE_EVENT_CLASS: u32 = u32::from_be_bytes(*b"aevt");
const REOPEN_APPLICATION: u32 = u32::from_be_bytes(*b"rapp");

/// What the user did in the native UI.
#[derive(Debug, Clone, PartialEq)]
pub enum UiEvent {
    Intent(Intent),
    /// "Quit Telescreen" (UI-15).
    Quit,
    /// "Telescreen Settings…" (UI-14).
    OpenSettings,
}

/// A click on the status item within this time after the panel lost the key
/// focus is the same click: it closes the menu and does not open it again.
const REOPEN_GUARD: Duration = Duration::from_millis(250);

define_class!(
    // SAFETY: a main-thread panel with no ivars or custom destruction.
    #[unsafe(super = NSPanel)]
    #[name = "TelescreenMenuPanel"]
    struct MenuPanel;

    impl MenuPanel {
        // SAFETY: the signature matches NSWindow. A borderless panel is not key by
        // default, and then SwiftUI draws inactive controls (spike 7).
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }
    }
);

define_class!(
    // SAFETY: a main-thread object with no ivars.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "TelescreenStatusTarget"]
    struct StatusTarget;

    impl StatusTarget {
        // SAFETY: an action method with one object argument.
        #[unsafe(method(toggleMenu:))]
        fn toggle_menu(&self, _sender: Option<&AnyObject>) {
            toggle();
        }
    }
);

define_class!(
    // SAFETY: a main-thread object with no ivars.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "TelescreenMenuDelegate"]
    struct MenuDelegate;

    unsafe impl NSObjectProtocol for MenuDelegate {}

    unsafe impl NSWindowDelegate for MenuDelegate {
        // SAFETY: the signature matches NSWindowDelegate.
        #[unsafe(method(windowDidResignKey:))]
        fn window_did_resign_key(&self, _notification: &NSNotification) {
            hide();
            HIDDEN_AT.set(Some(Instant::now()));
        }
    }
);

/// The receiver of the UI events (`MacUi::new`).
type Sink = Box<dyn Fn(UiEvent)>;

define_class!(
    // SAFETY: a main-thread object with no ivars.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "TelescreenMainDelegate"]
    struct MainDelegate;

    unsafe impl NSObjectProtocol for MainDelegate {}

    unsafe impl NSWindowDelegate for MainDelegate {
        // SAFETY: the signature matches NSWindowDelegate.
        #[unsafe(method(windowWillClose:))]
        fn window_will_close(&self, _notification: &NSNotification) {
            // UI-14 default: the Dock icon shows only while a window is open.
            // UI-15: the app continues to run.
            if let Some(mtm) = MainThreadMarker::new() {
                NSApplication::sharedApplication(mtm)
                    .setActivationPolicy(NSApplicationActivationPolicy::Accessory);
            }
        }

        // SAFETY: the signature matches NSWindowDelegate.
        #[unsafe(method(windowDidMove:))]
        fn window_did_move(&self, _notification: &NSNotification) {
            check_main();
        }

        // SAFETY: the signature matches NSWindowDelegate.
        #[unsafe(method(windowDidChangeScreen:))]
        fn window_did_change_screen(&self, _notification: &NSNotification) {
            check_main();
        }
    }
);

/// UI-13: the main window does not show on a display with a portal: it goes
/// to the Dock. AppKit tells each move and each change of the screen. First it
/// moves to a display with no portal, so that it comes back there from the
/// Dock.
fn check_main() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let key = NSString::from_str("NSScreenNumber");
    let id = |screen: &NSScreen| {
        screen
            .deviceDescription()
            .objectForKey(&key)
            .and_then(|n| n.downcast::<NSNumber>().ok())
            .map(|n| n.unsignedIntValue())
    };
    let portals = PORTALS.with_borrow(Clone::clone);
    STATE.with(|state| {
        let Ok(state) = state.try_borrow() else {
            return;
        };
        let Some(state) = state.as_ref() else { return };
        let main = &state.main;
        if !main.isVisible() || main.isMiniaturized() {
            return;
        }
        let on_portal = main
            .screen()
            .and_then(|screen| id(&screen))
            .is_some_and(|id| portals.contains(&id));
        if !on_portal {
            return;
        }
        eprintln!("telescreen: minimize the main window on a portal display");
        let free = NSScreen::screens(mtm)
            .iter()
            .find(|s| id(s).is_some_and(|id| !portals.contains(&id)));
        if let Some(free) = free {
            let area = free.visibleFrame();
            let size = main.frame().size;
            main.setFrameOrigin(NSPoint::new(
                area.origin.x + (area.size.width - size.width).max(0.0) / 2.0,
                area.origin.y + (area.size.height - size.height).max(0.0) / 2.0,
            ));
        }
        main.miniaturize(None);
        MINIMIZED.set(true);
    });
}

/// UI-13: when no portal is open, the main window that Telescreen minimized
/// comes back.
fn restore_main() {
    if !MINIMIZED.replace(false) {
        return;
    }
    STATE.with(|state| {
        if let Ok(state) = state.try_borrow()
            && let Some(state) = state.as_ref()
            && state.main.isMiniaturized()
        {
            state.main.deminiaturize(None);
        }
    });
}

define_class!(
    // SAFETY: a main-thread object with no ivars.
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[name = "TelescreenReopenHandler"]
    struct ReopenHandler;

    impl ReopenHandler {
        // SAFETY: the signature of an NSAppleEventManager handler.
        #[unsafe(method(handleReopen:withReplyEvent:))]
        fn handle_reopen(&self, _event: Option<&AnyObject>, _reply: Option<&AnyObject>) {
            // UI-15: a second start opens the main window of this instance.
            show_main();
        }
    }
);

struct State {
    main: Retained<NSWindow>,
    _main_delegate: Retained<MainDelegate>,
    _reopen: Retained<ReopenHandler>,
    panel: Retained<MenuPanel>,
    item: Retained<NSStatusItem>,
    _target: Retained<StatusTarget>,
    _delegate: Retained<MenuDelegate>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
    static SINK: RefCell<Option<Sink>> = const { RefCell::new(None) };
    /// The size that SwiftUI wants. It can arrive before the panel exists.
    static SIZE: Cell<Option<NSSize>> = const { Cell::new(None) };
    static HIDDEN_AT: Cell<Option<Instant>> = const { Cell::new(None) };
    /// The Core Graphics ids of the displays with a portal (UI-13).
    static PORTALS: RefCell<Vec<u32>> = const { RefCell::new(Vec::new()) };
    /// Telescreen minimized the main window (UI-13).
    static MINIMIZED: Cell<bool> = const { Cell::new(false) };
}

fn emit(event: UiEvent) {
    SINK.with_borrow(|sink| {
        if let Some(sink) = sink {
            sink(event);
        }
    });
}

extern "C" fn on_event(json: *const c_char) {
    // SAFETY: Swift passes a NUL-terminated string that is valid for the call.
    let text = unsafe { CStr::from_ptr(json) }.to_string_lossy();
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        eprintln!("telescreen: bad UI event: {text}");
        return;
    };
    if let Some(intent) = value.get("intent") {
        match serde_json::from_value::<Intent>(intent.clone()) {
            Ok(intent) => {
                hide();
                emit(UiEvent::Intent(intent));
            }
            Err(e) => eprintln!("telescreen: bad intent {intent}: {e}"),
        }
        return;
    }
    match value.get("ui").and_then(|v| v.as_str()) {
        Some("menu_size") => {
            let get = |key: &str| value.get(key).and_then(serde_json::Value::as_f64);
            if let (Some(w), Some(h)) = (get("width"), get("height")) {
                SIZE.set(Some(NSSize::new(w, h)));
                place();
            }
        }
        Some("quit") => emit(UiEvent::Quit),
        Some("open_settings") => {
            hide();
            emit(UiEvent::OpenSettings);
        }
        _ => eprintln!("telescreen: unknown UI event: {text}"),
    }
}

/// Puts the panel below the status item, at the size that SwiftUI wants (UI-10).
fn place() {
    let Some(size) = SIZE.get() else { return };
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    STATE.with(|state| {
        let Ok(state) = state.try_borrow() else {
            return;
        };
        let Some(state) = state.as_ref() else { return };
        let Some(button) = state.item.button(mtm) else {
            return;
        };
        let Some(item_window) = button.window() else {
            return;
        };
        let anchor = item_window.frame();
        let mut x = anchor.origin.x;
        if let Some(screen) = item_window.screen() {
            let visible = screen.visibleFrame();
            let right = visible.origin.x + visible.size.width - 8.0;
            if x + size.width > right {
                x = right - size.width;
            }
        }
        let top = anchor.origin.y - 4.0;
        let frame = NSRect::new(NSPoint::new(x, top - size.height), size);
        state.panel.setFrame_display(frame, true);
    });
}

fn hide() {
    STATE.with(|state| {
        if let Ok(state) = state.try_borrow()
            && let Some(state) = state.as_ref()
        {
            state.panel.orderOut(None);
        }
    });
}

fn toggle() {
    let visible = STATE.with(|state| {
        state
            .try_borrow()
            .ok()
            .and_then(|s| s.as_ref().map(|s| s.panel.isVisible()))
    });
    match visible {
        Some(true) => hide(),
        Some(false) => {
            if HIDDEN_AT.get().is_some_and(|t| t.elapsed() < REOPEN_GUARD) {
                return;
            }
            show();
        }
        None => {}
    }
}

fn show() {
    place();
    STATE.with(|state| {
        if let Ok(state) = state.try_borrow()
            && let Some(state) = state.as_ref()
        {
            state.panel.makeKeyAndOrderFront(None);
            state.panel.orderFrontRegardless();
        }
    });
}

fn show_main() {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    hide();
    STATE.with(|state| {
        if let Ok(state) = state.try_borrow()
            && let Some(state) = state.as_ref()
        {
            let app = NSApplication::sharedApplication(mtm);
            app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
            state.main.makeKeyAndOrderFront(None);
            // SAFETY: `activate` takes no argument (macOS 14 and later).
            let _: () = unsafe { msg_send![&*app, activate] };
        }
    });
    check_main();
}

/// The native UI. Main thread only; it is not `Send`.
#[derive(Debug)]
pub struct MacUi {
    _main_thread: PhantomData<*const ()>,
}

impl MacUi {
    /// Makes the status item and the menu panel. `sink` gets each event of the
    /// UI. It runs inside the event dispatch of AppKit, so it must only queue
    /// the event and wake the main loop.
    pub fn new(sink: impl Fn(UiEvent) + 'static) -> Result<Self, String> {
        let mtm = MainThreadMarker::new().ok_or("the macOS UI needs the main thread")?;
        SINK.with_borrow_mut(|s| *s = Some(Box::new(sink)));
        // SAFETY: the callback is a plain function that is valid for the program.
        unsafe { ts_set_event_callback(on_event) };
        // SAFETY: ts_make_menu_view returns a +1 retained NSHostingView.
        let view: Retained<NSView> = unsafe { Retained::from_raw(ts_make_menu_view()) }
            .ok_or("Swift returned no menu view")?;
        let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(330.0, 200.0));
        // SAFETY: the NSWindow designated initializer, on the main thread.
        let panel: Retained<MenuPanel> = unsafe {
            msg_send![MenuPanel::alloc(mtm), initWithContentRect: rect, styleMask: style,
                backing: NSBackingStoreType::Buffered, defer: false]
        };
        panel.setOpaque(false);
        panel.setBackgroundColor(Some(&NSColor::clearColor()));
        panel.setHasShadow(true);
        // UI-13: the menu shows on each Space and over the fullscreen portal, so
        // that macOS does not change the Space of a portal display for it.
        panel.setCollectionBehavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        // Above the portal (level 26): `NSPopUpMenuWindowLevel`.
        panel.setLevel(101);
        // SAFETY: the panel lives in STATE and is never released by AppKit.
        unsafe { panel.setReleasedWhenClosed(false) };
        panel.setContentView(Some(&view));
        // SAFETY: plain init of a class with no ivars.
        let delegate: Retained<MenuDelegate> = unsafe { msg_send![MenuDelegate::alloc(mtm), init] };
        panel.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(&*delegate)));

        let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
        // SAFETY: plain init of a class with no ivars.
        let target: Retained<StatusTarget> = unsafe { msg_send![StatusTarget::alloc(mtm), init] };
        if let Some(button) = item.button(mtm) {
            // SAFETY: the target lives in STATE as long as the button.
            unsafe {
                button.setTarget(Some(&target));
                button.setAction(Some(sel!(toggleMenu:)));
            }
        }
        // SAFETY: ts_make_main_view returns a +1 retained NSHostingView.
        let main_view: Retained<NSView> = unsafe { Retained::from_raw(ts_make_main_view()) }
            .ok_or("Swift returned no main view")?;
        let style = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable;
        let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(900.0, 600.0));
        // SAFETY: the NSWindow designated initializer, on the main thread.
        let main: Retained<NSWindow> = unsafe {
            msg_send![NSWindow::alloc(mtm), initWithContentRect: rect, styleMask: style,
                backing: NSBackingStoreType::Buffered, defer: false]
        };
        main.setTitle(&NSString::from_str("Telescreen"));
        // SAFETY: the window lives in STATE and is never released by AppKit.
        unsafe { main.setReleasedWhenClosed(false) };
        main.setContentView(Some(&main_view));
        // UI-13: the main window never becomes fullscreen or a tile. Else the
        // user can drag it into the portal Space in Mission Control, where it
        // cannot move away.
        main.setCollectionBehavior(
            NSWindowCollectionBehavior::FullScreenNone
                | NSWindowCollectionBehavior::FullScreenDisallowsTiling,
        );
        main.center();
        // SAFETY: plain init of a class with no ivars.
        let main_delegate: Retained<MainDelegate> =
            unsafe { msg_send![MainDelegate::alloc(mtm), init] };
        main.setDelegate(Some(objc2::runtime::ProtocolObject::from_ref(
            &*main_delegate,
        )));
        // SAFETY: plain init of a class with no ivars.
        let reopen: Retained<ReopenHandler> = unsafe { msg_send![ReopenHandler::alloc(mtm), init] };
        // SAFETY: NSAppleEventManager keeps an unretained handler; STATE keeps it.
        unsafe {
            let manager: *mut AnyObject =
                msg_send![objc2::class!(NSAppleEventManager), sharedAppleEventManager];
            let _: () = msg_send![manager, setEventHandler: &*reopen,
                andSelector: sel!(handleReopen:withReplyEvent:),
                forEventClass: CORE_EVENT_CLASS, andEventID: REOPEN_APPLICATION];
        }
        STATE.with_borrow_mut(|s| {
            *s = Some(State {
                main,
                _main_delegate: main_delegate,
                _reopen: reopen,
                panel,
                item,
                _target: target,
                _delegate: delegate,
            })
        });
        let ui = Self {
            _main_thread: PhantomData,
        };
        ui.set_glyph(Glyph::Idle);
        Ok(ui)
    }

    /// Gives a full snapshot to the menu (UI-18).
    pub fn push_menu(&self, menu: &MenuScreen) {
        match serde_json::to_vec(menu) {
            // SAFETY: the bytes are valid for the call; Swift copies them.
            Ok(json) => {
                if !unsafe { ts_push_menu(json.as_ptr(), json.len()) } {
                    eprintln!("telescreen: the menu did not accept the view model");
                }
            }
            Err(e) => eprintln!("telescreen: cannot encode the menu: {e}"),
        }
        self.set_glyph(menu.glyph);
    }

    /// The image of the status item (UI-17). The system symbols are placeholders
    /// until the custom glyph exists.
    pub fn set_glyph(&self, glyph: Glyph) {
        let Some(mtm) = MainThreadMarker::new() else {
            return;
        };
        let name = match glyph {
            Glyph::Idle => "display",
            Glyph::Controlling => "cursorarrow.rays",
            Glyph::Controlled => "av.remote",
        };
        STATE.with(|state| {
            let Ok(state) = state.try_borrow() else {
                return;
            };
            let Some(state) = state.as_ref() else { return };
            let Some(button) = state.item.button(mtm) else {
                return;
            };
            let image = NSImage::imageWithSystemSymbolName_accessibilityDescription(
                &NSString::from_str(name),
                Some(&NSString::from_str("Telescreen")),
            );
            if let Some(image) = image {
                image.setTemplate(true);
                button.setImage(Some(&image));
            }
        });
    }

    /// Gives a full snapshot to the main window (UI-13, UI-18).
    pub fn push_main(&self, main: &MainScreen) {
        match serde_json::to_vec(main) {
            // SAFETY: the bytes are valid for the call; Swift copies them.
            Ok(json) => {
                if !unsafe { ts_push_main(json.as_ptr(), json.len()) } {
                    eprintln!("telescreen: the main window did not accept the view model");
                }
            }
            Err(e) => eprintln!("telescreen: cannot encode the main window: {e}"),
        }
    }

    /// Opens the main window and shows the Dock icon (UI-13, UI-14).
    pub fn show_main_window(&self) {
        show_main();
    }

    pub fn main_window_visible(&self) -> bool {
        STATE.with(|state| {
            state
                .try_borrow()
                .ok()
                .and_then(|s| s.as_ref().map(|s| s.main.isVisible()))
                .unwrap_or(false)
        })
    }

    /// UI-13: the displays with a portal, as Core Graphics display ids. The
    /// main window goes to the Dock when it is on one of them, now and later.
    pub fn set_portal_displays(&self, portals: &[u32]) {
        let changed = PORTALS.with_borrow_mut(|old| {
            let changed = old.as_slice() != portals;
            if changed {
                *old = portals.to_vec();
            }
            changed
        });
        if changed && portals.is_empty() {
            restore_main();
        } else if changed {
            check_main();
        }
    }

    /// Closes the main window as the close button does.
    pub fn close_main_window(&self) {
        STATE.with(|state| {
            if let Ok(state) = state.try_borrow()
                && let Some(state) = state.as_ref()
            {
                state.main.performClose(None);
            }
        });
    }

    /// The activation policy: true while the Dock icon shows.
    pub fn dock_icon_visible(&self) -> bool {
        MainThreadMarker::new().is_some_and(|mtm| {
            NSApplication::sharedApplication(mtm).activationPolicy()
                == NSApplicationActivationPolicy::Regular
        })
    }

    /// Opens the menu below the status item.
    pub fn show_menu(&self) {
        show();
    }

    pub fn hide_menu(&self) {
        hide();
    }

    /// True while the menu panel shows.
    pub fn menu_visible(&self) -> bool {
        STATE.with(|state| {
            state
                .try_borrow()
                .ok()
                .and_then(|s| s.as_ref().map(|s| s.panel.isVisible()))
                .unwrap_or(false)
        })
    }

    /// The screen frame of the menu panel, top-left origin of the primary
    /// display, for `screencapture -R` in a check.
    pub fn menu_frame(&self) -> Option<(f64, f64, f64, f64)> {
        let mtm = MainThreadMarker::new()?;
        let primary = objc2_app_kit::NSScreen::screens(mtm).firstObject()?.frame();
        STATE.with(|state| {
            let state = state.try_borrow().ok()?;
            let f = state.as_ref()?.panel.frame();
            Some((
                f.origin.x,
                primary.size.height - f.origin.y - f.size.height,
                f.size.width,
                f.size.height,
            ))
        })
    }
}
