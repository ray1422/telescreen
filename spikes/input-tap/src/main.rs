//! SPIKE (throwaway): capture and suppress all keyboard/mouse/trackpad input on macOS
//! with a CGEventTap, freeze the cursor, read mouse deltas, and prove the exits.
//!
//! Usage: SpikeInputTap [--seconds N] [--crash-after N] [--session]
//!   Suppression starts 3 s after launch and lasts at most N s (default 30).
//!   Exits: Escape 10 times in 5 s, Control+Option+Command+Escape, or the deadline.
//!   The callback checks all exits itself, so a stuck main thread cannot hold input.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicPtr, AtomicU64, Ordering::*};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use core_foundation_sys::base::CFRelease;
use core_foundation_sys::runloop::{
    CFRunLoopAddSource, CFRunLoopGetCurrent, CFRunLoopRunInMode, kCFRunLoopCommonModes, kCFRunLoopDefaultMode,
};

type CGEventRef = *mut c_void;
type CFMachPortRef = *mut c_void;
type TapCallback = extern "C" fn(*mut c_void, u32, CGEventRef, *mut c_void) -> CGEventRef;

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapCreate(tap: u32, place: u32, options: u32, mask: u64, cb: TapCallback, info: *mut c_void) -> CFMachPortRef;
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
    fn CGEventGetIntegerValueField(e: CGEventRef, field: u32) -> i64;
    fn CGEventGetFlags(e: CGEventRef) -> u64;
    fn CGAssociateMouseAndMouseCursorPosition(connected: bool) -> i32;
    fn CGMainDisplayID() -> u32;
    fn CGDisplayHideCursor(display: u32) -> i32;
    fn CGDisplayShowCursor(display: u32) -> i32;
    fn CGPreflightListenEventAccess() -> bool;
    fn CGRequestListenEventAccess() -> bool;
    fn CGPreflightPostEventAccess() -> bool;
}
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
}
unsafe extern "C" {
    fn CFMachPortCreateRunLoopSource(a: *const c_void, port: CFMachPortRef, order: isize) -> *mut c_void;
}

const KEY_DOWN: u32 = 10;
const KEY_UP: u32 = 11;
const FLAGS_CHANGED: u32 = 12;
const MOUSE_MOVED: u32 = 5;
const TAP_DISABLED_TIMEOUT: u32 = 0xFFFF_FFFE;
const TAP_DISABLED_USER: u32 = 0xFFFF_FFFF;
const FIELD_KEYCODE: u32 = 9;
const FIELD_DX: u32 = 4;
const FIELD_DY: u32 = 5;
const KC_ESCAPE: i64 = 53;
const FLAG_CTRL: u64 = 0x40000;
const FLAG_ALT: u64 = 0x80000;
const FLAG_CMD: u64 = 0x100000;

static T0: OnceLock<Instant> = OnceLock::new();
static TAP: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static SUPPRESS: AtomicBool = AtomicBool::new(false);
static DEADLINE_MS: AtomicI64 = AtomicI64::new(i64::MAX);
static TYPE_COUNTS: [AtomicU64; 64] = [const { AtomicU64::new(0) }; 64];
static SUPPRESSED: AtomicU64 = AtomicU64::new(0);
static DX: AtomicI64 = AtomicI64::new(0);
static DY: AtomicI64 = AtomicI64::new(0);
static ESC_TIMES: Mutex<Vec<i64>> = Mutex::new(Vec::new());
static LOG: OnceLock<Mutex<Sender<String>>> = OnceLock::new();

fn ms() -> i64 {
    T0.get().unwrap().elapsed().as_millis() as i64
}
fn log(s: String) {
    if let Some(tx) = LOG.get() {
        let _ = tx.lock().unwrap().send(format!("[{:6}ms] {s}", ms()));
    }
}

fn start_suppress(seconds: u64) {
    unsafe {
        CGDisplayHideCursor(CGMainDisplayID());
        CGAssociateMouseAndMouseCursorPosition(false);
    }
    DEADLINE_MS.store(ms() + seconds as i64 * 1000, SeqCst);
    SUPPRESS.store(true, SeqCst);
    log(format!("SUPPRESS ON for at most {seconds} s"));
}

fn stop_suppress(why: &str) {
    if SUPPRESS.swap(false, SeqCst) {
        unsafe {
            CGAssociateMouseAndMouseCursorPosition(true);
            CGDisplayShowCursor(CGMainDisplayID());
        }
        log(format!("SUPPRESS OFF ({why}); suppressed {} events, mouse delta sum ({}, {})", SUPPRESSED.load(SeqCst), DX.load(SeqCst), DY.load(SeqCst)));
    }
}

extern "C" fn callback(_proxy: *mut c_void, ty: u32, ev: CGEventRef, _info: *mut c_void) -> CGEventRef {
    if ty == TAP_DISABLED_TIMEOUT || ty == TAP_DISABLED_USER {
        log(format!("tap disabled by {}; re-enable", if ty == TAP_DISABLED_TIMEOUT { "timeout" } else { "user input" }));
        unsafe { CGEventTapEnable(TAP.load(SeqCst), true) };
        return ev;
    }
    if (ty as usize) < 64 {
        TYPE_COUNTS[ty as usize].fetch_add(1, Relaxed);
    } else {
        log(format!("event type {ty} (>= 64)"));
    }
    let now = ms();
    let on = SUPPRESS.load(SeqCst);
    if on && now > DEADLINE_MS.load(SeqCst) {
        stop_suppress("deadline");
        return ev;
    }
    let mut release = false;
    match ty {
        KEY_DOWN | KEY_UP => {
            let kc = unsafe { CGEventGetIntegerValueField(ev, FIELD_KEYCODE) };
            let flags = unsafe { CGEventGetFlags(ev) };
            if on {
                log(format!("{} keycode={kc} flags=0x{flags:x}", if ty == KEY_DOWN { "down" } else { "up  " }));
            }
            if ty == KEY_DOWN && kc == KC_ESCAPE {
                if flags & (FLAG_CTRL | FLAG_ALT | FLAG_CMD) == FLAG_CTRL | FLAG_ALT | FLAG_CMD {
                    stop_suppress("release shortcut");
                    release = true;
                } else {
                    let mut times = ESC_TIMES.lock().unwrap();
                    times.push(now);
                    times.retain(|t| now - t <= 5000);
                    if times.len() >= 10 {
                        times.clear();
                        stop_suppress("safety exit: Escape x10 in 5 s");
                        release = true;
                    }
                }
            }
        }
        FLAGS_CHANGED if on => log(format!("flags 0x{:x}", unsafe { CGEventGetFlags(ev) })),
        MOUSE_MOVED | 6 | 7 | 27 if on => {
            DX.fetch_add(unsafe { CGEventGetIntegerValueField(ev, FIELD_DX) }, Relaxed);
            DY.fetch_add(unsafe { CGEventGetIntegerValueField(ev, FIELD_DY) }, Relaxed);
        }
        t if on && !matches!(t, MOUSE_MOVED | 6 | 7 | 27) => log(format!("event type {t}")),
        _ => {}
    }
    if on && !release {
        SUPPRESSED.fetch_add(1, Relaxed);
        return std::ptr::null_mut();
    }
    ev
}

fn main() {
    T0.set(Instant::now()).unwrap();
    let (tx, rx) = channel::<String>();
    LOG.set(Mutex::new(tx)).unwrap();
    std::thread::spawn(move || {
        for line in rx {
            println!("{line}");
        }
    });

    let args: Vec<String> = std::env::args().collect();
    let arg = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).and_then(|v| v.parse::<u64>().ok());
    let seconds = arg("--seconds").unwrap_or(30).min(120);
    let crash_after = arg("--crash-after");
    let session = args.iter().any(|a| a == "--session");

    unsafe {
        log(format!("AXIsProcessTrusted={} listenAccess={} postAccess={}", AXIsProcessTrusted(), CGPreflightListenEventAccess(), CGPreflightPostEventAccess()));
        if !CGPreflightListenEventAccess() {
            log(format!("CGRequestListenEventAccess -> {}", CGRequestListenEventAccess()));
        }
        let location = if session { 1 } else { 0 };
        let tap = CGEventTapCreate(location, 0, 0, u64::MAX, callback, std::ptr::null_mut());
        if tap.is_null() {
            log(format!("CGEventTapCreate failed at location {location} (grant Accessibility to SpikeInputTap.app)"));
            std::thread::sleep(Duration::from_millis(300));
            std::process::exit(2);
        }
        log(format!("tap created at {}", if session { "session" } else { "HID" }));
        TAP.store(tap, SeqCst);
        let src = CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0);
        CFRunLoopAddSource(CFRunLoopGetCurrent(), src as _, kCFRunLoopCommonModes);
        CFRelease(src as _);
        CGEventTapEnable(tap, true);
    }

    let start_at = Instant::now() + Duration::from_secs(3);
    let mut started = false;
    loop {
        unsafe { CFRunLoopRunInMode(kCFRunLoopDefaultMode, 0.1, 0) };
        if !started && Instant::now() >= start_at {
            started = true;
            start_suppress(seconds);
            if let Some(n) = crash_after {
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(n));
                    log("CRASH TEST: abort() while suppressing".into());
                    std::thread::sleep(Duration::from_millis(100));
                    std::process::abort();
                });
            }
        }
        if started && !SUPPRESS.load(SeqCst) {
            break;
        }
    }
    let mut counts: Vec<String> = TYPE_COUNTS.iter().enumerate().filter_map(|(i, c)| {
        let n = c.load(SeqCst);
        (n > 0).then(|| format!("{i}:{n}"))
    }).collect();
    counts.sort();
    log(format!("event type counts (type:count): {}", counts.join(" ")));
    std::thread::sleep(Duration::from_millis(300));
}
