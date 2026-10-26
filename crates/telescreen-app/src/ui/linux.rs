//! Linux (Wayland) parts of the window layer (ADR 0006, UI-6).
//!
//! The capture on Linux is the portal window itself. The cursor is always free
//! (FLOW-20): the portal does not lock or hide it. While forwarding, the portal
//! inhibits compositor shortcuts and forwards the input that [`KeyRoutes`] sends
//! to the target. Pointer enter and leave of the window give the pause. Not tested
//! on hardware yet.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use telescreen_core::ids::DisplayToken;
use telescreen_core::input::{Button, InputEvent};
use telescreen_core::key_route::{KeyRoutes, Route};
use telescreen_core::keys::Key;
use telescreen_core::platform::{CapturedEvent, InputCapture, PlatformError};
use telescreen_core::pointer::{Point, Rect};
use telescreen_platform_linux::displays::LinuxDisplays;
use telescreen_platform_linux::inhibit::ShortcutInhibitor;
use telescreen_platform_linux::inject::LinuxInjector;
use telescreen_platform_linux::prompt::LayerPrompt;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::EventLoopProxy;
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::monitor::MonitorHandle;
use winit::platform::scancode::PhysicalKeyExtScancode;
use winit::window::{Window, WindowAttributes};

use super::prompt_queue::{PromptQueue, PromptRequest};
use super::{Parts, UserEvent};
use crate::controller::Labels;

/// IN-8: Escape this many times in this time.
const SAFETY_PRESSES: usize = 10;
const SAFETY_WINDOW: Duration = Duration::from_secs(5);
/// Logical pixels for one wheel line.
const LINE: f32 = 15.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Off,
    Armed,
    Forwarding,
}

struct CaptureState {
    window: Option<Arc<Window>>,
    mode: Mode,
    portal: Rect,
    /// The last cursor position in the window, in logical pixels.
    cursor: Option<Point>,
    /// The cursor is in the portal window, and the window has the keyboard focus.
    inside: bool,
    inhibitor: Option<ShortcutInhibitor>,
    modifiers: ModifiersState,
    escapes: VecDeque<Instant>,
    /// Where each key and button that is down went (IN-3).
    routes: KeyRoutes,
    events: Vec<CapturedEvent>,
}

impl CaptureState {
    fn global(&self, local: Point) -> Point {
        Point {
            x: self.portal.x + local.x,
            y: self.portal.y + local.y,
        }
    }

    fn cursor_global(&self) -> Point {
        self.global(self.cursor.unwrap_or_default())
    }

    fn forward_now(&self) -> bool {
        self.mode == Mode::Forwarding && self.inside
    }

    /// Reports the latest cursor position while forwarding. Merges motions.
    fn report_position(&mut self) {
        if !self.forward_now() {
            return;
        }
        let at = self.cursor_global();
        if let Some(CapturedEvent::PointerMoved { at: last }) = self.events.last_mut() {
            *last = at;
        } else {
            self.events.push(CapturedEvent::PointerMoved { at });
        }
    }

    fn set_inside(&mut self, inside: bool) {
        if inside == self.inside {
            return;
        }
        self.inside = inside;
        if self.mode == Mode::Off {
            return;
        }
        let event = if inside {
            CapturedEvent::PointerEntered {
                at: self.cursor_global(),
            }
        } else {
            CapturedEvent::PointerLeft
        };
        self.events.push(event);
        self.report_position();
    }

    /// Routes one input event (IN-3). Reports it and returns true if it goes to
    /// the target.
    fn route(&mut self, input: InputEvent) -> bool {
        let forward_now = self.forward_now();
        if self.routes.route(&input, forward_now) == Route::Forward {
            self.events.push(CapturedEvent::Input(input));
            true
        } else {
            false
        }
    }

    fn stop(&mut self, event: CapturedEvent) {
        self.inhibitor = None;
        self.mode = Mode::Off;
        self.routes.clear();
        self.events.push(event);
    }
}

/// The capture handle that the controller gets. The window layer feeds it events.
#[derive(Clone)]
struct LinuxCapture(Rc<RefCell<CaptureState>>);

impl InputCapture for LinuxCapture {
    fn arm(&mut self, portal: Rect) -> Result<(), PlatformError> {
        let mut s = self.0.borrow_mut();
        s.portal = portal;
        if s.mode == Mode::Off {
            s.mode = Mode::Armed;
        }
        if s.inside {
            let at = s.cursor_global();
            s.events.push(CapturedEvent::PointerEntered { at });
        }
        Ok(())
    }

    fn forward(&mut self) -> Result<(), PlatformError> {
        let mut s = self.0.borrow_mut();
        let window = s
            .window
            .clone()
            .ok_or_else(|| PlatformError::Failed("no portal window".into()))?;
        s.inhibitor = match ShortcutInhibitor::new(&*window) {
            Ok(inhibitor) => Some(inhibitor),
            Err(e) => {
                // Forwarding still works; compositor shortcuts stay with the compositor.
                eprintln!("telescreen: shortcut inhibit: {e}");
                None
            }
        };
        s.escapes.clear();
        s.mode = Mode::Forwarding;
        s.report_position();
        Ok(())
    }

    fn disarm(&mut self) {
        let mut s = self.0.borrow_mut();
        s.inhibitor = None;
        s.mode = Mode::Off;
        s.routes.clear();
    }
}

pub(crate) struct Hooks {
    capture: LinuxCapture,
    prompts: Option<PromptQueue>,
}

pub(crate) fn parts(_proxy: EventLoopProxy<UserEvent>) -> Result<Parts, String> {
    let mut notices = Vec::new();
    let capabilities = telescreen_platform_linux::capabilities();
    let injector: Option<Box<dyn telescreen_core::platform::Injector>> = match LinuxInjector::new()
    {
        Ok(injector) => Some(Box::new(injector)),
        Err(e) => {
            notices.push(format!("Input injection is not available: {e}"));
            None
        }
    };
    // UI-5: layer-shell where the compositor has it, else a normal window.
    let (prompt, prompts): (
        Box<dyn telescreen_view::PromptPresenter>,
        Option<PromptQueue>,
    ) = match LayerPrompt::start() {
        Ok(prompt) => (Box::new(prompt), None),
        Err(e) => {
            notices.push(format!(
                "No layer-shell prompt ({e}); the prompt is a normal window."
            ));
            let queue = PromptQueue::default();
            (Box::new(queue.clone()), Some(queue))
        }
    };
    let capture = LinuxCapture(Rc::new(RefCell::new(CaptureState {
        window: None,
        mode: Mode::Off,
        portal: Rect::default(),
        cursor: None,
        inside: false,
        inhibitor: None,
        modifiers: ModifiersState::empty(),
        escapes: VecDeque::new(),
        routes: KeyRoutes::new(),
        events: Vec::new(),
    })));
    Ok(Parts {
        displays: Box::new(LinuxDisplays),
        capture: Box::new(capture.clone()),
        injector,
        prompt,
        capabilities,
        labels: Labels {
            confirm_key: "Return".into(),
            release_shortcut: "Control+Alt+Shift+Escape".into(),
        },
        hooks: Hooks { capture, prompts },
        notices,
    })
}

/// The token of a monitor: the Wayland output name, as `LinuxDisplays` uses.
pub(crate) fn monitor_token(monitor: &MonitorHandle) -> Option<DisplayToken> {
    monitor.name().map(DisplayToken)
}

pub(crate) fn portal_attributes(attributes: WindowAttributes) -> WindowAttributes {
    attributes
}

pub(crate) fn prompt_created(_: &Window) {}

fn button(button: MouseButton) -> Option<Button> {
    Some(match button {
        MouseButton::Left => Button::Left,
        MouseButton::Right => Button::Right,
        MouseButton::Middle => Button::Middle,
        MouseButton::Back => Button::Back,
        MouseButton::Forward => Button::Forward,
        MouseButton::Other(_) => return None,
    })
}

impl Hooks {
    pub(crate) fn drain_captured(&mut self) -> Vec<CapturedEvent> {
        std::mem::take(&mut self.capture.0.borrow_mut().events)
    }

    /// The compositor removes the inhibitor when the app stops, so no watchdog
    /// is needed here.
    pub(crate) fn heartbeat(&mut self) {}

    pub(crate) fn take_prompt_requests(&mut self) -> Vec<PromptRequest> {
        self.prompts
            .as_ref()
            .map(PromptQueue::take)
            .unwrap_or_default()
    }

    pub(crate) fn portal_created(&mut self, window: &Arc<Window>) {
        let mut s = self.capture.0.borrow_mut();
        s.window = Some(window.clone());
        s.inside = false;
        s.cursor = None;
    }

    pub(crate) fn portal_closed(&mut self) {
        let mut s = self.capture.0.borrow_mut();
        s.inhibitor = None;
        s.window = None;
        s.mode = Mode::Off;
        s.routes.clear();
    }

    /// Without the keyboard focus the portal gets no keys: that is the same as a
    /// leave. The focus coming back with the cursor in the window is an entry.
    pub(crate) fn portal_focus(&mut self, _: &Window, focused: bool, _: bool) {
        let mut s = self.capture.0.borrow_mut();
        if focused {
            if s.cursor.is_some() {
                s.set_inside(true);
            }
        } else {
            s.set_inside(false);
            // The source pauses and the target releases all held state.
            s.routes.clear();
        }
    }

    /// Returns true if the event is input of the capture, so the UI must not get it.
    pub(crate) fn portal_window_event(&mut self, event: &WindowEvent) -> bool {
        let mut s = self.capture.0.borrow_mut();
        match event {
            WindowEvent::ModifiersChanged(modifiers) => {
                s.modifiers = modifiers.state();
                false
            }
            WindowEvent::CursorMoved { position, .. } => {
                let scale = s.window.as_ref().map_or(1.0, |w| w.scale_factor());
                let logical = position.to_logical::<f64>(scale);
                s.cursor = Some(Point {
                    x: logical.x,
                    y: logical.y,
                });
                s.report_position();
                false
            }
            WindowEvent::CursorEntered { .. } => {
                s.set_inside(true);
                false
            }
            WindowEvent::CursorLeft { .. } => {
                s.set_inside(false);
                s.cursor = None;
                false
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let down = event.state == ElementState::Pressed;
                let escape = event.physical_key == PhysicalKey::Code(KeyCode::Escape);
                let release = ModifiersState::CONTROL | ModifiersState::ALT | ModifiersState::SHIFT;
                if s.mode == Mode::Forwarding && escape && down && s.modifiers.contains(release) {
                    // IN-7: never forwarded.
                    s.stop(CapturedEvent::ReleaseShortcut);
                    return true;
                }
                let Some(code) = event
                    .physical_key
                    .to_scancode()
                    .and_then(|c| u16::try_from(c).ok())
                else {
                    return s.forward_now();
                };
                let forwarded = s.route(InputEvent::Key {
                    key: Key(code),
                    down,
                });
                if forwarded && escape && down && !event.repeat {
                    let now = Instant::now();
                    s.escapes.push_back(now);
                    while s
                        .escapes
                        .front()
                        .is_some_and(|t| now.duration_since(*t) > SAFETY_WINDOW)
                    {
                        s.escapes.pop_front();
                    }
                    if s.escapes.len() >= SAFETY_PRESSES {
                        s.stop(CapturedEvent::SafetyExit);
                    }
                }
                forwarded
            }
            WindowEvent::MouseInput {
                state, button: b, ..
            } => match button(*b) {
                Some(button) => s.route(InputEvent::Button {
                    button,
                    down: *state == ElementState::Pressed,
                }),
                None => s.forward_now(),
            },
            WindowEvent::MouseWheel { delta, .. } => {
                // winit: positive is up or left. The wire is the opposite.
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (-x * LINE, -y * LINE),
                    MouseScrollDelta::PixelDelta(p) => (-p.x as f32, -p.y as f32),
                };
                s.route(InputEvent::Scroll { dx, dy })
            }
            _ => false,
        }
    }
}

/// The displays and the capabilities.
pub(crate) fn describe() -> (
    Result<Vec<telescreen_core::platform::DisplayInfo>, PlatformError>,
    telescreen_core::platform::InputCapabilities,
) {
    use telescreen_core::platform::Displays;
    (
        LinuxDisplays.displays(),
        telescreen_platform_linux::capabilities(),
    )
}
