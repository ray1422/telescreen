//! The window layer: the winit event loop, the control window, the portal window,
//! and (on macOS) the prompt windows. It gives events to the controller and draws
//! the view models with the renderer. The platform hooks are in `mac.rs` and
//! `linux.rs`.

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod mac;
mod ports;
mod prompt_queue;

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
use mac as platform;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use telescreen_core::config::Config;
use telescreen_core::ids::{ComputerId, DisplayToken, SessionId};
use telescreen_core::wire::{EndReason, Message};
use telescreen_render_egui::WindowRenderer;
use telescreen_transport::network::{Timing, TrustSource};
use telescreen_transport::{Identity, NetConfig, NetEvent, Network};
use telescreen_view::{Intent, PromptScreen};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{DeviceEvent, DeviceId, ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Fullscreen, Window, WindowId, WindowLevel};

use crate::controller::{Controller, JobId, JobOutput, Ports};
use crate::paths;
use prompt_queue::PromptRequest;

/// How often the main loop runs the timers, the heartbeat, and the display check.
const TICK: Duration = Duration::from_millis(100);
const DISPLAY_CHECK: Duration = Duration::from_secs(2);

/// Events from other threads to the event loop.
#[derive(Debug)]
pub enum UserEvent {
    Net(NetEvent),
    Job(JobId, JobOutput),
    /// The input capture has events in its queue.
    Captured,
}

/// Runs the app until the control window closes.
pub fn run(config: Config, local: ComputerId, config_path: PathBuf) -> Result<(), String> {
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|e| e.to_string())?;
    let proxy = event_loop.create_proxy();

    let identity = Identity::load_or_create(&paths::identity_dir()?).map_err(|e| e.to_string())?;
    let fingerprint = identity.fingerprint().to_string();
    let port = config
        .computer(&local)
        .map(|c| c.endpoint.port)
        .ok_or("no local computer")?;
    let net = start_network(&proxy, &local, port, identity)?;

    let config_dir = config_path
        .canonicalize()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    let parts = platform::parts(proxy.clone())?;
    let first_generation = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(1);
    let ports = Ports {
        net,
        jobs: Box::new(ports::ThreadJobs {
            proxy,
            cwd: config_dir.clone(),
        }),
        displays: parts.displays,
        capture: parts.capture,
        injector: parts.injector,
        prompt: parts.prompt,
        capabilities: parts.capabilities,
    };
    let mut controller = Controller::new(
        config,
        local,
        config_dir,
        parts.labels,
        fingerprint,
        first_generation,
        ports,
    );
    for notice in parts.notices {
        controller.add_notice(notice);
    }
    let mut app = App {
        controller,
        hooks: parts.hooks,
        control: None,
        portal: None,
        prompts: HashMap::new(),
        last_display_check: Instant::now(),
        swallow_release: false,
        drawn_control: None,
        drawn_portal: None,
    };
    event_loop.run_app(&mut app).map_err(|e| e.to_string())
}

fn start_network(
    proxy: &EventLoopProxy<UserEvent>,
    local: &ComputerId,
    port: u16,
    identity: Identity,
) -> Result<Box<dyn crate::controller::NetPort>, String> {
    let net_proxy = proxy.clone();
    let network = Network::start(
        NetConfig {
            local: local.clone(),
            listen: SocketAddr::from(([0, 0, 0, 0], port)),
            identity,
            trust: TrustSource::File(paths::trust_file()?),
            // The controller sends the heartbeat from the main loop (AC-18).
            timing: Timing {
                auto_heartbeat: false,
                ..Timing::default()
            },
        },
        move |event| {
            let _ = net_proxy.send_event(UserEvent::Net(event));
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(Box::new(ports::NetHandle(Rc::new(network))))
}

struct Surface {
    window: Arc<Window>,
    renderer: Option<WindowRenderer>,
}

impl Surface {
    fn new(window: Window) -> (Self, Option<String>) {
        let window = Arc::new(window);
        match WindowRenderer::new(window.clone()) {
            Ok(renderer) => (
                Self {
                    window,
                    renderer: Some(renderer),
                },
                None,
            ),
            Err(e) => (
                Self {
                    window,
                    renderer: None,
                },
                Some(format!("Cannot draw the window: {e}")),
            ),
        }
    }

    fn on_event(&mut self, event: &WindowEvent) {
        if let Some(renderer) = &mut self.renderer
            && renderer.on_window_event(event)
        {
            self.window.request_redraw();
        }
    }
}

struct PortalSurface {
    surface: Surface,
    token: DisplayToken,
    /// True after the controller got `open_portal`.
    opened: bool,
}

struct PromptSurface {
    surface: Surface,
    screen: PromptScreen,
}

struct App {
    controller: Controller,
    hooks: platform::Hooks,
    control: Option<Surface>,
    portal: Option<PortalSurface>,
    prompts: HashMap<SessionId, PromptSurface>,
    last_display_check: Instant,
    /// The release of a confirmation click must not go to the UI either (FLOW-9).
    swallow_release: bool,
    drawn_control: Option<telescreen_view::ControlScreen>,
    drawn_portal: Option<telescreen_view::PortalScreen>,
}

impl App {
    fn open_portal_window(&mut self, event_loop: &ActiveEventLoop, token: DisplayToken) {
        self.close_portal_window();
        let monitor = event_loop
            .available_monitors()
            .find(|m| platform::monitor_token(m) == Some(token.clone()));
        let Some(monitor) = monitor else {
            self.controller
                .add_notice(format!("No window system monitor for display {token}."));
            return;
        };
        let attributes = platform::portal_attributes(
            Window::default_attributes()
                .with_title("Telescreen portal")
                .with_position(monitor.position())
                .with_fullscreen(Some(Fullscreen::Borderless(Some(monitor)))),
        );
        match event_loop.create_window(attributes) {
            Ok(window) => {
                let (surface, error) = Surface::new(window);
                if let Some(error) = error {
                    self.controller.add_notice(error);
                }
                self.hooks.portal_created(&surface.window);
                self.portal = Some(PortalSurface {
                    surface,
                    token,
                    opened: false,
                });
            }
            Err(e) => self
                .controller
                .add_notice(format!("Cannot open the portal: {e}")),
        }
    }

    /// Calls `open_portal` when the window is on its display (FLOW-2).
    fn start_portal(&mut self) {
        let Some(portal) = self.portal.as_mut() else {
            return;
        };
        if portal.opened {
            return;
        }
        let Some(display) = self.controller.display(&portal.token) else {
            return;
        };
        portal.opened = true;
        let others = self.controller.other_display_bounds(&portal.token);
        self.controller.open_portal(display, others);
    }

    /// FLOW-4, AC-8: the portal window is on a different monitor now. Evaluate
    /// `portal-match` again; the old binding and coordinates do not apply.
    fn portal_monitor_changed(&mut self, window: &Window) {
        let token = window
            .current_monitor()
            .as_ref()
            .and_then(platform::monitor_token);
        let Some(portal) = self.portal.as_mut() else {
            return;
        };
        let Some(token) = token.filter(|t| *t != portal.token) else {
            return;
        };
        portal.token = token.clone();
        if !portal.opened {
            return;
        }
        match self.controller.display(&token) {
            Some(display) => {
                let others = self.controller.other_display_bounds(&token);
                self.controller.portal_display_changed(display, others);
            }
            None => self.controller.close_portal(EndReason::PortalGone),
        }
    }

    fn close_portal_window(&mut self) {
        if self.portal.take().is_some() {
            self.hooks.portal_closed();
            self.controller.close_portal(EndReason::SourceClosed);
        }
    }

    fn handle_intents(&mut self, event_loop: &ActiveEventLoop, intents: Vec<Intent>) {
        for intent in intents {
            match intent {
                Intent::OpenPortal { display } => self.open_portal_window(event_loop, display),
                Intent::ClosePortal => self.close_portal_window(),
                other => self.controller.on_intent(other),
            }
        }
    }

    /// Runs after each controller call: platform events, prompt windows, redraws.
    fn sync(&mut self, event_loop: &ActiveEventLoop) {
        for event in self.hooks.drain_captured() {
            self.controller.on_captured(event);
        }
        for request in self.hooks.take_prompt_requests() {
            self.prompt_request(event_loop, request);
        }
        if self.portal.as_ref().is_some_and(|p| p.opened) && !self.controller.has_portal() {
            // The controller closed the portal (FLOW-19).
            self.portal = None;
            self.hooks.portal_closed();
        }
        // Draw again only when a view model changed.
        if let Some(control) = &self.control
            && self.drawn_control.as_ref() != Some(&self.controller.control_screen())
        {
            control.window.request_redraw();
        }
        if let Some(portal) = &self.portal
            && self.drawn_portal != self.controller.portal_screen()
        {
            portal.surface.window.request_redraw();
        }
    }

    fn prompt_request(&mut self, event_loop: &ActiveEventLoop, request: PromptRequest) {
        match request {
            PromptRequest::Hide { session } => {
                self.prompts.remove(&session);
            }
            PromptRequest::Show {
                session,
                display,
                screen,
            } => {
                let Some(bounds) = display.bounds else {
                    self.controller.prompt_failed(session);
                    return;
                };
                let attributes = Window::default_attributes()
                    .with_title("Telescreen")
                    .with_decorations(false)
                    .with_active(false)
                    .with_window_level(WindowLevel::AlwaysOnTop)
                    .with_position(LogicalPosition::new(bounds.x, bounds.y))
                    .with_inner_size(LogicalSize::new(bounds.width, bounds.height));
                match event_loop.create_window(attributes) {
                    Ok(window) => {
                        platform::prompt_created(&window);
                        let (surface, error) = Surface::new(window);
                        if let Some(error) = error {
                            self.controller.add_notice(error);
                        }
                        self.prompts
                            .insert(session, PromptSurface { surface, screen });
                    }
                    Err(_) => self.controller.prompt_failed(session),
                }
            }
        }
    }

    fn draw(&mut self, event_loop: &ActiveEventLoop, id: WindowId) {
        let mut intents = Vec::new();
        if let Some(control) = self.control.as_mut().filter(|c| c.window.id() == id) {
            let screen = self.controller.control_screen();
            if let Some(renderer) = &mut control.renderer {
                intents = renderer.draw_control(&screen);
            }
            self.drawn_control = Some(screen);
        } else if let Some(portal) = self.portal.as_mut().filter(|p| p.surface.window.id() == id) {
            let screen = self.controller.portal_screen();
            if let (Some(renderer), Some(screen)) = (&mut portal.surface.renderer, &screen) {
                intents = renderer.draw_portal(screen);
            }
            self.drawn_portal = screen;
        } else if let Some(prompt) = self
            .prompts
            .values_mut()
            .find(|p| p.surface.window.id() == id)
            && let Some(renderer) = &mut prompt.surface.renderer
        {
            renderer.draw_prompt(&prompt.screen);
        }
        if !intents.is_empty() {
            self.handle_intents(event_loop, intents);
            self.sync(event_loop);
        }
    }

    /// Portal window events before the renderer sees them. Returns true if the
    /// event was used here (a confirmation, or input of a Linux capture).
    fn portal_event(&mut self, event: &WindowEvent) -> bool {
        if self.hooks.portal_window_event(event) {
            return true;
        }
        let window = match &self.portal {
            Some(portal) => portal.surface.window.clone(),
            None => return false,
        };
        match event {
            WindowEvent::Focused(focused) => {
                self.controller.set_portal_focus(*focused);
                self.hooks
                    .portal_focus(&window, *focused, self.controller.forwarding());
                if *focused {
                    self.start_portal();
                }
                false
            }
            WindowEvent::Occluded(true) if self.controller.forwarding() => {
                // ADR 0003: a Space change while forwarding. Take the focus back.
                self.hooks.portal_focus(&window, false, true);
                false
            }
            WindowEvent::Resized(_) | WindowEvent::Moved(_) => {
                self.portal_monitor_changed(&window);
                self.start_portal();
                false
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && !event.repeat
                    && matches!(
                        event.physical_key,
                        PhysicalKey::Code(KeyCode::Enter | KeyCode::NumpadEnter)
                    )
                    && self.controller.awaiting_confirmation() =>
            {
                self.controller.confirm()
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } if self.controller.awaiting_confirmation() => {
                let confirmed = self.controller.confirm();
                self.swallow_release = confirmed;
                confirmed
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } if self.swallow_release => {
                self.swallow_release = false;
                true
            }
            WindowEvent::CloseRequested => {
                self.close_portal_window();
                true
            }
            _ => false,
        }
    }
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.control.is_some() {
            return;
        }
        let attributes = Window::default_attributes()
            .with_title("Telescreen")
            .with_inner_size(LogicalSize::new(560.0, 520.0));
        match event_loop.create_window(attributes) {
            Ok(window) => {
                let (surface, error) = Surface::new(window);
                if let Some(error) = error {
                    eprintln!("telescreen: {error}");
                }
                self.control = Some(surface);
            }
            Err(e) => {
                eprintln!("telescreen: cannot open the window: {e}");
                event_loop.exit();
            }
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        match event {
            UserEvent::Net(event) => {
                log_net_event(&event);
                self.controller.on_net(event, Instant::now());
            }
            UserEvent::Job(id, output) => {
                log_job(&output);
                self.controller.on_job_done(id, output);
            }
            UserEvent::Captured => {}
        }
        self.sync(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if let WindowEvent::RedrawRequested = event {
            self.draw(event_loop, id);
            return;
        }
        let is_portal = self
            .portal
            .as_ref()
            .is_some_and(|p| p.surface.window.id() == id);
        let is_control = self.control.as_ref().is_some_and(|c| c.window.id() == id);
        let used = is_portal && self.portal_event(&event);
        if is_control && matches!(event, WindowEvent::CloseRequested) {
            self.close_portal_window();
            event_loop.exit();
            return;
        }
        if !used {
            let surface = if is_portal {
                self.portal.as_mut().map(|p| &mut p.surface)
            } else if is_control {
                self.control.as_mut()
            } else {
                self.prompts
                    .values_mut()
                    .map(|p| &mut p.surface)
                    .find(|s| s.window.id() == id)
            };
            if let Some(surface) = surface {
                surface.on_event(&event);
            }
        }
        self.sync(event_loop);
    }

    fn device_event(&mut self, event_loop: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if self.hooks.device_event(&event) {
            self.sync(event_loop);
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        self.hooks.heartbeat();
        self.controller.tick(now);
        if now.duration_since(self.last_display_check) >= DISPLAY_CHECK {
            self.last_display_check = now;
            self.controller.displays_changed();
            self.start_portal();
            self.sync(event_loop);
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(now + TICK));
    }
}

/// `telescreen displays`: the display list, the capabilities, and the monitors of
/// the window system with their tokens, as JSON. It opens no window and asks for
/// no permission.
pub fn describe_displays() -> Result<String, String> {
    struct Monitors(Vec<serde_json::Value>);
    impl ApplicationHandler<UserEvent> for Monitors {
        fn resumed(&mut self, event_loop: &ActiveEventLoop) {
            self.0 = event_loop
                .available_monitors()
                .map(|m| {
                    serde_json::json!({
                        "name": m.name(),
                        "token": platform::monitor_token(&m).map(|t| t.0),
                    })
                })
                .collect();
            event_loop.exit();
        }
        fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
    }
    let event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|e| e.to_string())?;
    let mut monitors = Monitors(Vec::new());
    event_loop
        .run_app(&mut monitors)
        .map_err(|e| e.to_string())?;
    let (displays, capabilities) = platform::describe();
    let capability = |c: &telescreen_core::platform::Capability| match c {
        telescreen_core::platform::Capability::Available => "available".to_owned(),
        telescreen_core::platform::Capability::Missing(reason) => format!("missing: {reason}"),
    };
    let value = serde_json::json!({
        "displays": displays.map_err(|e| e.to_string())?,
        "capabilities": {
            "capture": capability(&capabilities.capture),
            "suppress": capability(&capabilities.suppress),
            "inject": capability(&capabilities.inject),
        },
        "window_system_monitors": monitors.0,
    });
    serde_json::to_string_pretty(&value).map_err(|e| e.to_string())
}

/// The platform parts that `run` puts in the ports.
pub(crate) struct Parts {
    pub displays: Box<dyn telescreen_core::platform::Displays>,
    pub capture: Box<dyn telescreen_core::platform::InputCapture>,
    pub injector: Option<Box<dyn telescreen_core::platform::Injector>>,
    pub prompt: Box<dyn telescreen_view::PromptPresenter>,
    pub capabilities: telescreen_core::platform::InputCapabilities,
    pub labels: crate::controller::Labels,
    pub hooks: platform::Hooks,
    pub notices: Vec<String>,
}

/// Writes a network event to stderr for diagnostics. It does not write input
/// events or pointer datagrams: they can contain typed text.
fn log_net_event(event: &NetEvent) {
    match event {
        NetEvent::Pointer { .. } => {}
        NetEvent::Message {
            message: Message::Heartbeat | Message::Input { .. },
            ..
        } => {}
        NetEvent::Message { peer, message } => eprintln!("telescreen: from {peer}: {message:?}"),
        NetEvent::Connected { peer } => eprintln!("telescreen: connected to {peer}"),
        NetEvent::Disconnected { peer, reason } => {
            eprintln!("telescreen: disconnected from {peer}: {reason}")
        }
        NetEvent::DialFailed { peer, reason } => {
            eprintln!("telescreen: dial to {peer} failed: {reason}")
        }
        NetEvent::Refused { remote, reason } => {
            eprintln!("telescreen: refused {remote}: {reason}")
        }
    }
}

/// Writes the result of a driver job to stderr for diagnostics.
fn log_job(output: &JobOutput) {
    match output {
        JobOutput::Operation(result) => eprintln!("telescreen: operation: {result:?}"),
        JobOutput::Handoff { result, error } => {
            eprintln!("telescreen: handoff: {result:?} {error:?}")
        }
    }
}
