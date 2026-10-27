//! macOS parts of the window layer: the event tap, the injector, the portal window
//! adjustments, and the prompt windows (ADR 0003, ADR 0006).

use std::sync::Arc;

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

pub(crate) fn parts(proxy: EventLoopProxy<UserEvent>) -> Result<Parts, String> {
    telescreen_platform_macos::request_permissions();
    let mut notices = Vec::new();
    let capabilities = telescreen_platform_macos::capabilities();
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
        capabilities,
        labels: Labels {
            confirm_key: "Return".into(),
            release_shortcut: "Control+Option+Command+Escape".into(),
        },
        hooks: Hooks {
            capture,
            prompts,
            raised: false,
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
    }

    pub(crate) fn portal_closed(&mut self) {
        self.raised = false;
    }

    /// Updates native keyboard focus (FLOW-22, UI-1, UI-2).
    pub(crate) fn portal_focus(&mut self, portal: &Window, focused: bool, guarded: bool) {
        // The controller gives the focus to the capture (FLOW-22).
        if focused {
            window::allow_only_roman_input(portal);
            if !self.raised && portal.fullscreen().is_some() {
                window::raise_portal(portal);
                self.raised = true;
            }
        } else if guarded {
            // FLOW-21: keep the focus during the confirmation wait.
            window::activate_app();
            portal.focus_window();
        }
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
