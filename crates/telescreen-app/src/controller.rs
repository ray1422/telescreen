//! The controller: it connects the session rules of the core to the network, the
//! platform, the driver jobs, and the view. It has no OS code and no UI library,
//! so tests can run two controllers with fake ports (`tests/controller.rs`).
//!
//! All methods run on one thread (the UI thread). Ports must not block.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::PathBuf;
use std::time::Instant;

use telescreen_core::clipboard::{ClipboardChange, ClipboardSync, Outgoing};
use telescreen_core::config::{Config, Device, Driver, Operation};
use telescreen_core::driver::{self, DeviceRef, OperationKind, Outcome, PortalInfo, Request};
use telescreen_core::ids::{ComputerId, DeviceId, DisplayToken, SessionId};
use telescreen_core::input::InputEvent;
use telescreen_core::platform::{
    Capability, CapturedEvent, Clipboard, Discovery, DisplayInfo, Displays, FocusHolder, Injector,
    InputCapabilities, InputCapture, PlatformError,
};
use telescreen_core::pointer::{self, Point, Rect};
use telescreen_core::pointer_stats::{PointerStats, PointerSummary};
use telescreen_core::routes;
use telescreen_core::session::VideoInput;
use telescreen_core::session::lease::LeaseTable;
use telescreen_core::session::source::{HandoffResult, SourceAction, SourcePhase, SourceSession};
use telescreen_core::session::target::{TargetAction, TargetPhase, TargetSession};
use telescreen_core::verification::{Verification, VerificationAction, VerificationResult};
use telescreen_core::verifier::{PromptVerifier, VerifierEvent};
use telescreen_core::wire::{EndReason, HandBackOutcome, Message, RefuseReason};
use telescreen_transport::{DialTarget, NetCommand, NetEvent};
use telescreen_view::{
    CandidateRow, ControlScreen, DeviceChoice, DeviceRow, DiscoveryRow, DisplayRow, IncomingRow,
    Intent, Matching, PortalScreen, PromptId, PromptPresenter, PromptScreen, SessionPhase,
    SessionRow,
};

use crate::pointer_mailbox::ReceivedPointer;

mod actions;

use actions::{ControlMatch, PendingStart};

/// Sends commands to the transport.
pub trait NetPort {
    fn send(&mut self, command: NetCommand);

    /// Sends the queued commands and stops the transport. It blocks until the
    /// transport sent the close of each connection. The app calls it once, at
    /// exit.
    fn shutdown(&mut self) {}
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
    /// The handoff steps in sequence (FLOW-6). The destination verifies separately.
    Handoff {
        steps: Vec<Operation>,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct VerifierId(pub u64);

/// Runs `exec` and `command` verifiers off the UI thread. The app sends the
/// events back with `Controller::on_verifier_event`.
pub trait VerifierPort {
    fn start(&mut self, id: VerifierId, operation: Operation, request: Request);
    fn ack(&mut self, id: VerifierId);
    fn stop(&mut self, id: VerifierId);
}

enum Verifier {
    Prompt(PromptVerifier),
    Process(VerifierId),
}

enum TargetWork {
    Action(TargetAction),
    Event(VerifierEvent),
}

struct HandBackVerification {
    prompt: PromptId,
    display: Option<DisplayInfo>,
    verification: Verification,
    verifier: Option<Verifier>,
}

#[derive(Debug, thiserror::Error)]
enum HandBackVerificationError {
    #[error("the local device is gone (input-target)")]
    DeviceGone,
    #[error("{0} has no input-target")]
    NoInputTarget(String),
    #[error("input-target failed: {0}")]
    InputTargetFailed(String),
    #[error("input-target did not return a display")]
    InvalidInputTarget,
    #[error("the input-target display is gone")]
    DisplayGone,
}

/// Runs a job. The job runner of the app calls it on a worker thread.
pub fn run_job(job: &Job, context: &driver::Context) -> JobOutput {
    match job {
        Job::Operation {
            operation,
            kind,
            request,
        } => JobOutput::Operation(
            driver::run(operation, *kind, request, context).map_err(|e| e.to_string()),
        ),
        Job::Handoff { steps, request } => {
            for (completed_steps, step) in steps.iter().enumerate() {
                if let Err(e) = driver::run(step, OperationKind::Handoff, request, context) {
                    return JobOutput::Handoff {
                        result: HandoffResult::Failed { completed_steps },
                        error: Some(e.to_string()),
                    };
                }
            }
            JobOutput::Handoff {
                result: HandoffResult::Done,
                error: None,
            }
        }
    }
}

/// The platform ports of one peer.
#[allow(missing_debug_implementations)] // Trait objects of the platform.
pub struct Ports {
    pub net: Box<dyn NetPort>,
    pub jobs: Box<dyn JobPort>,
    pub verifiers: Box<dyn VerifierPort>,
    pub displays: Box<dyn Displays>,
    pub capture: Box<dyn InputCapture>,
    /// `None` if this peer cannot inject input.
    pub injector: Option<Box<dyn Injector>>,
    /// `None` if this peer has no focus holder and cannot be a target (UI-7).
    pub focus_holder: Option<Box<dyn FocusHolder>>,
    /// `None` if this peer has no clipboard backend (CLIP-8).
    pub clipboard: Option<Box<dyn Clipboard>>,
    pub prompt: Box<dyn PromptPresenter>,
    /// The system mDNS responder (NET-12).
    pub discovery: Box<dyn Discovery>,
    pub capabilities: InputCapabilities,
}

/// Fixed texts of this platform (v0.1.0 has no settings for them).
#[derive(Debug, Clone)]
pub struct Labels {
    pub confirm_key: String,
    pub hand_back_shortcut: String,
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
    HandBack {
        peer: ComputerId,
        device: DeviceId,
    },
    HandBackInputTarget {
        prompt: PromptId,
        peer: ComputerId,
        device: DeviceId,
    },
    /// The steps of "Switch display only" (ACT-1).
    SwitchOnly {
        target: ComputerId,
        device: DeviceId,
    },
    /// `portal-match` of one display for a control request (ACT-4).
    ControlMatch {
        request: u64,
        display: DisplayInfo,
    },
}

struct Portal {
    id: u64,
    display: DisplayInfo,
    bounds: Rect,
    focused: bool,
    guarded: bool,
    matching: Matching,
    match_pending: usize,
    matched: Vec<String>,
    match_error: Option<String>,
    device: Option<String>,
    /// The session to start after `portal-match` (FLOW-17, ACT-1, ACT-4).
    pending_start: Option<PendingStart>,
    /// The portal closes when its session ends (ACT-5). The portal of
    /// `open_portal` stays until the redesign replaces it.
    ephemeral: bool,
    session: Option<SourceSession>,
    pointer: Point,
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
    clipboard: ClipboardSync,
    clipboard_open: bool,
    clipboard_open_failed: bool,
    clipboard_session: Option<SessionId>,
    portal: Option<Portal>,
    portal_window_request: Option<DisplayToken>,
    next_portal: u64,
    connected: BTreeSet<ComputerId>,
    /// The peers with a dial that did not report a result (NET-10).
    dialing: BTreeSet<ComputerId>,
    /// The newest search result of each peer (NET-16).
    found: HashMap<ComputerId, Vec<DialTarget>>,
    /// The end of the running search (NET-15).
    search_deadline: Option<Instant>,
    /// The end of "Allow discovery" (NET-14).
    discovery_deadline: Option<Instant>,
    /// The time of the last `tick`.
    now: Instant,
    leases: LeaseTable,
    incoming: BTreeMap<SessionId, TargetSession>,
    incoming_pointer_stats: BTreeMap<SessionId, PointerStats>,
    last_pointer_summary: Option<PointerSummary>,
    /// Device state on this destination, independent of sessions (VER-6).
    /// A missing device is `Unchanged`: no handoff to this computer yet.
    device_states: HashMap<DeviceId, VideoInput>,
    injector_session: Option<SessionId>,
    jobs: HashMap<JobId, Pending>,
    next_job: u64,
    verifiers: BTreeMap<SessionId, Verifier>,
    next_verifier: u64,
    prompts: BTreeMap<SessionId, PromptId>,
    next_prompt: u64,
    pending_hand_backs: BTreeSet<(ComputerId, DeviceId)>,
    hand_back_verifications: BTreeMap<DeviceId, HandBackVerification>,
    /// Control requests that this peer sent, with their deadline (ACT-6). A
    /// request waits in `unsent_control` until its peer connects.
    control_requests: BTreeMap<(ComputerId, DeviceId), Instant>,
    unsent_control: BTreeSet<(ComputerId, DeviceId)>,
    /// `portal-match` runs for the control requests that this peer received (ACT-4).
    control_matches: BTreeMap<u64, ControlMatch>,
    next_control_match: u64,
    notices: Vec<String>,
    /// The last display list, for the view. `displays()` refreshes it.
    display_cache: Vec<DisplayInfo>,
    last_heartbeat: Option<Instant>,
    is_shutdown: bool,
}

const MAX_NOTICES: usize = 8;
/// ADR 0007: the heartbeat interval. The transport closes a connection after 5 s.
const HEARTBEAT: std::time::Duration = std::time::Duration::from_secs(1);
const DISCOVERY_WINDOW: std::time::Duration = std::time::Duration::from_secs(600);
const SEARCH_TIME: std::time::Duration = std::time::Duration::from_secs(30);

impl Controller {
    /// Holds keyboard focus on the named local display (UI-7).
    #[cfg(feature = "debug-protocol")]
    pub fn debug_hold_keyboard(&mut self, display: &DisplayToken) -> Result<(), String> {
        let holder = self
            .ports
            .focus_holder
            .as_mut()
            .ok_or_else(|| "no keyboard focus holder is available (UI-7)".to_string())?;
        let display = self
            .ports
            .displays
            .displays()
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(|info| &info.token == display)
            .ok_or_else(|| format!("no display {}", display.0))?;
        holder.hold(&display).map_err(|error| error.to_string())
    }

    /// Removes the debug keyboard focus hold (UI-7).
    #[cfg(feature = "debug-protocol")]
    pub fn debug_release_keyboard(&mut self) {
        if let Some(holder) = self.ports.focus_holder.as_mut() {
            holder.release();
        }
    }

    pub fn new(
        config: Config,
        local: ComputerId,
        config_dir: PathBuf,
        labels: Labels,
        fingerprint: String,
        first_generation: u64,
        ports: Ports,
    ) -> Self {
        let clipboard =
            ClipboardSync::new(config.computer(&local).is_some_and(|c| c.clipboard.text));
        let mut controller = Self {
            config,
            local,
            config_dir,
            labels,
            fingerprint,
            ports,
            clipboard,
            clipboard_open: false,
            clipboard_open_failed: false,
            clipboard_session: None,
            portal: None,
            portal_window_request: None,
            next_portal: 1,
            connected: BTreeSet::new(),
            dialing: BTreeSet::new(),
            found: HashMap::new(),
            search_deadline: None,
            discovery_deadline: None,
            now: Instant::now(),
            leases: LeaseTable::new(first_generation),
            incoming: BTreeMap::new(),
            incoming_pointer_stats: BTreeMap::new(),
            last_pointer_summary: None,
            device_states: HashMap::new(),
            injector_session: None,
            jobs: HashMap::new(),
            next_job: 1,
            verifiers: BTreeMap::new(),
            next_verifier: 1,
            prompts: BTreeMap::new(),
            next_prompt: 1,
            pending_hand_backs: BTreeSet::new(),
            hand_back_verifications: BTreeMap::new(),
            control_requests: BTreeMap::new(),
            unsent_control: BTreeSet::new(),
            control_matches: BTreeMap::new(),
            next_control_match: 1,
            notices: Vec::new(),
            display_cache: Vec::new(),
            last_heartbeat: None,
            is_shutdown: false,
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
        if controller.ports.focus_holder.is_none() {
            controller.notice("Keyboard focus holder is not available on this peer.".into());
        }
        controller
    }

    pub fn local(&self) -> &ComputerId {
        &self.local
    }

    pub fn config_dir(&self) -> &std::path::Path {
        &self.config_dir
    }

    /// The app exits: end each session with its held state released, close each
    /// connection, and shut down the transport, so that no peer waits for the
    /// heartbeat timeout (AC-18). The portal session ends with `SourceClosed`.
    /// Each session where this computer is the target ends with
    /// `TargetDisconnected`. A second call does nothing.
    pub fn shutdown(&mut self) {
        if self.is_shutdown {
            return;
        }
        self.is_shutdown = true;
        self.close_portal(EndReason::SourceClosed);
        self.stop_discovery();
        let sessions: Vec<_> = self.incoming.keys().copied().collect();
        for session in sessions {
            self.end_incoming(session, EndReason::TargetDisconnected);
        }
        let devices: Vec<_> = self.hand_back_verifications.keys().cloned().collect();
        for device in devices {
            self.stop_hand_back_verification(&device);
        }
        self.pending_hand_backs.clear();
        self.update_clipboard();
        for peer in std::mem::take(&mut self.connected) {
            self.ports.net.send(NetCommand::Close { peer });
        }
        self.ports.net.shutdown();
    }

    /// Adds a notice for the user (IN-12).
    pub fn add_notice(&mut self, text: String) {
        self.notice(text);
    }

    fn notice(&mut self, text: String) {
        eprintln!("telescreen: notice: {text}");
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

    /// A forwarded scroll, multiplied by the speed of this computer (IN-14).
    fn scaled(&self, event: InputEvent) -> InputEvent {
        let speed = self.config.computers[&self.local].scroll_speed;
        match event {
            InputEvent::Scroll { dx, dy } => InputEvent::Scroll {
                dx: dx * speed,
                dy: dy * speed,
            },
            other => other,
        }
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

    fn allocate_prompt(&mut self) -> PromptId {
        let id = PromptId(self.next_prompt);
        self.next_prompt += 1;
        id
    }

    // ----- Portal and source side -----

    /// The user opened a portal on a display. Starts `portal-match` (FLOW-2).
    pub fn open_portal(&mut self, display: DisplayInfo) {
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
            focused: true,
            guarded: false,
            matching: Matching::Evaluating,
            match_pending: 0,
            matched: Vec::new(),
            match_error: None,
            device: None,
            pending_start: None,
            ephemeral: false,
            session: None,
            pointer: Point::default(),
            notice: None,
        });
        self.evaluate_portal_match();
    }

    /// Takes the portal window request from FLOW-17.
    /// Returns the display token once.
    pub fn take_portal_window_request(&mut self) -> Option<DisplayToken> {
        self.portal_window_request.take()
    }

    /// The portal moved to a different display, or the display topology changed
    /// (FLOW-4). The old device binding does not apply.
    pub fn portal_display_changed(&mut self, display: DisplayInfo) {
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
        portal.device = None;
        portal.pending_start = None;
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
        self.start_pending_handoff();
    }

    /// Starts the reverse handoff after portal matching finishes (FLOW-17).
    fn start_pending_handoff(&mut self) {
        let Some(portal) = self.portal.as_mut() else {
            return;
        };
        if matches!(portal.matching, Matching::Evaluating) {
            return;
        }
        let Some(pending) = portal.pending_start.take() else {
            return;
        };
        self.start_pending(pending);
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
        let peers: Vec<_> = routes::candidates(&self.config, &device)
            .iter()
            .map(|r| r.target.clone())
            .collect();
        for peer in peers {
            if !self.dialing.contains(&peer) {
                self.dial_peer(peer);
            }
        }
        self.start_search();
    }

    /// Dials the peer, or adds its targets to the race of a running dial (NET-11).
    fn dial_peer(&mut self, peer: ComputerId) {
        if self.connected.contains(&peer) {
            return;
        }
        let Some(computer) = self.config.computer(&peer) else {
            return;
        };
        let mut targets = Vec::new();
        for target in self.found.get(&peer).into_iter().flatten().cloned().chain(
            computer.endpoint.hosts.iter().map(|host| DialTarget {
                host: host.clone(),
                port: computer.endpoint.port,
            }),
        ) {
            if !targets.contains(&target) {
                targets.push(target);
            }
        }
        if !targets.is_empty() {
            self.dialing.insert(peer.clone());
            self.ports.net.send(NetCommand::Dial { peer, targets });
        }
    }

    fn discovery_enabled(&self, peer: &ComputerId) -> bool {
        self.config
            .computer(peer)
            .is_some_and(|c| c.endpoint.discovery.is_some())
    }

    fn searching(&self, peer: &ComputerId) -> bool {
        self.search_deadline.is_some() && self.discovery_enabled(peer)
    }

    /// True if a candidate of the portal has no connection and can be found.
    fn candidate_to_search(&self) -> bool {
        self.portal_device().is_some_and(|device| {
            routes::candidates(&self.config, &device)
                .iter()
                .any(|route| {
                    !self.connected.contains(route.target) && self.discovery_enabled(route.target)
                })
        })
    }

    fn start_search(&mut self) {
        if self.search_deadline.is_none() && self.candidate_to_search() {
            match self.ports.discovery.search() {
                Ok(()) => self.search_deadline = Some(self.now + SEARCH_TIME),
                Err(error) => self.notice(format!("Cannot search: {error}")),
            }
        }
    }

    fn stop_search(&mut self) {
        if self.search_deadline.take().is_some() {
            self.ports.discovery.stop_search();
        }
    }

    fn stop_discovery(&mut self) {
        if self.discovery_deadline.take().is_some() {
            self.ports.discovery.stop_announce();
        }
    }

    /// True while discovery has a timer that can change a screen.
    pub fn discovery_active(&self) -> bool {
        self.search_deadline.is_some() || self.discovery_deadline.is_some()
    }

    pub fn close_portal(&mut self, reason: EndReason) {
        self.stop_search();
        self.portal_window_request = None;
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
            self.ports.capture.set_portal_focus(focused);
        }
    }

    /// The acknowledgement key or a click in the portal (FLOW-8). Returns true
    /// when accepted: then the window layer must suppress the event (FLOW-9).
    pub fn acknowledge(&mut self) -> bool {
        let Some(portal) = self.portal.as_mut() else {
            return false;
        };
        let focused = portal.focused;
        let Some(actions) = portal.session.as_mut().and_then(|s| s.acknowledge(focused)) else {
            return false;
        };
        self.run_source_actions(actions);
        true
    }

    /// True while the destination verifier wants an acknowledgement.
    pub fn awaiting_ack(&self) -> bool {
        self.portal
            .as_ref()
            .and_then(|p| p.session.as_ref())
            .is_some_and(SourceSession::ack_wanted)
    }

    /// The portal left fullscreen: request a hand back as for the shortcut (UI-3).
    pub fn portal_left_fullscreen(&mut self) {
        let actions = self
            .portal
            .as_mut()
            .and_then(|p| p.session.as_mut())
            .map(SourceSession::request_hand_back)
            .unwrap_or_default();
        self.run_source_actions(actions);
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
        // FLOW-6: stop each old input stream of this device first, in both roles.
        self.end_device_sessions(&device.id);
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
        // A session needs capture and suppress (IN-1, FLOW-3).
        if let Capability::Missing(reason) = &self.ports.capabilities.suppress {
            return Some(format!("input suppress: {reason}"));
        }
        if self.connected.contains(target) {
            None
        } else if self.dialing.contains(target) {
            Some("connecting".into())
        } else if self.searching(target) {
            Some("searching".into())
        } else {
            Some("offline".into())
        }
    }

    /// Ends the unfinished session of the portal if it uses this device.
    fn end_portal_session_of(&mut self, device: &DeviceId) {
        let active = self
            .portal
            .as_ref()
            .and_then(|p| p.session.as_ref())
            .is_some_and(|s| !s.is_finished() && s.device() == device);
        if active {
            self.end_portal_session(EndReason::NewHandoff);
        }
    }

    /// Ends each unfinished session of this device: the session of the portal and
    /// the sessions where this computer is the target. It releases the held state.
    fn end_device_sessions(&mut self, device: &DeviceId) {
        self.stop_hand_back_verification(device);
        self.pending_hand_backs.retain(|(_, id)| id != device);
        self.end_portal_session_of(device);
        let incoming: Vec<_> = self
            .incoming
            .iter()
            .filter(|(_, t)| !t.is_finished() && t.device() == device)
            .map(|(s, _)| *s)
            .collect();
        for session in incoming {
            self.end_incoming(session, EndReason::NewHandoff);
        }
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
                    if matches!(message, Message::HandBackRequest { .. }) {
                        let device = session.device().clone();
                        self.stop_hand_back_verification(&device);
                        self.pending_hand_backs.insert((target.clone(), device));
                    }
                    self.ports.net.send(NetCommand::Send {
                        peer: target,
                        message,
                    });
                }
                SourceAction::RunHandoff => self.run_handoff(),
                SourceAction::StartForwarding => {
                    self.set_guard(false);
                    match self.ports.capture.forward() {
                        Ok(()) => self.send_pointer(),
                        Err(e) => {
                            self.notice(format!("Input capture failed: {e}"));
                            let actions = self.end_actions(EndReason::PermissionLost);
                            self.run_source_actions(actions);
                            return;
                        }
                    }
                }
                // A finished session disarms the capture below (FLOW-16).
                SourceAction::StopForwarding => {}
            }
        }
        let finished = self
            .portal
            .as_ref()
            .and_then(|p| p.session.as_ref())
            .is_some_and(SourceSession::is_finished);
        if finished {
            self.set_guard(false);
            self.ports.capture.disarm();
            self.close_ephemeral_portal();
        }
        self.update_clipboard();
    }

    /// Changes the portal guard during the handoff and verification (FLOW-21).
    fn set_guard(&mut self, on: bool) {
        let Some(portal) = self.portal.as_mut() else {
            return;
        };
        if portal.guarded == on {
            return;
        }
        portal.guarded = on;
        if let Err(e) = self.ports.capture.guard(on) {
            self.notice(format!("Portal guard failed: {e}"));
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
        let bounds = portal.bounds;
        let focused = portal.focused;
        if let Err(e) = self.ports.capture.arm(bounds) {
            self.notice(format!("Input capture failed: {e}"));
            self.end_portal_session(EndReason::PermissionLost);
            return;
        }
        self.ports.capture.set_portal_focus(focused);
        self.set_guard(true);
        self.spawn(
            Pending::Handoff {
                session: session_id,
            },
            Job::Handoff {
                steps: handoff.steps,
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
        if let Some(datagram) = session.pointer(position, Instant::now()) {
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
            CapturedEvent::PointerMoved { at } => {
                portal.pointer = portal.bounds.clamp(at);
                if session.phase() == SourcePhase::Forwarding {
                    self.send_pointer();
                }
                return;
            }
            CapturedEvent::PointerEntered { at } => {
                portal.pointer = portal.bounds.clamp(at);
                session.forget_pointer();
                self.send_pointer();
                return;
            }
            CapturedEvent::PointerLeft => return,
            CapturedEvent::KeyboardEntered => session.on_keyboard(true),
            CapturedEvent::KeyboardLeft => session.on_keyboard(false),
            CapturedEvent::InputLost => session.on_input_lost(),
            CapturedEvent::HandBackShortcut => session.request_hand_back(),
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
        self.now = now;
        if self.search_deadline.is_some_and(|deadline| now >= deadline) {
            self.stop_search();
        }
        if self
            .discovery_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.stop_discovery();
        }
        self.expire_control_requests(now);
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
        let events: Vec<_> = self
            .verifiers
            .iter_mut()
            .filter_map(|(session, verifier)| match verifier {
                Verifier::Prompt(prompt) => prompt.tick(now).map(|event| (*session, event)),
                Verifier::Process(_) => None,
            })
            .collect();
        for (session, event) in events {
            let actions = self.verifier_actions(session, event);
            self.run_target_actions_at(session, actions, now);
        }
        let events: Vec<_> = self
            .hand_back_verifications
            .iter_mut()
            .filter_map(
                |(device, verification)| match verification.verifier.as_mut() {
                    Some(Verifier::Prompt(prompt)) => {
                        prompt.tick(now).map(|event| (device.clone(), event))
                    }
                    _ => None,
                },
            )
            .collect();
        for (device, event) in events {
            self.hand_back_verifier_event(&device, event);
        }
        self.update_clipboard();
        self.sync_clipboard();
    }

    // ----- Network -----

    /// Opens the clipboard only while sync is possible. A failed open is retried
    /// only after sync becomes unavailable and available again (CLIP-3, CLIP-5).
    fn update_clipboard(&mut self) {
        let sessions = self.clipboard_sessions();
        let can_sync =
            self.clipboard.enabled() && self.ports.clipboard.is_some() && !sessions.is_empty();
        if !can_sync {
            if self.clipboard_open {
                if let Some(clipboard) = self.ports.clipboard.as_mut() {
                    clipboard.clear_if_owned();
                    clipboard.close();
                }
                self.clipboard_open = false;
                self.clipboard.reset();
            }
            self.clipboard_open_failed = false;
            self.clipboard_session = None;
            return;
        }
        if !self.clipboard_open
            && !self.clipboard_open_failed
            && let Some(clipboard) = self.ports.clipboard.as_mut()
        {
            match clipboard.open() {
                Ok(()) => self.clipboard_open = true,
                Err(error) => {
                    self.clipboard_open_failed = true;
                    self.notice(format!("Cannot open the clipboard: {error}"));
                }
            }
        }
        if self.clipboard_open
            && self
                .clipboard_session
                .is_some_and(|session| !sessions.iter().any(|(id, _)| *id == session))
        {
            if let Some(clipboard) = self.ports.clipboard.as_mut() {
                clipboard.clear_if_owned();
            }
            self.clipboard_session = None;
        }
    }

    /// Lists the sessions that can sync text (CLIP-2).
    fn clipboard_sessions(&self) -> Vec<(SessionId, ComputerId)> {
        let mut sessions = Vec::new();
        if let Some(source) = self.portal.as_ref().and_then(|p| p.session.as_ref())
            && source.phase() == SourcePhase::Forwarding
            && let Some(session) = source.session()
        {
            sessions.push((session, source.target().clone()));
        }
        sessions.extend(
            self.incoming
                .iter()
                .filter(|(_, target)| target.phase() == TargetPhase::Forwarding)
                .map(|(session, target)| (*session, target.source().clone())),
        );
        sessions
    }

    /// Polls once and sends a local change to forwarding sessions (CLIP-2, CLIP-5).
    /// The app reads the clipboard only with the switch on and a session that can
    /// sync (CLIP-1, CLIP-5).
    fn sync_clipboard(&mut self) {
        if !self.clipboard_open {
            return;
        }
        let sessions = self.clipboard_sessions();
        let Some(item) = self.ports.clipboard.as_mut().and_then(|c| c.changed()) else {
            return;
        };
        let outgoing = self.clipboard.local_change(&item);
        // An empty result for nonempty text is the core suppressing our write.
        // Clears and empty text also replace the last applied session's item.
        if !outgoing.is_empty()
            || !matches!(&item, ClipboardChange::Text(item) if !item.text.is_empty())
        {
            self.clipboard_session = None;
        }
        for outgoing in outgoing {
            match outgoing {
                Outgoing::Send { text, concealed } => {
                    for (session, peer) in &sessions {
                        self.ports.net.send(NetCommand::Send {
                            peer: peer.clone(),
                            message: Message::ClipboardText {
                                session: *session,
                                text: text.clone(),
                                concealed,
                            },
                        });
                    }
                }
                Outgoing::Clear => {
                    for (session, peer) in &sessions {
                        self.ports.net.send(NetCommand::Send {
                            peer: peer.clone(),
                            message: Message::ClipboardClear { session: *session },
                        });
                    }
                }
                Outgoing::TooLarge { bytes } => {
                    self.notice(format!(
                        "Cannot sync clipboard text of {bytes} bytes: it is too large."
                    ));
                }
            }
        }
    }

    /// Applies text only from the peer of a forwarding session (CLIP-2, CLIP-5).
    fn receive_clipboard(
        &mut self,
        peer: &ComputerId,
        session: SessionId,
        text: &str,
        concealed: bool,
    ) {
        self.update_clipboard();
        if !self.clipboard_open
            || !self
                .clipboard_sessions()
                .iter()
                .any(|(id, computer)| *id == session && computer == peer)
        {
            return;
        }
        if let Some(clipboard) = self.ports.clipboard.as_mut()
            && self.clipboard.received(text)
        {
            match clipboard.set_text(text, concealed) {
                Ok(()) => self.clipboard_session = Some(session),
                // The error names no clipboard text (CLIP-9).
                Err(error) => self.notice(format!("Cannot set clipboard text: {error}")),
            }
        }
    }

    /// Clears only for the authenticated peer of a forwarding session (CLIP-7).
    fn receive_clipboard_clear(&mut self, peer: &ComputerId, session: SessionId) {
        self.update_clipboard();
        if self.clipboard_open
            && self
                .clipboard_sessions()
                .iter()
                .any(|(id, computer)| *id == session && computer == peer)
            && self.clipboard.received_clear()
            && let Some(clipboard) = self.ports.clipboard.as_mut()
        {
            clipboard.clear_if_owned();
            self.clipboard_session = None;
        }
    }

    /// Handles waiting datagrams in order, injecting only the newest accepted
    /// position of each incoming session (NET-2).
    pub fn on_pointers(&mut self, pointers: Vec<ReceivedPointer>, now: Instant) {
        let mut newest_actions = BTreeMap::new();
        for ReceivedPointer {
            peer,
            datagram,
            received,
        } in pointers
        {
            let session = datagram.session;
            let Some(target) = self.incoming.get_mut(&session) else {
                continue;
            };
            if target.source() != &peer {
                continue;
            }
            if let Some(stats) = self.incoming_pointer_stats.get_mut(&session) {
                stats.record(&datagram, received, now);
            }
            if let Some(action) = target.on_pointer(&datagram) {
                newest_actions.insert(session, action);
            }
        }
        for (session, action) in newest_actions {
            if let Some(stats) = self.incoming_pointer_stats.get_mut(&session) {
                stats.injected();
            }
            self.run_target_actions(session, vec![action]);
        }
    }

    /// A search found the addresses of a trusted computer (NET-16).
    pub fn on_discovered(&mut self, peer: ComputerId, targets: Vec<DialTarget>) {
        self.found.insert(peer.clone(), targets);
        if self.is_offline_candidate(&peer) {
            self.dial_peer(peer);
        }
    }

    fn is_offline_candidate(&self, peer: &ComputerId) -> bool {
        !self.connected.contains(peer)
            && self.portal_device().is_some_and(|device| {
                routes::candidates(&self.config, &device)
                    .iter()
                    .any(|route| route.target == peer)
            })
    }

    /// The discovery backend stopped because of an error (NET-12).
    pub fn on_discovery_failed(&mut self, reason: String) {
        self.notice(format!("Discovery failed: {reason}"));
        // The responder failed, so neither a search nor a record is certain now.
        self.stop_search();
        self.stop_discovery();
    }

    pub fn on_net(&mut self, event: NetEvent, now: Instant) {
        match event {
            NetEvent::Connected { peer } => {
                self.dialing.remove(&peer);
                self.connected.insert(peer.clone());
                if !self.candidate_to_search() {
                    self.stop_search();
                }
                self.send_control_requests(&peer);
                self.start_pending_handoff();
            }
            NetEvent::Disconnected { peer, reason } => {
                self.connected.remove(&peer);
                self.peer_lost(&peer);
                self.notice(format!("{} disconnected: {reason}", self.label_of(&peer)));
            }
            NetEvent::DialFailed { peer, reason } => {
                self.dialing.remove(&peer);
                self.dial_failed(&peer);
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
                self.on_pointers(
                    vec![ReceivedPointer {
                        peer,
                        datagram,
                        received: now,
                    }],
                    now,
                );
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
            Message::ClipboardText {
                session,
                text,
                concealed,
            } => {
                self.receive_clipboard(&peer, *session, text, *concealed);
            }
            Message::ClipboardClear { session } => {
                self.receive_clipboard_clear(&peer, *session);
            }
            Message::HandBackResult { device, outcome } => {
                self.hand_back_result(peer, device.clone(), *outcome, now);
            }
            // To the source side.
            Message::SessionGranted { .. }
            | Message::SessionRefused { .. }
            | Message::AckWanted { .. }
            | Message::Started { .. } => self.source_message(&peer, &message, now),
            Message::End { session, .. } if !self.incoming.contains_key(session) => {
                self.source_message(&peer, &message, now)
            }
            // To the target side.
            Message::RequestSession { device } => {
                // ACT-6: the attach of a control request ends the wait.
                self.control_requests
                    .remove(&(peer.clone(), device.clone()));
                self.request_session(peer, device.clone());
            }
            Message::HandoffDone { session }
            | Message::HandBackRequest { session }
            | Message::Ack { session }
            | Message::Input { session, .. }
            | Message::ReleaseAll { session }
            | Message::KeyboardAway { session }
            | Message::KeyboardBack { session }
            | Message::End { session, .. } => {
                let session = *session;
                let Some(target) = self.incoming.get_mut(&session) else {
                    return;
                };
                if target.source() != &peer {
                    return;
                }
                if matches!(
                    message,
                    Message::End {
                        reason: EndReason::HandoffPartial,
                        ..
                    }
                ) && !target.is_finished()
                {
                    self.device_states
                        .insert(target.device().clone(), VideoInput::Unknown);
                }
                let actions = target.on_message(&message);
                self.run_target_actions_at(session, actions, now);
            }
            Message::Hello { .. } | Message::Heartbeat => {}
            Message::DisplaySwitched { device } => {
                self.display_switched(peer, device.clone(), now);
            }
            Message::ControlRequest { device } => self.control_request(peer, device.clone()),
            Message::ControlRefused { device, reason } => {
                self.control_refused(&peer, device, *reason);
            }
        }
    }

    fn source_message(&mut self, peer: &ComputerId, message: &Message, now: Instant) {
        let Some(session) = self.portal.as_mut().and_then(|p| p.session.as_mut()) else {
            return;
        };
        if session.target() != peer {
            return;
        }
        let hand_back_device =
            matches!(message, Message::End { session: id, reason: EndReason::HandBack }
            if session.session() == Some(*id) && !session.is_finished())
            .then(|| session.device().clone());
        let actions = session.on_message(message, now);
        if let Some(device) = hand_back_device {
            self.stop_hand_back_verification(&device);
            self.pending_hand_backs.insert((peer.clone(), device));
        }
        if let Message::SessionRefused { reason, .. } = message
            && let Some(portal) = self.portal.as_mut()
        {
            portal.notice = Some(refuse_text(*reason));
            // ACT-5: the portal closes with its session, so keep the reason.
            if portal.ephemeral {
                let text = format!(
                    "{} refused the session: {}",
                    self.label_of(peer),
                    refuse_text(*reason)
                );
                self.notice(text);
            }
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

    /// Checks the target route, input-target, injection, focus holder, and lease (FLOW-5, UI-7).
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
        if self.ports.focus_holder.is_none() {
            return self.refuse(peer, device_id, RefuseReason::NoFocusHold);
        }
        // FLOW-6: a new handoff of this device ends the session of the local
        // portal. A session of a different source keeps its lease (AC-20).
        let taken = self
            .portal
            .as_ref()
            .and_then(|p| p.session.as_ref())
            .is_some_and(|s| !s.is_finished() && s.device() == &device_id);
        self.end_portal_session_of(&device_id);
        if taken {
            // FLOW-24: another computer shows on the display now.
            self.notice(format!(
                "Session ended: {} took this display",
                self.label_of(&peer)
            ));
        }
        let session = match self.leases.acquire(&device_id, &peer) {
            Ok(session) => session,
            Err(_) => return self.refuse(peer, device_id, RefuseReason::Busy),
        };
        self.stop_hand_back_verification(&device_id);
        self.pending_hand_backs.retain(|(_, id)| id != &device_id);
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
        let on_portal = self
            .portal
            .as_ref()
            .is_some_and(|p| p.display.token == token);
        let (target, actions) = TargetSession::new(session, peer, device, token);
        self.incoming.insert(session, target);
        self.incoming_pointer_stats
            .insert(session, PointerStats::new(session));
        self.run_target_actions(session, actions);
        // FLOW-24: the display shows this computer as the target, not a second portal.
        if on_portal {
            self.close_portal(EndReason::SourceClosed);
        }
    }

    fn find_display(&mut self, session: SessionId) -> Option<DisplayInfo> {
        let token = self.incoming.get(&session)?.display().clone();
        self.displays().into_iter().find(|d| d.token == token)
    }

    fn run_target_actions(&mut self, session: SessionId, actions: Vec<TargetAction>) {
        self.run_target_actions_at(session, actions, Instant::now());
    }

    /// Delivers events to a running session or hand back process verifier.
    pub fn on_verifier_event(&mut self, id: VerifierId, event: VerifierEvent) {
        let session = self.verifiers.iter().find_map(|(session, verifier)| {
            matches!(verifier, Verifier::Process(running) if *running == id).then_some(*session)
        });
        if let Some(session) = session {
            let actions = self.verifier_actions(session, event);
            self.run_target_actions(session, actions);
            return;
        }
        let device = self
            .hand_back_verifications
            .iter()
            .find_map(|(device, verification)| {
                matches!(&verification.verifier, Some(Verifier::Process(running)) if *running == id)
                    .then(|| device.clone())
            });
        if let Some(device) = device {
            self.hand_back_verifier_event(&device, event);
        }
    }

    fn verifier_actions(&mut self, session: SessionId, event: VerifierEvent) -> Vec<TargetAction> {
        if let Some(target) = self.incoming.get(&session)
            && target.phase() == TargetPhase::Verifying
        {
            let state = match &event {
                VerifierEvent::Done(true) => Some(VideoInput::Confirmed),
                VerifierEvent::Done(false) | VerifierEvent::Failed(_) => Some(VideoInput::Unknown),
                _ => None,
            };
            if let Some(state) = state {
                self.device_states.insert(target.device().clone(), state);
            }
        }
        if matches!(event, VerifierEvent::Done(_) | VerifierEvent::Failed(_)) {
            self.verifiers.remove(&session);
        }
        self.target_mut(session)
            .map(|t| t.on_verifier(event))
            .unwrap_or_default()
    }

    fn run_target_actions_at(
        &mut self,
        session: SessionId,
        actions: Vec<TargetAction>,
        now: Instant,
    ) {
        let Some(target) = self.incoming.get(&session) else {
            return;
        };
        // RunHandBack follows ReleaseLease, which removes the session.
        let source = target.source().clone();
        let device_id = target.device().clone();
        let mut queue: std::collections::VecDeque<_> =
            actions.into_iter().map(TargetWork::Action).collect();
        while let Some(work) = queue.pop_front() {
            let action = match work {
                TargetWork::Action(action) => action,
                TargetWork::Event(event) => {
                    let actions = self.verifier_actions(session, event);
                    for action in actions.into_iter().rev() {
                        queue.push_front(TargetWork::Action(action));
                    }
                    continue;
                }
            };
            match action {
                TargetAction::Send(message) => {
                    self.ports.net.send(NetCommand::Send {
                        peer: source.clone(),
                        message,
                    });
                }
                TargetAction::StartVerifier => {
                    self.device_states
                        .insert(device_id.clone(), VideoInput::Unverified);
                    let device = self
                        .config
                        .computer(&self.local)
                        .and_then(|c| c.device_by_id(&device_id))
                        .map(|(alias, device)| (alias.to_owned(), device.clone()));
                    let Some((alias, device)) = device else {
                        queue.push_front(TargetWork::Event(VerifierEvent::Failed(
                            "the local device is gone".into(),
                        )));
                        continue;
                    };
                    let Some(operation) = device.verify else {
                        let more = self.target_mut(session).map(TargetSession::on_no_verify);
                        queue.extend(more.unwrap_or_default().into_iter().map(TargetWork::Action));
                        continue;
                    };
                    match &operation.driver {
                        Driver::Prompt => {
                            let hint = self
                                .config
                                .computer(&self.local)
                                .and_then(|c| c.hint.clone());
                            let (prompt, events) = PromptVerifier::start(hint, now);
                            self.verifiers.insert(session, Verifier::Prompt(prompt));
                            for event in events.into_iter().rev() {
                                queue.push_front(TargetWork::Event(event));
                            }
                        }
                        Driver::Exec(_) | Driver::Command(_) => {
                            let id = VerifierId(self.next_verifier);
                            self.next_verifier += 1;
                            self.verifiers.insert(session, Verifier::Process(id));
                            let request = Request {
                                computer_id: self.local.clone(),
                                device: DeviceRef {
                                    id: device.id,
                                    alias,
                                },
                                source_computer_id: source.clone(),
                                target_computer_id: Some(self.local.clone()),
                                session_id: Some(session),
                                portal: None,
                                displays: self.displays(),
                            };
                            self.ports.verifiers.start(id, operation, request);
                        }
                        Driver::Extension { name, .. } => {
                            queue.push_front(TargetWork::Event(VerifierEvent::Failed(format!(
                                "the driver {name} is not available"
                            ))));
                        }
                    }
                }
                TargetAction::AckVerifier => match self.verifiers.get_mut(&session) {
                    Some(Verifier::Prompt(prompt)) => {
                        if let Some(event) = prompt.ack() {
                            queue.push_front(TargetWork::Event(event));
                        }
                    }
                    Some(Verifier::Process(id)) => self.ports.verifiers.ack(*id),
                    None => {}
                },
                TargetAction::StopVerifier => {
                    if let Some(Verifier::Process(id)) = self.verifiers.remove(&session) {
                        self.ports.verifiers.stop(id);
                    }
                }
                TargetAction::ShowVerifier { text, ack_wanted } => {
                    let screen = PromptScreen {
                        source: Some(self.label_of(&source)),
                        confirm_key: self.labels.confirm_key.clone(),
                        text,
                        ack_wanted,
                    };
                    let prompt = match self.prompts.get(&session) {
                        Some(prompt) => *prompt,
                        None => {
                            let prompt = self.allocate_prompt();
                            self.prompts.insert(session, prompt);
                            prompt
                        }
                    };
                    let shown = match self.find_display(session) {
                        Some(display) => self.ports.prompt.show(prompt, &display, &screen),
                        None => Err("the input-target display is gone".into()),
                    };
                    if let Err(e) = &shown {
                        self.notice(format!("Cannot show the prompt: {e}"));
                    }
                    let more = self
                        .target_mut(session)
                        .map(|t| t.on_verifier_shown(shown.is_ok()));
                    for action in more.unwrap_or_default().into_iter().rev() {
                        queue.push_front(TargetWork::Action(action));
                    }
                }
                TargetAction::HideVerifier => {
                    if let Some(prompt) = self.prompts.get(&session) {
                        self.ports.prompt.hide(*prompt);
                    }
                }
                TargetAction::HoldKeyboard => {
                    let display = self.find_display(session);
                    let result = match (display, self.ports.focus_holder.as_mut()) {
                        (Some(display), Some(holder)) => holder.hold(&display),
                        (None, _) => Err(PlatformError::Failed(
                            "the input-target display is gone".into(),
                        )),
                        (_, None) => Err(PlatformError::Unsupported(
                            "no focus holder is available".into(),
                        )),
                    };
                    if let Err(e) = result {
                        self.notice(format!("Cannot hold the keyboard focus: {e}"));
                        queue.clear();
                        let more = self
                            .target_mut(session)
                            .map(|t| t.end(EndReason::PermissionLost));
                        queue.extend(more.unwrap_or_default().into_iter().map(TargetWork::Action));
                    }
                }
                TargetAction::ReleaseKeyboard => {
                    if let Some(holder) = self.ports.focus_holder.as_mut() {
                        holder.release();
                    }
                }
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
                    queue.extend(more.unwrap_or_default().into_iter().map(TargetWork::Action));
                }
                TargetAction::Inject(event) => {
                    let event = self.scaled(event);
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
                    self.remove_incoming(session);
                    self.prompts.remove(&session);
                    if self.injector_session == Some(session) {
                        self.injector_session = None;
                    }
                }
                TargetAction::RunHandBack => self.run_hand_back(source.clone(), device_id.clone()),
            }
        }
        if self
            .incoming
            .get(&session)
            .is_some_and(TargetSession::is_finished)
        {
            self.remove_incoming(session);
            self.prompts.remove(&session);
            if self.injector_session == Some(session) {
                self.injector_session = None;
            }
        }
        self.update_clipboard();
    }

    fn target_mut(&mut self, session: SessionId) -> Option<&mut TargetSession> {
        self.incoming.get_mut(&session)
    }

    fn remove_incoming(&mut self, session: SessionId) {
        self.incoming.remove(&session);
        if let Some(stats) = self.incoming_pointer_stats.remove(&session) {
            let summary = stats.summary();
            eprintln!("telescreen: {summary}");
            self.last_pointer_summary = Some(summary);
        }
    }

    // ----- Hand back, without a session -----

    fn hand_back_cause(&self, target: &TargetSession) -> Option<String> {
        let has_route = self
            .config
            .computer(&self.local)
            .and_then(|c| c.device_by_id(target.device()))
            .is_some_and(|(_, device)| device.handoff.contains_key(target.source()));
        if !has_route {
            Some(format!(
                "No route back to {} in this config",
                self.label_of(target.source())
            ))
        } else if !target.can_hand_back() {
            Some("The source still switches the display".into())
        } else {
            None
        }
    }

    fn send_hand_back_result(
        &mut self,
        peer: ComputerId,
        device: DeviceId,
        outcome: HandBackOutcome,
    ) {
        self.ports.net.send(NetCommand::Send {
            peer,
            message: Message::HandBackResult { device, outcome },
        });
    }

    fn run_hand_back(&mut self, source: ComputerId, device_id: DeviceId) {
        self.stop_hand_back_verification(&device_id);
        let route = self
            .config
            .computer(&self.local)
            .and_then(|c| c.device_by_id(&device_id))
            .and_then(|(alias, device)| {
                device
                    .handoff
                    .get(&source)
                    .map(|route| (alias.to_owned(), route.clone()))
            });
        let Some((alias, route)) = route else {
            self.notice(format!(
                "Switch the display by hand: no route back to {} in this config.",
                self.label_of(&source)
            ));
            self.send_hand_back_result(source, device_id, HandBackOutcome::NoRoute);
            return;
        };
        let request = Request {
            computer_id: self.local.clone(),
            device: DeviceRef {
                id: device_id.clone(),
                alias,
            },
            source_computer_id: self.local.clone(),
            target_computer_id: Some(source.clone()),
            session_id: None,
            portal: None,
            displays: self.displays(),
        };
        self.spawn(
            Pending::HandBack {
                peer: source,
                device: device_id,
            },
            Job::Handoff {
                steps: route.steps,
                request,
            },
        );
    }

    fn hand_back_result(
        &mut self,
        peer: ComputerId,
        device: DeviceId,
        outcome: HandBackOutcome,
        now: Instant,
    ) {
        if !self
            .pending_hand_backs
            .remove(&(peer.clone(), device.clone()))
        {
            return;
        }
        self.stop_hand_back_verification(&device);
        match outcome {
            HandBackOutcome::NoRoute => self.notice(format!(
                "Switch the display by hand: {} has no route back to this computer.",
                self.label_of(&peer)
            )),
            HandBackOutcome::Failed { partial } => {
                self.notice(format!(
                    "The hand back from {} failed.",
                    self.label_of(&peer)
                ));
                if partial {
                    self.device_states.insert(device, VideoInput::Unknown);
                }
            }
            HandBackOutcome::Done => self.verify_as_destination(peer, device, now),
        }
    }

    /// Verifies as the destination with no session: after a hand back (HB-1) and
    /// after "Switch display only" (ACT-2).
    fn verify_as_destination(&mut self, peer: ComputerId, device: DeviceId, now: Instant) {
        self.device_states
            .insert(device.clone(), VideoInput::Unverified);
        let prompt = self.allocate_prompt();
        self.hand_back_verifications.insert(
            device.clone(),
            HandBackVerification {
                prompt,
                display: None,
                verification: Verification::new(),
                verifier: None,
            },
        );
        self.start_hand_back_input_target(peer, device, prompt, now);
    }

    fn hand_back_verification_failed(
        &mut self,
        device: &DeviceId,
        error: HandBackVerificationError,
    ) {
        self.notice(format!("Cannot verify the hand back: {error}."));
        self.stop_hand_back_verification(device);
        self.device_states
            .insert(device.clone(), VideoInput::Unknown);
    }

    fn start_hand_back_input_target(
        &mut self,
        peer: ComputerId,
        device_id: DeviceId,
        prompt: PromptId,
        _now: Instant,
    ) {
        let device = self
            .config
            .computer(&self.local)
            .and_then(|c| c.device_by_id(&device_id))
            .map(|(alias, device)| (alias.to_owned(), device.clone()));
        let Some((alias, device)) = device else {
            self.hand_back_verification_failed(&device_id, HandBackVerificationError::DeviceGone);
            return;
        };
        let Some(operation) = device.input_target else {
            self.hand_back_verification_failed(
                &device_id,
                HandBackVerificationError::NoInputTarget(alias),
            );
            return;
        };
        let request = self.hand_back_verifier_request(&peer, &device_id, alias);
        self.spawn(
            Pending::HandBackInputTarget {
                prompt,
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

    fn hand_back_verifier_request(
        &mut self,
        peer: &ComputerId,
        device: &DeviceId,
        alias: String,
    ) -> Request {
        Request {
            computer_id: self.local.clone(),
            device: DeviceRef {
                id: device.clone(),
                alias,
            },
            source_computer_id: peer.clone(),
            target_computer_id: Some(self.local.clone()),
            session_id: None,
            portal: None,
            displays: self.displays(),
        }
    }

    fn hand_back_input_target_done(
        &mut self,
        peer: ComputerId,
        device_id: DeviceId,
        output: JobOutput,
    ) {
        let result = match output {
            JobOutput::Operation(Ok(Outcome::Display(token))) => self
                .displays()
                .into_iter()
                .find(|display| display.token == token)
                .ok_or(HandBackVerificationError::DisplayGone),
            JobOutput::Operation(Err(error)) => {
                Err(HandBackVerificationError::InputTargetFailed(error))
            }
            _ => Err(HandBackVerificationError::InvalidInputTarget),
        };
        let display = match result {
            Ok(display) => display,
            Err(error) => {
                self.hand_back_verification_failed(&device_id, error);
                return;
            }
        };
        let Some(verification) = self.hand_back_verifications.get_mut(&device_id) else {
            return;
        };
        verification.display = Some(display);
        let device = self
            .config
            .computer(&self.local)
            .and_then(|c| c.device_by_id(&device_id))
            .map(|(alias, device)| (alias.to_owned(), device.clone()));
        let Some((alias, device)) = device else {
            self.hand_back_verification_failed(&device_id, HandBackVerificationError::DeviceGone);
            return;
        };
        let Some(operation) = device.verify else {
            self.stop_hand_back_verification(&device_id);
            return;
        };
        match &operation.driver {
            Driver::Prompt => {
                let hint = self
                    .config
                    .computer(&self.local)
                    .and_then(|c| c.hint.clone());
                let (prompt, events) = PromptVerifier::start(hint, Instant::now());
                verification.verifier = Some(Verifier::Prompt(prompt));
                for event in events {
                    self.hand_back_verifier_event(&device_id, event);
                }
            }
            Driver::Exec(_) | Driver::Command(_) => {
                let id = VerifierId(self.next_verifier);
                self.next_verifier += 1;
                verification.verifier = Some(Verifier::Process(id));
                let request = self.hand_back_verifier_request(&peer, &device_id, alias);
                self.ports.verifiers.start(id, operation, request);
            }
            Driver::Extension { name, .. } => self.hand_back_verifier_event(
                &device_id,
                VerifierEvent::Failed(format!("the driver {name} is not available")),
            ),
        }
    }

    fn hand_back_verifier_event(&mut self, device: &DeviceId, event: VerifierEvent) {
        let Some(verification) = self.hand_back_verifications.get_mut(device) else {
            return;
        };
        if matches!(event, VerifierEvent::Done(_) | VerifierEvent::Failed(_)) {
            verification.verifier = None;
        }
        let actions = verification.verification.on_verifier(event);
        self.run_hand_back_verification_actions(device, actions);
    }

    fn run_hand_back_verification_actions(
        &mut self,
        device: &DeviceId,
        actions: Vec<VerificationAction>,
    ) {
        let mut queue: std::collections::VecDeque<_> = actions.into();
        while let Some(action) = queue.pop_front() {
            let Some(verification) = self.hand_back_verifications.get_mut(device) else {
                return;
            };
            match action {
                VerificationAction::Show { text, ack_wanted } => {
                    let prompt = verification.prompt;
                    let token = verification.display.as_ref().map(|d| d.token.clone());
                    let display = self
                        .displays()
                        .into_iter()
                        .find(|d| Some(&d.token) == token.as_ref());
                    let screen = PromptScreen {
                        source: None,
                        confirm_key: self.labels.confirm_key.clone(),
                        text,
                        ack_wanted,
                    };
                    let shown = match display {
                        Some(display) => self.ports.prompt.show(prompt, &display, &screen),
                        None => Err(HandBackVerificationError::DisplayGone.to_string()),
                    };
                    if let Err(error) = &shown {
                        self.notice(format!("Cannot show the hand back prompt: {error}"));
                    }
                    if let Some(verification) = self.hand_back_verifications.get_mut(device) {
                        let more = verification.verification.on_shown(shown.is_ok());
                        for action in more.into_iter().rev() {
                            queue.push_front(action);
                        }
                    }
                }
                VerificationAction::Hide => self.ports.prompt.hide(verification.prompt),
                VerificationAction::Ack => {
                    let event = match verification.verifier.as_mut() {
                        Some(Verifier::Prompt(prompt)) => prompt.ack(),
                        Some(Verifier::Process(id)) => {
                            self.ports.verifiers.ack(*id);
                            None
                        }
                        None => None,
                    };
                    if let Some(event) = event {
                        verification.verifier = None;
                        let more = verification.verification.on_verifier(event);
                        for action in more.into_iter().rev() {
                            queue.push_front(action);
                        }
                    }
                }
                VerificationAction::Stop => {
                    if let Some(Verifier::Process(id)) = verification.verifier.take() {
                        self.ports.verifiers.stop(id);
                    }
                }
                VerificationAction::AckWanted => {}
                VerificationAction::Finished(result) => {
                    if !verification.verification.hide().is_empty() {
                        self.ports.prompt.hide(verification.prompt);
                    }
                    self.device_states.insert(
                        device.clone(),
                        match result {
                            VerificationResult::Ok => VideoInput::Confirmed,
                            VerificationResult::NotOk | VerificationResult::WindowFailed => {
                                VideoInput::Unknown
                            }
                        },
                    );
                    self.hand_back_verifications.remove(device);
                }
            }
        }
    }

    fn stop_hand_back_verification(&mut self, device: &DeviceId) {
        let Some(mut verification) = self.hand_back_verifications.remove(device) else {
            return;
        };
        for action in verification.verification.stop() {
            match action {
                VerificationAction::Stop => {
                    if let Some(Verifier::Process(id)) = verification.verifier.take() {
                        self.ports.verifiers.stop(id);
                    }
                }
                VerificationAction::Hide => self.ports.prompt.hide(verification.prompt),
                _ => {}
            }
        }
    }

    /// True when a hand back verifier wants an acknowledgement on this destination.
    pub fn awaiting_local_ack(&self) -> bool {
        self.hand_back_verifications
            .values()
            .any(|v| v.verification.ack_wanted())
    }

    /// Acknowledges a hand back verifier. Suppress an accepted event (FLOW-9).
    pub fn acknowledge_local(&mut self) -> bool {
        let device = self
            .hand_back_verifications
            .iter()
            .find_map(|(device, v)| v.verification.ack_wanted().then(|| device.clone()));
        let Some(device) = device else {
            return false;
        };
        let Some(verification) = self.hand_back_verifications.get_mut(&device) else {
            return false;
        };
        let actions = verification.verification.acknowledge();
        let accepted = !actions.is_empty();
        self.run_hand_back_verification_actions(&device, actions);
        accepted
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
            Pending::HandBack { peer, device } => {
                let (outcome, error) = match output {
                    JobOutput::Handoff {
                        result: HandoffResult::Done,
                        ..
                    } => (HandBackOutcome::Done, None),
                    JobOutput::Handoff {
                        result: HandoffResult::Failed { completed_steps },
                        error,
                    } => (
                        HandBackOutcome::Failed {
                            partial: completed_steps > 0,
                        },
                        error,
                    ),
                    other => (
                        HandBackOutcome::Failed { partial: false },
                        Some(format!("unexpected handoff result: {other:?}")),
                    ),
                };
                // VER-6: the state of this computer after its route back.
                match outcome {
                    HandBackOutcome::Done => {
                        self.device_states.remove(&device);
                    }
                    HandBackOutcome::Failed { partial: true } => {
                        self.device_states
                            .insert(device.clone(), VideoInput::Unknown);
                    }
                    _ => {}
                }
                if let HandBackOutcome::Failed { .. } = outcome {
                    self.notice(format!(
                        "The hand back to {} failed: {}",
                        self.label_of(&peer),
                        error.as_deref().unwrap_or("the handoff steps failed")
                    ));
                }
                self.send_hand_back_result(peer, device, outcome);
            }
            Pending::SwitchOnly { target, device } => {
                self.switch_only_done(target, device, output);
            }
            Pending::ControlMatch { request, display } => {
                self.control_match_done(request, display, output);
            }
            Pending::HandBackInputTarget {
                prompt,
                peer,
                device,
            } => {
                if self
                    .hand_back_verifications
                    .get(&device)
                    .is_some_and(|v| v.prompt == prompt)
                {
                    self.hand_back_input_target_done(peer, device, output);
                }
            }
        }
    }

    // ----- Intents -----

    pub fn on_intent(&mut self, intent: Intent) {
        match intent {
            Intent::DeviceIsBack { device } => {
                if let Some(state) = self.device_states.get_mut(&device)
                    && *state == VideoInput::Unknown
                {
                    *state = VideoInput::Confirmed;
                }
            }
            Intent::SetClipboard { on } => {
                self.clipboard.set_enabled(on);
                self.update_clipboard();
            }
            Intent::OpenPortal { .. } | Intent::ClosePortal => {
                // The window layer opens and closes the portal window, then calls
                // `open_portal` or `close_portal`.
            }
            Intent::Refresh => self.dial_candidates(),
            Intent::AllowDiscovery => {
                if self.discovery_enabled(&self.local) {
                    if self.discovery_deadline.is_some() {
                        self.discovery_deadline = Some(self.now + DISCOVERY_WINDOW);
                    } else if let Some(computer) = self.config.computer(&self.local) {
                        match self
                            .ports
                            .discovery
                            .announce(computer.endpoint.port, &self.fingerprint)
                        {
                            Ok(()) => self.discovery_deadline = Some(self.now + DISCOVERY_WINDOW),
                            Err(error) => self.notice(format!("Cannot allow discovery: {error}")),
                        }
                    }
                }
            }
            Intent::StopDiscovery => self.stop_discovery(),
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
            Intent::HandBack { session } => {
                if self
                    .incoming
                    .get(&session)
                    .is_some_and(|target| self.hand_back_cause(target).is_none())
                    && let Some(target) = self.incoming.get_mut(&session)
                {
                    let actions = target.hand_back();
                    self.run_target_actions(session, actions);
                }
            }
            Intent::StopSession { session } => self.end_incoming(session, EndReason::TargetStopped),
            Intent::ControlFromHere { session } => self.control_from_here(session),
            Intent::SwitchAndControl {
                display,
                device,
                target,
            } => self.start_on_display(&display, device, target, false),
            Intent::ControlOnly {
                display,
                device,
                target,
            } => self.start_on_display(&display, device, target, true),
            Intent::SwitchDisplayOnly {
                display,
                device,
                target,
            } => self.switch_display_only(&display, device, target),
            Intent::LetControl { peer, device } => self.let_control(peer, device),
        }
    }

    /// Ends the incoming session (FLOW-17).
    /// Opens a portal on its input-target display.
    /// Starts a handoff to the old source.
    fn control_from_here(&mut self, session: SessionId) {
        let Some(target) = self.incoming.get(&session) else {
            self.notice("Cannot control from here: the incoming session has ended.".into());
            return;
        };
        let source = target.source().clone();
        let token = target.display().clone();
        self.end_incoming(session, EndReason::TargetRequestedControl);
        let Some(display) = self.displays().into_iter().find(|d| d.token == token) else {
            self.notice("Cannot control from here: the input-target display is gone.".into());
            return;
        };
        self.open_portal(display);
        let Some(portal) = self.portal.as_mut() else {
            return;
        };
        portal.pending_start = Some(PendingStart {
            target: source,
            device: None,
            attach: false,
        });
        self.portal_window_request = Some(token);
        self.start_pending_handoff();
    }

    /// The window layer could not show a prompt that the presenter accepted.
    pub fn prompt_failed(&mut self, prompt: PromptId) {
        let session = self
            .prompts
            .iter()
            .find_map(|(session, id)| (*id == prompt).then_some(*session));
        if let Some(session) = session {
            self.notice("Cannot show the prompt window.".into());
            self.device_states.extend(
                self.incoming
                    .get(&session)
                    .map(|t| (t.device().clone(), VideoInput::Unknown)),
            );
            self.end_incoming(session, EndReason::PromptFailed);
            return;
        }
        let device = self
            .hand_back_verifications
            .iter()
            .find_map(|(device, v)| (v.prompt == prompt).then(|| device.clone()));
        let Some(device) = device else {
            return;
        };
        self.notice("Cannot show the prompt window.".into());
        if let Some(verification) = self.hand_back_verifications.get_mut(&device) {
            let actions = verification.verification.on_shown(false);
            self.run_hand_back_verification_actions(&device, actions);
        }
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
                        self.portal_display_changed(display);
                    }
                }
            }
        }
    }

    // ----- View models -----

    /// Pointer statistics of the newest incoming session, else the last ended
    /// incoming session (NET-9).
    pub fn pointer_stats(&self) -> Option<PointerSummary> {
        self.incoming_pointer_stats
            .last_key_value()
            .map(|(_, stats)| stats.summary())
            .or_else(|| self.last_pointer_summary.clone())
    }

    /// The display of the current display list with this token.
    pub fn display(&self, token: &telescreen_core::ids::DisplayToken) -> Option<DisplayInfo> {
        self.display_cache
            .iter()
            .find(|d| &d.token == token)
            .cloned()
    }

    /// The bounds of all displays except the one with this token.
    /// True while a control request to `peer` for `device` waits (ACT-6).
    pub fn control_request_pending(&self, peer: &ComputerId, device: &DeviceId) -> bool {
        self.control_requests
            .contains_key(&(peer.clone(), device.clone()))
    }

    pub fn has_portal(&self) -> bool {
        self.portal.is_some()
    }

    /// True while input is forwarded (FLOW-13).
    pub fn forwarding(&self) -> bool {
        self.portal_phase() == Some(SourcePhase::Forwarding)
    }

    /// True while the portal keeps focus during handoff and verification (FLOW-21).
    pub fn guarded(&self) -> bool {
        self.portal.as_ref().is_some_and(|portal| portal.guarded)
    }

    pub fn portal_screen(&self) -> Option<PortalScreen> {
        let portal = self.portal.as_ref()?;
        let device = self.portal_device();
        let routes = device
            .as_ref()
            .map(|device| routes::candidates(&self.config, device))
            .unwrap_or_default();
        let show_refresh = routes.iter().any(|r| !self.connected.contains(r.target));
        let can_refresh = routes.iter().any(|r| {
            !self.connected.contains(r.target)
                && (!self.dialing.contains(r.target) && !self.searching(r.target)
                    || self.discovery_enabled(r.target) && self.search_deadline.is_none())
        });
        let candidates = routes
            .into_iter()
            .map(|route| CandidateRow {
                target: route.target.clone(),
                label: self.label_of(route.target),
                unavailable: self.candidate_unavailable(route.target),
            })
            .collect();
        let session = portal.session.as_ref().map(|s| SessionRow {
            target: self.label_of(s.target()),
            phase: self.phase_view(s),
            video_input: s.video_input(),
        });
        Some(PortalScreen {
            display: display_name(&portal.display),
            matching: portal.matching.clone(),
            candidates,
            show_refresh,
            can_refresh,
            session,
            notice: portal.notice.clone(),
        })
    }

    fn phase_view(&self, session: &SourceSession) -> SessionPhase {
        match session.phase() {
            SourcePhase::Requesting => SessionPhase::Requesting,
            SourcePhase::HandingOff => SessionPhase::HandingOff,
            SourcePhase::Verifying if session.ack_wanted() => SessionPhase::WaitingForAck {
                key: self.labels.confirm_key.clone(),
            },
            SourcePhase::Verifying => SessionPhase::Verifying,
            SourcePhase::Forwarding => SessionPhase::Forwarding,
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
        let devices = self
            .local_devices()
            .into_iter()
            .map(|(alias, device)| DeviceRow {
                alias,
                state: self
                    .device_states
                    .get(&device.id)
                    .copied()
                    .unwrap_or(VideoInput::Unchanged),
                device: device.id,
            })
            .collect();
        let incoming = self
            .incoming
            .iter()
            .map(|(session, t)| IncomingRow {
                session: *session,
                source: self.label_of(t.source()),
                state: target_phase_text(t.phase()).into(),
                hand_back: self.hand_back_cause(t),
            })
            .collect();
        ControlScreen {
            computer: self.label_of(&self.local),
            fingerprint: self.fingerprint.clone(),
            displays,
            devices,
            incoming,
            clipboard: self
                .ports
                .clipboard
                .as_ref()
                .map(|_| self.clipboard.enabled()),
            discovery: self.discovery_enabled(&self.local).then(|| DiscoveryRow {
                remaining_seconds: self.discovery_deadline.map(|deadline| {
                    let remaining = deadline.saturating_duration_since(self.now);
                    remaining.as_secs() + u64::from(remaining.subsec_nanos() != 0)
                }),
            }),
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
        RefuseReason::NoFocusHold => "the target has no keyboard focus holder",
        RefuseReason::Busy => "the device is in use by a different session",
    }
    .into()
}

fn end_text(reason: EndReason) -> &'static str {
    match reason {
        EndReason::HandBack => "handed back",
        EndReason::SafetyExit => "safety exit",
        EndReason::SourceClosed => "ended on this computer",
        EndReason::TargetDisconnected => "disconnected on the target",
        EndReason::TargetStopped => "the target stopped the session",
        EndReason::TargetRequestedControl => "the target took control back",
        EndReason::VerifyFailed => "the verification failed",
        EndReason::HandoffFailed => "the handoff failed",
        EndReason::HandoffPartial => "the handoff stopped after some steps",
        EndReason::PromptFailed => "the target cannot show the prompt",
        EndReason::DisplayGone => "the target display is gone",
        EndReason::PortalGone => "the portal display is gone",
        EndReason::QueueOverflow => "input queue overflow",
        EndReason::PermissionLost => "a permission is missing",
        EndReason::PeerLost => "connection lost",
        EndReason::ProtocolError => "protocol error",
        EndReason::NewHandoff => "a new handoff of this display started",
    }
}

fn target_phase_text(phase: TargetPhase) -> &'static str {
    match phase {
        TargetPhase::AwaitingHandoff => "switching the display",
        TargetPhase::Verifying => "verifying the display",
        TargetPhase::CheckingDisplay => "starting",
        TargetPhase::Forwarding => "controls this computer",
        TargetPhase::Ended(reason) => end_text(reason),
    }
}
