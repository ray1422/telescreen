//! Spike 6 (ADR 0012): can a cxx-qt window give its `wl_surface` to the keyboard
//! shortcuts inhibit protocol (UI-6)?
//!
//! The Rust `main` starts `QGuiApplication` through cxx-qt-lib and loads a QML
//! window. A Qt timer calls `tick`. `tick` gets the `wl_surface` of the window,
//! requests the inhibitor on the Wayland connection of Qt, and logs the `active`
//! and `inactive` events of the compositor. The script also hides and shows the
//! window (Qt makes a new surface) and releases and requests the inhibitor again
//! (the UI-6 sequence).
//!
//! `HOLD=<seconds>` keeps the inhibitor after the script, so that a user can press
//! a compositor shortcut and see that it does not work.

use std::cell::RefCell;
use std::time::Instant;

use cxx_qt_lib::{QByteArray, QGuiApplication, QQmlApplicationEngine, QUrl};
use wayland_backend::client::{Backend, ObjectId};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat::WlSeat, wl_surface::WlSurface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::wp::keyboard_shortcuts_inhibit::zv1::client::{
    zwp_keyboard_shortcuts_inhibit_manager_v1::ZwpKeyboardShortcutsInhibitManagerV1,
    zwp_keyboard_shortcuts_inhibitor_v1::{self, ZwpKeyboardShortcutsInhibitorV1},
};

#[cxx_qt::bridge]
mod ffi {
    unsafe extern "C++" {
        include!("shim.h");
        fn ts_platform_is_wayland() -> bool;
        fn ts_wl_display() -> usize;
        fn ts_window_surface() -> usize;
        fn ts_window_active() -> bool;
        fn ts_set_window_visible(visible: bool);
        fn ts_request_activate();
        fn ts_start_timer(ms: i32);
        fn ts_quit();
    }
}

const QML: &str = r##"
import QtQuick
import QtQuick.Window
Window {
    visible: true
    width: 520
    height: 160
    title: "Telescreen spike 6: shortcut inhibit"
    color: "#202530"
    Text {
        anchors.centerIn: parent
        color: "white"
        text: "Telescreen spike 6. This window closes itself."
    }
}
"##;

fn log(start: Instant, msg: impl AsRef<str>) {
    println!("[{:6.2}] {}", start.elapsed().as_secs_f64(), msg.as_ref());
}

#[derive(Default)]
struct State {
    /// Events from the inhibitor, in sequence.
    events: Vec<&'static str>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(_: &mut Self, _: &WlSeat, _: <WlSeat as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

impl Dispatch<ZwpKeyboardShortcutsInhibitManagerV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwpKeyboardShortcutsInhibitManagerV1,
        _: <ZwpKeyboardShortcutsInhibitManagerV1 as Proxy>::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

impl Dispatch<ZwpKeyboardShortcutsInhibitorV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwpKeyboardShortcutsInhibitorV1,
        event: zwp_keyboard_shortcuts_inhibitor_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        state.events.push(match event {
            zwp_keyboard_shortcuts_inhibitor_v1::Event::Active => "active",
            zwp_keyboard_shortcuts_inhibitor_v1::Event::Inactive => "inactive",
            _ => "unknown",
        });
    }
}

struct Spike {
    start: Instant,
    connection: Connection,
    queue: EventQueue<State>,
    state: State,
    manager: ZwpKeyboardShortcutsInhibitManagerV1,
    seat: WlSeat,
    inhibitor: Option<(usize, ZwpKeyboardShortcutsInhibitorV1)>,
    window_active: Option<bool>,
    step: usize,
    hold: f64,
}

impl Spike {
    fn new(start: Instant) -> Self {
        let display = ffi::ts_wl_display();
        assert!(display != 0, "no wl_display");
        // SAFETY: Qt owns the wl_display and keeps it open while the application lives.
        let backend = unsafe { Backend::from_foreign_display(display as *mut _) };
        let connection = Connection::from_backend(backend);
        let (globals, queue) = registry_queue_init::<State>(&connection).expect("registry");
        let qh = queue.handle();
        let manager: ZwpKeyboardShortcutsInhibitManagerV1 =
            globals.bind(&qh, 1..=1, ()).expect("no zwp_keyboard_shortcuts_inhibit_manager_v1");
        let seat: WlSeat = globals.bind(&qh, 1..=7, ()).expect("no wl_seat");
        log(start, "bound the inhibit manager and a wl_seat on the Qt connection");
        let hold = std::env::var("HOLD").ok().and_then(|v| v.parse().ok()).unwrap_or(0.0);
        Self {
            start,
            connection,
            queue,
            state: State::default(),
            manager,
            seat,
            inhibitor: None,
            window_active: None,
            step: 0,
            hold,
        }
    }

    fn inhibit(&mut self) {
        let ptr = ffi::ts_window_surface();
        if ptr == 0 {
            log(self.start, "no surface yet");
            return;
        }
        // SAFETY: Qt owns the surface; the spike destroys the inhibitor before it
        // asks Qt to hide the window.
        let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), ptr as *mut _) }.expect("wl_surface");
        let surface = WlSurface::from_id(&self.connection, id).expect("wl_surface proxy");
        let inhibitor = self.manager.inhibit_shortcuts(&surface, &self.seat, &self.queue.handle(), ());
        log(self.start, format!("inhibit_shortcuts on surface {ptr:#x}"));
        self.inhibitor = Some((ptr, inhibitor));
    }

    fn release(&mut self) {
        if let Some((_, inhibitor)) = self.inhibitor.take() {
            inhibitor.destroy();
            log(self.start, "released the inhibitor");
        }
    }

    fn tick(&mut self) {
        let _ = self.connection.flush();
        if let Err(e) = self.queue.dispatch_pending(&mut self.state) {
            log(self.start, format!("dispatch error: {e}"));
        }
        for event in self.state.events.drain(..) {
            log(self.start, format!("inhibitor event: {event}"));
        }
        let active = ffi::ts_window_active();
        if self.window_active != Some(active) {
            log(self.start, format!("QWindow::isActive = {active}"));
            self.window_active = Some(active);
        }
        let t = self.start.elapsed().as_secs_f64();
        // (time, action) in sequence; each runs one time.
        let script: [(f64, fn(&mut Spike)); 7] = [
            (1.0, |s| s.inhibit()),
            (4.0, |s| {
                s.release();
                ffi::ts_set_window_visible(false);
                log(s.start, "hid the window");
            }),
            (5.0, |s| {
                ffi::ts_set_window_visible(true);
                ffi::ts_request_activate();
                log(s.start, "showed the window");
            }),
            (6.0, |s| s.inhibit()),
            (9.0, |s| s.release()),
            (10.0, |s| s.inhibit()),
            (13.0, |s| {
                if s.hold == 0.0 {
                    s.release();
                }
            }),
        ];
        while self.step < script.len() && t >= script[self.step].0 {
            (script[self.step].1)(self);
            self.step += 1;
        }
        if let Some((ptr, _)) = &self.inhibitor {
            let now = ffi::ts_window_surface();
            if now != *ptr {
                log(self.start, format!("surface changed: {ptr:#x} -> {now:#x}"));
            }
        }
        if t >= 13.5 + self.hold {
            self.release();
            let _ = self.connection.flush();
            log(self.start, "quit");
            ffi::ts_quit();
        }
    }
}

thread_local! {
    static SPIKE: RefCell<Option<Spike>> = const { RefCell::new(None) };
    static START: Instant = Instant::now();
}

/// Qt timer callback (see `ts_start_timer` in shim.cpp).
#[unsafe(no_mangle)]
extern "C" fn ts_tick() {
    SPIKE.with_borrow_mut(|spike| {
        let start = START.with(|s| *s);
        spike.get_or_insert_with(|| Spike::new(start)).tick();
    });
}

fn main() {
    let start = START.with(|s| *s);
    let mut app = QGuiApplication::new();
    log(start, format!("platform is wayland: {}", ffi::ts_platform_is_wayland()));
    assert!(ffi::ts_platform_is_wayland(), "set QT_QPA_PLATFORM=wayland");
    let mut engine = QQmlApplicationEngine::new();
    if let Some(engine) = engine.as_mut() {
        engine.load_data(&QByteArray::from(QML), &QUrl::default());
    }
    ffi::ts_start_timer(100);
    if let Some(app) = app.as_mut() {
        app.exec();
    }
    // Drop the Wayland proxies before Qt closes its wl_display.
    drop(SPIKE.with_borrow_mut(Option::take));
    drop(engine);
    drop(app);
    log(start, "clean exit");
}
