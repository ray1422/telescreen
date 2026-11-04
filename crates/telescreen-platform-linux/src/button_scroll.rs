//! Button scroll on KWin (IN-15, spike 5).
//!
//! Applications animate a scroll from a wheel. KWin sends no other scroll source
//! from a client, so this device uses the button scroll of libinput: KWin then
//! sends the source `continuous`, and applications follow each event.
//!
//! The device has no wheel and has a middle button, so libinput enables button
//! scroll for it by default. While button scroll is active, libinput changes the
//! motion of the device to scroll, and no client sees the button.
//!
//! libinput has these rules (libinput 1.32, `src/evdev.c`, `src/evdev-fallback.c`):
//!
//! - It discards the motion for 200 ms after the button goes down.
//! - If the button goes up before a motion, it sends a click of the middle button.
//!   A click can paste text. Thus the button goes up only after [`arm`].
//! - The first scroll has a threshold of 5 counts. A scroll on the other axis has
//!   a threshold of 5 counts in one event. They end only when the button goes up.
//! - A change of the scroll method takes effect only when no button is down.
//!
//! KWin shows a scroll cursor for each scroll from a device while its setting
//! `scrollOnButtonDown` is on (KWin 6.8, `src/pointer_input.cpp`). The cursor stays
//! until the button goes up. KWin reads its own copy of the setting. Thus [`arm`]
//! turns the setting off while the button is down: libinput continues the button
//! scroll, and KWin shows no scroll cursor. When the button goes up, libinput has
//! no button scroll, so [`press`] must turn the setting on before each button down.
//!
//! [`arm`]: ButtonScroll::arm
//! [`press`]: ButtonScroll::press

use std::io;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use evdev::uinput::VirtualDevice;
use evdev::{AttributeSet, BusType, EventType, InputId, KeyCode, RelativeAxisCode};
use telescreen_core::input::whole_units;
use zbus::blocking::{Connection, Proxy, proxy::Builder};
use zbus::proxy::CacheProperties;

/// The name of the device. KWin stores the scroll factor for this name.
const NAME: &str = "Telescreen scroll";

const BTN_MIDDLE: u16 = 0x112;

/// The time from the button down to the first motion. It is longer than the
/// timeout of libinput.
const ARM_WAIT: Duration = Duration::from_millis(250);

/// The thresholds of libinput.
const ARM_COUNTS: i32 = 5;

/// Device counts for one logical pixel of a scroll. Whole counts give the steps of
/// a slow scroll, so one pixel has more than one count.
const COUNTS_PER_PIXEL: f32 = 4.0;

/// The scroll factor of KWin for this device. Clients take 10 axis units as one
/// wheel notch. One notch is 60 pixels, as for the wheel scroll, thus one pixel is
/// 1/6 axis unit.
const KWIN_SCROLL_FACTOR: f64 = 1.0 / 24.0;

const KWIN: &str = "org.kde.KWin";
const KWIN_DEVICE: &str = "org.kde.KWin.InputDevice";

/// The time that KWin can use to add the device.
const KWIN_WAIT: Duration = Duration::from_secs(5);
const KWIN_RETRY: Duration = Duration::from_millis(20);

#[derive(Debug, Clone, Copy, PartialEq)]
enum Step {
    ButtonDown,
    ButtonUp,
    Arm,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Hold {
    Up,
    Waiting(Instant),
    Armed,
}

/// The state of the button. It has no device, so that tests can run it.
#[derive(Debug)]
struct Button {
    hold: Hold,
    /// A session started, so the button must be down.
    wanted: bool,
}

impl Button {
    fn new() -> Self {
        Self {
            hold: Hold::Up,
            wanted: false,
        }
    }

    /// A new session starts. libinput can lose the button state between two
    /// sessions (a VT change or a restart of the compositor), thus press it again.
    fn start_session(&mut self, now: Instant) -> Vec<Step> {
        self.wanted = true;
        match self.hold {
            Hold::Armed => {
                self.hold = Hold::Waiting(now);
                vec![Step::ButtonUp, Step::ButtonDown]
            }
            Hold::Up | Hold::Waiting(_) => Vec::new(),
        }
    }

    /// The next step at this time, if one is due.
    fn poll(&mut self, ready: bool, now: Instant) -> Option<Step> {
        match self.hold {
            Hold::Up if self.wanted && ready => {
                self.hold = Hold::Waiting(now);
                Some(Step::ButtonDown)
            }
            Hold::Waiting(since) if now.duration_since(since) >= ARM_WAIT => {
                self.hold = Hold::Armed;
                Some(Step::Arm)
            }
            Hold::Up | Hold::Waiting(_) | Hold::Armed => None,
        }
    }

    fn armed(&self) -> bool {
        self.hold == Hold::Armed
    }

    /// The device closes. Returns the time to wait, and the steps after this time.
    fn finish(&mut self, now: Instant) -> (Duration, Vec<Step>) {
        let result = match self.hold {
            Hold::Up => (Duration::ZERO, Vec::new()),
            Hold::Waiting(since) => (
                ARM_WAIT.saturating_sub(now.duration_since(since)),
                vec![Step::Arm, Step::ButtonUp],
            ),
            Hold::Armed => (Duration::ZERO, vec![Step::ButtonUp]),
        };
        self.hold = Hold::Up;
        self.wanted = false;
        result
    }
}

/// The scroll device of the uinput backend.
pub(crate) struct ButtonScroll {
    device: VirtualDevice,
    button: Button,
    /// The device in KWin, when KWin has the device and the scroll factor is set.
    kwin: Arc<OnceLock<Proxy<'static>>>,
    /// KWin refused button scroll. The caller uses the wheel.
    refused: bool,
    /// The counts that the last scrolls did not inject (IN-14).
    rest: (f32, f32),
}

impl ButtonScroll {
    pub(crate) fn new() -> io::Result<Self> {
        let mut device = scroll_device()?;
        let node = device
            .enumerate_dev_nodes_blocking()?
            .next()
            .ok_or_else(|| io::Error::other("the scroll device has no node"))??;
        let node = node
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| io::Error::other("the scroll device node has no name"))?
            .to_owned();
        let kwin = Arc::new(OnceLock::new());
        let thread_kwin = Arc::clone(&kwin);
        std::thread::Builder::new()
            .name("kwin-scroll".into())
            .spawn(move || {
                if let Some(proxy) = configure_kwin(&node) {
                    let _ = thread_kwin.set(proxy);
                    eprintln!("telescreen: scroll: button scroll on KWin");
                } else {
                    eprintln!("telescreen: scroll: no button scroll on KWin, use the wheel");
                }
            })?;
        Ok(Self {
            device,
            button: Button::new(),
            kwin,
            refused: false,
            rest: (0.0, 0.0),
        })
    }

    fn ready(&self) -> bool {
        self.kwin.get().is_some() && !self.refused
    }

    pub(crate) fn start_session(&mut self) -> io::Result<()> {
        let steps = self.button.start_session(Instant::now());
        self.run(&steps)?;
        self.poll()
    }

    /// Does the step that is due. The injector calls this for each pointer motion
    /// and each scroll, so no thread is necessary.
    pub(crate) fn poll(&mut self) -> io::Result<()> {
        let ready = self.ready();
        match self.button.poll(ready, Instant::now()) {
            Some(step) => self.run(&[step]),
            None => Ok(()),
        }
    }

    /// Injects a scroll. Returns `false` if this device is not in use: the caller
    /// then uses the wheel.
    pub(crate) fn scroll(&mut self, dx: f32, dy: f32) -> io::Result<bool> {
        self.poll()?;
        if !self.ready() {
            return Ok(false);
        }
        self.rest.0 += dx * COUNTS_PER_PIXEL;
        self.rest.1 += dy * COUNTS_PER_PIXEL;
        if self.button.armed() {
            self.flush()?;
        }
        Ok(true)
    }

    /// Injects the whole counts that wait.
    fn flush(&mut self) -> io::Result<()> {
        let x = whole_units(0.0, &mut self.rest.0);
        let y = whole_units(0.0, &mut self.rest.1);
        if x == 0 && y == 0 {
            return Ok(());
        }
        self.motion(x, y)
    }

    fn motion(&mut self, x: i32, y: i32) -> io::Result<()> {
        let mut events = Vec::with_capacity(2);
        for (axis, value) in [(RelativeAxisCode::REL_X, x), (RelativeAxisCode::REL_Y, y)] {
            if value != 0 {
                events.push(evdev::InputEvent::new(EventType::RELATIVE.0, axis.0, value));
            }
        }
        self.device.emit(&events)
    }

    fn run(&mut self, steps: &[Step]) -> io::Result<()> {
        for step in steps {
            match step {
                Step::ButtonDown => self.press()?,
                Step::ButtonUp => self.emit_button(false)?,
                Step::Arm => self.arm()?,
            }
        }
        Ok(())
    }

    fn emit_button(&mut self, down: bool) -> io::Result<()> {
        self.device.emit(&[evdev::InputEvent::new(
            EventType::KEY.0,
            BTN_MIDDLE,
            i32::from(down),
        )])
    }

    /// Turns on button scroll and presses the button. Without button scroll, a
    /// button down is a real middle button. Thus if KWin does not confirm button
    /// scroll, there is no button down, and the caller uses the wheel from now on.
    fn press(&mut self) -> io::Result<()> {
        let confirmed = self.kwin.get().is_some_and(|kwin| {
            enable_button_scroll(kwin).unwrap_or_else(|e| {
                eprintln!("telescreen: scroll: KWin: {e}");
                false
            })
        });
        if !confirmed {
            eprintln!("telescreen: scroll: no button scroll on KWin, use the wheel");
            self.refused = true;
            self.button = Button::new();
            self.rest = (0.0, 0.0);
            return Ok(());
        }
        self.emit_button(true)
    }

    /// Hides the scroll cursor of KWin, then goes past the thresholds on both axes.
    /// The two motions give a net scroll of zero. After them, each count is a scroll,
    /// and the button up sends no click.
    ///
    /// The button is down for [`ARM_WAIT`] before this call. This gives KWin the time
    /// to read the button down: libinput must see it while button scroll is on.
    fn arm(&mut self) -> io::Result<()> {
        if let Some(kwin) = self.kwin.get()
            && let Err(e) = kwin.set_property("scrollOnButtonDown", false)
        {
            eprintln!("telescreen: scroll: KWin: {e}");
        }
        self.motion(ARM_COUNTS, ARM_COUNTS)?;
        self.motion(-ARM_COUNTS, -ARM_COUNTS)?;
        self.flush()
    }
}

impl Drop for ButtonScroll {
    fn drop(&mut self) {
        let (wait, steps) = self.button.finish(Instant::now());
        std::thread::sleep(wait);
        let _ = self.run(&steps);
    }
}

fn scroll_device() -> io::Result<VirtualDevice> {
    // The left button makes it a mouse for udev. The virtual bus keeps it from the
    // KDE setting that turns off the touchpad when an external mouse is present.
    let buttons: AttributeSet<KeyCode> = [0x110, 0x111, BTN_MIDDLE]
        .into_iter()
        .map(KeyCode::new)
        .collect();
    let axes: AttributeSet<RelativeAxisCode> = [RelativeAxisCode::REL_X, RelativeAxisCode::REL_Y]
        .into_iter()
        .collect();
    VirtualDevice::builder()?
        .name(NAME)
        .input_id(InputId::new(BusType::BUS_VIRTUAL, 0x1234, 0x5679, 1))
        .with_keys(&buttons)?
        .with_relative_axes(&axes)?
        .build()
}

/// Waits for KWin to add the device, and sets its scroll factor. Returns `None` if
/// this is not possible, for example with a different compositor.
fn configure_kwin(node: &str) -> Option<Proxy<'static>> {
    let connection = Connection::session().ok()?;
    let path = format!("/org/kde/KWin/InputDevice/{node}");
    let deadline = Instant::now() + KWIN_WAIT;
    while Instant::now() < deadline {
        if let Ok(Some(proxy)) = try_configure_kwin(&connection, &path) {
            return Some(proxy);
        }
        std::thread::sleep(KWIN_RETRY);
    }
    None
}

fn try_configure_kwin(connection: &Connection, path: &str) -> zbus::Result<Option<Proxy<'static>>> {
    let proxy: Proxy<'static> = Builder::new(connection)
        .destination(KWIN)?
        .path(path.to_owned())?
        .interface(KWIN_DEVICE)?
        .cache_properties(CacheProperties::No)
        .build()?;
    let supported: bool = proxy.get_property("supportsScrollOnButtonDown")?;
    if !supported {
        return Ok(None);
    }
    proxy.set_property("scrollFactor", KWIN_SCROLL_FACTOR)?;
    Ok(Some(proxy))
}

/// Turns on button scroll. Returns `true` if KWin confirms it for the middle button.
fn enable_button_scroll(kwin: &Proxy<'static>) -> zbus::Result<bool> {
    kwin.set_property("scrollOnButtonDown", true)?;
    let on: bool = kwin.get_property("scrollOnButtonDown")?;
    let button: u32 = kwin.get_property("scrollButton")?;
    Ok(on && button == u32::from(BTN_MIDDLE))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn after(start: Instant, ms: u64) -> Instant {
        start + Duration::from_millis(ms)
    }

    #[test]
    fn in15_the_button_goes_down_only_in_a_session_and_when_kwin_is_ready() {
        let start = Instant::now();
        let mut button = Button::new();
        assert_eq!(button.poll(true, start), None, "no session");
        assert_eq!(button.start_session(start), []);
        assert_eq!(button.poll(false, start), None, "KWin is not ready");
        assert_eq!(button.poll(true, start), Some(Step::ButtonDown));
    }

    #[test]
    fn in15_the_motion_waits_for_the_timeout_of_libinput() {
        let start = Instant::now();
        let mut button = Button::new();
        button.start_session(start);
        button.poll(true, start);
        assert_eq!(button.poll(true, after(start, 249)), None);
        assert!(!button.armed());
        assert_eq!(button.poll(true, after(start, 250)), Some(Step::Arm));
        assert!(button.armed());
        assert_eq!(button.poll(true, after(start, 900)), None);
    }

    #[test]
    fn in15_a_new_session_presses_the_button_again() {
        let start = Instant::now();
        let mut button = Button::new();
        button.start_session(start);
        button.poll(true, start);
        assert_eq!(button.start_session(after(start, 10)), [], "not armed yet");
        button.poll(true, after(start, 250));

        let next = after(start, 5000);
        assert_eq!(
            button.start_session(next),
            [Step::ButtonUp, Step::ButtonDown]
        );
        assert!(!button.armed());
        assert_eq!(button.poll(true, after(next, 250)), Some(Step::Arm));
    }

    #[test]
    fn in15_the_button_never_goes_up_before_a_motion() {
        let start = Instant::now();
        let mut button = Button::new();
        assert_eq!(button.finish(start), (Duration::ZERO, Vec::new()));

        button.start_session(start);
        button.poll(true, start);
        assert_eq!(
            button.finish(after(start, 100)),
            (Duration::from_millis(150), vec![Step::Arm, Step::ButtonUp]),
        );

        let mut button = Button::new();
        button.start_session(start);
        button.poll(true, start);
        button.poll(true, after(start, 250));
        assert_eq!(
            button.finish(after(start, 300)),
            (Duration::ZERO, vec![Step::ButtonUp])
        );
    }

    #[test]
    fn in15_one_notch_is_60_pixels() {
        let axis_units = 60.0 * f64::from(COUNTS_PER_PIXEL) * KWIN_SCROLL_FACTOR;
        assert!((axis_units - 10.0).abs() < 1e-9);
    }
}
