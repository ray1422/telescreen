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
mod setup;

#[cfg(target_os = "linux")]
use linux as platform;
#[cfg(target_os = "macos")]
use mac as platform;

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use telescreen_core::config::Config;
use telescreen_core::driver::{self, SearchPath};
use telescreen_core::ids::{ComputerId, DisplayToken};
use telescreen_core::verifier::VerifierEvent;
use telescreen_core::wire::{EndReason, Message};
use telescreen_render_egui::{SharedGpu, WindowRenderer};
use telescreen_transport::network::{Timing, TrustSource};
use telescreen_transport::{Identity, NetConfig, NetEvent, Network};
use telescreen_view::{ComputerChoice, Intent, PromptId, PromptScreen, SetupIntent, SetupScreen};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::monitor::MonitorHandle;
use winit::platform::run_on_demand::EventLoopExtRunOnDemand;
use winit::window::{Fullscreen, Window, WindowId, WindowLevel};

use crate::controller::{Controller, JobId, JobOutput, Ports, VerifierId};
use crate::paths;
use crate::pointer_mailbox::{PointerMailbox, ReceivedPointer};
use crate::startup::{self, Startup};
use prompt_queue::PromptRequest;

/// How often the main loop runs the timers, the heartbeat, and the display check.
const TICK: Duration = Duration::from_millis(100);
const DISPLAY_CHECK: Duration = Duration::from_secs(2);

/// Events from other threads to the event loop.
#[derive(Debug)]
pub enum UserEvent {
    Net(NetEvent),
    /// Pointer datagrams wait in the mailbox.
    Pointers,
    Job(JobId, JobOutput),
    Verifier(VerifierId, VerifierEvent),
    /// The input capture has events in its queue.
    Captured,
    /// A report of the discovery backend (NET-12).
    Discovery(telescreen_core::platform::DiscoveryEvent),
    /// A request of the debug protocol and the channel for its response line.
    #[cfg(feature = "debug-protocol")]
    Debug(crate::debug::Request, crossbeam_channel::Sender<String>),
    /// An event of the native UI (ADR 0012).
    #[cfg(target_os = "macos")]
    Ui(telescreen_ui_macos::UiEvent),
}

/// The command-line options for starting Telescreen.
#[derive(Debug)]
pub struct Launch {
    /// `--config`.
    pub config: Option<PathBuf>,
    /// `--computer`.
    pub computer: Option<String>,
    /// `--debug-socket`.
    pub debug_socket: Option<PathBuf>,
}

/// Resolves the local computer, then runs until the control window closes.
pub fn launch(launch: Launch) -> Result<(), String> {
    let interactive = launch.config.is_none() && launch.computer.is_none();
    let config_path = match launch.config {
        Some(path) => path,
        None => paths::default_config()?,
    };
    let computer_file = paths::computer_file()?;
    let resolve = || {
        startup::resolve(
            &config_path,
            launch.computer.as_deref(),
            paths::saved_computer(&computer_file).as_deref(),
        )
    };
    let mut startup = resolve();
    if let Startup::Problem(text) = &startup {
        if !interactive {
            return Err(text.clone());
        }
        eprintln!("telescreen: {text}");
    }
    let mut event_loop = EventLoop::<UserEvent>::with_user_event()
        .build()
        .map_err(|e| match &startup {
            Startup::Problem(text) => text.clone(),
            _ => e.to_string(),
        })?;
    // Before the start window: a test can select the computer (UI-8).
    #[cfg(feature = "debug-protocol")]
    let debug_server = match &launch.debug_socket {
        Some(path) => Some(debug_server(path, event_loop.create_proxy())?),
        None => None,
    };
    #[cfg(not(feature = "debug-protocol"))]
    let _ = launch.debug_socket;
    let mut gpu = SharedGpu::default();
    let (config, local) = match startup {
        Startup::Run { config, computer } => (config, computer),
        _ => {
            let screen = setup_screen(&startup, &config_path);
            let selected = setup::run_setup(
                &mut event_loop,
                &mut gpu,
                |intent| {
                    match intent {
                        SetupIntent::CheckAgain => startup = resolve(),
                        SetupIntent::Choose { computer } => {
                            if let Startup::Choose { config, .. } = &startup
                                && config.computer(&computer).is_some()
                            {
                                match paths::save_computer(&computer_file, &computer.0) {
                                    Ok(()) => return Ok(Some((config.clone(), computer))),
                                    Err(text) => startup = Startup::Problem(text),
                                }
                            } else {
                                return Ok(None);
                            }
                        }
                    }
                    if let Startup::Run { config, computer } = &startup {
                        return Ok(Some((config.clone(), computer.clone())));
                    }
                    if interactive && let Startup::Problem(text) = &startup {
                        eprintln!("telescreen: {text}");
                    }
                    Err(setup_screen(&startup, &config_path))
                },
                screen,
            )?;
            let Some(selected) = selected else {
                return Ok(());
            };
            selected
        }
    };
    let mut app = match start_app(&event_loop, &mut gpu, config, local, config_path.clone()) {
        Ok(app) => app,
        Err(text) if interactive => {
            eprintln!("telescreen: {text}");
            setup::run_setup(
                &mut event_loop,
                &mut gpu,
                |_| Ok(Some(())),
                setup_screen(&Startup::Problem(text), &config_path),
            )?;
            return Ok(());
        }
        Err(text) => return Err(text),
    };
    #[cfg(feature = "debug-protocol")]
    {
        app._debug_server = debug_server;
    }
    // Not `run_app`: only this call clears the exit request of the start window.
    event_loop
        .run_app_on_demand(&mut app)
        .map_err(|e| e.to_string())
}

fn setup_screen(startup: &Startup, config_path: &Path) -> SetupScreen {
    let mut screen = SetupScreen {
        config_path: config_path.display().to_string(),
        ..Default::default()
    };
    match startup {
        Startup::Problem(text) => screen.problem = Some(text.clone()),
        Startup::Choose { config, notice } => {
            screen.notice = notice.clone();
            screen.computers = config
                .computers
                .iter()
                .map(|(id, computer)| ComputerChoice {
                    id: id.clone(),
                    label: computer.label.clone(),
                })
                .collect();
        }
        Startup::Run { .. } => {}
    }
    screen
}

/// Prepares the app while leaving the event loop available if startup fails.
fn start_app(
    event_loop: &EventLoop<UserEvent>,
    gpu: &mut SharedGpu,
    config: Config,
    local: ComputerId,
    config_path: PathBuf,
) -> Result<App, String> {
    let proxy = event_loop.create_proxy();

    let identity = Identity::load_or_create(&paths::identity_dir()?).map_err(|e| e.to_string())?;
    let fingerprint = identity.fingerprint().to_string();
    let port = config
        .computer(&local)
        .map(|c| c.endpoint.port)
        .ok_or("no local computer")?;
    let pointer_mailbox = Arc::new(PointerMailbox::new());
    let net = start_network(&proxy, &local, port, identity, pointer_mailbox.clone())?;

    let config_dir = config_path
        .canonicalize()
        .ok()
        .and_then(|p| p.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    let local_computer = config.computer(&local).ok_or("no local computer")?;
    let parts = platform::parts(proxy.clone(), local_computer)?;
    let first_generation = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(1);
    let mut path_notice = None;
    let search_path = match config
        .computer(&local)
        .and_then(|computer| computer.path.as_ref())
    {
        Some(path) => SearchPath(path.clone()),
        None => {
            let result = match std::env::var_os("SHELL") {
                Some(shell) => {
                    SearchPath::from_shell(&PathBuf::from(shell), Duration::from_secs(5))
                        .map_err(|e| e.to_string())
                }
                None => Err("SHELL is not set".into()),
            };
            match result {
                Ok(path) => path,
                Err(cause) => {
                    path_notice = Some(format!(
                        "Cannot read PATH from the login shell ({cause}). Telescreen uses its own PATH."
                    ));
                    SearchPath::from_env()
                }
            }
        }
    };
    let context = driver::Context {
        default_cwd: config_dir.clone(),
        search_path,
    };
    let ports = Ports {
        net,
        jobs: Box::new(ports::ThreadJobs {
            proxy: proxy.clone(),
            context: context.clone(),
        }),
        verifiers: Box::new(ports::ThreadVerifiers {
            proxy,
            context,
            running: Default::default(),
        }),
        displays: parts.displays,
        capture: parts.capture,
        injector: parts.injector,
        focus_holder: parts.focus_holder,
        clipboard: parts.clipboard,
        prompt: parts.prompt,
        discovery: parts.discovery,
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
    if let Some(notice) = path_notice {
        controller.add_notice(notice);
    }
    for notice in parts.notices {
        controller.add_notice(notice);
    }
    Ok(App {
        controller,
        pointer_mailbox,
        gpu: std::mem::take(gpu),
        #[cfg(feature = "debug-protocol")]
        _debug_server: None,
        hooks: parts.hooks,
        control: None,
        portal: None,
        prompts: HashMap::new(),
        last_display_check: Instant::now(),
        swallow_release: false,
        drawn_control: None,
        drawn_portal: None,
        #[cfg(target_os = "macos")]
        native: None,
        #[cfg(target_os = "macos")]
        menu: telescreen_view::Publisher::new(),
        #[cfg(target_os = "macos")]
        proxy: event_loop.create_proxy(),
    })
}

fn start_network(
    proxy: &EventLoopProxy<UserEvent>,
    local: &ComputerId,
    port: u16,
    identity: Identity,
    pointer_mailbox: Arc<PointerMailbox>,
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
            if let NetEvent::Pointer { peer, datagram } = event {
                if pointer_mailbox.put(ReceivedPointer {
                    peer,
                    datagram,
                    received: Instant::now(),
                }) {
                    let _ = net_proxy.send_event(UserEvent::Pointers);
                }
            } else {
                let _ = net_proxy.send_event(UserEvent::Net(event));
            }
        },
    )
    .map_err(|e| e.to_string())?;
    Ok(Box::new(ports::NetHandle(Some(Rc::new(network)))))
}

struct Surface {
    window: Arc<Window>,
    renderer: Option<WindowRenderer>,
}

impl Surface {
    fn new(window: Window, gpu: &mut SharedGpu) -> (Self, Option<String>) {
        let window = Arc::new(window);
        match WindowRenderer::new(window.clone(), gpu) {
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
    pointer_mailbox: Arc<PointerMailbox>,
    gpu: SharedGpu,
    #[cfg(feature = "debug-protocol")]
    _debug_server: Option<crate::debug::Server>,
    hooks: platform::Hooks,
    control: Option<Surface>,
    portal: Option<PortalSurface>,
    prompts: HashMap<PromptId, PromptSurface>,
    last_display_check: Instant,
    /// The release of an acknowledgement click must not go to the UI (FLOW-9).
    swallow_release: bool,
    drawn_control: Option<telescreen_view::ControlScreen>,
    drawn_portal: Option<telescreen_view::PortalScreen>,
    /// The native menu (ADR 0012, UI-9). The egui control window stays until
    /// the main window replaces it.
    #[cfg(target_os = "macos")]
    native: Option<telescreen_ui_macos::MacUi>,
    #[cfg(target_os = "macos")]
    menu: telescreen_view::Publisher<telescreen_view::MenuScreen>,
    #[cfg(target_os = "macos")]
    proxy: EventLoopProxy<UserEvent>,
}

impl App {
    /// Opens the portal on the selected display. A different portal closes first
    /// (FLOW-2).
    fn open_portal_window(&mut self, event_loop: &ActiveEventLoop, token: DisplayToken) {
        self.close_portal_window();
        let monitor = event_loop
            .available_monitors()
            .find(|monitor| platform::monitor_token(monitor).as_ref() == Some(&token));
        let Some(monitor) = monitor else {
            self.controller
                .add_notice(format!("Cannot find the display {token}."));
            return;
        };
        self.create_portal_window(event_loop, monitor, token, false);
    }

    /// Creates a portal at its logical display bounds (FLOW-2).
    /// Enters fullscreen after creating the window.
    fn create_portal_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        monitor: MonitorHandle,
        token: DisplayToken,
        opened: bool,
    ) {
        let mut attributes = platform::window_attributes(Window::default_attributes())
            .with_title("Telescreen portal");
        // Make the window on the display first. The fullscreen request then uses
        // the display of the window.
        if let Some(bounds) = self.controller.display(&token).and_then(|d| d.bounds) {
            attributes = attributes
                .with_position(LogicalPosition::new(bounds.x, bounds.y))
                .with_inner_size(LogicalSize::new(bounds.width, bounds.height));
        }
        let attributes = platform::portal_attributes(attributes);
        match event_loop.create_window(attributes) {
            Ok(window) => {
                window.set_fullscreen(Some(Fullscreen::Borderless(Some(monitor))));
                eprintln!(
                    "telescreen: portal for {token}: window on {:?}",
                    window
                        .current_monitor()
                        .as_ref()
                        .and_then(platform::monitor_token)
                );
                let (surface, error) = Surface::new(window, &mut self.gpu);
                if let Some(error) = error {
                    self.controller.add_notice(error);
                }
                self.hooks.portal_created(&surface.window);
                self.portal = Some(PortalSurface {
                    surface,
                    token,
                    opened,
                });
            }
            Err(e) => {
                self.controller
                    .add_notice(format!("Cannot open the portal: {e}"));
                if opened {
                    self.controller.close_portal(EndReason::SourceClosed);
                }
            }
        }
    }

    /// Makes the window for a portal opened by the controller (FLOW-17).
    fn take_portal_window_request(&mut self, event_loop: &ActiveEventLoop) {
        let Some(token) = self.controller.take_portal_window_request() else {
            return;
        };
        if self.portal.take().is_some() {
            self.hooks.portal_closed();
        }
        let monitor = event_loop
            .available_monitors()
            .find(|monitor| platform::monitor_token(monitor).as_ref() == Some(&token));
        let Some(monitor) = monitor else {
            self.controller
                .add_notice(format!("Cannot find the portal display {token}."));
            self.controller.close_portal(EndReason::PortalGone);
            return;
        };
        self.create_portal_window(event_loop, monitor, token, true);
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
        self.controller.open_portal(display);
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
        eprintln!("telescreen: portal window moved to {token}");
        portal.token = token.clone();
        if !portal.opened {
            return;
        }
        match self.controller.display(&token) {
            Some(display) => {
                self.controller.portal_display_changed(display);
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
            self.take_portal_window_request(event_loop);
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
        // UI-18: push the menu only when it changed.
        #[cfg(target_os = "macos")]
        if let Some(native) = &self.native
            && let Some(menu) = self.menu.update(self.controller.menu_screen())
        {
            native.push_menu(menu);
        }
    }

    /// The native UI and the first `portal-match` of each display (UI-18).
    fn start_native_ui(&mut self) {
        #[cfg(target_os = "macos")]
        if self.native.is_none() {
            let proxy = self.proxy.clone();
            match telescreen_ui_macos::MacUi::new(move |event| {
                let _ = proxy.send_event(UserEvent::Ui(event));
            }) {
                Ok(native) => self.native = Some(native),
                Err(error) => eprintln!("telescreen: no native menu: {error}"),
            }
        }
        let trusted = paths::trust_file().and_then(|path| {
            telescreen_transport::TrustStore::load(&path).map_err(|e| e.to_string())
        });
        match trusted {
            Ok(store) => self
                .controller
                .set_trusted(store.peers().map(|(id, _)| id.clone()).collect()),
            Err(error) => eprintln!("telescreen: cannot load the trust list: {error}"),
        }
        self.controller.evaluate_displays();
    }

    #[cfg(target_os = "macos")]
    fn ui_event(&mut self, event_loop: &ActiveEventLoop, event: telescreen_ui_macos::UiEvent) {
        use telescreen_ui_macos::UiEvent;
        match event {
            UiEvent::Intent(intent) => self.handle_intents(event_loop, vec![intent]),
            UiEvent::Quit => {
                // UI-15: the same as a quit of the app.
                self.controller.shutdown();
                self.close_portal_window();
                event_loop.exit();
            }
            UiEvent::OpenSettings => {
                eprintln!("telescreen: the settings window is not implemented yet");
            }
        }
    }

    fn prompt_request(&mut self, event_loop: &ActiveEventLoop, request: PromptRequest) {
        match request {
            PromptRequest::Hide { prompt } => {
                self.prompts.remove(&prompt);
            }
            PromptRequest::Show {
                prompt,
                display,
                screen,
            } => {
                if let Some(surface) = self.prompts.get_mut(&prompt) {
                    surface.screen = screen;
                    surface.surface.window.request_redraw();
                    return;
                }
                let Some(bounds) = display.bounds else {
                    self.controller.prompt_failed(prompt);
                    return;
                };
                let attributes = platform::window_attributes(Window::default_attributes())
                    .with_title("Telescreen")
                    .with_decorations(false)
                    .with_active(false)
                    .with_window_level(WindowLevel::AlwaysOnTop)
                    .with_position(LogicalPosition::new(bounds.x, bounds.y))
                    .with_inner_size(LogicalSize::new(bounds.width, bounds.height));
                match event_loop.create_window(attributes) {
                    Ok(window) => {
                        platform::prompt_created(&window);
                        let (surface, error) = Surface::new(window, &mut self.gpu);
                        if let Some(error) = error {
                            self.controller.add_notice(error);
                        }
                        self.prompts
                            .insert(prompt, PromptSurface { surface, screen });
                    }
                    Err(_) => self.controller.prompt_failed(prompt),
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

    /// Accepts acknowledgements before the renderer sees them (VER-4, FLOW-9).
    fn acknowledgement_event(
        &mut self,
        event: &WindowEvent,
        is_portal: bool,
        is_prompt: bool,
    ) -> bool {
        match event {
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && !event.repeat
                    && matches!(
                        event.physical_key,
                        PhysicalKey::Code(KeyCode::Enter | KeyCode::NumpadEnter)
                    ) =>
            {
                (is_portal && self.controller.acknowledge())
                    || (self.controller.awaiting_local_ack() && self.controller.acknowledge_local())
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } if is_portal || is_prompt => {
                let accepted = (is_portal && self.controller.acknowledge())
                    || (self.controller.awaiting_local_ack()
                        && self.controller.acknowledge_local());
                self.swallow_release = accepted;
                accepted
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } if self.swallow_release => {
                self.swallow_release = false;
                true
            }
            _ => false,
        }
    }

    /// Portal window events before the renderer sees them. Returns true if used.
    fn portal_event(&mut self, event: &WindowEvent) -> bool {
        if self
            .hooks
            .portal_window_event(event, self.controller.forwarding())
        {
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
                    .portal_focus(&window, *focused, self.controller.guarded());
                if *focused {
                    self.start_portal();
                }
                false
            }
            WindowEvent::Occluded(true) if self.controller.guarded() => {
                // FLOW-21: keep the portal in place during verification.
                self.hooks.portal_focus(&window, false, true);
                false
            }
            WindowEvent::Resized(_) | WindowEvent::Moved(_) => {
                if self.controller.forwarding() && window.fullscreen().is_none() {
                    self.controller.portal_left_fullscreen();
                }
                self.portal_monitor_changed(&window);
                self.start_portal();
                false
            }
            WindowEvent::CloseRequested => {
                self.close_portal_window();
                true
            }
            _ => false,
        }
    }
}

#[cfg(feature = "debug-protocol")]
impl App {
    /// Answers one debug request on the main thread (docs/debug-protocol.md).
    fn debug_request(
        &mut self,
        event_loop: &ActiveEventLoop,
        request: crate::debug::Request,
    ) -> String {
        use crate::debug::{self, Request, StateReport, WindowRole};
        match request {
            Request::State => {
                let mut windows = Vec::new();
                if let Some(control) = &self.control {
                    windows.push(window_facts(WindowRole::Control, &control.window));
                }
                if let Some(portal) = &self.portal {
                    windows.push(window_facts(WindowRole::Portal, &portal.surface.window));
                }
                for prompt in self.prompts.values() {
                    windows.push(window_facts(WindowRole::Prompt, &prompt.surface.window));
                }
                debug::state_response(&StateReport {
                    control: self.controller.control_screen(),
                    portal: self.controller.portal_screen(),
                    windows,
                })
            }
            Request::PointerStats => serde_json::json!({
                "ok": true,
                "stats": self.controller.pointer_stats(),
            })
            .to_string(),
            Request::Intent(intent) => {
                self.handle_intents(event_loop, vec![intent]);
                debug::ok_response()
            }
            Request::Ack => {
                if self.controller.acknowledge() || self.controller.acknowledge_local() {
                    debug::ok_response()
                } else if !self.controller.awaiting_ack() && !self.controller.awaiting_local_ack() {
                    debug::error_response("nothing waits for an acknowledgement")
                } else {
                    debug::error_response("the portal must have the focus to acknowledge")
                }
            }
            Request::HoldKeyboard { display } => {
                match self.controller.debug_hold_keyboard(&display) {
                    Ok(()) => debug::ok_response(),
                    Err(error) => debug::error_response(&error),
                }
            }
            Request::ReleaseKeyboard => {
                self.controller.debug_release_keyboard();
                debug::ok_response()
            }
            Request::ChooseComputer { .. } => debug::error_response("no start window is open"),
            Request::MoveControl { x, y } => match &self.control {
                Some(control) => {
                    control
                        .window
                        .set_outer_position(LogicalPosition::new(x, y));
                    debug::ok_response()
                }
                None => debug::error_response("no control window"),
            },
            Request::Quit => {
                self.controller.shutdown();
                self.close_portal_window();
                event_loop.exit();
                debug::ok_response()
            }
            Request::Menu { show } => {
                #[cfg(target_os = "macos")]
                if let Some(native) = &self.native {
                    match show {
                        Some(true) => native.show_menu(),
                        Some(false) => native.hide_menu(),
                        None => {}
                    }
                    return serde_json::json!({
                        "ok": true,
                        "visible": native.menu_visible(),
                        "frame": native.menu_frame(),
                        "menu": self.controller.menu_screen(),
                    })
                    .to_string();
                }
                let _ = show;
                serde_json::json!({"ok": true, "menu": self.controller.menu_screen()}).to_string()
            }
        }
    }
}

/// What the window system reports for one window (docs/debug-protocol.md).
#[cfg(feature = "debug-protocol")]
fn window_facts(role: crate::debug::WindowRole, window: &Window) -> crate::debug::WindowFacts {
    let scale = window.scale_factor();
    let size = window.inner_size().to_logical::<f64>(scale);
    crate::debug::WindowFacts {
        role,
        monitor: window
            .current_monitor()
            .as_ref()
            .and_then(platform::monitor_token),
        fullscreen: window.fullscreen().is_some(),
        focused: window.has_focus(),
        position: window.outer_position().ok().map(|p| {
            let p = p.to_logical::<f64>(scale);
            (p.x, p.y)
        }),
        size: (size.width, size.height),
    }
}

/// Starts the debug server. Each request goes to the event loop, and the
/// response comes back through a channel.
#[cfg(feature = "debug-protocol")]
fn debug_server(
    path: &std::path::Path,
    proxy: EventLoopProxy<UserEvent>,
) -> Result<crate::debug::Server, String> {
    let proxy = std::sync::Mutex::new(proxy);
    crate::debug::serve(path, move |request| {
        let (reply, response) = crossbeam_channel::bounded(1);
        let sent = proxy
            .lock()
            .map(|proxy| proxy.send_event(UserEvent::Debug(request, reply)).is_ok())
            .unwrap_or(false);
        if !sent {
            return crate::debug::error_response("the app is not running");
        }
        response
            .recv_timeout(Duration::from_secs(5))
            .unwrap_or_else(|_| crate::debug::error_response("no response in 5 s"))
    })
    .map_err(|e| e.to_string())
}

impl ApplicationHandler<UserEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.control.is_some() {
            return;
        }
        self.start_native_ui();
        let attributes = platform::window_attributes(Window::default_attributes())
            .with_title("Telescreen")
            .with_inner_size(LogicalSize::new(560.0, 520.0));
        match event_loop.create_window(attributes) {
            Ok(window) => {
                let (surface, error) = Surface::new(window, &mut self.gpu);
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
        let pointers = self.pointer_mailbox.take();
        if !pointers.is_empty() {
            self.controller.on_pointers(pointers, Instant::now());
        }
        match event {
            UserEvent::Pointers => {}
            UserEvent::Net(event) => {
                log_net_event(&event);
                self.controller.on_net(event, Instant::now());
                self.take_portal_window_request(event_loop);
            }
            UserEvent::Job(id, output) => {
                log_job(&output);
                self.controller.on_job_done(id, output);
            }
            UserEvent::Verifier(id, event) => self.controller.on_verifier_event(id, event),
            UserEvent::Captured => {}
            UserEvent::Discovery(event) => match event {
                telescreen_core::platform::DiscoveryEvent::Found(found) => {
                    let trust = paths::trust_file().and_then(|path| {
                        telescreen_transport::TrustStore::load(&path).map_err(|e| e.to_string())
                    });
                    match trust {
                        Ok(trust) => {
                            for (peer, targets) in crate::discovery::accepted(&found, &trust) {
                                eprintln!("telescreen: discovered {peer}: {targets:?}");
                                self.controller.on_discovered(peer, targets);
                            }
                        }
                        Err(error) => {
                            eprintln!("telescreen: cannot load the trust list: {error}");
                        }
                    }
                }
                telescreen_core::platform::DiscoveryEvent::Failed(reason) => {
                    eprintln!("telescreen: discovery failed: {reason}");
                    self.controller.on_discovery_failed(reason);
                }
            },
            #[cfg(feature = "debug-protocol")]
            UserEvent::Debug(request, reply) => {
                let response = self.debug_request(event_loop, request);
                let _ = reply.send(response);
            }
            #[cfg(target_os = "macos")]
            UserEvent::Ui(event) => self.ui_event(event_loop, event),
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
        let is_prompt = self.prompts.values().any(|p| p.surface.window.id() == id);
        let used = (is_portal || is_control || is_prompt)
            && self.acknowledgement_event(&event, is_portal, is_prompt)
            || (is_portal && self.portal_event(&event));
        if is_control && matches!(event, WindowEvent::CloseRequested) {
            self.controller.shutdown();
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

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let now = Instant::now();
        self.hooks.heartbeat();
        let discovery_active = self.controller.discovery_active();
        self.controller.tick(now);
        if discovery_active {
            self.sync(event_loop);
        }
        if now.duration_since(self.last_display_check) >= DISPLAY_CHECK {
            self.last_display_check = now;
            self.controller.displays_changed();
            self.start_portal();
            self.sync(event_loop);
        }
        event_loop.set_control_flow(ControlFlow::WaitUntil(now + TICK));
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        self.controller.shutdown();
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

/// The platform parts that `start_app` puts in the ports.
pub(crate) struct Parts {
    pub displays: Box<dyn telescreen_core::platform::Displays>,
    pub capture: Box<dyn telescreen_core::platform::InputCapture>,
    pub injector: Option<Box<dyn telescreen_core::platform::Injector>>,
    pub focus_holder: Option<Box<dyn telescreen_core::platform::FocusHolder>>,
    pub clipboard: Option<Box<dyn telescreen_core::platform::Clipboard>>,
    pub prompt: Box<dyn telescreen_view::PromptPresenter>,
    pub discovery: Box<dyn telescreen_core::platform::Discovery>,
    pub capabilities: telescreen_core::platform::InputCapabilities,
    pub labels: crate::controller::Labels,
    pub hooks: platform::Hooks,
    pub notices: Vec<String>,
}

/// Writes a network event to stderr for diagnostics. It does not write input
/// events, clipboard text, or pointer datagrams: they can contain private text.
fn log_net_event(event: &NetEvent) {
    match event {
        NetEvent::Pointer { .. } => {}
        NetEvent::Message {
            message: Message::Heartbeat | Message::Input { .. } | Message::ClipboardText { .. },
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
