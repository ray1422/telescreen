//! The controller: it connects the session rules of the core to the network, the
//! platform, the driver jobs, and the view. It has no OS code and no UI library,
//! so tests can run two controllers with fake ports (`tests/controller.rs`).
//!
//! All methods run on one thread (the UI thread). Ports must not block.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::Instant;

use telescreen_core::config::{Config, Device, Operation};
use telescreen_core::driver::{self, DeviceRef, OperationKind, Outcome, PortalInfo, Request};
use telescreen_core::ids::{ComputerId, DeviceId, SessionId};
use telescreen_core::platform::{
    Capability, CapturedEvent, DisplayInfo, Displays, Injector, InputCapabilities, InputCapture,
};
use telescreen_core::pointer::{self, Motion, Point, Rect};
use telescreen_core::routes;
use telescreen_core::session::lease::LeaseTable;
use telescreen_core::session::source::{HandoffResult, SourceAction, SourcePhase, SourceSession};
use telescreen_core::session::target::{TargetAction, TargetPhase, TargetSession};
use telescreen_core::wire::{EndReason, Message, RefuseReason};
use telescreen_transport::{NetCommand, NetEvent};
use telescreen_view::{
    CandidateRow, ControlScreen, DeviceChoice, DisplayRow, IncomingRow, Intent, Matching,
    PortalScreen, PromptPresenter, PromptScreen, SessionPhase, SessionRow,
};

/// Sends commands to the transport.
pub trait NetPort {
    fn send(&mut self, command: NetCommand);
}

/// A driver job. The runner runs it on a worker and reports the result with
/// [`Controller::on_job_done`]. Never run a job on the UI thread.
#[derive(Debug, Clone)]
pub enum Job {
    Operation {
        operation: Operation,
        kind: OperationKind,
        request: Request,
    },
    /// The steps in sequence, then `verify` if present (FLOW-6).
    Handoff {
        steps: Vec<Operation>,
        verify: Option<Operation>,
        request: Request,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub enum JobOutput {
    Operation(Result<Outcome, String>),
    Handoff {
        result: HandoffResult,
        error: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct JobId(pub u64);

pub trait JobPort {
    fn spawn(&mut self, id: JobId, job: Job);
}

/// Runs a job. The job runner of the app calls it on a worker thread.
pub fn run_job(job: &Job, default_cwd: &std::path::Path) -> JobOutput {
    match job {
        Job::Operation {
            operation,
            kind,
            request,
        } => JobOutput::Operation(
            driver::run(operation, *kind, request, default_cwd).map_err(|e| e.to_string()),
        ),
        Job::Handoff {
            steps,
            verify,
            request,
        } => {
            for (completed_steps, step) in steps.iter().enumerate() {
                if let Err(e) = driver::run(step, OperationKind::Handoff, request, default_cwd) {
                    return JobOutput::Handoff {
                        result: HandoffResult::Failed { completed_steps },
                        error: Some(e.to_string()),
                    };
                }
            }
            let Some(verify) = verify else {
                return JobOutput::Handoff {
                    result: HandoffResult::Done { verified: None },
                    error: None,
                };
            };
            match driver::run(verify, OperationKind::Verify, request, default_cwd) {
                Ok(Outcome::Matched(matched)) => JobOutput::Handoff {
                    result: HandoffResult::Done {
                        verified: Some(matched),
                    },
                    error: None,
                },
                Ok(other) => JobOutput::Handoff {
                    result: HandoffResult::Failed {
                        completed_steps: steps.len(),
                    },
                    error: Some(format!("verify gave {other:?}")),
                },
                Err(e) => JobOutput::Handoff {
                    result: HandoffResult::Failed {
                        completed_steps: steps.len(),
                    },
                    error: Some(format!("verify: {e}")),
                },
            }
        }
    }
}

/// The platform ports of one peer.
#[allow(missing_debug_implementations)] // Trait objects of the platform.
pub struct Ports {
    pub net: Box<dyn NetPort>,
    pub jobs: Box<dyn JobPort>,
    pub displays: Box<dyn Displays>,
    pub capture: Box<dyn InputCapture>,
    /// `None` if this peer cannot inject input.
    pub injector: Option<Box<dyn Injector>>,
    pub prompt: Box<dyn PromptPresenter>,
    pub capabilities: InputCapabilities,
}

/// Fixed texts of this platform (v0.1.0 has no settings for them).
#[derive(Debug, Clone)]
pub struct Labels {
    pub confirm_key: String,
    pub release_shortcut: String,
}

enum Pending {
    PortalMatch {
        portal: u64,
        alias: String,
    },
    InputTarget {
        session: SessionId,
        peer: ComputerId,
        device: DeviceId,
    },
    Handoff {
        session: Option<SessionId>,
    },
}

struct Portal {
    id: u64,
    display: DisplayInfo,
    bounds: Rect,
    others: Vec<Rect>,
    focused: bool,
    matching: Matching,
    match_pending: usize,
    matched: Vec<String>,
    match_error: Option<String>,
    device: Option<String>,
    session: Option<SourceSession>,
    pointer: Point,
    warp_on_pause: Option<Point>,
    notice: Option<String>,
}

#[allow(missing_debug_implementations)] // Holds the ports.
pub struct Controller {
    config: Config,
    local: ComputerId,
    config_dir: PathBuf,
    labels: Labels,
    fingerprint: String,
    ports: Ports,
    portal: Option<Portal>,
    next_portal: u64,
    connected: BTreeSet<ComputerId>,
    leases: LeaseTable,
    incoming: BTreeMap<SessionId, TargetSession>,
    injector_session: Option<SessionId>,
    jobs: HashMap<JobId, Pending>,
    next_job: u64,
    notices: Vec<String>,
    /// The last display list, for the view. `displays()` refreshes it.
    display_cache: Vec<DisplayInfo>,
    last_heartbeat: Option<Instant>,
}

const MAX_NOTICES: usize = 8;
/// ADR 0007: the heartbeat interval. The transport closes a connection after 5 s.
const HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(1);

impl Controller {
    pub fn new(
        config: Config,
        local: ComputerId,
        config_dir: PathBuf,
        labels: Labels,
        fingerprint: String,
        first_generation: u64,
        ports: Ports,
    ) -> Self {
        let mut controller = Self {
            config,
            local,
            config_dir,
            labels,
            fingerprint,
            ports,
            portal: None,
            next_portal: 1,
            connected: BTreeSet::new(),
            leases: LeaseTable::new(first_generation),
            incoming: BTreeMap::new(),
            injector_session: None,
            jobs: HashMap::new(),
            next_job: 1,
            notices: Vec::new(),
            display_cache: Vec::new(),
            last_heartbeat: None,
        };
        controller.displays();
        let caps = controller.ports.capabilities.clone();
        for (name, cap) in [
            ("capture", &caps.capture),
            ("suppress", &caps.suppress),
            ("inject", &caps.inject),
        ] {
            if let Capability::Missing(reason) = cap {
                controller.notice(format!("Input {name} is not available: {reason}"));
            }
        }
        controller
    }

    pub fn local(&self) -> &ComputerId {
        &self.local
    }

    pub fn config_dir(&self) -> &std::path::Path {
        &self.config_dir
    }

    /// Adds a notice for the user (IN-12).
    pub fn add_notice(&mut self, text: String) {
        self.notice(text);
    }

    fn notice(&mut self, text: String) {
        self.notices.push(text);
        if self.notices.len() > MAX_NOTICES {
            self.notices.remove(0);
        }
    }

    fn label_of(&self, computer: &ComputerId) -> String {
        self.config
            .computer(computer)
            .and_then(|c| c.label.clone())
            .unwrap_or_else(|| computer.0.clone())
    }

    fn local_devices(&self) -> Vec<(String, Device)> {
        self.config
            .computer(&self.local)
            .map(|c| {
                c.devices
                    .iter()
                    .map(|(a, d)| (a.clone(), d.clone()))
                    .collect()
            })
            .unwrap_or_default()
    }

    fn displays(&mut self) -> Vec<DisplayInfo> {
        match self.ports.displays.displays() {
            Ok(displays) => {
                self.display_cache = displays.clone();
                displays
            }
            Err(e) => {
                self.notice(format!("Cannot list the displays: {e}"));
                Vec::new()
            }
        }
    }

    fn spawn(&mut self, pending: Pending, job: Job) {
        let id = JobId(self.next_job);
        self.next_job += 1;
        self.jobs.insert(id, pending);
        self.ports.jobs.spawn(id, job);
    }

    // ----- Portal and source side -----

    /// The user opened a portal on a display. `others` are the bounds of the other
    /// local displays (for the pause edges, FLOW-14). Starts `portal-match` (FLOW-2).
    pub fn open_portal(&mut self, display: DisplayInfo, others: Vec<Rect>) {
        self.close_portal(EndReason::SourceClosed);
        let Some(bounds) = display.bounds else {
            self.notice(
                "The portal display has no coordinates. Forwarding is not possible.".into(),
            );
            return;
        };
        let id = self.next_portal;
        self.next_portal += 1;
        self.portal = Some(Portal {
            id,
            display,
            bounds,
            others,
            focused: true,
            matching: Matching::Evaluating,
            match_pending: 0,
            matched: Vec::new(),
            match_error: None,
            device: None,
            session: None,
            pointer: Point::default(),
            warp_on_pause: None,
            notice: None,
        });
        self.evaluate_portal_match();
    }

    /// The portal moved to a different display, or the display topology changed
    /// (FLOW-4). The old device binding does not apply.
    pub fn portal_display_changed(&mut self, display: DisplayInfo, others: Vec<Rect>) {
        self.end_portal_session(EndReason::PortalGone);
        let Some(portal) = self.portal.as_mut() else {
            return;
        };
        let Some(bounds) = display.bounds else {
            self.close_portal(EndReason::PortalGone);
            return;
        };
        portal.display = display;
        portal.bounds = bounds;
        portal.others = others;
        portal.device = None;
        self.evaluate_portal_match();
    }

    fn evaluate_portal_match(&mut self) {
        let displays = self.displays();
        let devices = self.local_devices();
        let Some(portal) = self.portal.as_mut() else {
            return;
        };
        portal.id = self.next_portal;
        self.next_portal += 1;
        portal.matching = Matching::Evaluating;
        portal.matched.clear();
        portal.match_error = None;
        let portal_id = portal.id;
        let portal_info = portal_info(portal);
        let mut jobs = Vec::new();
        for (alias, device) in devices {
            let Some(operation) = device.portal_match.clone() else {
                continue;
            };
            let request = Request {
                computer_id: self.local.clone(),
                device: DeviceRef {
                    id: device.id.clone(),
                    alias: alias.clone(),
                },
                source_computer_id: self.local.clone(),
                target_computer_id: None,
                session_id: None,
                portal: Some(portal_info.clone()),
                displays: displays.clone(),
            };
            jobs.push((
                alias,
                Job::Operation {
                    operation,
                    kind: OperationKind::PortalMatch,
                    request,
                },
            ));
        }
        portal.match_pending = jobs.len();
        if jobs.is_empty() {
            portal.matching = Matching::NoMatch;
        }
        for (alias, job) in jobs {
            self.spawn(
                Pending::PortalMatch {
                    portal: portal_id,
                    alias,
                },
                job,
            );
        }
    }

    fn finish_portal_match(&mut self) {
        let devices = self.local_devices();
        let Some(portal) = self.portal.as_mut() else {
            return;
        };
        if portal.match_pending > 0 {
            return;
        }
        let choice = |alias: &String| DeviceChoice {
            alias: alias.clone(),
            label: devices
                .iter()
                .find(|(a, _)| a == alias)
                .and_then(|(_, d)| d.label.clone())
                .unwrap_or_else(|| alias.clone()),
        };
        // AC-9: an error, no match, or more than one match never selects a device.
        portal.matching = if let Some(error) = portal.match_error.clone() {
            Matching::Failed(error)
        } else {
            match portal.matched.as_slice() {
                [] => Matching::NoMatch,
                [alias] => {
                    portal.device = Some(alias.clone());
                    Matching::Matched(choice(alias))
                }
                many => Matching::Ambiguous(many.iter().map(choice).collect()),
            }
        };
        self.dial_candidates();
    }

    fn portal_device(&self) -> Option<Device> {
        let alias = self.portal.as_ref()?.device.as_ref()?;
        self.config
            .computer(&self.local)?
            .devices
            .get(alias)
            .cloned()
    }

    fn dial_candidates(&mut self) {
        let Some(device) = self.portal_device() else {
            return;
        };
        let targets: Vec<_> = routes::candidates(&self.config, &device)
            .iter()
            .filter_map(|r| {
                Some((
                    r.target.clone(),
                    self.config.computer(r.target)?.endpoint.clone(),
                ))
            })
            .collect();
        for (peer, endpoint) in targets {
            if !self.connected.contains(&peer) {
                self.ports.net.send(NetCommand::Dial { peer, endpoint });
            }
        }
    }

    pub fn close_portal(&mut self, reason: EndReason) {
        self.end_portal_session(reason);
        if self.portal.take().is_some() {
            self.ports.capture.disarm();
        }
    }

    fn end_portal_session(&mut self, reason: EndReason) {
        let actions = match self.portal.as_mut().and_then(|p| p.session.as_mut()) {
            Some(session) => session.end(reason),
            None => return,
        };
        self.run_source_actions(actions);
    }

    pub fn set_portal_focus(&mut self, focused: bool) {
        if let Some(portal) = self.portal.as_mut() {
            portal.focused = focused;
        }
    }

    /// The confirmation key or a click in the portal (FLOW-8). Returns true if it
    /// was a confirmation: then the window layer must not use the event (FLOW-9).
    pub fn confirm(&mut self) -> bool {
        let Some(portal) = self.portal.as_mut() else {
            return false;
        };
        let focused = portal.focused;
        let bounds = portal.bounds;
        let Some(actions) = portal.session.as_mut().and_then(|s| s.confirm(focused)) else {
            return false;
        };
        if let Err(e) = self.ports.capture.arm(bounds) {
            self.notice(format!("Input capture failed: {e}"));
            self.end_portal_session(EndReason::PermissionLost);
            return true;
        }
        self.run_source_actions(actions);
        true
    }

    /// True if the next confirmation key or click is a confirmation.
    pub fn awaiting_confirmation(&self) -> bool {
        self.portal_phase() == Some(SourcePhase::AwaitingConfirm)
    }

    fn portal_phase(&self) -> Option<SourcePhase> {
        Some(self.portal.as_ref()?.session.as_ref()?.phase())
    }

    pub fn start_handoff(&mut self, target: ComputerId) {
        if let Some(reason) = self.candidate_unavailable(&target) {
            self.notice(format!(
                "Cannot switch to {}: {reason}",
                self.label_of(&target)
            ));
            return;
        }
        let Some(device) = self.portal_device() else {
            return;
        };
        let (session, actions) = SourceSession::start(target, device.id);
        if let Some(portal) = self.portal.as_mut() {
            portal.session = Some(session);
            portal.notice = None;
        }
        self.run_source_actions(actions);
    }

    fn candidate_unavailable(&self, target: &ComputerId) -> Option<String> {
        let device = self.portal_device()?;
        let route = routes::candidates(&self.config, &device)
            .into_iter()
            .find(|r| r.target == target);
        let Some(route) = route else {
            return Some("no handoff route".into());
        };
        if route.target_device.input_target.is_none() {
            return Some(format!("{} has no input-target for this device", target));
        }
        if let Capability::Missing(reason) = &self.ports.capabilities.capture {
            return Some(format!("input capture: {reason}"));
        }
        if !self.connected.contains(target) {
            return Some("offline".into());
        }
        let busy = self
            .portal
            .as_ref()?
            .session
            .as_ref()
            .is_some_and(|s| !s.is_finished());
        busy.then(|| "a session is active".into())
    }

    fn run_source_actions(&mut self, actions: Vec<SourceAction>) {
        for action in actions {
            let Some(portal) = self.portal.as_mut() else {
                return;
            };
            let Some(session) = portal.session.as_ref() else {
                return;
            };
            let target = session.target().clone();
            match action {
                SourceAction::Send(message) => {
                    self.ports.net.send(NetCommand::Send {
                        peer: target,
                        message,
                    });
                }
                SourceAction::RunHandoff => self.run_handoff(),
                SourceAction::StartForwarding => match self.ports.capture.forward() {
                    Ok(at) => {
                        if let Some(portal) = self.portal.as_mut() {
                            portal.pointer = portal.bounds.clamp(at);
                        }
                        self.send_pointer();
                    }
                    Err(e) => {
                        self.notice(format!("Input capture failed: {e}"));
                        let actions = self.end_actions(EndReason::PermissionLost);
                        self.run_source_actions(actions);
                        return;
                    }
                },
                SourceAction::StopForwarding => {
                    let at = portal.warp_on_pause.take();
                    self.ports.capture.pause(at);
                }
            }
        }
        let finished = self
            .portal
            .as_ref()
            .and_then(|p| p.session.as_ref())
            .is_some_and(SourceSession::is_finished);
        if finished {
            self.ports.capture.disarm();
        }
    }

    fn end_actions(&mut self, reason: EndReason) -> Vec<SourceAction> {
        self.portal
            .as_mut()
            .and_then(|p| p.session.as_mut())
            .map(|s| s.end(reason))
            .unwrap_or_default()
    }

    fn run_handoff(&mut self) {
        let displays = self.displays();
        let Some(portal) = self.portal.as_ref() else {
            return;
        };
        let Some(session) = portal.session.as_ref() else {
            return;
        };
        let Some(alias) = portal.device.clone() else {
            return;
        };
        let Some(device) = self.portal_device() else {
            return;
        };
        let target = session.target().clone();
        let session_id = session.session();
        let Some(handoff) = device.handoff.get(&target).cloned() else {
            return;
        };
        let request = Request {
            computer_id: self.local.clone(),
            device: DeviceRef {
                id: device.id.clone(),
                alias,
            },
            source_computer_id: self.local.clone(),
            target_computer_id: Some(target),
            session_id,
            portal: Some(portal_info(portal)),
            displays,
        };
        self.spawn(
            Pending::Handoff {
                session: session_id,
            },
            Job::Handoff {
                steps: handoff.steps,
                verify: handoff.verify,
                request,
            },
        );
    }

    fn send_pointer(&mut self) {
        let Some(portal) = self.portal.as_mut() else {
            return;
        };
        let position = pointer::normalize(portal.bounds, portal.pointer);
        let Some(session) = portal.session.as_mut() else {
            return;
        };
        if let Some(datagram) = session.pointer(position) {
            let peer = session.target().clone();
            self.ports
                .net
                .send(NetCommand::SendPointer { peer, datagram });
        }
    }

    /// An event from the input capture (IN-2: it comes through a bounded queue).
    pub fn on_captured(&mut self, event: CapturedEvent) {
        let Some(portal) = self.portal.as_mut() else {
            return;
        };
        let Some(session) = portal.session.as_mut() else {
            return;
        };
        let actions = match event {
            CapturedEvent::Input(input) => session.on_input(input),
            CapturedEvent::Motion { dx, dy } => {
                if session.phase() != SourcePhase::Forwarding {
                    return;
                }
                match pointer::step(portal.bounds, &portal.others, portal.pointer, dx, dy) {
                    Motion::Inside(at) => {
                        portal.pointer = at;
                        self.send_pointer();
                        return;
                    }
                    Motion::Exit(at) => {
                        portal.warp_on_pause = Some(at);
                        session.set_pointer_inside(false)
                    }
                }
            }
            CapturedEvent::PointerEntered { at } => {
                portal.pointer = portal.bounds.clamp(at);
                session.set_pointer_inside(true)
            }
            CapturedEvent::PointerLeft => session.set_pointer_inside(false),
            CapturedEvent::ReleaseShortcut => session.end(EndReason::ReleaseShortcut),
            CapturedEvent::SafetyExit => session.end(EndReason::SafetyExit),
            CapturedEvent::Overflow => session.end(EndReason::QueueOverflow),
        };
        self.run_source_actions(actions);
    }

    // ----- Timers -----

    /// The main loop calls this often. It runs the timers and sends the heartbeat
    /// to each connected peer once a second, from this thread, so that a hung app
    /// stops its heartbeat and the other peer ends the sessions (ADR 0007, AC-18).
    pub fn tick(&mut self, now: Instant) {
        if self
            .last_heartbeat
            .is_none_or(|t| now.duration_since(t) >= HEARTBEAT)
        {
            self.last_heartbeat = Some(now);
            for peer in self.connected.clone() {
                self.ports.net.send(NetCommand::Send {
                    peer,
                    message: Message::Heartbeat,
                });
            }
        }
        let actions = self
            .portal
            .as_mut()
            .and_then(|p| p.session.as_mut())
            .map(|s| s.tick(now))
            .unwrap_or_default();
        self.run_source_actions(actions);
    }

    // ----- Network -----

    pub fn on_net(&mut self, event: NetEvent, now: Instant) {
        match event {
            NetEvent::Connected { peer } => {
                self.connected.insert(peer);
            }
            NetEvent::Disconnected { peer, reason } => {
                self.connected.remove(&peer);
                self.peer_lost(&peer);
                self.notice(format!("{} disconnected: {reason}", self.label_of(&peer)));
            }
            NetEvent::DialFailed { peer, reason } => {
                self.notice(format!(
                    "Cannot connect to {}: {reason}",
                    self.label_of(&peer)
                ));
            }
            NetEvent::Refused { remote, reason } => {
                self.notice(format!("Refused a connection from {remote}: {reason}"));
            }
            NetEvent::Message { peer, message } => self.on_message(peer, message, now),
            NetEvent::Pointer { peer, datagram } => {
                let Some(target) = self.incoming.get_mut(&datagram.session) else {
                    return;
                };
                if target.source() != &peer {
                    return;
                }
                if let Some(action) = target.on_pointer(&datagram) {
                    let session = datagram.session;
                    self.run_target_actions(session, vec![action]);
                }
            }
        }
    }

    fn peer_lost(&mut self, peer: &ComputerId) {
        let source_actions = self
            .portal
            .as_mut()
            .and_then(|p| p.session.as_mut())
            .filter(|s| s.target() == peer)
            .map(SourceSession::on_peer_lost)
            .unwrap_or_default();
        self.run_source_actions(source_actions);
        let sessions: Vec<_> = self
            .incoming
            .iter()
            .filter(|(_, t)| t.source() == peer)
            .map(|(s, _)| *s)
            .collect();
        for session in sessions {
            if let Some(target) = self.incoming.get_mut(&session) {
                let actions = target.on_peer_lost();
                self.run_target_actions(session, actions);
            }
        }
    }

    fn on_message(&mut self, peer: ComputerId, message: Message, now: Instant) {
        match &message {
            // To the source side.
            Message::SessionGranted { .. }
            | Message::SessionRefused { .. }
            | Message::PromptShown { .. }
            | Message::Started { .. } => self.source_message(&peer, &message, now),
            Message::End { session, .. } if !self.incoming.contains_key(session) => {
                self.source_message(&peer, &message, now)
            }
            // To the target side.
            Message::RequestSession { device } => self.request_session(peer, device.clone()),
            Message::HandoffDone { session }
            | Message::Confirm { session }
            | Message::Input { session, .. }
            | Message::ReleaseAll { session }
            | Message::End { session, .. } => {
                let session = *session;
                let Some(target) = self.incoming.get_mut(&session) else {
                    return;
                };
                if target.source() != &peer {
                    return;
                }
                let actions = target.on_message(&message);
                self.run_target_actions(session, actions);
            }
            Message::Hello { .. } | Message::Heartbeat => {}
        }
    }

    fn source_message(&mut self, peer: &ComputerId, message: &Message, now: Instant) {
        let Some(session) = self.portal.as_mut().and_then(|p| p.session.as_mut()) else {
            return;
        };
        if session.target() != peer {
            return;
        }
        let actions = session.on_message(message, now);
        if let Message::SessionRefused { reason, .. } = message
            && let Some(portal) = self.portal.as_mut()
        {
            portal.notice = Some(refuse_text(*reason));
        }
        self.run_source_actions(actions);
    }

    // ----- Target side -----

    fn refuse(&mut self, peer: ComputerId, device: DeviceId, reason: RefuseReason) {
        self.ports.net.send(NetCommand::Send {
            peer,
            message: Message::SessionRefused { device, reason },
        });
    }

    /// FLOW-5 on the target: route, input-target, injection, lease.
    fn request_session(&mut self, peer: ComputerId, device_id: DeviceId) {
        let local = self
            .config
            .computer(&self.local)
            .and_then(|c| c.device_by_id(&device_id));
        let Some((alias, device)) = local.map(|(a, d)| (a.to_owned(), d.clone())) else {
            return self.refuse(peer, device_id, RefuseReason::UnknownDevice);
        };
        // NET-4: an authenticated peer still needs a configured route to this computer.
        let route = self
            .config
            .computer(&peer)
            .and_then(|c| c.device_by_id(&device_id))
            .is_some_and(|(_, d)| d.handoff.contains_key(&self.local));
        if !route {
            return self.refuse(peer, device_id, RefuseReason::NoRoute);
        }
        let Some(operation) = device.input_target.clone() else {
            return self.refuse(peer, device_id, RefuseReason::NoInputTarget);
        };
        if self.ports.injector.is_none()
            || matches!(self.ports.capabilities.inject, Capability::Missing(_))
        {
            return self.refuse(peer, device_id, RefuseReason::NoInjection);
        }
        let session = match self.leases.acquire(&device_id, &peer) {
            Ok(session) => session,
            Err(_) => return self.refuse(peer, device_id, RefuseReason::Busy),
        };
        let request = Request {
            computer_id: self.local.clone(),
            device: DeviceRef {
                id: device_id.clone(),
                alias,
            },
            source_computer_id: peer.clone(),
            target_computer_id: Some(self.local.clone()),
            session_id: Some(session),
            portal: None,
            displays: self.displays(),
        };
        self.spawn(
            Pending::InputTarget {
                session,
                peer,
                device: device_id,
            },
            Job::Operation {
                operation,
                kind: OperationKind::InputTarget,
                request,
            },
        );
    }

    fn input_target_done(
        &mut self,
        session: SessionId,
        peer: ComputerId,
        device: DeviceId,
        output: JobOutput,
    ) {
        let token = match output {
            JobOutput::Operation(Ok(Outcome::Display(token))) => token,
            JobOutput::Operation(Err(e)) => {
                self.notice(format!("input-target failed: {e}"));
                self.leases.release(session);
                return self.refuse(peer, device, RefuseReason::NoInputTarget);
            }
            _ => {
                self.leases.release(session);
                return self.refuse(peer, device, RefuseReason::NoInputTarget);
            }
        };
        let (target, actions) = TargetSession::new(session, peer, device, token);
        self.incoming.insert(session, target);
        self.run_target_actions(session, actions);
    }

    fn find_display(&mut self, session: SessionId) -> Option<DisplayInfo> {
        let token = self.incoming.get(&session)?.display().clone();
        self.displays().into_iter().find(|d| d.token == token)
    }

    fn run_target_actions(&mut self, session: SessionId, actions: Vec<TargetAction>) {
        let mut queue: std::collections::VecDeque<_> = actions.into();
        while let Some(action) = queue.pop_front() {
            let Some(target) = self.incoming.get(&session) else {
                return;
            };
            let source = target.source().clone();
            match action {
                TargetAction::Send(message) => {
                    self.ports.net.send(NetCommand::Send {
                        peer: source,
                        message,
                    });
                }
                TargetAction::ShowPrompt => {
                    let screen = PromptScreen {
                        source: self.label_of(&source),
                        confirm_key: self.labels.confirm_key.clone(),
                        hint: self
                            .config
                            .computer(&self.local)
                            .and_then(|c| c.hint.clone()),
                    };
                    let shown = match self.find_display(session) {
                        Some(display) => self.ports.prompt.show(session, &display, &screen),
                        None => Err("the input-target display is gone".into()),
                    };
                    if let Err(e) = &shown {
                        self.notice(format!("Cannot show the prompt: {e}"));
                    }
                    let more = self
                        .target_mut(session)
                        .map(|t| t.on_prompt_shown(shown.is_ok()));
                    queue.extend(more.unwrap_or_default());
                }
                TargetAction::HidePrompt => self.ports.prompt.hide(session),
                TargetAction::CheckDisplay => {
                    let display = self.find_display(session);
                    let valid = match (&display, self.ports.injector.as_mut()) {
                        (Some(display), Some(injector)) => match injector.set_display(display) {
                            Ok(()) => true,
                            Err(e) => {
                                self.notice(format!("Cannot inject on the display: {e}"));
                                false
                            }
                        },
                        _ => false,
                    };
                    if valid {
                        self.injector_session = Some(session);
                    }
                    let more = self
                        .target_mut(session)
                        .map(|t| t.on_display_checked(valid));
                    queue.extend(more.unwrap_or_default());
                }
                TargetAction::Inject(event) => {
                    if let Some(injector) = self.ports.injector.as_mut()
                        && let Err(e) = injector.input(&event)
                    {
                        self.notice(format!("Injection failed: {e}"));
                    }
                }
                TargetAction::MovePointer(at) => {
                    if self.injector_session != Some(session) {
                        let display = self.find_display(session);
                        if let (Some(display), Some(injector)) =
                            (display, self.ports.injector.as_mut())
                        {
                            let _ = injector.set_display(&display);
                            self.injector_session = Some(session);
                        }
                    }
                    if let Some(injector) = self.ports.injector.as_mut() {
                        let _ = injector.pointer(at);
                    }
                }
                TargetAction::ReleaseLease => {
                    self.leases.release(session);
                }
            }
        }
        if self
            .incoming
            .get(&session)
            .is_some_and(TargetSession::is_finished)
        {
            self.incoming.remove(&session);
            if self.injector_session == Some(session) {
                self.injector_session = None;
            }
        }
    }

    fn target_mut(&mut self, session: SessionId) -> Option<&mut TargetSession> {
        self.incoming.get_mut(&session)
    }

    // ----- Jobs -----

    pub fn on_job_done(&mut self, id: JobId, output: JobOutput) {
        let Some(pending) = self.jobs.remove(&id) else {
            return;
        };
        match pending {
            Pending::PortalMatch { portal, alias } => {
                let Some(p) = self.portal.as_mut().filter(|p| p.id == portal) else {
                    return;
                };
                p.match_pending = p.match_pending.saturating_sub(1);
                match output {
                    JobOutput::Operation(Ok(Outcome::Matched(true))) => p.matched.push(alias),
                    JobOutput::Operation(Ok(Outcome::Matched(false))) => {}
                    JobOutput::Operation(Err(e)) => p.match_error = Some(format!("{alias}: {e}")),
                    other => p.match_error = Some(format!("{alias}: unexpected {other:?}")),
                }
                self.finish_portal_match();
            }
            Pending::InputTarget {
                session,
                peer,
                device,
            } => self.input_target_done(session, peer, device, output),
            Pending::Handoff { session } => {
                let JobOutput::Handoff { result, error } = output else {
                    return;
                };
                if let Some(error) = error {
                    self.notice(format!("Handoff failed: {error}"));
                }
                let current = self
                    .portal
                    .as_ref()
                    .and_then(|p| p.session.as_ref())
                    .and_then(|s| s.session());
                if current != session || session.is_none() {
                    return;
                }
                let actions = self
                    .portal
                    .as_mut()
                    .and_then(|p| p.session.as_mut())
                    .map(|s| s.on_handoff_finished(result))
                    .unwrap_or_default();
                self.run_source_actions(actions);
            }
        }
    }

    // ----- Intents -----

    pub fn on_intent(&mut self, intent: Intent) {
        match intent {
            Intent::OpenPortal { .. } | Intent::ClosePortal => {
                // The window layer opens and closes the portal window, then calls
                // `open_portal` or `close_portal`.
            }
            Intent::SelectDevice { alias } => {
                if let Some(portal) = self.portal.as_mut()
                    && let Matching::Ambiguous(choices) = &portal.matching
                    && let Some(choice) = choices.iter().find(|c| c.alias == alias).cloned()
                {
                    portal.device = Some(alias);
                    portal.matching = Matching::Matched(choice);
                    self.dial_candidates();
                }
            }
            Intent::StartHandoff { target } => self.start_handoff(target),
            Intent::EndSession => self.end_portal_session(EndReason::SourceClosed),
            Intent::Disconnect { session } => {
                self.end_incoming(session, EndReason::TargetDisconnected)
            }
            Intent::TakeControl { session } => {
                self.end_incoming(session, EndReason::TargetRequestedControl)
            }
        }
    }

    /// The window layer could not show a prompt that the presenter accepted.
    pub fn prompt_failed(&mut self, session: SessionId) {
        self.notice("Cannot show the prompt window.".into());
        self.end_incoming(session, EndReason::PromptFailed);
    }

    fn end_incoming(&mut self, session: SessionId, reason: EndReason) {
        if let Some(target) = self.incoming.get_mut(&session) {
            let actions = target.end(reason);
            self.run_target_actions(session, actions);
        }
    }

    /// The local display list changed (FLOW-4, FLOW-19, and FLOW-10 on the target).
    pub fn displays_changed(&mut self) {
        let displays = self.displays();
        let gone: Vec<_> = self
            .incoming
            .iter()
            .filter(|(_, t)| t.phase() == TargetPhase::Forwarding)
            .filter(|(_, t)| !displays.iter().any(|d| &d.token == t.display()))
            .map(|(s, _)| *s)
            .collect();
        for session in gone {
            self.end_incoming(session, EndReason::DisplayGone);
        }
        let portal_token = self.portal.as_ref().map(|p| p.display.token.clone());
        if let Some(token) = portal_token {
            match displays.iter().find(|d| d.token == token).cloned() {
                None => self.close_portal(EndReason::PortalGone),
                Some(display) => {
                    let changed = self.portal.as_ref().is_some_and(|p| p.display != display);
                    if changed {
                        let others = displays
                            .iter()
                            .filter(|d| d.token != token)
                            .filter_map(|d| d.bounds)
                            .collect();
                        self.portal_display_changed(display, others);
                    }
                }
            }
        }
    }

    // ----- View models -----

    /// The display of the current display list with this token.
    pub fn display(&self, token: &telescreen_core::ids::DisplayToken) -> Option<DisplayInfo> {
        self.display_cache
            .iter()
            .find(|d| &d.token == token)
            .cloned()
    }

    /// The bounds of all displays except the one with this token.
    pub fn other_display_bounds(&self, token: &telescreen_core::ids::DisplayToken) -> Vec<Rect> {
        self.display_cache
            .iter()
            .filter(|d| &d.token != token)
            .filter_map(|d| d.bounds)
            .collect()
    }

    pub fn has_portal(&self) -> bool {
        self.portal.is_some()
    }

    /// True while input is forwarded: the portal must keep the focus (ADR 0003).
    pub fn forwarding(&self) -> bool {
        self.portal_phase() == Some(SourcePhase::Forwarding)
    }

    pub fn portal_screen(&self) -> Option<PortalScreen> {
        let portal = self.portal.as_ref()?;
        let device = self.portal_device();
        let candidates = device
            .as_ref()
            .map(|device| {
                routes::candidates(&self.config, device)
                    .into_iter()
                    .map(|route| CandidateRow {
                        target: route.target.clone(),
                        label: self.label_of(route.target),
                        unavailable: self.candidate_unavailable(route.target),
                    })
                    .collect()
            })
            .unwrap_or_default();
        let session = portal.session.as_ref().map(|s| SessionRow {
            target: self.label_of(s.target()),
            phase: self.phase_view(s.phase()),
            video_input: s.video_input(),
        });
        Some(PortalScreen {
            display: display_name(&portal.display),
            matching: portal.matching.clone(),
            candidates,
            session,
            notice: portal.notice.clone(),
        })
    }

    fn phase_view(&self, phase: SourcePhase) -> SessionPhase {
        match phase {
            SourcePhase::Requesting => SessionPhase::Requesting,
            SourcePhase::HandingOff => SessionPhase::HandingOff,
            SourcePhase::AwaitingPrompt => SessionPhase::WaitingForPrompt,
            SourcePhase::AwaitingConfirm => SessionPhase::WaitingForConfirmation {
                key: self.labels.confirm_key.clone(),
            },
            SourcePhase::Starting => SessionPhase::Starting,
            SourcePhase::Forwarding => SessionPhase::Forwarding,
            SourcePhase::Paused => SessionPhase::Paused {
                release: self.labels.release_shortcut.clone(),
            },
            SourcePhase::Refused(reason) => SessionPhase::Ended {
                reason: refuse_text(reason),
            },
            SourcePhase::Ended(reason) => SessionPhase::Ended {
                reason: end_text(reason).into(),
            },
        }
    }

    pub fn control_screen(&self) -> ControlScreen {
        let portal_token = self.portal.as_ref().map(|p| p.display.token.clone());
        let displays = self
            .display_cache
            .iter()
            .cloned()
            .map(|d| DisplayRow {
                has_portal: Some(&d.token) == portal_token.as_ref(),
                name: display_name(&d),
                token: d.token,
            })
            .collect();
        let incoming = self
            .incoming
            .iter()
            .map(|(session, t)| IncomingRow {
                session: *session,
                source: self.label_of(t.source()),
                state: target_phase_text(t.phase()).into(),
            })
            .collect();
        ControlScreen {
            computer: self.label_of(&self.local),
            fingerprint: self.fingerprint.clone(),
            displays,
            incoming,
            notices: self.notices.clone(),
        }
    }
}

fn portal_info(portal: &Portal) -> PortalInfo {
    PortalInfo {
        id: format!("portal-{}", portal.id),
        bounds: Some(portal.bounds),
        coordinate_space: "local-desktop-logical",
        display_token: Some(portal.display.token.clone()),
    }
}

fn display_name(display: &DisplayInfo) -> String {
    display
        .metadata
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| display.token.0.clone())
}

fn refuse_text(reason: RefuseReason) -> String {
    match reason {
        RefuseReason::UnknownDevice => "the target does not know this device",
        RefuseReason::NoRoute => "the target config has no route from this computer",
        RefuseReason::NoInputTarget => "the target cannot resolve its input-target display",
        RefuseReason::NoInjection => "the target cannot inject input",
        RefuseReason::Busy => "the device is in use by a different session",
    }
    .into()
}

fn end_text(reason: EndReason) -> &'static str {
    match reason {
        EndReason::ReleaseShortcut => "release shortcut",
        EndReason::SafetyExit => "safety exit",
        EndReason::SourceClosed => "ended on this computer",
        EndReason::TargetDisconnected => "disconnected on the target",
        EndReason::TargetRequestedControl => "the target took control back",
        EndReason::ConfirmTimeout => "no confirmation in time",
        EndReason::HandoffFailed => "the handoff failed",
        EndReason::PromptFailed => "the target cannot show the prompt",
        EndReason::DisplayGone => "the target display is gone",
        EndReason::PortalGone => "the portal display is gone",
        EndReason::QueueOverflow => "input queue overflow",
        EndReason::PermissionLost => "a permission is missing",
        EndReason::PeerLost => "connection lost",
        EndReason::ProtocolError => "protocol error",
    }
}

fn target_phase_text(phase: TargetPhase) -> &'static str {
    match phase {
        TargetPhase::AwaitingHandoff => "switching the display",
        TargetPhase::ShowingPrompt | TargetPhase::Prompting => "waiting for confirmation",
        TargetPhase::CheckingDisplay => "starting",
        TargetPhase::Forwarding => "controls this computer",
        TargetPhase::Ended(reason) => end_text(reason),
    }
}
