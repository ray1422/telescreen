//! SPIKE (throwaway): can a native fullscreen Space on the external display hide the
//! menu bar and Dock completely, and which window events show a Space change?
//!
//! Run: cargo run --release -- <display-name-part> [plain|level]
//!   level: after fullscreen, raise the window above the menu bar level.
//! Quit: press the physical Q key, or wait 600 s.

use std::num::NonZeroU32;
use std::rc::Rc;
use std::time::{Duration, Instant};

use objc2::MainThreadMarker;
use objc2_app_kit::{NSAllRomanInputSourcesLocaleIdentifier, NSApplication, NSMenu, NSView};
use objc2_foundation::NSArray;
use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use winit::application::ApplicationHandler;
use winit::event::{ElementState, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::platform::macos::{WindowAttributesExtMacOS, WindowExtMacOS};
use winit::window::{Fullscreen, Window, WindowId};

struct App {
    want: String,
    raise: bool,
    raised: bool,
    window: Option<Rc<Window>>,
    surface: Option<softbuffer::Surface<Rc<Window>, Rc<Window>>>,
    started: Instant,
    last_poll: Instant,
    last_move: Instant,
}

fn log(t0: Instant, msg: impl AsRef<str>) {
    println!("[{:7.2}s] {}", t0.elapsed().as_secs_f64(), msg.as_ref());
}

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn TISCopyCurrentKeyboardInputSource() -> *const std::ffi::c_void;
    fn TISGetInputSourceProperty(src: *const std::ffi::c_void, key: core_foundation::string::CFStringRef) -> core_foundation::string::CFStringRef;
    static kTISPropertyInputSourceID: core_foundation::string::CFStringRef;
}

fn current_input_source() -> String {
    use core_foundation::base::TCFType;
    unsafe {
        let src = TISCopyCurrentKeyboardInputSource();
        let id = TISGetInputSourceProperty(src, kTISPropertyInputSourceID);
        let s = core_foundation::string::CFString::wrap_under_get_rule(id).to_string();
        core_foundation::base::CFRelease(src);
        s
    }
}

fn ns_view(w: &Window) -> &NSView {
    let RawWindowHandle::AppKit(h) = w.window_handle().unwrap().as_raw() else { unreachable!() };
    unsafe { h.ns_view.cast::<NSView>().as_ref() }
}

impl App {
    fn on_focus(&mut self, focused: bool) {
        let Some(w) = self.window.clone() else { return };
        let view = ns_view(&w);
        if focused {
            // Like a password field: only Roman (ASCII-capable) input sources while focused.
            if let Some(ctx) = view.inputContext() {
                let locales = NSArray::from_slice(&[unsafe { NSAllRomanInputSourcesLocaleIdentifier }]);
                ctx.setAllowedInputSourceLocales(Some(&locales));
                log(self.started, "allowedInputSourceLocales = Roman only");
            }
        }
        if focused && self.raise && !self.raised {
            let win = view.window().unwrap();
            // NSMainMenuWindowLevel is 24. Put the portal above the menu bar and the fullscreen title bar.
            win.setLevel(26);
            self.raised = true;
            log(self.started, format!("window level -> {}", win.level()));
        }
    }

    fn poll_presentation(&mut self) {
        let mtm = MainThreadMarker::new().unwrap();
        let app = NSApplication::sharedApplication(mtm);
        let opts = app.presentationOptions();
        let menu_visible = NSMenu::menuBarVisible(mtm);
        log(self.started, format!("presentationOptions=0x{:x} menuBarVisible={menu_visible} active={} inputSource={}", opts.0, app.isActive(), current_input_source()));
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, el: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        for m in el.available_monitors() {
            log(self.started, format!("monitor {:?} pos={:?} size={:?} scale={}", m.name(), m.position(), m.size(), m.scale_factor()));
        }
        let monitor = el
            .available_monitors()
            .find(|m| m.name().is_some_and(|n| n.contains(&self.want)))
            .expect("display not found");
        let attrs = Window::default_attributes()
            .with_title("telescreen portal spike")
            .with_position(monitor.position())
            .with_borderless_game(true)
            .with_fullscreen(Some(Fullscreen::Borderless(Some(monitor.clone()))));
        let window = Rc::new(el.create_window(attrs).unwrap());
        log(self.started, format!("window created, borderless_game={}", window.is_borderless_game()));
        let ctx = softbuffer::Context::new(window.clone()).unwrap();
        self.surface = Some(softbuffer::Surface::new(&ctx, window.clone()).unwrap());
        self.window = Some(window);
    }

    fn window_event(&mut self, el: &ActiveEventLoop, _: WindowId, event: WindowEvent) {
        let t0 = self.started;
        match event {
            WindowEvent::RedrawRequested => {
                let (Some(w), Some(s)) = (&self.window, &mut self.surface) else { return };
                let size = w.inner_size();
                let (Some(width), Some(height)) = (NonZeroU32::new(size.width), NonZeroU32::new(size.height)) else { return };
                s.resize(width, height).unwrap();
                let mut buf = s.buffer_mut().unwrap();
                buf.fill(0x0020_2830);
                buf.present().unwrap();
            }
            WindowEvent::Focused(f) => {
                log(t0, format!("Focused({f})"));
                self.on_focus(f);
            }
            WindowEvent::Occluded(o) => log(t0, format!("Occluded({o})")),
            WindowEvent::Resized(s) => log(t0, format!("Resized({s:?})")),
            WindowEvent::Moved(p) => log(t0, format!("Moved({p:?})")),
            WindowEvent::CursorEntered { .. } => log(t0, "CursorEntered"),
            WindowEvent::CursorLeft { .. } => log(t0, "CursorLeft"),
            WindowEvent::CursorMoved { position, .. } => {
                if self.last_move.elapsed() > Duration::from_millis(500) {
                    self.last_move = Instant::now();
                    log(t0, format!("CursorMoved({:.0}, {:.0})", position.x, position.y));
                }
            }
            WindowEvent::KeyboardInput { event, .. } if event.state == ElementState::Pressed => {
                log(t0, format!("KeyPressed logical={:?} physical={:?}", event.logical_key, event.physical_key));
                match event.physical_key {
                    PhysicalKey::Code(KeyCode::KeyQ) => el.exit(),
                    PhysicalKey::Code(KeyCode::Enter) => log(t0, "  -> confirmation key seen by the window"),
                    _ => {}
                }
            }
            WindowEvent::CloseRequested => el.exit(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, el: &ActiveEventLoop) {
        if self.started.elapsed() > Duration::from_secs(600) {
            log(self.started, "timeout, exit");
            el.exit();
        }
        if self.last_poll.elapsed() > Duration::from_secs(2) {
            self.last_poll = Instant::now();
            self.poll_presentation();
        }
        el.set_control_flow(ControlFlow::WaitUntil(Instant::now() + Duration::from_millis(250)));
    }
}

fn main() {
    let want = std::env::args().nth(1).unwrap_or_else(|| "Desktop".into());
    let raise = std::env::args().nth(2).as_deref() == Some("level");
    let el = EventLoop::new().unwrap();
    let now = Instant::now();
    let mut app = App { want, raise, raised: false, window: None, surface: None, started: now, last_poll: now, last_move: now };
    el.run_app(&mut app).unwrap();
}
