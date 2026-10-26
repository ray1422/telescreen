//! Input capture and suppression on the source with a CGEventTap at the HID
//! location (spike 2, ADR 0003, IN-1..IN-10).
//!
//! The tap runs on its own thread with its own run loop. The callback only reads
//! the event, decides to suppress it, and pushes a [`CapturedEvent`] to a bounded
//! queue (IN-2). The callback itself handles the release shortcut and the safety
//! exit, so they work when the main thread does not respond (IN-7, IN-8). A
//! watchdog thread stops suppression when the main thread stops its heartbeat (IN-9).

use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicU64, Ordering::SeqCst};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use telescreen_core::input::{Button, InputEvent};
use telescreen_core::keys::{self, Key};
use telescreen_core::platform::{CapturedEvent, InputCapture, PlatformError};
use telescreen_core::pointer::{Point, Rect};

use crate::ffi::{self, CGEventRef, CGPoint};

/// The queue size. A full queue ends the session (IN-3).
const QUEUE: usize = 4096;
/// IN-9: the main thread must give a heartbeat in this time while forwarding.
const WATCHDOG: Duration = Duration::from_secs(3);
/// IN-8: Escape this many times in this time.
const SAFETY_PRESSES: usize = 10;
const SAFETY_WINDOW: Duration = Duration::from_secs(5);
const KEYCODE_ESCAPE: i64 = 0x35;
const RELEASE_FLAGS: u64 = ffi::FLAG_CONTROL | ffi::FLAG_ALTERNATE | ffi::FLAG_COMMAND;

const OFF: u8 = 0;
const ARMED: u8 = 1;
const FORWARDING: u8 = 2;

type Wake = Box<dyn Fn() + Send + Sync>;

struct Shared {
    start: Instant,
    mode: AtomicU8,
    /// The portal bounds as f64 bits: x, y, width, height.
    portal: [AtomicU64; 4],
    entered: AtomicBool,
    frozen: AtomicBool,
    heartbeat_ms: AtomicU64,
    overflow: AtomicBool,
    woken: AtomicBool,
    tap: AtomicPtr<c_void>,
    /// Callback-only state. The mutex is never contended.
    keys: Mutex<KeyState>,
    tx: Sender<CapturedEvent>,
    wake: Wake,
}

#[derive(Default)]
struct KeyState {
    escapes: VecDeque<Instant>,
    /// Modifier keys that are down (evdev codes).
    modifiers: Vec<Key>,
}

/// The event tap. Clone it to keep a handle for [`MacCapture::drain`] and
/// [`InputCapture::heartbeat`] after the controller gets one.
#[derive(Clone)]
pub struct MacCapture {
    shared: Arc<Shared>,
    rx: Receiver<CapturedEvent>,
}

impl std::fmt::Debug for MacCapture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MacCapture")
            .field("mode", &self.shared.mode.load(SeqCst))
            .finish()
    }
}

impl MacCapture {
    /// Starts the tap thread and the watchdog thread. `wake` is called (from those
    /// threads) when events are in the queue; it must not block.
    pub fn start(wake: impl Fn() + Send + Sync + 'static) -> Result<Self, PlatformError> {
        let (tx, rx) = bounded(QUEUE);
        let shared = Arc::new(Shared {
            start: Instant::now(),
            mode: AtomicU8::new(OFF),
            portal: Default::default(),
            entered: AtomicBool::new(false),
            frozen: AtomicBool::new(false),
            heartbeat_ms: AtomicU64::new(0),
            overflow: AtomicBool::new(false),
            woken: AtomicBool::new(false),
            tap: AtomicPtr::new(std::ptr::null_mut()),
            keys: Mutex::default(),
            tx,
            wake: Box::new(wake),
        });
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let tap_shared = shared.clone();
        thread::Builder::new()
            .name("telescreen-event-tap".into())
            .spawn(move || run_tap(tap_shared, ready_tx))
            .map_err(|e| PlatformError::Failed(e.to_string()))?;
        ready_rx
            .recv()
            .map_err(|_| PlatformError::Failed("the event tap thread stopped".into()))??;
        let watchdog = shared.clone();
        thread::Builder::new()
            .name("telescreen-watchdog".into())
            .spawn(move || run_watchdog(watchdog))
            .map_err(|e| PlatformError::Failed(e.to_string()))?;
        Ok(Self { shared, rx })
    }

    /// Takes the queued events. Adds `Overflow` if the queue was full (IN-3).
    pub fn drain(&self) -> Vec<CapturedEvent> {
        self.shared.woken.store(false, SeqCst);
        let mut events: Vec<_> = self.rx.try_iter().collect();
        if self.shared.overflow.swap(false, SeqCst) {
            events.push(CapturedEvent::Overflow);
        }
        events
    }

    pub fn is_forwarding(&self) -> bool {
        self.shared.mode.load(SeqCst) == FORWARDING
    }
}

impl InputCapture for MacCapture {
    fn arm(&mut self, portal: Rect) -> Result<(), PlatformError> {
        let s = &self.shared;
        for (cell, value) in s
            .portal
            .iter()
            .zip([portal.x, portal.y, portal.width, portal.height])
        {
            cell.store(value.to_bits(), SeqCst);
        }
        s.entered.store(false, SeqCst);
        if s.mode.load(SeqCst) == OFF {
            s.mode.store(ARMED, SeqCst);
        }
        let at = ffi::cursor_position();
        if portal.contains(Point { x: at.x, y: at.y }) && !s.entered.swap(true, SeqCst) {
            s.push(CapturedEvent::PointerEntered {
                at: Point { x: at.x, y: at.y },
            });
        }
        Ok(())
    }

    fn forward(&mut self) -> Result<Point, PlatformError> {
        if crate::secure_event_input() {
            return Err(PlatformError::Permission(
                "Secure Event Input is on (a password field has the focus). macOS blocks the capture of keys."
                    .into(),
            ));
        }
        let s = &self.shared;
        s.beat();
        let at = ffi::cursor_position();
        s.lock_keys().escapes.clear();
        s.mode.store(FORWARDING, SeqCst);
        s.freeze();
        Ok(Point { x: at.x, y: at.y })
    }

    fn pause(&mut self, at: Option<Point>) {
        let s = &self.shared;
        s.entered.store(false, SeqCst);
        if s.mode.load(SeqCst) == FORWARDING {
            s.mode.store(ARMED, SeqCst);
        }
        if let Some(at) = at {
            // SAFETY: moves the cursor; it is frozen until restore below.
            unsafe { ffi::CGWarpMouseCursorPosition(CGPoint { x: at.x, y: at.y }) };
        }
        s.restore();
    }

    fn disarm(&mut self) {
        self.shared.mode.store(OFF, SeqCst);
        self.shared.restore();
    }

    fn heartbeat(&mut self) {
        self.shared.beat();
    }
}

impl Shared {
    fn now_ms(&self) -> u64 {
        u64::try_from(self.start.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn beat(&self) {
        self.heartbeat_ms.store(self.now_ms(), SeqCst);
    }

    fn lock_keys(&self) -> std::sync::MutexGuard<'_, KeyState> {
        self.keys.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn portal(&self) -> Rect {
        let v = |i: usize| f64::from_bits(self.portal[i].load(SeqCst));
        Rect {
            x: v(0),
            y: v(1),
            width: v(2),
            height: v(3),
        }
    }

    fn freeze(&self) {
        if !self.frozen.swap(true, SeqCst) {
            // SAFETY: CG cursor calls; `restore` undoes them once.
            unsafe {
                ffi::CGAssociateMouseAndMouseCursorPosition(false);
                ffi::CGDisplayHideCursor(ffi::CGMainDisplayID());
            }
        }
    }

    fn restore(&self) {
        if self.frozen.swap(false, SeqCst) {
            // SAFETY: undoes `freeze`.
            unsafe {
                ffi::CGAssociateMouseAndMouseCursorPosition(true);
                ffi::CGDisplayShowCursor(ffi::CGMainDisplayID());
            }
        }
    }

    /// Stops suppression from the callback or the watchdog, and reports why.
    fn stop(&self, event: CapturedEvent) {
        self.mode.store(OFF, SeqCst);
        self.restore();
        self.push(event);
    }

    fn push(&self, event: CapturedEvent) {
        match self.tx.try_send(event) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                // IN-3: a lost event ends the session. Stop suppression now.
                self.overflow.store(true, SeqCst);
                self.mode.store(OFF, SeqCst);
                self.restore();
            }
            Err(TrySendError::Disconnected(_)) => return,
        }
        if !self.woken.swap(true, SeqCst) {
            (self.wake)();
        }
    }
}

fn run_tap(shared: Arc<Shared>, ready: std::sync::mpsc::Sender<Result<(), PlatformError>>) {
    // The callback gets a pointer to the shared state. This thread keeps one
    // reference for the life of the process.
    let info = Arc::into_raw(shared.clone()) as *mut c_void;
    // SAFETY: creates the tap with a valid callback and info pointer, and adds it to
    // the run loop of this thread. The run loop source is released after the add.
    unsafe {
        let tap = ffi::CGEventTapCreate(
            ffi::HID_EVENT_TAP,
            ffi::HEAD_INSERT,
            ffi::TAP_DEFAULT,
            u64::MAX,
            callback,
            info,
        );
        if tap.is_null() {
            let _ = ready.send(Err(PlatformError::Permission(
                "cannot make the event tap: grant Accessibility and Input Monitoring to Telescreen"
                    .into(),
            )));
            return;
        }
        shared.tap.store(tap, SeqCst);
        let source = ffi::CFMachPortCreateRunLoopSource(std::ptr::null(), tap, 0);
        core_foundation_sys::runloop::CFRunLoopAddSource(
            core_foundation_sys::runloop::CFRunLoopGetCurrent(),
            source as _,
            core_foundation_sys::runloop::kCFRunLoopCommonModes,
        );
        ffi::release(source);
        ffi::CGEventTapEnable(tap, true);
    }
    let _ = ready.send(Ok(()));
    // SAFETY: runs the run loop of this thread until the process ends.
    unsafe { core_foundation_sys::runloop::CFRunLoopRun() };
}

fn run_watchdog(shared: Arc<Shared>) {
    loop {
        thread::sleep(Duration::from_millis(200));
        if shared.mode.load(SeqCst) != FORWARDING {
            continue;
        }
        let silent = shared
            .now_ms()
            .saturating_sub(shared.heartbeat_ms.load(SeqCst));
        if silent > WATCHDOG.as_millis() as u64 {
            shared.stop(CapturedEvent::SafetyExit);
        }
    }
}

extern "C" fn callback(
    _proxy: *mut c_void,
    kind: u32,
    event: CGEventRef,
    info: *mut c_void,
) -> CGEventRef {
    // SAFETY: `info` is the pointer from `run_tap`, which is never freed.
    let shared = unsafe { &*(info as *const Shared) };
    if kind == ffi::TAP_DISABLED_BY_TIMEOUT || kind == ffi::TAP_DISABLED_BY_USER {
        // SAFETY: the tap pointer is valid for the life of the process.
        unsafe { ffi::CGEventTapEnable(shared.tap.load(SeqCst), true) };
        return event;
    }
    match shared.mode.load(SeqCst) {
        FORWARDING => forwarding(shared, kind, event),
        ARMED => {
            armed(shared, kind, event);
            event
        }
        _ => event,
    }
}

/// While paused: report when the pointer enters the portal (FLOW-14). No suppression.
fn armed(shared: &Shared, kind: u32, event: CGEventRef) {
    if !matches!(
        kind,
        ffi::MOUSE_MOVED | ffi::LEFT_DRAGGED | ffi::RIGHT_DRAGGED | ffi::OTHER_DRAGGED
    ) {
        return;
    }
    // SAFETY: the event is valid during the callback.
    let at = unsafe { ffi::CGEventGetLocation(event) };
    let at = Point { x: at.x, y: at.y };
    if shared.portal().contains(at) {
        if !shared.entered.swap(true, SeqCst) {
            shared.push(CapturedEvent::PointerEntered { at });
        }
    } else {
        shared.entered.store(false, SeqCst);
    }
}

/// While forwarding: capture and suppress all input (FLOW-13, IN-10).
fn forwarding(shared: &Shared, kind: u32, event: CGEventRef) -> CGEventRef {
    // SAFETY (for the field reads below): the event is valid during the callback.
    let field = |f: u32| unsafe { ffi::CGEventGetIntegerValueField(event, f) };
    match kind {
        ffi::KEY_DOWN | ffi::KEY_UP => {
            let code = field(ffi::FIELD_KEYCODE);
            let down = kind == ffi::KEY_DOWN;
            if down && code == KEYCODE_ESCAPE {
                // SAFETY: as above.
                let flags = unsafe { ffi::CGEventGetFlags(event) };
                if flags & RELEASE_FLAGS == RELEASE_FLAGS {
                    // IN-7: never forwarded.
                    shared.stop(CapturedEvent::ReleaseShortcut);
                    return std::ptr::null_mut();
                }
                shared.push(CapturedEvent::Input(InputEvent::Key {
                    key: Key::ESC,
                    down,
                }));
                if safety_exit(shared) {
                    shared.stop(CapturedEvent::SafetyExit);
                }
                return std::ptr::null_mut();
            }
            if let Some(key) = u16::try_from(code).ok().and_then(keys::from_mac) {
                shared.push(CapturedEvent::Input(InputEvent::Key { key, down }));
            }
        }
        ffi::FLAGS_CHANGED => {
            // SAFETY: as above.
            let flags = unsafe { ffi::CGEventGetFlags(event) };
            let code = field(ffi::FIELD_KEYCODE);
            for input in modifier_events(shared, code, flags) {
                shared.push(CapturedEvent::Input(input));
            }
        }
        ffi::MOUSE_MOVED | ffi::LEFT_DRAGGED | ffi::RIGHT_DRAGGED | ffi::OTHER_DRAGGED => {
            let (dx, dy) = (field(ffi::FIELD_DELTA_X), field(ffi::FIELD_DELTA_Y));
            if dx != 0 || dy != 0 {
                shared.push(CapturedEvent::Motion {
                    dx: dx as f64,
                    dy: dy as f64,
                });
            }
        }
        ffi::LEFT_DOWN
        | ffi::LEFT_UP
        | ffi::RIGHT_DOWN
        | ffi::RIGHT_UP
        | ffi::OTHER_DOWN
        | ffi::OTHER_UP => {
            let down = matches!(kind, ffi::LEFT_DOWN | ffi::RIGHT_DOWN | ffi::OTHER_DOWN);
            let button = match kind {
                ffi::LEFT_DOWN | ffi::LEFT_UP => Some(Button::Left),
                ffi::RIGHT_DOWN | ffi::RIGHT_UP => Some(Button::Right),
                _ => match field(ffi::FIELD_BUTTON_NUMBER) {
                    2 => Some(Button::Middle),
                    3 => Some(Button::Back),
                    4 => Some(Button::Forward),
                    _ => None,
                },
            };
            if let Some(button) = button {
                shared.push(CapturedEvent::Input(InputEvent::Button { button, down }));
            }
        }
        ffi::SCROLL_WHEEL => {
            // Point deltas: positive is up or left on macOS; the wire is the opposite.
            let dy = -(field(ffi::FIELD_SCROLL_POINT_DELTA_1) as f32);
            let dx = -(field(ffi::FIELD_SCROLL_POINT_DELTA_2) as f32);
            if dx != 0.0 || dy != 0.0 {
                shared.push(CapturedEvent::Input(InputEvent::Scroll { dx, dy }));
            }
        }
        // Gestures (Space swipes, Mission Control), system-defined events, and all
        // other types: suppress and do not forward (IN-10, ADR 0003).
        _ => {}
    }
    std::ptr::null_mut()
}

/// Counts Escape presses in the callback (IN-8). True at the tenth press in 5 s.
fn safety_exit(shared: &Shared) -> bool {
    let now = Instant::now();
    let mut keys = shared.lock_keys();
    keys.escapes.push_back(now);
    while keys
        .escapes
        .front()
        .is_some_and(|t| now.duration_since(*t) > SAFETY_WINDOW)
    {
        keys.escapes.pop_front();
    }
    if keys.escapes.len() >= SAFETY_PRESSES {
        keys.escapes.clear();
        return true;
    }
    false
}

/// A flags-changed event names the modifier key that changed. The device bits of
/// the flags tell if that key is now down.
fn modifier_events(shared: &Shared, code: i64, flags: u64) -> Vec<InputEvent> {
    let Some(key) = u16::try_from(code).ok().and_then(keys::from_mac) else {
        return Vec::new();
    };
    let bit = match code {
        0x38 => ffi::DEVICE_LSHIFT,
        0x3C => ffi::DEVICE_RSHIFT,
        0x3B => ffi::DEVICE_LCTL,
        0x3E => ffi::DEVICE_RCTL,
        0x3A => ffi::DEVICE_LALT,
        0x3D => ffi::DEVICE_RALT,
        0x37 => ffi::DEVICE_LCMD,
        0x36 => ffi::DEVICE_RCMD,
        0x3F => ffi::FLAG_SECONDARY_FN,
        // Caps Lock reports a toggle, not down and up: send a press.
        0x39 => {
            return vec![
                InputEvent::Key { key, down: true },
                InputEvent::Key { key, down: false },
            ];
        }
        _ => return Vec::new(),
    };
    let down = flags & bit != 0;
    let mut keys = shared.lock_keys();
    let held = keys.modifiers.contains(&key);
    match (down, held) {
        (true, false) => keys.modifiers.push(key),
        (false, true) => keys.modifiers.retain(|k| *k != key),
        _ => return Vec::new(),
    }
    vec![InputEvent::Key { key, down }]
}
