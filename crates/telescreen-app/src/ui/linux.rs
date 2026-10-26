//! Linux (Wayland) parts of the window layer (ADR 0006, UI-6).
//!
//! The capture on Linux is the portal window itself: while forwarding it locks the
//! pointer, reads relative motion, and inhibits compositor shortcuts. Wayland does
//! not let a client move the pointer, so a pause unlocks the pointer with a
//! position hint at the portal edge. Not tested on hardware yet.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use telescreen_core::ids::DisplayToken;
use telescreen_core::input::{Button, InputEvent};
use telescreen_core::keys::Key;
use telescreen_core::platform::{CapturedEvent, InputCapture, PlatformError};
use telescreen_core::pointer::{Point, Rect};
use telescreen_platform_linux::displays::LinuxDisplays;
use telescreen_platform_linux::inhibit::ShortcutInhibitor;
use telescreen_platform_linux::inject::LinuxInjector;
use telescreen_platform_linux::prompt::LayerPrompt;
use winit::dpi::LogicalPosition;
use winit::event::{DeviceEvent, ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::EventLoopProxy;
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::monitor::MonitorHandle;
use winit::platform::scancode::PhysicalKeyExtScancode;
use winit::window::{CursorGrabMode, Window, WindowAttributes};

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
    inside: bool,
    inhibitor: Option<ShortcutInhibitor>,
    modifiers: ModifiersState,
    escapes: VecDeque<Instant>,
    events: Vec<CapturedEvent>,
}

/// The capture handle that the controller gets. The window layer feeds it events.
#[derive(Clone)]
struct LinuxCapture(Rc<RefCell<CaptureState>>);

impl LinuxCapture {
    fn global(&self, local: Point) -> Point {
        let s = self.0.borrow();
        Point {
            x: s.portal.x + local.x,
            y: s.portal.y + local.y,
        }
    }

    fn unlock(state: &mut CaptureState) {
        state.inhibitor = None;
        if let Some(window) = &state.window {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            window.set_cursor_visible(true);
        }
    }
}

impl InputCapture for LinuxCapture {
    fn arm(&mut self, portal: Rect) -> Result<(), PlatformError> {
        let mut s = self.0.borrow_mut();
        s.portal = portal;
        if s.mode == Mode::Off {
            s.mode = Mode::Armed;
        }
        if s.inside {
            let at = s.cursor.unwrap_or_default();
            let at = Point {
                x: portal.x + at.x,
                y: portal.y + at.y,
            };
            s.events.push(CapturedEvent::PointerEntered { at });
        }
        Ok(())
    }

    fn forward(&mut self) -> Result<Point, PlatformError> {
        let at = self.global(self.0.borrow().cursor.unwrap_or_default());
        let mut s = self.0.borrow_mut();
        let window = s
            .window
            .clone()
            .ok_or_else(|| PlatformError::Failed("no portal window".into()))?;
        window
            .set_cursor_grab(CursorGrabMode::Locked)
            .map_err(|e| PlatformError::Unsupported(format!("pointer lock: {e}")))?;
        window.set_cursor_visible(false);
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
        Ok(at)
    }

    fn pause(&mut self, at: Option<Point>) {
        let mut s = self.0.borrow_mut();
        if let (Some(at), Some(window)) = (at, &s.window) {
            // The hint must be in the surface: put it at the nearest portal edge.
            let local = s.portal.clamp(at);
            let hint = LogicalPosition::new(local.x - s.portal.x, local.y - s.portal.y);
            let _ = window.set_cursor_position(hint);
        }
        Self::unlock(&mut s);
        if s.mode == Mode::Forwarding {
            s.mode = Mode::Armed;
        }
        // The pointer is at the edge, still in the window. A new entry needs a leave.
        s.inside = true;
    }

    fn disarm(&mut self) {
        let mut s = self.0.borrow_mut();
        Self::unlock(&mut s);
        s.mode = Mode::Off;
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

    /// The compositor removes the pointer lock and the inhibitor when the app
    /// stops, so no watchdog is needed here.
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
        LinuxCapture::unlock(&mut s);
        s.window = None;
        s.mode = Mode::Off;
    }

    pub(crate) fn portal_focus(&mut self, _: &Window, focused: bool, _: bool) {
        let mut s = self.capture.0.borrow_mut();
        if !focused && s.mode == Mode::Forwarding {
            // The compositor ended the pointer lock with the focus. Pause.
            LinuxCapture::unlock(&mut s);
            s.mode = Mode::Armed;
            s.inside = false;
            s.events.push(CapturedEvent::PointerLeft);
        }
    }

    /// Returns true if the event is input of the capture, so the UI must not get it.
    pub(crate) fn portal_window_event(&mut self, event: &WindowEvent) -> bool {
        let mut s = self.capture.0.borrow_mut();
        match event {
            WindowEvent::ModifiersChanged(modifiers) => s.modifiers = modifiers.state(),
            WindowEvent::CursorMoved { position, .. } => {
                let scale = s.window.as_ref().map_or(1.0, |w| w.scale_factor());
                let logical = position.to_logical::<f64>(scale);
                s.cursor = Some(Point {
                    x: logical.x,
                    y: logical.y,
                });
            }
            WindowEvent::CursorEntered { .. } => {
                if s.mode == Mode::Armed && !s.inside {
                    let local = s.cursor.unwrap_or_default();
                    let at = Point {
                        x: s.portal.x + local.x,
                        y: s.portal.y + local.y,
                    };
                    s.events.push(CapturedEvent::PointerEntered { at });
                }
                s.inside = true;
            }
            WindowEvent::CursorLeft { .. } => s.inside = false,
            _ => {}
        }
        if s.mode != Mode::Forwarding {
            return false;
        }
        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                let down = event.state == ElementState::Pressed;
                let escape = event.physical_key == PhysicalKey::Code(KeyCode::Escape);
                let release = ModifiersState::CONTROL | ModifiersState::ALT | ModifiersState::SHIFT;
                if escape && down && s.modifiers.contains(release) {
                    // IN-7: never forwarded.
                    LinuxCapture::unlock(&mut s);
                    s.mode = Mode::Off;
                    s.events.push(CapturedEvent::ReleaseShortcut);
                    return true;
                }
                if let Some(code) = event
                    .physical_key
                    .to_scancode()
                    .and_then(|c| u16::try_from(c).ok())
                {
                    s.events.push(CapturedEvent::Input(InputEvent::Key {
                        key: Key(code),
                        down,
                    }));
                }
                if escape && down && !event.repeat {
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
                        LinuxCapture::unlock(&mut s);
                        s.mode = Mode::Off;
                        s.events.push(CapturedEvent::SafetyExit);
                    }
                }
            }
            WindowEvent::MouseInput {
                state, button: b, ..
            } => {
                if let Some(button) = button(*b) {
                    let down = *state == ElementState::Pressed;
                    s.events
                        .push(CapturedEvent::Input(InputEvent::Button { button, down }));
                }
            }
            WindowEvent::MouseWheel { delta, .. } => {
                // winit: positive is up or left. The wire is the opposite.
                let (dx, dy) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (-x * LINE, -y * LINE),
                    MouseScrollDelta::PixelDelta(p) => (-p.x as f32, -p.y as f32),
                };
                s.events
                    .push(CapturedEvent::Input(InputEvent::Scroll { dx, dy }));
            }
            _ => return false,
        }
        true
    }

    /// Relative motion while the pointer is locked.
    pub(crate) fn device_event(&mut self, event: &DeviceEvent) -> bool {
        let mut s = self.capture.0.borrow_mut();
        if let (Mode::Forwarding, DeviceEvent::MouseMotion { delta: (dx, dy) }) = (s.mode, event) {
            s.events.push(CapturedEvent::Motion { dx: *dx, dy: *dy });
            return true;
        }
        false
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
