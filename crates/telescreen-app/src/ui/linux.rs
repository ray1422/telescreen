//! Linux (Wayland) parts of the window layer (ADR 0006, UI-6).
//!
//! The capture on Linux is the portal window itself. The cursor is always free
//! (FLOW-20): the portal does not lock or hide it. While forwarding, the portal
//! inhibits compositor shortcuts and forwards the input that [`InputRouter`] sends
//! to the target. The session has no pause. Keys follow the native keyboard focus
//! of the portal window (FLOW-22, UI-6). Not tested on hardware yet.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use telescreen_core::config::{Computer, Quirk};
use telescreen_core::ids::{DisplayToken, PortalId};
use telescreen_core::input::{Button, InputEvent, ScrollPhase};
use telescreen_core::key_route::{InputRouter, Route};
use telescreen_core::keys::Key;
use telescreen_core::platform::{CapturedEvent, InputCapture, PlatformError};
use telescreen_core::pointer::{Point, Rect};
use telescreen_core::portal_capture::{self, FocusLoss, SafetyExit};
use telescreen_platform_linux::clipboard::LinuxClipboard;
use telescreen_platform_linux::displays::LinuxDisplays;
use telescreen_platform_linux::focus::LinuxFocusHolder;
use telescreen_platform_linux::inhibit::ShortcutInhibitor;
use telescreen_platform_linux::inject::LinuxInjector;
use telescreen_platform_linux::prompt::LayerPrompt;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, TouchPhase, WindowEvent};
use winit::event_loop::EventLoopProxy;
use winit::keyboard::{KeyCode, ModifiersState, PhysicalKey};
use winit::monitor::MonitorHandle;
use winit::platform::scancode::PhysicalKeyExtScancode;
use winit::platform::wayland::WindowAttributesExtWayland;
use winit::window::{Window, WindowAttributes};

use super::prompt_queue::{PromptQueue, PromptRequest};
use super::{Parts, UserEvent};
use crate::controller::Labels;

/// Logical pixels for one wheel line.
const LINE: f32 = 15.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Off,
    Armed,
    Forwarding,
}

/// The capture state of one portal window (FLOW-25).
struct PortalState {
    window: Option<Arc<Window>>,
    mode: Mode,
    area: Rect,
    /// The last cursor position in the window, in logical pixels.
    cursor: Option<Point>,
    /// The cursor is in the portal window (FLOW-20).
    inside: bool,
    inhibitor: Option<ShortcutInhibitor>,
}

impl PortalState {
    fn new() -> Self {
        Self {
            window: None,
            mode: Mode::Off,
            area: Rect::default(),
            cursor: None,
            inside: false,
            inhibitor: None,
        }
    }

    fn cursor_global(&self) -> Point {
        let local = self.cursor.unwrap_or_default();
        Point {
            x: self.area.x + local.x,
            y: self.area.y + local.y,
        }
    }

    fn forward_now(&self) -> bool {
        self.mode == Mode::Forwarding && self.inside
    }

    /// Keeps the inhibitor across the guard and forwarding (FLOW-21, UI-6).
    fn inhibit(&mut self) -> Result<(), PlatformError> {
        if self.inhibitor.is_none() {
            let window = self
                .window
                .as_ref()
                .ok_or_else(|| PlatformError::Failed("no portal window".into()))?;
            self.inhibitor = Some(ShortcutInhibitor::new(&**window)?);
        }
        Ok(())
    }

    fn stop(&mut self) {
        self.inhibitor = None;
        self.mode = Mode::Off;
    }
}

/// The capture of all portal windows. Each portal window gives the input in
/// it; keys go to the window with the native focus (FLOW-22, FLOW-25).
struct CaptureState {
    portals: BTreeMap<PortalId, PortalState>,
    modifiers: ModifiersState,
    /// IN-8: the Escape presses.
    escapes: SafetyExit,
    /// Where each key and button that is down went, for all portals (IN-3).
    router: InputRouter,
    events: Vec<CapturedEvent>,
}

impl CaptureState {
    fn forwarding(&self, portal: PortalId) -> bool {
        self.portals
            .get(&portal)
            .is_some_and(|p| p.mode == Mode::Forwarding)
    }

    /// Reports the latest cursor position while forwarding. Merges motions.
    fn report_position(&mut self, portal: PortalId) {
        let Some(state) = self.portals.get(&portal).filter(|p| p.forward_now()) else {
            return;
        };
        let at = state.cursor_global();
        if let Some(CapturedEvent::PointerMoved {
            portal: last,
            at: last_at,
        }) = self.events.last_mut()
            && *last == portal
        {
            *last_at = at;
        } else {
            self.events.push(CapturedEvent::PointerMoved { portal, at });
        }
    }

    fn set_inside(&mut self, portal: PortalId, inside: bool) {
        let Some(state) = self.portals.get_mut(&portal) else {
            return;
        };
        if inside == state.inside {
            return;
        }
        state.inside = inside;
        if state.mode == Mode::Off {
            return;
        }
        let event = if inside {
            CapturedEvent::PointerEntered {
                portal,
                at: state.cursor_global(),
            }
        } else {
            CapturedEvent::PointerLeft { portal }
        };
        self.events.push(event);
        self.report_position(portal);
    }

    /// Routes one input event of the portal window (IN-3). Reports it and
    /// returns true if it goes to a target. A key goes to the portal with the
    /// keyboard focus; while that portal does not forward, the key stays local.
    fn route(&mut self, portal: PortalId, input: InputEvent) -> bool {
        let cursor_in = self
            .portals
            .get(&portal)
            .filter(|p| p.forward_now())
            .map(|_| portal);
        // The compositor focuses the portal for the click, and `portal_focus`
        // reports it (FLOW-22). Thus `focus_wanted` needs nothing here.
        let decision = self.router.route(&input, cursor_in);
        match decision.route {
            Route::Forward(to) if self.forwarding(to) => {
                self.events.push(CapturedEvent::Input {
                    portal: to,
                    event: input,
                });
                true
            }
            _ => false,
        }
    }

    /// Reports keyboard focus before input (FLOW-23). Releases held input when
    /// native keyboard focus is lost (IN-13, UI-6).
    fn portal_focus(&mut self, portal: PortalId, focused: bool) {
        if !self.portals.contains_key(&portal) {
            return;
        }
        let portals = &self.portals;
        let events = portal_capture::window_focus(
            &mut self.router,
            portal,
            focused,
            |p| portals.get(&p).is_some_and(|p| p.mode == Mode::Forwarding),
            FocusLoss::UpEventsLost,
        );
        self.events.extend(events);
    }

    /// Stops one portal, for the hand back shortcut (IN-7).
    fn stop_portal(&mut self, portal: PortalId) {
        if let Some(state) = self.portals.get_mut(&portal) {
            state.stop();
        }
        self.router.forget(portal);
        self.events.push(CapturedEvent::HandBackShortcut { portal });
    }

    /// Stops all portals, for the safety exit (IN-8).
    fn stop_all(&mut self, event: CapturedEvent) {
        for state in self.portals.values_mut() {
            state.stop();
        }
        self.router.clear();
        self.events.push(event);
    }

    /// Forgets a portal that has no window and no capture.
    fn drop_unused(&mut self, portal: PortalId) {
        if self
            .portals
            .get(&portal)
            .is_some_and(|p| p.window.is_none() && p.mode == Mode::Off)
        {
            self.portals.remove(&portal);
        }
    }
}

/// The capture handle that the controller gets. The window layer feeds it events.
#[derive(Clone)]
struct LinuxCapture(Rc<RefCell<CaptureState>>);

impl InputCapture for LinuxCapture {
    fn arm(&mut self, portal: PortalId, area: Rect) -> Result<(), PlatformError> {
        let mut s = self.0.borrow_mut();
        let state = s.portals.entry(portal).or_insert_with(PortalState::new);
        state.area = area;
        if state.mode == Mode::Off {
            state.mode = Mode::Armed;
        }
        if state.inside {
            let at = state.cursor_global();
            s.events.push(CapturedEvent::PointerEntered { portal, at });
        }
        Ok(())
    }

    fn forward(&mut self, portal: PortalId) -> Result<(), PlatformError> {
        let mut s = self.0.borrow_mut();
        let Some(state) = s.portals.get_mut(&portal).filter(|p| p.mode != Mode::Off) else {
            return Err(PlatformError::Failed(format!("{portal} is not armed")));
        };
        if state.window.is_none() {
            return Err(PlatformError::Failed("no portal window".into()));
        }
        if let Err(error) = state.inhibit() {
            // Forwarding still works; compositor shortcuts stay with the compositor.
            eprintln!("telescreen: shortcut inhibit: {error}");
        }
        state.mode = Mode::Forwarding;
        s.escapes.reset();
        s.report_position(portal);
        Ok(())
    }

    fn set_portal_focus(&mut self, portal: PortalId, focused: bool) {
        self.0.borrow_mut().portal_focus(portal, focused);
    }

    fn guard(&mut self, portal: PortalId, on: bool) -> Result<(), PlatformError> {
        let mut s = self.0.borrow_mut();
        let Some(state) = s.portals.get_mut(&portal).filter(|p| p.mode != Mode::Off) else {
            return if on {
                Err(PlatformError::Failed(
                    "the portal guard needs an armed capture".into(),
                ))
            } else {
                Ok(())
            };
        };
        if on {
            state.inhibit()?;
        } else if state.mode != Mode::Forwarding {
            state.inhibitor = None;
        }
        Ok(())
    }

    fn disarm(&mut self, portal: PortalId) {
        let mut s = self.0.borrow_mut();
        if let Some(state) = s.portals.get_mut(&portal) {
            state.stop();
        }
        s.router.forget(portal);
        s.drop_unused(portal);
    }
}

pub(crate) struct Hooks {
    capture: LinuxCapture,
    prompts: Option<PromptQueue>,
}

pub(crate) fn parts(proxy: EventLoopProxy<UserEvent>, local: &Computer) -> Result<Parts, String> {
    let mut notices = Vec::new();
    let clipboard: Option<Box<dyn telescreen_core::platform::Clipboard>> =
        match LinuxClipboard::start() {
            Ok(clipboard) => Some(Box::new(clipboard)),
            Err(reason) => {
                notices.push(format!(
                    "Text clipboard sync is not available: {reason} (CLIP-8)."
                ));
                None
            }
        };
    let capabilities = telescreen_platform_linux::capabilities();
    let injector: Option<Box<dyn telescreen_core::platform::Injector>> =
        match LinuxInjector::new(local.uses_quirk(Quirk::KwinButtonScroll)) {
            Ok(injector) => Some(Box::new(injector)),
            Err(e) => {
                notices.push(format!("Input injection is not available: {e}"));
                None
            }
        };
    let focus_holder: Option<Box<dyn telescreen_core::platform::FocusHolder>> =
        match LinuxFocusHolder::start() {
            Ok(holder) => Some(Box::new(holder)),
            Err(reason) => {
                notices.push(format!(
                    "This computer cannot be a target: {reason} (UI-7)."
                ));
                None
            }
        };
    // UI-5: layer-shell where the compositor has it, else a normal window.
    let (prompt, prompts): (
        Box<dyn telescreen_view::PromptPresenter>,
        Option<PromptQueue>,
    ) = match LayerPrompt::start({
        let proxy = proxy.clone();
        move |prompt| {
            let _ = proxy.send_event(UserEvent::PromptPressed(prompt));
        }
    }) {
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
        portals: BTreeMap::new(),
        modifiers: ModifiersState::empty(),
        escapes: SafetyExit::new(),
        router: InputRouter::new(),
        events: Vec::new(),
    })));
    Ok(Parts {
        displays: Box::new(LinuxDisplays),
        capture: Box::new(capture.clone()),
        injector,
        focus_holder,
        clipboard,
        prompt,
        discovery: Box::new(telescreen_platform_linux::discovery::AvahiDiscovery::new(
            std::sync::Arc::new(move |event| {
                let _ = proxy.send_event(UserEvent::Discovery(event));
            }),
        )),
        capabilities,
        labels: Labels {
            confirm_key: "Return".into(),
            hand_back_shortcut: "Control+Alt+Shift+Escape".into(),
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

/// The Wayland app ID of each window, so that the compositor and a test tool can
/// find the windows of Telescreen.
pub(crate) fn window_attributes(attributes: WindowAttributes) -> WindowAttributes {
    attributes.with_name("dev.telescreen.Telescreen", "telescreen")
}

pub(crate) fn prompt_created(_: &Window) {}

/// VER-4: the layer-shell prompt sets its input itself, and reports a press
/// with `UserEvent::PromptPressed`. A normal window takes input.
pub(crate) fn prompt_input(_: &Window, _: bool) {}

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

    pub(crate) fn portal_created(&mut self, portal: PortalId, window: &Arc<Window>) {
        let mut s = self.capture.0.borrow_mut();
        let state = s.portals.entry(portal).or_insert_with(PortalState::new);
        state.window = Some(window.clone());
        state.inside = false;
        state.cursor = None;
    }

    pub(crate) fn portal_closed(&mut self, portal: PortalId) {
        let mut s = self.capture.0.borrow_mut();
        s.portals.remove(&portal);
        s.router.forget(portal);
    }

    /// Records native keyboard focus independently of the cursor (FLOW-22, UI-6).
    /// FLOW-22: the compositor gives the portal the focus for the click itself. A
    /// Wayland client cannot take the focus (FLOW-21).
    pub(crate) fn focus_portal(&mut self, _: &Window) {}

    /// FLOW-26: not specified on Linux. A Wayland client cannot reliably find out
    /// whether its window is shown.
    pub(crate) fn reports_portal_visibility(&self) -> bool {
        false
    }

    pub(crate) fn portal_hidden(&mut self, _: PortalId, _: &Window, _: bool, _: bool) {}

    /// A Wayland client cannot take the focus (FLOW-21).
    pub(crate) fn reclaim_focus(&mut self, _: &[(PortalId, &Window)]) {}

    pub(crate) fn portal_focus(&mut self, portal: PortalId, _: &Window, focused: bool, _: bool) {
        self.capture.0.borrow_mut().portal_focus(portal, focused);
    }

    /// Returns true if the event is input of the capture, so the UI must not get it.
    pub(crate) fn portal_window_event(
        &mut self,
        portal: PortalId,
        event: &WindowEvent,
        _: bool,
    ) -> bool {
        let mut s = self.capture.0.borrow_mut();
        match event {
            WindowEvent::ModifiersChanged(modifiers) => {
                s.modifiers = modifiers.state();
                false
            }
            WindowEvent::CursorMoved { position, .. } => {
                let Some(state) = s.portals.get_mut(&portal) else {
                    return false;
                };
                let scale = state.window.as_ref().map_or(1.0, |w| w.scale_factor());
                let logical = position.to_logical::<f64>(scale);
                state.cursor = Some(Point {
                    x: logical.x,
                    y: logical.y,
                });
                s.report_position(portal);
                false
            }
            WindowEvent::CursorEntered { .. } => {
                s.set_inside(portal, true);
                false
            }
            WindowEvent::CursorLeft { .. } => {
                s.set_inside(portal, false);
                if let Some(state) = s.portals.get_mut(&portal) {
                    state.cursor = None;
                }
                false
            }
            WindowEvent::KeyboardInput { event, .. } => {
                let down = event.state == ElementState::Pressed;
                let escape = event.physical_key == PhysicalKey::Code(KeyCode::Escape);
                let release = ModifiersState::CONTROL | ModifiersState::ALT | ModifiersState::SHIFT;
                if s.forwarding(portal) && escape && down && s.modifiers.contains(release) {
                    // IN-7: never forwarded. Only this portal stops.
                    s.stop_portal(portal);
                    return true;
                }
                let Some(code) = event
                    .physical_key
                    .to_scancode()
                    .and_then(|c| u16::try_from(c).ok())
                else {
                    return s.forwarding(portal) && s.router.keyboard() == Some(portal);
                };
                let forwarded = s.route(
                    portal,
                    InputEvent::Key {
                        key: Key(code),
                        down,
                    },
                );
                // IN-8: count while the portal forwards or waits for the
                // confirmation (FLOW-21).
                let counts =
                    forwarded || s.portals.get(&portal).is_some_and(|p| p.mode != Mode::Off);
                if counts && escape && down && !event.repeat && s.escapes.press(Instant::now()) {
                    s.stop_all(CapturedEvent::SafetyExit);
                }
                forwarded
            }
            WindowEvent::MouseInput {
                state, button: b, ..
            } => match button(*b) {
                Some(button) => s.route(
                    portal,
                    InputEvent::Button {
                        button,
                        down: *state == ElementState::Pressed,
                    },
                ),
                None => s.portals.get(&portal).is_some_and(PortalState::forward_now),
            },
            WindowEvent::MouseWheel { delta, phase, .. } => {
                // winit: positive is up or left. The wire is the opposite.
                let (dx, dy, phase) = match delta {
                    MouseScrollDelta::LineDelta(x, y) => (-x * LINE, -y * LINE, ScrollPhase::None),
                    MouseScrollDelta::PixelDelta(p) => (
                        -p.x as f32,
                        -p.y as f32,
                        // IN-18: Wayland ends a finger scroll with `axis_stop`.
                        match phase {
                            TouchPhase::Started => ScrollPhase::Begin,
                            TouchPhase::Moved => ScrollPhase::Update,
                            TouchPhase::Ended | TouchPhase::Cancelled => ScrollPhase::End,
                        },
                    ),
                };
                s.route(portal, InputEvent::Scroll { dx, dy, phase })
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
