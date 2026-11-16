//! The target side of the login stage (STG-2 to STG-5, STG-9, ADR 0015).
//!
//! A stage process runs this agent and not the [`crate::controller::Controller`].
//! The agent has no port for a pointer, a display, a prompt, a clipboard, or a
//! driver command, so a stage process cannot do these things. It grants one
//! stage session to a source that the config permits, and it injects the key
//! events of that session.

use std::collections::{BTreeSet, VecDeque};
use std::time::{Duration, Instant};

use telescreen_core::config::Config;
use telescreen_core::ids::{ComputerId, SessionId};
use telescreen_core::input::InputEvent;
use telescreen_core::keys::Key;
use telescreen_core::platform::{Discovery, PlatformError};
use telescreen_core::session::stage_target::{StageTargetAction, StageTargetSession};
use telescreen_core::wire::{EndReason, Message, RefuseReason, Stage};
use telescreen_transport::{NetCommand, NetEvent};

use crate::controller::NetPort;

const DISCOVERY_DURATION: Duration = Duration::from_secs(600);

/// Injects key events. A stage process has no other injector (STG-5).
pub trait KeyInjector {
    fn key(&mut self, key: Key, down: bool) -> Result<(), PlatformError>;
}

/// The ports of a stage agent.
pub struct StagePorts {
    pub net: Box<dyn NetPort>,
    pub keys: Box<dyn KeyInjector>,
    pub discovery: Box<dyn Discovery>,
}

impl std::fmt::Debug for StagePorts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StagePorts").finish_non_exhaustive()
    }
}

/// A config that cannot run the stage.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum StageError {
    #[error("the config has no computer {0}")]
    UnknownComputer(ComputerId),
    /// STG-2: the section of this computer has no `stages.login`.
    #[error("the config of {0} has no stages.login")]
    NoStage(ComputerId),
}

/// The agent of a stage process. The caller gives it the network events, the
/// login event, and the stop request, and it uses the ports.
pub struct StageAgent {
    config: Config,
    local: ComputerId,
    fingerprint: String,
    ports: StagePorts,
    finished: bool,
    session: Option<StageTargetSession>,
    next_session: u64,
    peers: BTreeSet<ComputerId>,
    discovery_until: Option<Instant>,
}

impl std::fmt::Debug for StageAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StageAgent")
            .field("local", &self.local)
            .field("finished", &self.finished)
            .finish_non_exhaustive()
    }
}

impl StageAgent {
    /// The stage of this agent.
    pub const STAGE: Stage = Stage::Login;

    /// Makes the agent of the login stage of `local`. `fingerprint` is the
    /// fingerprint of this computer, for the discovery record.
    pub fn new(
        config: Config,
        local: ComputerId,
        fingerprint: String,
        ports: StagePorts,
    ) -> Result<Self, StageError> {
        let computer = config
            .computers
            .get(&local)
            .ok_or_else(|| StageError::UnknownComputer(local.clone()))?;
        if computer.stages.login.is_none() {
            return Err(StageError::NoStage(local));
        }
        Ok(Self {
            config,
            local,
            fingerprint,
            ports,
            finished: false,
            session: None,
            next_session: 1,
            peers: BTreeSet::new(),
            discovery_until: None,
        })
    }

    /// Starts the stage (STG-2): makes this computer discoverable if its
    /// config has `endpoint.discovery`. Returns the lines for the terminal.
    pub fn start(&mut self, now: Instant) -> Vec<String> {
        if self.finished {
            return Vec::new();
        }
        let Some(computer) = self.config.computers.get(&self.local) else {
            return Vec::new();
        };
        let label = computer.label.as_deref().unwrap_or(&self.local.0);
        let sources = computer
            .stages
            .login
            .iter()
            .flat_map(|stage| &stage.from)
            .map(|source| {
                self.config
                    .computers
                    .get(source)
                    .and_then(|computer| computer.label.as_deref())
                    .unwrap_or(&source.0)
            })
            .collect::<Vec<_>>()
            .join(", ");
        let mut lines = vec![
            format!("Telescreen login stage on {label}."),
            format!("These computers can send keys to the login screen: {sources}."),
        ];
        lines.push(if computer.endpoint.discovery.is_some() {
            match self
                .ports
                .discovery
                .announce(computer.endpoint.port, &self.fingerprint)
            {
                Ok(()) => {
                    self.discovery_until = Some(now + DISCOVERY_DURATION);
                    "This computer is discoverable for 10 minutes.".into()
                }
                Err(error) => format!("Discovery did not start: {error}."),
            }
        } else {
            "Discovery is off in the config of this computer.".into()
        });
        lines.push("To stop, press Control+C.".into());
        lines
    }

    /// A report from the transport.
    pub fn on_net(&mut self, event: NetEvent, _now: Instant) {
        if self.finished {
            return;
        }
        match event {
            NetEvent::Connected { peer, .. } => {
                self.peers.insert(peer);
            }
            NetEvent::Disconnected { peer, .. } => {
                self.peers.remove(&peer);
                if self.session.as_ref().is_some_and(|s| s.source() == &peer)
                    && let Some(mut session) = self.session.take()
                {
                    let actions = session.on_peer_lost();
                    self.run_actions(&mut session, actions);
                }
            }
            NetEvent::Message {
                peer,
                message: Message::RequestSession { device },
            } => {
                let permitted = self
                    .config
                    .computers
                    .get(&self.local)
                    .is_some_and(|computer| computer.login_stage_permits(&peer));
                let refusal = if !permitted {
                    Some(RefuseReason::NoRoute)
                } else if self.session.is_some() {
                    Some(RefuseReason::Busy)
                } else {
                    None
                };
                if let Some(reason) = refusal {
                    self.ports.net.send(NetCommand::Send {
                        peer,
                        message: Message::SessionRefused { device, reason },
                    });
                } else {
                    let (mut session, actions) =
                        StageTargetSession::new(SessionId(self.next_session), peer, device);
                    self.next_session += 1;
                    self.run_actions(&mut session, actions);
                    self.session = Some(session);
                }
            }
            NetEvent::Message { peer, message } => {
                if self.session.as_ref().is_some_and(|s| s.source() == &peer)
                    && let Some(mut session) = self.session.take()
                {
                    let actions = session.on_message(&message);
                    self.run_actions(&mut session, actions);
                    if !session.is_finished() {
                        self.session = Some(session);
                    }
                }
            }
            _ => {}
        }
    }

    /// The user logged in to a desktop session (STG-9): ends the session with
    /// `LoginComplete`, then finishes.
    pub fn on_login(&mut self) {
        if !self.finished {
            self.end_session(EndReason::LoginComplete);
            self.finish();
        }
    }

    /// A stop request (STG-4): ends the session with `TargetDisconnected`,
    /// then finishes.
    pub fn stop(&mut self) {
        if !self.finished {
            self.end_session(EndReason::TargetDisconnected);
            self.finish();
        }
    }

    /// The time passes: the discovery record ends after the time of NET-14.
    pub fn tick(&mut self, now: Instant) {
        if self.discovery_until.is_some_and(|until| now >= until) {
            self.stop_announce();
        }
    }

    /// True after `on_login` or `stop`: the caller ends the process.
    pub fn is_finished(&self) -> bool {
        self.finished
    }

    fn run_actions(&mut self, session: &mut StageTargetSession, actions: Vec<StageTargetAction>) {
        let mut actions = VecDeque::from(actions);
        let mut failed_keys = BTreeSet::new();
        while let Some(action) = actions.pop_front() {
            match action {
                StageTargetAction::Send(message) => self.ports.net.send(NetCommand::Send {
                    peer: session.source().clone(),
                    message,
                }),
                StageTargetAction::Inject(InputEvent::Key { key, down }) => {
                    if !failed_keys.contains(&key) && self.ports.keys.key(key, down).is_err() {
                        failed_keys.insert(key);
                        actions.extend(session.end(EndReason::PermissionLost));
                    }
                }
                StageTargetAction::Inject(_) => {}
            }
        }
    }

    fn end_session(&mut self, reason: EndReason) {
        if let Some(mut session) = self.session.take() {
            let actions = session.end(reason);
            self.run_actions(&mut session, actions);
        }
    }

    fn stop_announce(&mut self) {
        if self.discovery_until.take().is_some() {
            self.ports.discovery.stop_announce();
        }
    }

    fn finish(&mut self) {
        self.stop_announce();
        for peer in std::mem::take(&mut self.peers) {
            self.ports.net.send(NetCommand::Close { peer });
        }
        self.ports.net.shutdown();
        self.finished = true;
    }
}
