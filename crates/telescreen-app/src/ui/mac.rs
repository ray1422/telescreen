//! macOS parts of the window layer: the event tap, the injector, the portal window
//! adjustments, and the prompt windows (ADR 0003, ADR 0006).

use std::sync::Arc;
use std::time::{Duration, Instant};

use telescreen_core::config::Computer;
use telescreen_core::ids::DisplayToken;
use telescreen_core::platform::{CapturedEvent, InputCapture};
use telescreen_platform_macos::capture::MacCapture;
use telescreen_platform_macos::clipboard::MacClipboard;
use telescreen_platform_macos::displays::{self, MacDisplays};
use telescreen_platform_macos::focus::MacFocusHolder;
use telescreen_platform_macos::inject::MacInjector;
use telescreen_platform_macos::window;
use winit::event::WindowEvent;
use winit::event_loop::EventLoopProxy;
use winit::monitor::MonitorHandle;
use winit::platform::macos::{MonitorHandleExtMacOS, WindowAttributesExtMacOS};
use winit::window::{Window, WindowAttributes};

use super::prompt_queue::{PromptQueue, PromptRequest};
use super::{Parts, UserEvent};
use crate::controller::Labels;

pub(crate) struct Hooks {
    capture: Option<MacCapture>,
    prompts: PromptQueue,
    raised: bool,
    /// The time of the last Space change (FLOW-26).
    space_changed: Option<Instant>,
    /// The portal was hidden at the last sync while forwarding (FLOW-26).
    hidden: bool,
    /// The portal had the focus when it went off screen (FLOW-26).
    focused_at_hide: bool,
    /// When the portal lost the focus. macOS takes the focus just before the
    /// occlusion event.
    focus_lost: Option<Instant>,
}

/// A capture that is not available (no permission): every call reports it.
struct NoCapture(String);

impl InputCapture for NoCapture {
    fn arm(
        &mut self,
        _: telescreen_core::pointer::Rect,
    ) -> Result<(), telescreen_core::platform::PlatformError> {
        Err(telescreen_core::platform::PlatformError::Permission(
            self.0.clone(),
        ))
    }
    fn forward(&mut self) -> Result<(), telescreen_core::platform::PlatformError> {
        Err(telescreen_core::platform::PlatformError::Permission(
            self.0.clone(),
        ))
    }
    fn disarm(&mut self) {}
}

pub(crate) fn parts(proxy: EventLoopProxy<UserEvent>, _local: &Computer) -> Result<Parts, String> {
    telescreen_platform_macos::request_permissions();
    let mut notices = Vec::new();
    let capabilities = telescreen_platform_macos::capabilities();
    let discovery_proxy = proxy.clone();
    let capture = match MacCapture::start(move || {
        let _ = proxy.send_event(UserEvent::Captured);
    }) {
        Ok(capture) => Some(capture),
        Err(e) => {
            notices.push(format!("Input capture is not available: {e}"));
            None
        }
    };
    let injector: Option<Box<dyn telescreen_core::platform::Injector>> = match MacInjector::new() {
        Ok(injector) => Some(Box::new(injector)),
        Err(e) => {
            notices.push(format!("Input injection is not available: {e}"));
            None
        }
    };
    let prompts = PromptQueue::default();
    Ok(Parts {
        displays: Box::new(MacDisplays),
        capture: match &capture {
            Some(capture) => Box::new(capture.clone()),
            None => Box::new(NoCapture(
                "grant Accessibility and Input Monitoring, then start Telescreen again".into(),
            )),
        },
        injector,
        focus_holder: Some(Box::new(MacFocusHolder::new())),
        clipboard: Some(Box::new(MacClipboard::new())),
        prompt: Box::new(prompts.clone()),
        discovery: Box::new(telescreen_platform_macos::discovery::MacDiscovery::new(
            std::sync::Arc::new(move |event| {
                let _ = discovery_proxy.send_event(UserEvent::Discovery(event));
            }),
        )),
        capabilities,
        labels: Labels {
            confirm_key: "Return".into(),
            hand_back_shortcut: "Control+Option+Command+Escape".into(),
        },
        hooks: Hooks {
            capture,
            prompts,
            raised: false,
            space_changed: None,
            hidden: false,
            focused_at_hide: false,
            focus_lost: None,
        },
        notices,
    })
}

pub(crate) fn monitor_token(monitor: &MonitorHandle) -> Option<DisplayToken> {
    Some(displays::token(monitor.native_id()))
}

/// UI-1: a native fullscreen Space; the presentation options hide the Dock.
pub(crate) fn portal_attributes(attributes: WindowAttributes) -> WindowAttributes {
    attributes.with_borderless_game(true)
}

pub(crate) fn window_attributes(attributes: WindowAttributes) -> WindowAttributes {
    attributes
}

pub(crate) fn prompt_created(prompt: &Window) {
    window::make_overlay(prompt);
}

/// VER-4: a prompt that waits for the acknowledgement on this computer takes
/// clicks and the keyboard focus.
pub(crate) fn prompt_input(prompt: &Window, local_ack: bool) {
    window::set_prompt_input(prompt, local_ack);
    if local_ack {
        prompt.focus_window();
        window::make_frontmost(None::<&Window>);
    }
}

impl Hooks {
    pub(crate) fn drain_captured(&mut self) -> Vec<CapturedEvent> {
        self.capture
            .as_ref()
            .map(MacCapture::drain)
            .unwrap_or_default()
    }

    pub(crate) fn heartbeat(&mut self) {
        if let Some(capture) = &mut self.capture {
            capture.heartbeat();
        }
    }

    pub(crate) fn take_prompt_requests(&mut self) -> Vec<PromptRequest> {
        self.prompts.take()
    }

    pub(crate) fn portal_created(&mut self, _: &Arc<Window>) {
        self.raised = false;
        window::watch_space_changes();
    }

    /// FLOW-26: macOS reports when the portal is not on screen.
    pub(crate) fn reports_portal_visibility(&self) -> bool {
        true
    }

    /// FLOW-26: after a Space change, show a hidden portal again. Opening Mission
    /// Control is not a Space change, so this does not close Mission Control.
    /// The Space notification and the occlusion event can come in either order,
    /// so a Space change counts for a short time.
    pub(crate) fn portal_hidden(&mut self, portal: &Window, hidden: bool, forwarding: bool) {
        const SPACE_CHANGE_TIME: Duration = Duration::from_secs(1);
        let now = Instant::now();
        let was_hidden = std::mem::replace(&mut self.hidden, hidden);
        if hidden && !was_hidden {
            self.focused_at_hide = portal.has_focus()
                || self
                    .focus_lost
                    .is_some_and(|at| now.duration_since(at) < Duration::from_millis(200));
        }
        let shown = was_hidden && !hidden;
        // FLOW-26: when Mission Control closes, the portal is on screen again but
        // macOS does not always give it back the focus. If the portal had the
        // focus and no other app took it, the portal takes it again.
        if shown
            && forwarding
            && self.focused_at_hide
            && !portal.has_focus()
            && window::is_frontmost()
        {
            eprintln!(
                "telescreen: {}: focus the portal on screen again",
                telescreen_platform_macos::clock()
            );
            self.take_focus(portal);
        }
        if window::take_space_change() {
            self.space_changed = Some(now);
        }
        let recent = self
            .space_changed
            .is_some_and(|at| now.duration_since(at) < SPACE_CHANGE_TIME);
        if hidden && recent {
            eprintln!(
                "telescreen: {}: show the hidden portal after a Space change",
                telescreen_platform_macos::clock()
            );
            self.space_changed = None;
            self.take_focus(portal);
        }
    }

    /// The portal takes the native focus, also after Telescreen becomes the
    /// frontmost app, so that the focus does not go to the main window (FLOW-22,
    /// FLOW-26).
    fn take_focus(&mut self, portal: &Window) {
        portal.focus_window();
        window::make_frontmost(Some(portal));
    }

    pub(crate) fn portal_closed(&mut self) {
        self.raised = false;
    }

    /// Updates native keyboard focus (FLOW-22, UI-1, UI-2).
    pub(crate) fn portal_focus(&mut self, portal: &Window, focused: bool, guarded: bool) {
        // The controller gives the focus to the capture (FLOW-22).
        if !focused {
            self.focus_lost = Some(Instant::now());
        }
        if focused {
            window::allow_only_roman_input(portal);
            if !self.raised && portal.fullscreen().is_some() {
                window::raise_portal(portal);
                self.raised = true;
            }
        } else if guarded {
            // FLOW-21: keep the focus during handoff and verification.
            self.take_focus(portal);
        }
    }

    /// FLOW-22: a click in the portal asks for the native focus. The capture
    /// suppresses the click (IN-17), so macOS does not focus the portal for it.
    /// The key window takes effect when the app is frontmost.
    pub(crate) fn focus_portal(&mut self, portal: &Window) {
        self.take_focus(portal);
    }

    /// Keeps forwarded pointer input out of the portal UI (FLOW-22).
    pub(crate) fn portal_window_event(&mut self, event: &WindowEvent, forwarding: bool) -> bool {
        forwarding
            && matches!(
                event,
                WindowEvent::MouseInput { .. }
                    | WindowEvent::MouseWheel { .. }
                    | WindowEvent::CursorMoved { .. }
                    | WindowEvent::CursorEntered { .. }
                    | WindowEvent::CursorLeft { .. }
            )
    }
}

/// The displays and the capabilities, with no prompt.
pub(crate) fn describe() -> (
    Result<Vec<telescreen_core::platform::DisplayInfo>, telescreen_core::platform::PlatformError>,
    telescreen_core::platform::InputCapabilities,
) {
    use telescreen_core::platform::Displays;
    (
        MacDisplays.displays(),
        telescreen_platform_macos::capabilities(),
    )
}
