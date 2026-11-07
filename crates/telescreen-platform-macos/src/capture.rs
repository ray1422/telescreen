//! Input capture and suppression on the source with a CGEventTap at the HID
//! location (spike 2, ADR 0003, IN-1..IN-10).
//!
//! The tap runs on its own thread with its own run loop. The callback only reads
//! the event, decides to suppress it, and pushes a [`CapturedEvent`] to a bounded
//! queue (IN-2). The callback itself handles the hand back shortcut and the safety
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
use telescreen_core::ids::PortalId;
use telescreen_core::input::{Button, InputEvent, ScrollPhase};
use telescreen_core::key_route::{FocusChange, InputRouter, Route};
use telescreen_core::keys::{self, Key};
use telescreen_core::platform::{CapturedEvent, InputCapture, PlatformError};
use telescreen_core::pointer::{Point, Rect};

use crate::ffi::{self, CGEventRef};

/// The queue size. A full queue ends the session (IN-3).
const QUEUE: usize = 4096;
/// The number of portals that the capture can watch at the same time (FLOW-25).
/// Each portal is on a different display.
const SLOTS: usize = 4;
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

/// The capture state of one portal. The callback reads it with no lock (IN-2).
#[derive(Default)]
struct Slot {
    /// The portal id, or 0 while the slot is free.
    id: AtomicU64,
    mode: AtomicU8,
    guarded: AtomicBool,
    /// The portal bounds as f64 bits: x, y, width, height.
    area: [AtomicU64; 4],
    /// The portal is on screen (FLOW-26). While it is not, the portal area does
    /// not count as the portal.
    visible: AtomicBool,
    /// The cursor is in the portal area while the portal is not on screen.
    over_hidden: AtomicBool,
    /// The cursor is in the portal.
    entered: AtomicBool,
    /// The latest cursor position as f64 bits, for the merged `PointerMoved`.
    moved: [AtomicU64; 2],
    /// A `PointerMoved` is in the queue and not drained yet.
    move_queued: AtomicBool,
}

struct Shared {
    start: Instant,
    slots: [Slot; SLOTS],
    secure_input: AtomicBool,
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
    /// Routes and native keyboard focus of all portals (IN-3, FLOW-22, FLOW-25).
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
        let modes: Vec<_> = self
            .shared
            .slots
            .iter()
            .map(|slot| (slot.id.load(SeqCst), slot.mode.load(SeqCst)))
            .collect();
        f.debug_struct("MacCapture").field("slots", &modes).finish()
    }
}

impl MacCapture {
    /// Starts the tap thread and the watchdog thread. `wake` is called (from those
    /// threads) when events are in the queue; it must not block.
    pub fn start(wake: impl Fn() + Send + Sync + 'static) -> Result<Self, PlatformError> {
        let (tx, rx) = bounded(QUEUE);
        let shared = Arc::new(Shared {
            start: Instant::now(),
            slots: Default::default(),
            secure_input: AtomicBool::new(false),
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
    /// `PointerMoved` gets the latest cursor position of its portal.
    pub fn drain(&self) -> Vec<CapturedEvent> {
        let s = &self.shared;
        s.woken.store(false, SeqCst);
        for slot in &s.slots {
            slot.move_queued.store(false, SeqCst);
        }
        let mut events: Vec<_> = self
            .rx
            .try_iter()
            .map(|event| match event {
                CapturedEvent::PointerMoved { portal, at } => CapturedEvent::PointerMoved {
                    portal,
                    at: s.slot(portal).map_or(at, Slot::moved),
                },
                other => other,
            })
            .collect();
        if self.shared.overflow.swap(false, SeqCst) {
            events.push(CapturedEvent::Overflow);
        }
        events
    }

    /// True while a portal forwards.
    pub fn is_forwarding(&self) -> bool {
        self.shared.forwarding_any()
    }
}

impl InputCapture for MacCapture {
    fn arm(&mut self, portal: PortalId, area: Rect) -> Result<(), PlatformError> {
        let s = &self.shared;
        let slot = match s.slot(portal) {
            Some(slot) => slot,
            None => {
                let slot = s
                    .slots
                    .iter()
                    .find(|slot| slot.id.load(SeqCst) == 0)
                    .ok_or_else(|| {
                        PlatformError::Unsupported(format!("more than {SLOTS} portals"))
                    })?;
                slot.visible.store(true, SeqCst);
                slot.id.store(portal.0, SeqCst);
                slot
            }
        };
        for (cell, value) in slot
            .area
            .iter()
            .zip([area.x, area.y, area.width, area.height])
        {
            cell.store(value.to_bits(), SeqCst);
        }
        if slot.mode.load(SeqCst) == OFF {
            slot.mode.store(ARMED, SeqCst);
        }
        let at = ffi::cursor_position();
        s.cursor_at(slot, Point { x: at.x, y: at.y });
        Ok(())
    }

    fn forward(&mut self, portal: PortalId) -> Result<(), PlatformError> {
        let s = &self.shared;
        let Some(slot) = s.slot(portal) else {
            return Err(PlatformError::Failed(format!("{portal} is not armed")));
        };
        slot.guarded.store(false, SeqCst);
        if crate::secure_event_input() {
            return Err(PlatformError::Permission(
                "Secure Event Input is on (a password field has the focus). macOS blocks the capture of keys."
                    .into(),
            ));
        }
        s.secure_input.store(false, SeqCst);
        s.beat();
        s.lock_keys().escapes.clear();
        slot.mode.store(FORWARDING, SeqCst);
        let at = ffi::cursor_position();
        s.cursor_at(slot, Point { x: at.x, y: at.y });
        Ok(())
    }

    fn set_portal_focus(&mut self, portal: PortalId, focused: bool) {
        let s = &self.shared;
        if s.slot(portal).is_none() {
            return;
        }
        let mut keys = s.lock_keys();
        let change = keys.router.set_window_focus(portal, focused);
        s.keyboard_changed(change);
    }

    fn set_portal_visible(&mut self, portal: PortalId, visible: bool) {
        let s = &self.shared;
        let Some(slot) = s.slot(portal) else {
            return;
        };
        slot.visible.store(visible, SeqCst);
        let at = ffi::cursor_position();
        s.cursor_at(slot, Point { x: at.x, y: at.y });
    }

    fn guard(&mut self, portal: PortalId, on: bool) -> Result<(), PlatformError> {
        let s = &self.shared;
        let slot = s.slot(portal).filter(|slot| slot.mode.load(SeqCst) != OFF);
        match slot {
            Some(slot) => {
                slot.guarded.store(on, SeqCst);
                Ok(())
            }
            None if on => Err(PlatformError::Failed(
                "the portal guard needs an armed capture".into(),
            )),
            None => Ok(()),
        }
    }

    fn disarm(&mut self, portal: PortalId) {
        let s = &self.shared;
        let Some(slot) = s.slot(portal) else {
            return;
        };
        slot.mode.store(OFF, SeqCst);
        slot.guarded.store(false, SeqCst);
        slot.entered.store(false, SeqCst);
        slot.visible.store(true, SeqCst);
        slot.over_hidden.store(false, SeqCst);
        slot.move_queued.store(false, SeqCst);
        {
            let mut keys = s.lock_keys();
            keys.router.forget(portal);
            if s.slots.iter().all(|other| other.mode.load(SeqCst) == OFF) {
                keys.router = InputRouter::new();
                keys.modifiers.clear();
            }
        }
        slot.id.store(0, SeqCst);
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

impl Slot {
    fn portal(&self) -> PortalId {
        PortalId(self.id.load(SeqCst))
    }

    fn active(&self) -> bool {
        self.mode.load(SeqCst) != OFF
    }

    fn forwarding(&self) -> bool {
        self.mode.load(SeqCst) == FORWARDING
    }

    fn area(&self) -> Rect {
        let v = |i: usize| f64::from_bits(self.area[i].load(SeqCst));
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

    /// The slot of an armed portal.
    fn slot(&self, portal: PortalId) -> Option<&Slot> {
        if portal.0 == 0 {
            return None;
        }
        self.slots
            .iter()
            .find(|slot| slot.id.load(SeqCst) == portal.0)
    }

    /// The slot of a forwarding portal.
    fn forwarding_slot(&self, portal: PortalId) -> Option<&Slot> {
        self.slot(portal).filter(|slot| slot.forwarding())
    }

    fn any_active(&self) -> bool {
        self.slots.iter().any(Slot::active)
    }

    fn forwarding_any(&self) -> bool {
        self.slots.iter().any(Slot::forwarding)
    }

    /// The forwarding portal under the cursor.
    fn cursor_in(&self) -> Option<PortalId> {
        self.slots
            .iter()
            .find(|slot| slot.forwarding() && slot.entered.load(SeqCst))
            .map(Slot::portal)
    }

    /// True while the input at the cursor goes to a target.
    fn forward_now(&self) -> bool {
        self.cursor_in().is_some()
    }

    /// FLOW-26: while a portal forwards and is not on screen, input in its area
    /// goes to neither computer. Pointer motion still goes to macOS.
    fn blocked_now(&self) -> bool {
        self.slots
            .iter()
            .any(|slot| slot.forwarding() && slot.over_hidden.load(SeqCst))
    }

    /// True if a portal waits for the confirmation (FLOW-21).
    fn guarded_any(&self) -> bool {
        self.slots
            .iter()
            .any(|slot| slot.active() && slot.guarded.load(SeqCst))
    }

    /// True if `at` is in the area of an armed portal.
    fn in_a_portal(&self, at: Point) -> bool {
        self.slots
            .iter()
            .any(|slot| slot.active() && slot.area().contains(at))
    }

    /// Reports keyboard focus before the input that caused it (FLOW-23).
    fn keyboard_changed(&self, change: FocusChange) {
        if let Some(portal) = change.left {
            self.push(CapturedEvent::KeyboardLeft { portal });
        }
        if let Some(portal) = change.entered {
            self.push(CapturedEvent::KeyboardEntered { portal });
        }
    }

    /// The cursor is at `at`: report a crossing of the edge of the portal of the
    /// slot (FLOW-14), and the position while forwarding in the portal.
    fn cursor_at(&self, slot: &Slot, at: Point) {
        let portal = slot.portal();
        if portal.0 == 0 {
            return;
        }
        let in_area = slot.area().contains(at);
        let visible = slot.visible.load(SeqCst);
        let over_hidden = in_area && !visible;
        let was_over_hidden = slot.over_hidden.swap(over_hidden, SeqCst);
        let inside = in_area && visible;
        if inside != slot.entered.swap(inside, SeqCst) {
            self.push(if inside {
                CapturedEvent::PointerEntered { portal, at }
            } else {
                CapturedEvent::PointerLeft { portal }
            });
        }
        // FLOW-26: while the portal is hidden, the position in the portal area
        // and the move out of it go to the warning on the target.
        let report = inside || over_hidden || was_over_hidden;
        if report && slot.forwarding() {
            slot.moved[0].store(at.x.to_bits(), SeqCst);
            slot.moved[1].store(at.y.to_bits(), SeqCst);
            if !slot.move_queued.swap(true, SeqCst) {
                self.push(CapturedEvent::PointerMoved { portal, at });
            }
        }
    }

    /// The cursor is at `at`: update each forwarding portal.
    fn cursor_at_all(&self, at: Point) {
        for slot in self.slots.iter().filter(|slot| slot.forwarding()) {
            self.cursor_at(slot, at);
        }
    }

    /// Stops suppression of all portals from the callback or the watchdog, and
    /// reports why.
    fn stop(&self, event: CapturedEvent) {
        self.stop_all();
        self.push(event);
    }

    fn stop_all(&self) {
        for slot in &self.slots {
            slot.mode.store(OFF, SeqCst);
        }
    }

    fn push(&self, event: CapturedEvent) {
        match self.tx.try_send(event) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                // IN-3: a lost event ends the sessions. Stop suppression now.
                self.overflow.store(true, SeqCst);
                self.stop_all();
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
        if !shared.forwarding_any() {
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
        let forwarding = shared.forwarding_any();
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
    if !shared.any_active() {
        return event;
    }
    if handle(shared, kind, event) {
        std::ptr::null_mut()
    } else {
        event
    }
}

/// Handles one event while a portal is armed or forwards. Returns true to
/// suppress it.
fn handle(shared: &Shared, kind: u32, event: CGEventRef) -> bool {
    // SAFETY (for the event reads below): the event is valid during the callback.
    let field = |f: u32| unsafe { ffi::CGEventGetIntegerValueField(event, f) };
    let forwarding = shared.forwarding_any();
    let motion = matches!(
        kind,
        ffi::MOUSE_MOVED | ffi::LEFT_DRAGGED | ffi::RIGHT_DRAGGED | ffi::OTHER_DRAGGED
    );
    if motion {
        if forwarding {
            // SAFETY: the event is valid during the callback.
            let at = unsafe { ffi::CGEventGetLocation(event) };
            shared.cursor_at_all(Point { x: at.x, y: at.y });
        }
        return false;
    }
    let guarded = shared.guarded_any();
    // IN-8: the safety exit works also while a portal waits for the
    // confirmation (FLOW-21). While forwarding, `forward` counts it.
    if !forwarding
        && guarded
        && kind == ffi::KEY_DOWN
        && field(ffi::FIELD_KEYCODE) == KEYCODE_ESCAPE
        && safety_exit(&mut shared.lock_keys())
    {
        shared.stop(CapturedEvent::SafetyExit);
        return false;
    }
    let forwarded = forwarding && forward(shared, kind, event, &field);
    forwarded || (guarded && guard_suppresses(shared, kind, event))
}

/// FLOW-21: while a portal waits for the confirmation, suppress the shortcuts
/// that switch Spaces or applications, the clicks out of all portals, and the
/// gestures.
fn guard_suppresses(shared: &Shared, kind: u32, event: CGEventRef) -> bool {
    match kind {
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
            !shared.in_a_portal(Point { x: at.x, y: at.y })
        }
        _ => true,
    }
}

/// Handles one event while a portal forwards. Returns true to suppress it.
fn forward(shared: &Shared, kind: u32, event: CGEventRef, field: &dyn Fn(u32) -> i64) -> bool {
    // The main thread holds this lock only for short updates.
    let mut keys = shared.lock_keys();
    // SAFETY: the event is valid during the callback.
    let at = unsafe { ffi::CGEventGetLocation(event) };
    shared.cursor_at_all(Point { x: at.x, y: at.y });
    match kind {
        ffi::KEY_DOWN | ffi::KEY_UP => {
            let code = field(ffi::FIELD_KEYCODE);
            let down = kind == ffi::KEY_DOWN;
            if down && code == KEYCODE_ESCAPE {
                // SAFETY: as above.
                let flags = unsafe { ffi::CGEventGetFlags(event) };
                if flags & RELEASE_FLAGS == RELEASE_FLAGS
                    && let Some(portal) = hand_back_portal(shared, &keys)
                {
                    // IN-7: never forwarded. Only this portal stops.
                    if let Some(slot) = shared.slot(portal) {
                        slot.mode.store(OFF, SeqCst);
                    }
                    shared.push(CapturedEvent::HandBackShortcut { portal });
                    return true;
                }
            }
            let Some(key) = u16::try_from(code).ok().and_then(keys::from_mac) else {
                return keys
                    .router
                    .keyboard()
                    .is_some_and(|portal| shared.forwarding_slot(portal).is_some());
            };
            let forwarded = route(shared, &mut keys, InputEvent::Key { key, down });
            // IN-8: count each Escape while forwarding, also when the key stays
            // on macOS, for example while the portal has no focus (FLOW-26).
            if down && key == Key::ESC && safety_exit(&mut keys) {
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
                // IN-17: a forwarded button does not reach the source.
                Some(button) => {
                    route(shared, &mut keys, InputEvent::Button { button, down })
                        || shared.blocked_now()
                }
                None => shared.forward_now() || shared.blocked_now(),
            }
        }
        ffi::SCROLL_WHEEL => {
            // Point deltas: positive is up or left on macOS; the wire is the opposite.
            let dy = -(field(ffi::FIELD_SCROLL_POINT_DELTA_1) as f32);
            let dx = -(field(ffi::FIELD_SCROLL_POINT_DELTA_2) as f32);
            let phase = scroll_phase(
                field(ffi::FIELD_SCROLL_PHASE),
                field(ffi::FIELD_MOMENTUM_PHASE),
            );
            route(shared, &mut keys, InputEvent::Scroll { dx, dy, phase }) || shared.blocked_now()
        }
        // Gestures (Space swipes, Mission Control), system-defined events, and all
        // other types: in the portal, suppress and do not forward (IN-10, ADR 0003).
        _ => shared.forward_now() || shared.blocked_now(),
    }
}

/// The portal of the hand back shortcut (IN-7, FLOW-25): the forwarding portal
/// with the keyboard focus, else the only forwarding portal.
fn hand_back_portal(shared: &Shared, keys: &KeyState) -> Option<PortalId> {
    if let Some(portal) = keys.router.keyboard()
        && shared.forwarding_slot(portal).is_some()
    {
        return Some(portal);
    }
    let mut forwarding = shared.slots.iter().filter(|slot| slot.forwarding());
    match (forwarding.next(), forwarding.next()) {
        (Some(slot), None) => Some(slot.portal()),
        _ => None,
    }
}

/// The phase of a scroll from the trackpad fields (IN-18). A mouse wheel has
/// neither phase. The gesture end is not the end of the scroll, because momentum
/// can follow: with no momentum, the target ends the scroll after
/// `SCROLL_IDLE_END`.
fn scroll_phase(scroll: i64, momentum: i64) -> ScrollPhase {
    // CGMomentumScrollPhase: 1 begin, 2 continue, 3 end.
    match momentum {
        1 | 2 => return ScrollPhase::Update,
        3 => return ScrollPhase::End,
        _ => {}
    }
    // CGScrollPhase: 1 began, 2 changed, 4 ended, 8 cancelled, 128 may begin.
    match scroll {
        0 => ScrollPhase::None,
        1 => ScrollPhase::Begin,
        8 => ScrollPhase::End,
        _ => ScrollPhase::Update,
    }
}

/// Routes one input event (IN-3). Reports it if it goes to the target of a
/// forwarding portal, and returns true to suppress it. A key goes to the portal
/// with the keyboard focus; while that portal does not forward (it waits for the
/// confirmation, FLOW-21), the key and its up event stay on macOS.
fn route(shared: &Shared, keys: &mut KeyState, input: InputEvent) -> bool {
    let decision = keys.router.route(&input, shared.cursor_in());
    if let Some(portal) = decision.focus_wanted {
        shared.push(CapturedEvent::FocusWanted { portal });
    }
    match decision.route {
        Route::Forward(portal) if shared.forwarding_slot(portal).is_some() => {
            shared.push(CapturedEvent::Input {
                portal,
                event: input,
            });
            true
        }
        _ => false,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in18_the_scroll_ends_after_the_momentum() {
        assert_eq!(scroll_phase(0, 0), ScrollPhase::None, "a mouse wheel");
        assert_eq!(scroll_phase(128, 0), ScrollPhase::Update, "may begin");
        assert_eq!(scroll_phase(1, 0), ScrollPhase::Begin);
        assert_eq!(scroll_phase(2, 0), ScrollPhase::Update);
        assert_eq!(
            scroll_phase(4, 0),
            ScrollPhase::Update,
            "momentum can follow"
        );
        assert_eq!(scroll_phase(0, 1), ScrollPhase::Update);
        assert_eq!(scroll_phase(0, 2), ScrollPhase::Update);
        assert_eq!(scroll_phase(0, 3), ScrollPhase::End);
        assert_eq!(scroll_phase(8, 0), ScrollPhase::End, "cancelled");
    }
}
