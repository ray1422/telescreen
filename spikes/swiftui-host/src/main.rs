//! Spike 7 (ADR 0012): the Rust `main` owns the AppKit run loop. A SwiftUI view in
//! an `NSHostingView` (a Swift static library with a C ABI) shows a view model
//! that Rust pushes as JSON. Intents come back through a C callback.
//!
//! The spike pushes snapshots from a background thread and measures the time
//! from the push to the `onChange` of the SwiftUI view (the view sends an
//! `applied` intent). `HOLD=<seconds>` keeps the window open after the pushes.

use std::collections::HashMap;
use std::ffi::{CStr, c_char};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use objc2::{MainThreadMarker, MainThreadOnly, define_class};
use objc2::rc::Retained;
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSBackingStoreType, NSColor, NSPanel,
    NSStatusBar, NSVariableStatusItemLength, NSView, NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use serde::Serialize;

unsafe extern "C" {
    fn ts_make_view(callback: extern "C" fn(*const c_char)) -> *mut NSView;
    fn ts_push_model(bytes: *const u8, len: usize);
    fn ts_quit();
}

#[derive(Serialize)]
struct Row {
    id: String,
    label: String,
    detail: String,
}

#[derive(Serialize)]
struct Snapshot {
    seq: u64,
    title: &'static str,
    rows: Vec<Row>,
}

define_class!(
    // SAFETY: this main-thread panel has no ivars or custom destruction.
    #[unsafe(super = NSPanel)]
    #[name = "TelescreenSpikeMenuPanel"]
    struct MenuPanel;

    impl MenuPanel {
        // SAFETY: the signature matches NSWindow's AppKit method. A borderless
        // panel is not key by default, and then SwiftUI draws inactive controls.
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key_window(&self) -> bool {
            true
        }
    }
);

static PUSHED: Mutex<Option<HashMap<u64, Instant>>> = Mutex::new(None);
static APPLIED: Mutex<Vec<(u64, Duration)>> = Mutex::new(Vec::new());

thread_local! {
    static SIZE: std::cell::Cell<Option<NSSize>> = const { std::cell::Cell::new(None) };
    static PANEL: std::cell::RefCell<Option<Retained<MenuPanel>>> = const { std::cell::RefCell::new(None) };
}

fn push(seq: u64) {
    let rows = (0..3)
        .map(|i| Row {
            id: format!("computer-{i}"),
            label: format!("Computer {i}"),
            detail: format!("update {seq}"),
        })
        .collect();
    let json = serde_json::to_vec(&Snapshot { seq, title: "Telescreen", rows }).unwrap();
    PUSHED.lock().unwrap().get_or_insert_default().insert(seq, Instant::now());
    // SAFETY: the bytes are valid for the call; Swift copies them.
    unsafe { ts_push_model(json.as_ptr(), json.len()) };
}

/// SwiftUI can report its size before the panel exists; apply the last size.
fn resize() {
    let Some(size) = SIZE.get() else { return };
    PANEL.with_borrow(|panel| {
        let Some(panel) = panel else { return };
        // Keep the top right corner where it is.
        let frame = panel.frame();
        let top = frame.origin.y + frame.size.height;
        let right = frame.origin.x + frame.size.width;
        panel.setFrame_display(NSRect::new(NSPoint::new(right - size.width, top - size.height), size), true);
        // For `screencapture -R`: top-left coordinates of the primary display.
        let mtm = MainThreadMarker::new().unwrap();
        let primary = objc2_app_kit::NSScreen::screens(mtm).firstObject().unwrap().frame();
        let f = panel.frame();
        println!("region {:.0},{:.0},{:.0},{:.0} key={}", f.origin.x, primary.size.height - f.origin.y - f.size.height, f.size.width, f.size.height, panel.isKeyWindow());
    });
}

extern "C" fn on_intent(json: *const c_char) {
    // SAFETY: Swift passes a NUL-terminated string that is valid for the call.
    let json = unsafe { CStr::from_ptr(json) }.to_string_lossy();
    let main = MainThreadMarker::new().is_some();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    if value["intent"] == "size" {
        let (w, h) = (value["w"].as_f64().unwrap(), value["h"].as_f64().unwrap());
        SIZE.set(Some(NSSize::new(w, h)));
        resize();
        println!("size intent on main thread = {main}: {w:.0}x{h:.0}");
    } else if value["intent"] == "applied" {
        let seq = value["seq"].as_u64().unwrap();
        if let Some(t) = PUSHED.lock().unwrap().get_or_insert_default().remove(&seq) {
            APPLIED.lock().unwrap().push((seq, t.elapsed()));
        }
    } else {
        println!("intent on main thread = {main}: {json}");
    }
}

fn main() {
    let mtm = MainThreadMarker::new().expect("main thread");
    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);

    // SAFETY: ts_make_view returns a +1 retained NSHostingView.
    let view: Retained<NSView> = unsafe { Retained::from_raw(ts_make_view(on_intent)) }.unwrap();
    let rect = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(330.0, 200.0));
    let style = NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel;
    // SAFETY: the NSWindow designated initializer, on the main thread.
    let panel: Retained<MenuPanel> = unsafe {
        objc2::msg_send![MenuPanel::alloc(mtm), initWithContentRect: rect, styleMask: style,
            backing: NSBackingStoreType::Buffered, defer: false]
    };
    panel.setOpaque(false);
    panel.setBackgroundColor(Some(&NSColor::clearColor()));
    panel.setHasShadow(true);
    panel.setContentView(Some(&view));
    push(0);

    let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    if let Some(button) = item.button(mtm) {
        button.setTitle(&NSString::from_str("TS"));
    }

    // Place the panel below the right of the menu bar, at the size SwiftUI wants.
    if let Some(screen) = panel.screen() {
        let frame = screen.visibleFrame();
        let size = view.fittingSize();
        panel.setContentSize(size);
        let origin = NSPoint::new(
            frame.origin.x + frame.size.width - size.width - 12.0,
            frame.origin.y + frame.size.height - size.height - 8.0,
        );
        panel.setFrameOrigin(origin);
        println!("panel size {:.0}x{:.0}", size.width, size.height);
    }
    panel.makeKeyAndOrderFront(None);
    panel.orderFrontRegardless();
    PANEL.with_borrow_mut(|p| *p = Some(panel.clone()));
    resize();
    println!("window number {}", panel.windowNumber());

    let hold: u64 = std::env::var("HOLD").ok().and_then(|v| v.parse().ok()).unwrap_or(0);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(500));
        // Bursts: 100 pushes at 60 Hz, then 20 pushes with no wait.
        for seq in 1..=100 {
            push(seq);
            std::thread::sleep(Duration::from_millis(16));
        }
        std::thread::sleep(Duration::from_millis(300));
        for seq in 101..=120 {
            push(seq);
        }
        std::thread::sleep(Duration::from_millis(500));
        let applied = APPLIED.lock().unwrap();
        let mut lat: Vec<f64> = applied.iter().filter(|(s, _)| *s <= 100).map(|(_, d)| d.as_secs_f64() * 1e3).collect();
        lat.sort_by(f64::total_cmp);
        let pct = |p: f64| lat[((lat.len() as f64 - 1.0) * p) as usize];
        println!(
            "60 Hz pushes: {} of 100 applied; push to onChange p50 {:.2} ms, p95 {:.2} ms, max {:.2} ms",
            lat.len(), pct(0.5), pct(0.95), lat[lat.len() - 1]
        );
        let burst: Vec<u64> = applied.iter().map(|(s, _)| *s).filter(|s| *s > 100).collect();
        println!("burst of 20 pushes: onChange saw {} of them, last {:?}", burst.len(), burst.last());
        drop(applied);
        std::thread::sleep(Duration::from_secs(hold));
        // SAFETY: ts_quit can run on any thread.
        unsafe { ts_quit() };
    });

    let start = Instant::now();
    app.run();
    // terminate: normally exits the process before this line.
    println!("run returned after {:?}", start.elapsed());
    drop(item);
}
