//! The window layer: the winit event loop, the start window, the portal windows,
//! and the prompt windows. It gives events to the controller, draws the view
//! models with the renderer, and pushes the view models to the native UI
//! (ADR 0012). The platform hooks are in `mac.rs` and `linux.rs`.
//!
//! On Linux the Qt event loop of the native UI runs on the main thread, and the
//! winit event loop runs on a second thread (M20 step g).

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

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use telescreen_core::config::Config;
use telescreen_core::driver::{self, SearchPath};
use telescreen_core::ids::{ComputerId, DisplayToken, PortalId};
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

use crate::controller::{AckOrigin, Controller, JobId, JobOutput, Ports, VerifierId};
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
    /// A press on the layer-shell prompt that takes input (VER-4, IN-8).
    #[cfg(target_os = "linux")]
    PromptPressed(PromptId, telescreen_view::PromptPress),
    /// A report of the discovery backend (NET-12).
    Discovery(telescreen_core::platform::DiscoveryEvent),
    /// A request of the debug protocol and the channel for its response line.
    #[cfg(feature = "debug-protocol")]
    Debug(crate::debug::Request, crossbeam_channel::Sender<String>),
    /// An event of the native UI (ADR 0012).
    #[cfg(target_os = "macos")]
    Ui(telescreen_ui_macos::UiEvent),
    /// An event of the native UI (ADR 0012).
    #[cfg(target_os = "linux")]
    Ui(telescreen_ui_linux::UiEvent),
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

/// Resolves the local computer, then runs until the user quits (UI-15).
#[cfg(not(target_os = "linux"))]
pub fn launch(launch: Launch) -> Result<(), String> {
    run_window_layer(launch, EventLoop::<UserEvent>::with_user_event())
}

/// Resolves the local computer, then runs until the user quits (UI-15).
///
/// First it takes the D-Bus name of the single instance (UI-15). If a different
/// process has the name, that process shows its main window, and this one
/// stops. With no session bus, Telescreen runs with no single instance and no
/// applet.
///
/// Qt must run on the main thread, so the winit loop runs on a second thread.
/// The Qt loop starts only when the app starts. Before that, the window layer
/// can stop with an error or show the start window (UI-8). At quit, the winit
/// thread drops its windows and its Wayland objects, then it stops the Qt loop
/// (spike 6).
#[cfg(target_os = "linux")]
pub fn launch(launch: Launch) -> Result<(), String> {
    use telescreen_ui_linux::Claim;
    use winit::platform::wayland::EventLoopBuilderExtWayland;
    use winit::platform::x11::EventLoopBuilderExtX11;

    let menu_server = match telescreen_ui_linux::claim() {
        Ok(Claim::Owner(server)) => Some(server),
        // A debug start must not end as a success with no debug socket.
        Ok(Claim::Running(_)) if launch.debug_socket.is_some() => {
            return Err("Telescreen already runs; quit it to start with a debug socket".into());
        }
        Ok(Claim::Running(Ok(()))) => {
            println!("telescreen: Telescreen already runs; it shows its main window");
            return Ok(());
        }
        Ok(Claim::Running(Err(error))) => {
            return Err(format!(
                "Telescreen already runs, but it cannot show its main window: {error}"
            ));
        }
        Err(error) => {
            eprintln!(
                "telescreen: no session bus ({error}); Telescreen runs with no single instance and no applet"
            );
            None
        }
    };

    /// Stops the Qt loop when the winit thread ends, also after a panic.
    struct QuitQt;
    impl Drop for QuitQt {
        fn drop(&mut self) {
            telescreen_ui_linux::quit();
        }
    }

    let (qt_start, start) = std::sync::mpsc::channel();
    let winit = std::thread::Builder::new()
        .name("telescreen-winit".into())
        .spawn(move || {
            let _quit = QuitQt;
            let mut builder = EventLoop::<UserEvent>::with_user_event();
            EventLoopBuilderExtWayland::with_any_thread(&mut builder, true);
            EventLoopBuilderExtX11::with_any_thread(&mut builder, true);
            QT_START.with_borrow_mut(|s| *s = Some(qt_start));
            MENU_SERVER.with_borrow_mut(|s| *s = menu_server);
            run_window_layer(launch, builder)
        })
        .map_err(|e| e.to_string())?;
    // An error: the winit thread ended, and the sender is gone.
    if start.recv().is_ok() {
        let code = telescreen_ui_linux::run();
        if code != 0 {
            eprintln!("telescreen: the Qt UI stopped with code {code}");
        }
    }
    winit
        .join()
        .unwrap_or_else(|_| Err("the window thread panicked".into()))
}

#[cfg(target_os = "linux")]
thread_local! {
    /// The window layer sends one message when the Qt loop must start.
    static QT_START: std::cell::RefCell<Option<std::sync::mpsc::Sender<()>>> =
        const { std::cell::RefCell::new(None) };
    /// The D-Bus service of the applet, until the app starts (UI-11).
    static MENU_SERVER: std::cell::RefCell<Option<telescreen_ui_linux::MenuServer>> =
        const { std::cell::RefCell::new(None) };
}

fn run_window_layer(
    launch: Launch,
    mut builder: winit::event_loop::EventLoopBuilder<UserEvent>,
) -> Result<(), String> {
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
    let mut event_loop = builder.build().map_err(|e| match &startup {
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
    let mut gpu = SharedGpu::new(event_loop.owned_display_handle());
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
        gpu: std::mem::replace(gpu, SharedGpu::new(event_loop.owned_display_handle())),
        #[cfg(feature = "debug-protocol")]
        _debug_server: None,
        hooks: parts.hooks,
        portals: BTreeMap::new(),
        prompts: HashMap::new(),
        frontmost_before: None,
        last_display_check: Instant::now(),
        swallow_release: false,
        drawn_portals: HashMap::new(),
        native: None,
        #[cfg(target_os = "linux")]
        menu_server: None,
        menu: telescreen_view::Publisher::new(),
        main: telescreen_view::Publisher::new(),
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
    /// The portal of the controller that this window shows.
    portal: PortalId,
    token: DisplayToken,
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
    /// One window for each portal of the controller (FLOW-25).
    portals: BTreeMap<PortalId, PortalSurface>,
    prompts: HashMap<PromptId, PromptSurface>,
    /// The app that was frontmost before the first warning took the focus. It
    /// is frontmost again when the last warning goes away (FLOW-26).
    frontmost_before: platform::Frontmost,
    last_display_check: Instant,
    /// The release of an acknowledgement click must not go to the UI (FLOW-9).
    swallow_release: bool,
    drawn_portals: HashMap<PortalId, telescreen_view::PortalScreen>,
    /// The native UI (ADR 0012): the menu (UI-9) and the main window (UI-13).
    native: Option<platform::NativeUi>,
    /// The D-Bus service of the applet (UI-11). `None` with no session bus.
    #[cfg(target_os = "linux")]
    menu_server: Option<telescreen_ui_linux::MenuServer>,
    menu: telescreen_view::Publisher<telescreen_view::MenuScreen>,
    main: telescreen_view::Publisher<telescreen_view::MainScreen>,
    proxy: EventLoopProxy<UserEvent>,
}

impl App {
    /// Creates a portal at its logical display bounds (FLOW-2).
    /// Enters fullscreen after creating the window.
    fn create_portal_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        monitor: MonitorHandle,
        portal: PortalId,
        token: DisplayToken,
    ) {
        // FLOW-26: the title names this portal, so that Accessibility can find it.
        static NEXT_PORTAL: AtomicU64 = AtomicU64::new(1);
        let id = NEXT_PORTAL.fetch_add(1, Ordering::Relaxed);
        // UI-13: the main window leaves the display before the portal enters
        // fullscreen there.
        #[cfg(target_os = "macos")]
        if let Some(native) = &self.native {
            let mut ids = self.portal_display_ids();
            ids.extend(cg_ids(&token));
            native.set_portal_displays(&ids);
        }
        let mut attributes = platform::window_attributes(Window::default_attributes())
            .with_title(format!("Telescreen portal {id}"));
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
                self.hooks.portal_created(portal, &surface.window);
                self.portals.insert(
                    portal,
                    PortalSurface {
                        surface,
                        portal,
                        token,
                    },
                );
            }
            Err(e) => {
                self.controller
                    .add_notice(format!("Cannot open the portal: {e}"));
                self.controller
                    .close_portal(portal, EndReason::SourceClosed);
            }
        }
    }

    /// Makes the window for a portal opened by the controller (ACT-5, FLOW-17).
    fn take_portal_window_request(&mut self, event_loop: &ActiveEventLoop) {
        let Some((portal, token)) = self.controller.take_portal_window_request() else {
            return;
        };
        if self.portals.remove(&portal).is_some() {
            self.hooks.portal_closed(portal);
        }
        let monitor = event_loop
            .available_monitors()
            .find(|monitor| platform::monitor_token(monitor).as_ref() == Some(&token));
        let Some(monitor) = monitor else {
            self.controller
                .add_notice(format!("Cannot find the portal display {token}."));
            self.controller.close_portal(portal, EndReason::PortalGone);
            return;
        };
        self.create_portal_window(event_loop, monitor, portal, token);
    }

    /// FLOW-4, AC-8: the portal window is on a different monitor now. Evaluate
    /// `portal-match` again; the old binding and coordinates do not apply.
    fn portal_monitor_changed(&mut self, id: PortalId, window: &Window) {
        let token = window
            .current_monitor()
            .as_ref()
            .and_then(platform::monitor_token);
        let Some(portal) = self.portals.get_mut(&id) else {
            return;
        };
        let Some(token) = token.filter(|t| *t != portal.token) else {
            return;
        };
        eprintln!("telescreen: portal window moved to {token}");
        portal.token = token.clone();
        let id = portal.portal;
        match self.controller.display(&token) {
            Some(display) => {
                self.controller.portal_display_changed(id, display);
            }
            None => self.controller.close_portal(id, EndReason::PortalGone),
        }
    }

    fn close_portal_window(&mut self, portal: PortalId) {
        if self.portals.remove(&portal).is_some() {
            self.hooks.portal_closed(portal);
            self.controller
                .close_portal(portal, EndReason::SourceClosed);
        }
    }

    fn close_portal_windows(&mut self) {
        for portal in self.portals.keys().copied().collect::<Vec<_>>() {
            self.close_portal_window(portal);
        }
    }

    /// The CoreGraphics ids of the displays with a portal (UI-13).
    #[cfg(target_os = "macos")]
    fn portal_display_ids(&self) -> Vec<u32> {
        self.portals
            .values()
            .flat_map(|portal| cg_ids(&portal.token))
            .collect()
    }

    fn handle_intents(&mut self, event_loop: &ActiveEventLoop, intents: Vec<Intent>) {
        for intent in intents {
            self.controller.on_intent(intent);
            self.take_portal_window_request(event_loop);
        }
    }

    /// Runs after each controller call: platform events, prompt windows, redraws.
    fn sync(&mut self, event_loop: &ActiveEventLoop) {
        for event in self.hooks.drain_captured() {
            self.controller.on_captured(event);
        }
        for portal in self.controller.take_portal_focus_requests() {
            if let Some(portal) = self.portals.get(&portal) {
                self.hooks.focus_portal(&portal.surface.window);
            }
        }
        #[cfg(target_os = "macos")]
        if let Some(native) = &self.native {
            // UI-13: the main window does not take the Space of a portal display.
            native.set_portal_displays(&self.portal_display_ids());
        }
        let windows: Vec<_> = self
            .portals
            .values()
            .map(|p| (p.portal, &*p.surface.window))
            .collect();
        self.hooks.reclaim_focus(&windows);
        for portal in self.portals.values() {
            // UI-3: the fullscreen state can change with no resize event after it.
            let fullscreen = portal.surface.window.fullscreen().is_some();
            self.controller
                .set_portal_fullscreen(portal.portal, fullscreen);
            let hidden = self.controller.portal_hidden(portal.portal);
            let forwarding = self.controller.forwarding(portal.portal);
            self.hooks
                .portal_hidden(portal.portal, &portal.surface.window, hidden, forwarding);
        }
        for request in self.hooks.take_prompt_requests() {
            self.prompt_request(event_loop, request);
        }
        let open = self.controller.portal_ids();
        let closed: Vec<_> = self
            .portals
            .keys()
            .filter(|id| !open.contains(id))
            .copied()
            .collect();
        for portal in closed {
            // The controller closed the portal (FLOW-19).
            self.portals.remove(&portal);
            self.drawn_portals.remove(&portal);
            self.hooks.portal_closed(portal);
        }
        // Draw again only when a view model changed.
        for portal in self.portals.values() {
            if self.drawn_portals.get(&portal.portal)
                != self.controller.portal_screen(portal.portal).as_ref()
            {
                portal.surface.window.request_redraw();
            }
        }
        // UI-18: push the menu only when it changed.
        #[cfg(target_os = "macos")]
        if let Some(native) = &self.native {
            if let Some(menu) = self.menu.update(self.controller.menu_screen()) {
                native.push_menu(menu);
            }
            if let Some(main) = self.main.update(self.controller.main_screen()) {
                native.push_main(main);
            }
        }
        #[cfg(target_os = "linux")]
        if let Some(native) = &self.native
            && let Some(main) = self.main.update(self.controller.main_screen())
            && let Err(error) = native.push_main(main)
        {
            eprintln!("telescreen: cannot show the main window: {error}");
        }
        #[cfg(target_os = "linux")]
        if let Some(server) = &self.menu_server
            && let Some(menu) = self.menu.update(self.controller.menu_screen())
            && let Err(error) = server.push_menu(menu)
        {
            eprintln!("telescreen: cannot give the menu to the applet: {error}");
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
        #[cfg(target_os = "linux")]
        if self.native.is_none() {
            let proxy = std::sync::Mutex::new(self.proxy.clone());
            self.native = Some(telescreen_ui_linux::LinuxUi::new(move |event| {
                if let Ok(proxy) = proxy.lock() {
                    let _ = proxy.send_event(UserEvent::Ui(event));
                }
            }));
            // The Qt loop starts on the main thread.
            if let Some(start) = QT_START.with_borrow_mut(Option::take) {
                let _ = start.send(());
            }
        }
        #[cfg(target_os = "linux")]
        if self.menu_server.is_none()
            && let Some(server) = MENU_SERVER.with_borrow_mut(Option::take)
        {
            let proxy = std::sync::Mutex::new(self.proxy.clone());
            match server.set_sink(move |event| {
                if let Ok(proxy) = proxy.lock() {
                    let _ = proxy.send_event(UserEvent::Ui(event));
                }
            }) {
                Ok(()) => self.menu_server = Some(server),
                Err(error) => eprintln!("telescreen: no applet: {error}"),
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

    fn ui_event(&mut self, event_loop: &ActiveEventLoop, event: platform::UiEvent) {
        use platform::UiEvent;
        match event {
            UiEvent::Intent(intent) => self.handle_intents(event_loop, vec![intent]),
            UiEvent::Quit => {
                // UI-15: the same as a quit of the app.
                self.controller.shutdown();
                self.close_portal_windows();
                event_loop.exit();
            }
            UiEvent::OpenSettings => {
                eprintln!("telescreen: the settings window is not implemented yet");
            }
            #[cfg(target_os = "linux")]
            UiEvent::ShowMainWindow => {
                if let Some(native) = &self.native {
                    native.show_main_window();
                }
            }
        }
    }

    /// FLOW-26: when no warning takes input, the app that was frontmost before
    /// the first warning is frontmost again. A second warning that still shows
    /// keeps the focus, so that its safety exit works (IN-8).
    fn restore_frontmost(&mut self) {
        let warning = self
            .prompts
            .values()
            .any(|surface| surface.screen.portal_hidden && surface.screen.takes_input());
        if !warning {
            platform::restore_frontmost(self.frontmost_before.take());
        }
    }

    fn prompt_request(&mut self, event_loop: &ActiveEventLoop, request: PromptRequest) {
        match request {
            PromptRequest::Hide { prompt } => {
                if self.prompts.remove(&prompt).is_some() {
                    self.restore_frontmost();
                }
            }
            PromptRequest::Show {
                prompt,
                display,
                screen,
            } => {
                // VER-4, FLOW-26: the prompt takes input for the
                // acknowledgement here, and the warning for the safety exit.
                let input = screen.takes_input();
                let warning = screen.portal_hidden;
                if let Some(surface) = self.prompts.get_mut(&prompt) {
                    let changed = input != surface.screen.takes_input();
                    let before = changed
                        .then(|| platform::prompt_input(&surface.surface.window, input))
                        .flatten();
                    surface.screen = screen;
                    surface.surface.window.request_redraw();
                    if warning {
                        self.frontmost_before = self.frontmost_before.take().or(before);
                    }
                    if changed && !input {
                        self.restore_frontmost();
                    }
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
                        let before = platform::prompt_input(&window, input);
                        let (surface, error) = Surface::new(window, &mut self.gpu);
                        if let Some(error) = error {
                            self.controller.add_notice(error);
                        }
                        if warning {
                            self.frontmost_before = self.frontmost_before.take().or(before);
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
        if let Some(portal) = self
            .portals
            .values_mut()
            .find(|p| p.surface.window.id() == id)
        {
            let screen = self.controller.portal_screen(portal.portal);
            if let (Some(renderer), Some(screen)) = (&mut portal.surface.renderer, &screen) {
                intents = renderer.draw_portal(screen);
            }
            match screen {
                Some(screen) => self.drawn_portals.insert(portal.portal, screen),
                None => self.drawn_portals.remove(&portal.portal),
            };
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
    /// `portal` and `prompt` name the window of the event.
    fn acknowledgement_event(
        &mut self,
        event: &WindowEvent,
        portal: Option<PortalId>,
        prompt: Option<PromptId>,
    ) -> bool {
        let origin = match (portal, prompt) {
            (Some(portal), _) => AckOrigin::Portal(portal),
            (None, Some(prompt)) => AckOrigin::Prompt(prompt),
            (None, None) => AckOrigin::Computer,
        };
        match event {
            // IN-8: Escape on the warning of FLOW-26 counts for the safety exit.
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && event.physical_key == PhysicalKey::Code(KeyCode::Escape)
                    && prompt.is_some() =>
            {
                if let Some(prompt) = prompt {
                    self.controller
                        .prompt_escape(prompt, event.repeat, Instant::now());
                }
                false
            }
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && !event.repeat
                    && matches!(
                        event.physical_key,
                        PhysicalKey::Code(KeyCode::Enter | KeyCode::NumpadEnter)
                    ) =>
            {
                let origin = match origin {
                    AckOrigin::Prompt(prompt) => AckOrigin::PromptKey(prompt),
                    origin => origin,
                };
                self.controller.acknowledge(origin)
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } if portal.is_some() || prompt.is_some() => {
                let accepted = self.controller.acknowledge(origin);
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
    fn portal_event(&mut self, portal: PortalId, event: &WindowEvent) -> bool {
        if self
            .hooks
            .portal_window_event(portal, event, self.controller.forwarding(portal))
        {
            return true;
        }
        let Some(window) = self.portals.get(&portal).map(|p| p.surface.window.clone()) else {
            return false;
        };
        match event {
            WindowEvent::Focused(focused) => {
                #[cfg(target_os = "macos")]
                eprintln!(
                    "telescreen: {}: {portal} focused: {focused} (key window: {:?})",
                    telescreen_platform_macos::clock(),
                    telescreen_platform_macos::window::key_window_title()
                );
                self.controller.set_portal_focus(portal, *focused);
                let guarded = self.controller.guarded(portal);
                self.hooks.portal_focus(portal, &window, *focused, guarded);
                false
            }
            WindowEvent::Occluded(occluded) => {
                #[cfg(target_os = "macos")]
                eprintln!(
                    "telescreen: {}: {portal} occluded: {occluded}",
                    telescreen_platform_macos::clock()
                );
                if self.hooks.reports_portal_visibility() {
                    self.controller.set_portal_visible(portal, !occluded);
                }
                if *occluded && self.controller.guarded(portal) {
                    // FLOW-21: keep the portal in place during verification.
                    self.hooks.portal_focus(portal, &window, false, true);
                }
                false
            }
            WindowEvent::Resized(_) | WindowEvent::Moved(_) => {
                // UI-3: a portal that is not fullscreen is not on screen
                // (FLOW-26). The source does not hand back for it.
                self.controller
                    .set_portal_fullscreen(portal, window.fullscreen().is_some());
                self.portal_monitor_changed(portal, &window);
                false
            }
            WindowEvent::CloseRequested => {
                self.close_portal_window(portal);
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
                for portal in self.portals.values() {
                    windows.push(window_facts(WindowRole::Portal, &portal.surface.window));
                }
                for prompt in self.prompts.values() {
                    windows.push(window_facts(WindowRole::Prompt, &prompt.surface.window));
                }
                debug::state_response(&StateReport {
                    control: self.controller.control_screen(),
                    portal: self
                        .controller
                        .main_portal()
                        .and_then(|id| self.controller.portal_screen(id)),
                    portals: self.controller.portal_states(),
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
                if self.controller.acknowledge(AckOrigin::Computer) {
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
            Request::MoveControl { .. } => debug::error_response("no control window"),
            Request::Quit => {
                self.controller.shutdown();
                self.close_portal_windows();
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
            Request::MainWindow { show } => {
                #[cfg(target_os = "macos")]
                if let Some(native) = &self.native {
                    match show {
                        Some(true) => native.show_main_window(),
                        Some(false) => native.close_main_window(),
                        None => {}
                    }
                    return serde_json::json!({
                        "ok": true,
                        "visible": native.main_window_visible(),
                        "dock_icon": native.dock_icon_visible(),
                    })
                    .to_string();
                }
                #[cfg(target_os = "linux")]
                if let Some(native) = &self.native {
                    match show {
                        Some(true) => native.show_main_window(),
                        Some(false) => native.close_main_window(),
                        None => {}
                    }
                    // The Qt thread runs the request later; `visible` is the
                    // state before the request.
                    return serde_json::json!({
                        "ok": true,
                        "visible": native.main_window_visible(),
                    })
                    .to_string();
                }
                let _ = show;
                debug::error_response("no native main window")
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
        // The native main window replaces the egui control window (UI-13).
        if self.native.is_none() {
            self.start_native_ui();
            match &self.native {
                Some(native) => native.show_main_window(),
                None => event_loop.exit(),
            }
            self.sync(event_loop);
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
            #[cfg(target_os = "linux")]
            UserEvent::PromptPressed(prompt, press) => match press {
                telescreen_view::PromptPress::Acknowledge => {
                    self.controller.acknowledge(AckOrigin::Prompt(prompt));
                }
                telescreen_view::PromptPress::Escape => {
                    self.controller.prompt_escape(prompt, false, Instant::now());
                }
            },
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
            UserEvent::Ui(event) => self.ui_event(event_loop, event),
        }
        self.sync(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if let WindowEvent::RedrawRequested = event {
            self.draw(event_loop, id);
            return;
        }
        let portal = self
            .portals
            .values()
            .find(|p| p.surface.window.id() == id)
            .map(|p| p.portal);
        let is_portal = portal.is_some();
        let prompt = self
            .prompts
            .iter()
            .find(|(_, p)| p.surface.window.id() == id)
            .map(|(prompt, _)| *prompt);
        let is_prompt = prompt.is_some();
        let used = (is_portal || is_prompt) && self.acknowledgement_event(&event, portal, prompt)
            || portal.is_some_and(|portal| self.portal_event(portal, &event));
        if !used {
            let surface = if let Some(portal) = portal {
                self.portals.get_mut(&portal).map(|p| &mut p.surface)
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
        let windows: Vec<_> = self
            .portals
            .values()
            .map(|p| (p.portal, &*p.surface.window))
            .collect();
        self.hooks.reclaim_focus(&windows);
        if now.duration_since(self.last_display_check) >= DISPLAY_CHECK {
            self.last_display_check = now;
            self.controller.displays_changed();
            self.sync(event_loop);
        }
        // IN-19: while a scroll waits, the injector needs its steps.
        let wake = self
            .controller
            .next_tick()
            .map_or(now + TICK, |at| at.min(now + TICK));
        event_loop.set_control_flow(ControlFlow::WaitUntil(wake));
    }

    fn exiting(&mut self, _: &ActiveEventLoop) {
        self.controller.shutdown();
    }
}

/// `telescreen self-check`: the QML of the Linux UI loads (DIST-10).
#[cfg(target_os = "linux")]
pub fn self_check() -> Result<(), String> {
    telescreen_ui_linux::self_check()
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
/// It does not write `PortalCursor` either: it comes with each pointer motion.
fn log_net_event(event: &NetEvent) {
    match event {
        NetEvent::Pointer { .. } => {}
        NetEvent::WakeSent { peer } => eprintln!("telescreen: wake request sent to {peer}"),
        NetEvent::WakeFailed { peer, reason } => {
            eprintln!("telescreen: wake request to {peer} failed: {reason}")
        }
        NetEvent::Message {
            message:
                Message::Heartbeat
                | Message::Input { .. }
                | Message::ClipboardText { .. }
                | Message::PortalCursor { .. },
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

/// The Core Graphics display id of a macOS display token (`cg-<id>`).
#[cfg(target_os = "macos")]
fn cg_ids(token: &DisplayToken) -> Vec<u32> {
    token
        .0
        .strip_prefix("cg-")
        .and_then(|id| id.parse().ok())
        .into_iter()
        .collect()
}
