//! SPIKE (throwaway): move the pointer on niri without clicks or keys.
//!   vp      zwlr_virtual_pointer_v1 absolute motion, bound to the first output
//!   uinput  uinput relative pointer (REL_X/REL_Y + BTN_LEFT capability, never pressed)
//! Points stay inside 25%..75% of the output, so the pointer never reaches a corner.

use std::time::Duration;

use wayland_client::protocol::{wl_output, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
use wayland_protocols_wlr::virtual_pointer::v1::client::{
    zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1, zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
};

#[derive(Default)]
struct State {
    seat: Option<wl_seat::WlSeat>,
    output: Option<wl_output::WlOutput>,
    manager: Option<ZwlrVirtualPointerManagerV1>,
    mode: Option<(i32, i32)>,
}

impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(s: &mut Self, r: &wl_registry::WlRegistry, e: wl_registry::Event, _: &(), _: &Connection, qh: &QueueHandle<Self>) {
        if let wl_registry::Event::Global { name, interface, version } = e {
            match interface.as_str() {
                "wl_seat" if s.seat.is_none() => s.seat = Some(r.bind(name, version.min(7), qh, ())),
                "wl_output" if s.output.is_none() => s.output = Some(r.bind(name, version.min(4), qh, ())),
                "zwlr_virtual_pointer_manager_v1" => {
                    println!("virtual pointer manager version {version}");
                    s.manager = Some(r.bind(name, version.min(2), qh, ()));
                }
                _ => {}
            }
        }
    }
}
impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(s: &mut Self, _: &wl_output::WlOutput, e: wl_output::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {
        match e {
            wl_output::Event::Mode { width, height, .. } => s.mode = Some((width, height)),
            wl_output::Event::Name { name } => println!("output name {name}"),
            _ => {}
        }
    }
}
impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(_: &mut Self, _: &wl_seat::WlSeat, _: wl_seat::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<ZwlrVirtualPointerManagerV1, ()> for State {
    fn event(_: &mut Self, _: &ZwlrVirtualPointerManagerV1, _: <ZwlrVirtualPointerManagerV1 as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}
impl Dispatch<ZwlrVirtualPointerV1, ()> for State {
    fn event(_: &mut Self, _: &ZwlrVirtualPointerV1, _: <ZwlrVirtualPointerV1 as wayland_client::Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
}

const POINTS: [(f64, f64); 5] = [(0.5, 0.5), (0.75, 0.5), (0.75, 0.75), (0.25, 0.75), (0.5, 0.5)];

fn virtual_pointer() {
    run_vp(&POINTS, false, 1500);
}

/// Move through `points` (fractions of the output). If `click`, press and release BTN_LEFT at the last point.
fn run_vp(points: &[(f64, f64)], click: bool, dwell_ms: u64) {
    let conn = Connection::connect_to_env().expect("wayland connect");
    let mut q = conn.new_event_queue();
    let qh = q.handle();
    conn.display().get_registry(&qh, ());
    let mut s = State::default();
    q.roundtrip(&mut s).unwrap();
    q.roundtrip(&mut s).unwrap();
    let mgr = s.manager.clone().expect("no zwlr_virtual_pointer_manager_v1");
    let vp = if mgr.version() >= 2 {
        mgr.create_virtual_pointer_with_output(s.seat.as_ref(), s.output.as_ref(), &qh, ())
    } else {
        mgr.create_virtual_pointer(s.seat.as_ref(), &qh, ())
    };
    // Extent in arbitrary units; the compositor scales it to the output (or the layout for v1).
    let (w, h) = (10000u32, 10000u32);
    println!("output mode {:?}; extent {w}x{h}", s.mode);
    for (i, (x, y)) in points.iter().enumerate() {
        vp.motion_absolute(i as u32 * 1000, (x * w as f64) as u32, (y * h as f64) as u32, w, h);
        vp.frame();
        q.roundtrip(&mut s).unwrap();
        println!("vp: moved to {:.0}% , {:.0}%", x * 100.0, y * 100.0);
        std::thread::sleep(Duration::from_millis(dwell_ms));
    }
    if click {
        const BTN_LEFT: u32 = 0x110;
        vp.button(90_000, BTN_LEFT, wayland_client::protocol::wl_pointer::ButtonState::Pressed);
        vp.frame();
        q.roundtrip(&mut s).unwrap();
        std::thread::sleep(Duration::from_millis(80));
        vp.button(90_080, BTN_LEFT, wayland_client::protocol::wl_pointer::ButtonState::Released);
        vp.frame();
        q.roundtrip(&mut s).unwrap();
        println!("vp: clicked BTN_LEFT at the last point");
    }
    vp.destroy();
    q.roundtrip(&mut s).unwrap();
}

fn uinput() {
    use evdev::uinput::VirtualDevice;
    use evdev::{AttributeSet, EventType, InputEvent, KeyCode, RelativeAxisCode};
    let mut dev = VirtualDevice::builder()
        .unwrap()
        .name("telescreen-spike-pointer")
        .with_keys(&AttributeSet::from_iter([KeyCode::BTN_LEFT, KeyCode::BTN_RIGHT]))
        .unwrap()
        .with_relative_axes(&AttributeSet::from_iter([RelativeAxisCode::REL_X, RelativeAxisCode::REL_Y]))
        .unwrap()
        .build()
        .unwrap();
    println!("uinput device created; wait 1 s for the compositor");
    std::thread::sleep(Duration::from_secs(1));
    for (label, dx) in [("right", 10), ("left", -10)] {
        for _ in 0..30 {
            dev.emit(&[InputEvent::new(EventType::RELATIVE.0, RelativeAxisCode::REL_X.0, dx)]).unwrap();
            std::thread::sleep(Duration::from_millis(16));
        }
        println!("uinput: moved 300 units {label}");
        std::thread::sleep(Duration::from_millis(1000));
    }
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("vp") => virtual_pointer(),
        Some("uinput") => uinput(),
        Some(m @ ("points" | "click")) => {
            // points/click x,y x,y ...  (fractions 0..1)
            let pts: Vec<(f64, f64)> = std::env::args()
                .skip(2)
                .map(|a| {
                    let (x, y) = a.split_once(',').unwrap();
                    (x.parse().unwrap(), y.parse().unwrap())
                })
                .collect();
            for &(x, y) in &pts {
                assert!(!(x < 0.1 && y < 0.1), "refuse: top-left corner opens the niri overview");
            }
            run_vp(&pts, m == "click", 2000);
        }
        _ => eprintln!("usage: spike-linux-inject vp|uinput"),
    }
}
