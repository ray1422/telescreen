//! The headless Linux login stage process (STG-2, STG-3, STG-4, STG-9).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, mpsc};
use std::thread;
use std::time::{Duration, Instant};

use signal_hook::consts::{SIGHUP, SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use telescreen_core::keys::Key;
use telescreen_core::platform::PlatformError;
use telescreen_core::wire::Stage;
use telescreen_platform_linux::discovery::AvahiDiscovery;
use telescreen_platform_linux::keyboard::UinputKeyboard;
use telescreen_platform_linux::logind;
use telescreen_transport::network::{NetError, Timing, TrustSource};
use telescreen_transport::{Identity, NetCommand, NetConfig, NetEvent, Network};

use crate::controller::NetPort;
use crate::stage_agent::{KeyInjector, StageAgent, StageError, StagePorts};
use crate::startup::{self, Startup};
use crate::{paths, stage_record};

enum Event {
    Net(NetEvent),
    Login,
    Stop,
}

struct NetworkPort(Option<Network>);

impl NetPort for NetworkPort {
    fn send(&mut self, command: NetCommand) {
        if let Some(network) = &self.0 {
            network.send(command);
        }
    }

    fn shutdown(&mut self) {
        self.0 = None;
    }
}

impl KeyInjector for UinputKeyboard {
    fn key(&mut self, key: Key, down: bool) -> Result<(), PlatformError> {
        UinputKeyboard::key(self, key, down)
    }
}

struct ProcessRecord;

impl Drop for ProcessRecord {
    fn drop(&mut self) {
        stage_record::remove();
    }
}

struct SignalWatch(signal_hook::iterator::Handle);

impl Drop for SignalWatch {
    fn drop(&mut self) {
        self.0.close();
    }
}

pub fn run(config: Option<PathBuf>, computer: Option<String>) -> Result<(), String> {
    let config_path = match config {
        Some(path) => path,
        None => paths::default_config()?,
    };
    let computer_file = paths::computer_file()?;
    let (config, local) = match startup::resolve(
        &config_path,
        computer.as_deref(),
        paths::saved_computer(&computer_file).as_deref(),
    ) {
        Startup::Run { config, computer } => (config, computer),
        Startup::Problem(text) => return Err(text),
        Startup::Choose { .. } => {
            return Err("the login stage needs the ID of this computer: use --computer, or start Telescreen one time in the desktop session".into());
        }
    };
    let local_computer = config
        .computer(&local)
        .ok_or_else(|| StageError::UnknownComputer(local.clone()).to_string())?;
    if local_computer.stages.login.is_none() {
        return Err(StageError::NoStage(local).to_string());
    }
    let port = local_computer.endpoint.port;
    match logind::desktop_session_exists() {
        Ok(true) => {
            return Err(
                "You are logged in on this computer. Use Telescreen in the desktop session.".into(),
            );
        }
        Ok(false) => {}
        Err(error) => eprintln!("telescreen: cannot ask logind: {error}"),
    }
    let identity = Identity::load_or_create(&paths::identity_dir()?).map_err(|e| e.to_string())?;
    let fingerprint = identity.fingerprint().to_string();
    let (events, receiver) = mpsc::channel();
    let net_events = events.clone();
    let network = Network::start(
        NetConfig {
            local: local.clone(),
            listen: SocketAddr::from(([0, 0, 0, 0], port)),
            identity,
            trust: TrustSource::File(paths::trust_file()?),
            timing: Timing::default(),
            stage: Some(Stage::Login),
        },
        move |event| {
            let _ = net_events.send(Event::Net(event));
        },
    )
    .map_err(|error| match error {
        NetError::Listen { .. } => format!("The port {port} is in use. Telescreen possibly runs in the desktop session of this computer. ({error})"),
        _ => error.to_string(),
    })?;
    let keyboard =
        UinputKeyboard::new().map_err(|e| format!("{e} The user must be in the group input."))?;
    let discovery = AvahiDiscovery::new(Arc::new(|_| {}));
    stage_record::write()?;
    let _record = ProcessRecord;
    let login_events = events.clone();
    let _login_watch = match logind::watch_desktop_login(move || {
        let _ = login_events.send(Event::Login);
    }) {
        Ok(watch) => Some(watch),
        Err(error) => {
            eprintln!(
                "telescreen: cannot watch logind: {error}. The login stage does not stop at the login."
            );
            None
        }
    };
    let mut signals = Signals::new([SIGINT, SIGTERM, SIGHUP])
        .map_err(|e| format!("cannot listen for stop signals: {e}"))?;
    let _signal_watch = SignalWatch(signals.handle());
    thread::Builder::new()
        .name("telescreen-stage-signals".into())
        .spawn(move || {
            if signals.forever().next().is_some() {
                let _ = events.send(Event::Stop);
            }
        })
        .map_err(|e| format!("cannot start the stop signal thread: {e}"))?;
    let mut agent = StageAgent::new(
        config,
        local,
        fingerprint,
        StagePorts {
            net: Box::new(NetworkPort(Some(network))),
            keys: Box::new(keyboard),
            discovery: Box::new(discovery),
        },
    )
    .map_err(|e| e.to_string())?;
    for line in agent.start(Instant::now()) {
        println!("{line}");
    }
    while !agent.is_finished() {
        match receiver.recv_timeout(Duration::from_millis(100)) {
            Ok(Event::Net(event)) => agent.on_net(event, Instant::now()),
            Ok(Event::Login) => {
                println!("Login complete. The login stage stops.");
                agent.on_login();
            }
            Ok(Event::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => agent.stop(),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        agent.tick(Instant::now());
    }
    Ok(())
}
