//! Input capture and suppression on the source with a CGEventTap at the HID
//! location (spike 2, ADR 0003, IN-1..IN-10).
//!
//! The tap runs on its own thread with its own run loop. The callback only reads
//! the event, decides to suppress it, and pushes a [`CapturedEvent`] to a bounded
//! queue (IN-2). The callback itself handles the release shortcut and the safety
//! exit, so they work when the main thread does not respond (IN-7, IN-8). A
//! watchdog thread stops suppression when the main thread stops its heartbeat (IN-9).
//!
//! The cursor is always free (FLOW-20). The tap never suppresses pointer motion,
//! drags included. While forwarding, it suppresses and reports the key, button,
//! and scroll events that [`InputRouter`] sends to the target (FLOW-22).
//! Forwarded buttons also reach the portal for native keyboard focus.
//! Other event types are suppressed while the cursor is in the portal (IN-10).

use std::collections::VecDeque;
use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU8, AtomicU64, Ordering::SeqCst};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use crossbeam_channel::{Receiver, Sender, TrySendError, bounded};
use telescreen_core::input::{Button, InputEvent};
use telescreen_core::key_route::{InputRouter, Route};
use telescreen_core::keys::{self, Key};
use telescreen_core::platform::{CapturedEvent, InputCapture, PlatformError};
use telescreen_core::pointer::{Point, Rect};

use crate::ffi::{self, CGEventRef};

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
    guarded: AtomicBool,
    secure_input: AtomicBool,
    /// The portal bounds as f64 bits: x, y, width, height.
    portal: [AtomicU64; 4],
    /// The cursor is in the portal.
    entered: AtomicBool,
    /// The latest cursor position as f64 bits, for the merged `PointerMoved`.
    moved: [AtomicU64; 2],
    /// A `PointerMoved` is in the queue and not drained yet.
    move_queued: AtomicBool,
    heartbeat_ms: AtomicU64,
    overflow: AtomicBool,
    woken: AtomicBool,
    tap: AtomicPtr<c_void>,
    /// The main thread holds this lock only for short updates, so the callback
    /// waits for it only briefly (IN-2).
    keys: Mutex<KeyState>,
    tx: Sender<CapturedEvent>,
    wake: Wake,
}

#[derive(Default)]
struct KeyState {
    escapes: VecDeque<Instant>,
    /// Modifier keys that are down (evdev codes).
    modifiers: Vec<Key>,
    /// Routes and native keyboard focus (IN-3, FLOW-22).
    router: InputRouter,
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
            guarded: AtomicBool::new(false),
            secure_input: AtomicBool::new(false),
            portal: Default::default(),
            entered: AtomicBool::new(false),
            moved: Default::default(),
            move_queued: AtomicBool::new(false),
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

    /// Takes the queued events. Adds `Overflow` if the queue was full (IN-3). A
    /// `PointerMoved` gets the latest cursor position.
    pub fn drain(&self) -> Vec<CapturedEvent> {
        let s = &self.shared;
        s.woken.store(false, SeqCst);
        s.move_queued.store(false, SeqCst);
        let mut events: Vec<_> = self
            .rx
            .try_iter()
            .map(|event| match event {
                CapturedEvent::PointerMoved { .. } => CapturedEvent::PointerMoved { at: s.moved() },
                other => other,
            })
            .collect();
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
        if s.mode.load(SeqCst) == OFF {
            s.mode.store(ARMED, SeqCst);
        }
        let at = ffi::cursor_position();
        s.cursor_at(Point { x: at.x, y: at.y });
        Ok(())
    }

    fn forward(&mut self) -> Result<(), PlatformError> {
        self.shared.guarded.store(false, SeqCst);
        if crate::secure_event_input() {
            return Err(PlatformError::Permission(
                "Secure Event Input is on (a password field has the focus). macOS blocks the capture of keys."
                    .into(),
            ));
        }
        let s = &self.shared;
        s.secure_input.store(false, SeqCst);
        s.beat();
        s.lock_keys().escapes.clear();
        s.mode.store(FORWARDING, SeqCst);
        let at = ffi::cursor_position();
        s.cursor_at(Point { x: at.x, y: at.y });
        Ok(())
    }

    fn set_portal_focus(&mut self, focused: bool) {
        let s = &self.shared;
        let mut keys = s.lock_keys();
        if let Some(focused) = keys.router.set_window_focus(focused) {
            s.keyboard_changed(focused);
        }
    }

    fn guard(&mut self, on: bool) -> Result<(), PlatformError> {
        let s = &self.shared;
        if on && s.mode.load(SeqCst) == OFF {
            return Err(PlatformError::Failed(
                "the portal guard needs an armed capture".into(),
            ));
        }
        s.guarded.store(on, SeqCst);
        Ok(())
    }

    fn disarm(&mut self) {
        let s = &self.shared;
        s.mode.store(OFF, SeqCst);
        s.guarded.store(false, SeqCst);
        s.entered.store(false, SeqCst);
        let mut keys = s.lock_keys();
        keys.router = InputRouter::new();
        keys.modifiers.clear();
    }

    fn heartbeat(&mut self) {
        let s = &self.shared;
        s.beat();
        let secure = crate::secure_event_input();
        let was_secure = s.secure_input.swap(secure, SeqCst);
        if secure && !was_secure && self.is_forwarding() {
            let mut keys = s.lock_keys();
            keys.router.clear();
            keys.modifiers.clear();
            s.push(CapturedEvent::InputLost);
        }
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

    /// Reports keyboard focus before the input that caused it (FLOW-23).
    fn keyboard_changed(&self, focused: bool) {
        self.push(if focused {
            CapturedEvent::KeyboardEntered
        } else {
            CapturedEvent::KeyboardLeft
        });
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

    fn moved(&self) -> Point {
        Point {
            x: f64::from_bits(self.moved[0].load(SeqCst)),
            y: f64::from_bits(self.moved[1].load(SeqCst)),
        }
    }

    /// True while the input in the portal goes to the target.
    fn forward_now(&self) -> bool {
        self.mode.load(SeqCst) == FORWARDING && self.entered.load(SeqCst)
    }

    /// The cursor is at `at`: report a crossing of the portal edge (FLOW-14), and
    /// the position while forwarding in the portal.
    fn cursor_at(&self, at: Point) {
        let inside = self.portal().contains(at);
        if inside != self.entered.swap(inside, SeqCst) {
            self.push(if inside {
                CapturedEvent::PointerEntered { at }
            } else {
                CapturedEvent::PointerLeft
            });
        }
        if inside && self.mode.load(SeqCst) == FORWARDING {
            self.moved[0].store(at.x.to_bits(), SeqCst);
            self.moved[1].store(at.y.to_bits(), SeqCst);
            if !self.move_queued.swap(true, SeqCst) {
                self.push(CapturedEvent::PointerMoved { at });
            }
        }
    }

    /// Stops suppression from the callback or the watchdog, and reports why.
    fn stop(&self, event: CapturedEvent) {
        self.mode.store(OFF, SeqCst);
        self.push(event);
    }

    fn push(&self, event: CapturedEvent) {
        match self.tx.try_send(event) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                // IN-3: a lost event ends the session. Stop suppression now.
                self.overflow.store(true, SeqCst);
                self.mode.store(OFF, SeqCst);
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
        let forwarding = shared.mode.load(SeqCst) == FORWARDING;
        let mut keys = if forwarding {
            Some(shared.lock_keys())
        } else {
            None
        };
        if let Some(keys) = keys.as_mut() {
            keys.router.clear();
            keys.modifiers.clear();
        }
        // SAFETY: the tap pointer is valid for the life of the process.
        unsafe { ffi::CGEventTapEnable(shared.tap.load(SeqCst), true) };
        if keys.is_some() {
            shared.push(CapturedEvent::InputLost);
        }
        return event;
    }
    if shared.mode.load(SeqCst) == OFF {
        return event;
    }
    if handle(shared, kind, event) {
        std::ptr::null_mut()
    } else {
        event
    }
}

/// Handles one event while armed or forwarding. Returns true to suppress it.
fn handle(shared: &Shared, kind: u32, event: CGEventRef) -> bool {
    // SAFETY (for the event reads below): the event is valid during the callback.
    let field = |f: u32| unsafe { ffi::CGEventGetIntegerValueField(event, f) };
    let forwarding = shared.mode.load(SeqCst) == FORWARDING;
    let motion = matches!(
        kind,
        ffi::MOUSE_MOVED | ffi::LEFT_DRAGGED | ffi::RIGHT_DRAGGED | ffi::OTHER_DRAGGED
    );
    if !forwarding {
        if !shared.guarded.load(SeqCst) || motion {
            return false;
        }
        return match kind {
            ffi::KEY_DOWN | ffi::KEY_UP | ffi::FLAGS_CHANGED => {
                // SAFETY: the event is valid during the callback.
                let flags = unsafe { ffi::CGEventGetFlags(event) };
                flags & (ffi::FLAG_COMMAND | ffi::FLAG_CONTROL) != 0
            }
            ffi::LEFT_DOWN
            | ffi::LEFT_UP
            | ffi::RIGHT_DOWN
            | ffi::RIGHT_UP
            | ffi::OTHER_DOWN
            | ffi::OTHER_UP => {
                // SAFETY: the event is valid during the callback.
                let at = unsafe { ffi::CGEventGetLocation(event) };
                !shared.portal().contains(Point { x: at.x, y: at.y })
            }
            _ => true,
        };
    }
    if motion {
        // SAFETY: the event is valid during the callback.
        let at = unsafe { ffi::CGEventGetLocation(event) };
        shared.cursor_at(Point { x: at.x, y: at.y });
        return false;
    }
    // The main thread holds this lock only for short updates.
    let mut keys = shared.lock_keys();
    // SAFETY: the event is valid during the callback.
    let at = unsafe { ffi::CGEventGetLocation(event) };
    shared.cursor_at(Point { x: at.x, y: at.y });
    match kind {
        ffi::KEY_DOWN | ffi::KEY_UP => {
            let code = field(ffi::FIELD_KEYCODE);
            let down = kind == ffi::KEY_DOWN;
            if forwarding && down && code == KEYCODE_ESCAPE {
                // SAFETY: as above.
                let flags = unsafe { ffi::CGEventGetFlags(event) };
                if flags & RELEASE_FLAGS == RELEASE_FLAGS {
                    // IN-7: never forwarded.
                    shared.stop(CapturedEvent::ReleaseShortcut);
                    return true;
                }
            }
            let Some(key) = u16::try_from(code).ok().and_then(keys::from_mac) else {
                return keys.router.keyboard_in_portal();
            };
            let forwarded = route(shared, &mut keys, InputEvent::Key { key, down });
            if forwarded && down && key == Key::ESC && safety_exit(&mut keys) {
                shared.stop(CapturedEvent::SafetyExit);
            }
            forwarded
        }
        ffi::FLAGS_CHANGED => {
            // SAFETY: as above.
            let flags = unsafe { ffi::CGEventGetFlags(event) };
            let code = field(ffi::FIELD_KEYCODE);
            let mut forwarded = false;
            for input in modifier_events(&mut keys, code, flags) {
                forwarded |= route(shared, &mut keys, input);
            }
            forwarded
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
            match button {
                Some(button) => {
                    route(shared, &mut keys, InputEvent::Button { button, down });
                    false
                }
                None => shared.forward_now(),
            }
        }
        ffi::SCROLL_WHEEL => {
            // Point deltas: positive is up or left on macOS; the wire is the opposite.
            let dy = -(field(ffi::FIELD_SCROLL_POINT_DELTA_1) as f32);
            let dx = -(field(ffi::FIELD_SCROLL_POINT_DELTA_2) as f32);
            route(shared, &mut keys, InputEvent::Scroll { dx, dy })
        }
        // Gestures (Space swipes, Mission Control), system-defined events, and all
        // other types: in the portal, suppress and do not forward (IN-10, ADR 0003).
        _ => shared.forward_now(),
    }
}

/// Routes one input event (IN-3). Reports it if it goes to the target, and
/// returns true to suppress it.
fn route(shared: &Shared, keys: &mut KeyState, input: InputEvent) -> bool {
    let decision = keys.router.route(&input, shared.entered.load(SeqCst));
    if let Some(focused) = decision.keyboard {
        shared.keyboard_changed(focused);
    }
    if decision.route == Route::Forward {
        shared.push(CapturedEvent::Input(input));
        true
    } else {
        false
    }
}

/// Counts Escape presses in the callback (IN-8). True at the tenth press in 5 s.
fn safety_exit(keys: &mut KeyState) -> bool {
    let now = Instant::now();
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
fn modifier_events(keys: &mut KeyState, code: i64, flags: u64) -> Vec<InputEvent> {
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
    let held = keys.modifiers.contains(&key);
    match (down, held) {
        (true, false) => keys.modifiers.push(key),
        (false, true) => keys.modifiers.retain(|k| *k != key),
        _ => return Vec::new(),
    }
    vec![InputEvent::Key { key, down }]
}
