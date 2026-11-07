//! The controller: it connects the session rules of the core to the network, the
//! platform, the driver jobs, and the view. It has no OS code and no UI library,
//! so tests can run two controllers with fake ports (`tests/controller.rs`).
//!
//! All methods run on one thread (the UI thread). Ports must not block.
//!
//! This file holds the state, the timers, and the dispatch of network, job,
//! and intent events. The submodules add the methods of each responsibility:
//! `source` (portals), `target` (incoming sessions), `hand_back`, `clipboard`,
//! `view` (view models), `actions`, `menu`, `portal_match`, and `verification`.

use std::collections::{BTreeMap, BTreeSet, HashMap, VecDeque};
use std::path::PathBuf;
use std::time::Instant;

use telescreen_core::clipboard::ClipboardSync;
use telescreen_core::config::{Config, Device, Operation};
use telescreen_core::driver::{self, OperationKind, Outcome, Request};
use telescreen_core::ids::{ComputerId, DeviceId, DisplayToken, PortalId, SessionId};
use telescreen_core::input::InputEvent;
use telescreen_core::platform::{
    Capability, Clipboard, Discovery, DisplayInfo, Displays, FocusHolder, Injector,
    InputCapabilities, InputCapture,
};
use telescreen_core::pointer::{Point, Rect};
use telescreen_core::pointer_stats::{PointerStats, PointerSummary};
use telescreen_core::session::VideoInput;
use telescreen_core::session::lease::LeaseTable;
use telescreen_core::session::source::{HandoffResult, SourcePhase, SourceSession};
use telescreen_core::session::target::{TargetPhase, TargetSession};
use telescreen_core::verifier::VerifierEvent;
use telescreen_core::wire::{EndReason, HandBackOutcome, Message};
use telescreen_transport::{DialTarget, NetCommand, NetEvent};
use telescreen_view::{Intent, Matching, PromptId, PromptPresenter};

use crate::pointer_mailbox::ReceivedPointer;

mod actions;
mod clipboard;
mod hand_back;
mod menu;
mod portal_match;
mod source;
mod target;
mod verification;
mod view;

use actions::{ControlMatch, PendingStart};
use menu::DisplayEvaluation;
use portal_match::{MatchOwner, PortalMatches, Ticket};
use verification::{Owner, Verifications};
use view::{display_name, end_text, refuse_text, target_phase_text};

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

/// Where the user gave the acknowledgement key or a left click. The controller
/// selects the verification that it acknowledges (FLOW-8, VER-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AckOrigin {
    /// A portal window. The portal must have the focus (FLOW-8). If its
    /// session does not accept, the first hand back verification that waits
    /// on this computer gets it (VER-4).
    Portal(PortalId),
    /// A click in a prompt window on this computer. Only the hand back
    /// verification of this prompt gets it (VER-4).
    Prompt(PromptId),
    /// The acknowledgement key in a prompt window on this computer. The hand
    /// back verification of this prompt gets it; else the first hand back
    /// verification that waits gets it, because Return in each window of
    /// Telescreen on the destination is an acknowledgement (VER-4).
    PromptKey(PromptId),
    /// Another window of this computer, or a debug request: the portal with
    /// the native focus, else the newest portal (FLOW-8). If it does not
    /// accept, the first hand back verification that waits gets it (VER-4).
    Computer,
}

/// Runs `exec` and `command` verifiers off the UI thread. The app sends the
/// events back with `Controller::on_verifier_event`.
pub trait VerifierPort {
    fn start(&mut self, id: VerifierId, operation: Operation, request: Request);
    fn ack(&mut self, id: VerifierId);
    fn stop(&mut self, id: VerifierId);
}

/// One incoming session on this target: the session, its pointer statistics
/// (NET-9), and its portal warning (FLOW-26). `remove_incoming` removes all of
/// it at one time.
struct Incoming {
    target: TargetSession,
    stats: PointerStats,
    warning: Option<PromptId>,
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
    /// `portal-match` of one candidate of a portal, the menu, or a control
    /// request (FLOW-2, UI-18, ACT-4).
    PortalMatch(Ticket),
    InputTarget {
        session: SessionId,
        peer: ComputerId,
        device: DeviceId,
    },
    Handoff {
        portal: PortalId,
        session: Option<SessionId>,
    },
    HandBack {
        peer: ComputerId,
        device: DeviceId,
    },
    HandBackInputTarget {
        prompt: PromptId,
        device: DeviceId,
    },
    /// The steps of "Switch display only" (ACT-1).
    SwitchOnly {
        target: ComputerId,
        device: DeviceId,
    },
}

struct Portal {
    /// The number in the `portal.id` of the requests. Each `portal-match`
    /// evaluation of the portal gets a new number (FLOW-4).
    generation: u64,
    display: DisplayInfo,
    bounds: Rect,
    focused: bool,
    /// A click in the portal asks for the native focus, and the window layer has
    /// not asked the OS yet (FLOW-22).
    focus_wanted: bool,
    /// The portal is on screen (FLOW-26).
    visible: bool,
    guarded: bool,
    matching: Matching,
    /// The selected local device: its config alias and its config entry.
    device: Option<(String, Device)>,
    /// The session to start after `portal-match` (FLOW-17, ACT-1, ACT-4).
    pending_start: Option<PendingStart>,
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
    /// At most one portal on each display (FLOW-2, FLOW-25).
    portals: BTreeMap<PortalId, Portal>,
    /// The portal windows that the window layer must show (FLOW-17, ACT-5).
    portal_window_requests: VecDeque<(PortalId, DisplayToken)>,
    /// The next portal id. An id is never used two times.
    next_portal: u64,
    next_match: u64,
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
    incoming: BTreeMap<SessionId, Incoming>,
    last_pointer_summary: Option<PointerSummary>,
    /// Device state on this destination, independent of sessions (VER-6).
    /// A missing device is `Unchanged`: no handoff to this computer yet.
    device_states: HashMap<DeviceId, VideoInput>,
    injector_session: Option<SessionId>,
    jobs: HashMap<JobId, Pending>,
    next_job: u64,
    /// The destination verifications of sessions and hand backs (VER-1).
    verifications: Verifications,
    next_prompt: u64,
    pending_hand_backs: BTreeSet<(ComputerId, DeviceId)>,
    /// Control requests that this peer sent, with their deadline (ACT-6). A
    /// request waits in `unsent_control` until its peer connects.
    control_requests: BTreeMap<(ComputerId, DeviceId), Instant>,
    unsent_control: BTreeSet<(ComputerId, DeviceId)>,
    /// The control requests that this peer received, while `portal-match`
    /// runs for them (ACT-4).
    control_matches: BTreeMap<u64, ControlMatch>,
    /// The `portal-match` evaluations of the portals, the menu, and the control
    /// requests (AC-9, FLOW-4).
    matches: PortalMatches,
    next_control_match: u64,
    /// `portal-match` of each display for the menu (UI-18). `None` until the
    /// first `evaluate_displays`.
    display_evaluation: Option<DisplayEvaluation>,
    /// The computers with a fingerprint in the local trust list (UI-13).
    trusted: BTreeSet<ComputerId>,
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

/// Adds a notice for the user and keeps only the newest ones.
fn push_notice(notices: &mut Vec<String>, text: String) {
    eprintln!("telescreen: notice: {text}");
    notices.push(text);
    if notices.len() > MAX_NOTICES {
        notices.remove(0);
    }
}

/// Lists the local displays and keeps the list for the view.
fn list_displays(
    displays: &dyn Displays,
    cache: &mut Vec<DisplayInfo>,
    notices: &mut Vec<String>,
) -> Vec<DisplayInfo> {
    match displays.displays() {
        Ok(displays) => {
            cache.clone_from(&displays);
            displays
        }
        Err(e) => {
            push_notice(notices, format!("Cannot list the displays: {e}"));
            Vec::new()
        }
    }
}

/// The parts of the controller that the destination verifications use.
struct VerificationEnv<'a> {
    ports: &'a mut Ports,
    display_cache: &'a mut Vec<DisplayInfo>,
    notices: &'a mut Vec<String>,
    states: &'a mut HashMap<DeviceId, VideoInput>,
}

impl verification::Env for VerificationEnv<'_> {
    fn verifiers(&mut self) -> &mut dyn VerifierPort {
        &mut *self.ports.verifiers
    }

    fn prompt(&mut self) -> &mut dyn PromptPresenter {
        &mut *self.ports.prompt
    }

    fn displays(&mut self) -> Vec<DisplayInfo> {
        list_displays(&*self.ports.displays, self.display_cache, self.notices)
    }

    fn notice(&mut self, text: String) {
        push_notice(self.notices, text);
    }

    fn states(&mut self) -> &mut HashMap<DeviceId, VideoInput> {
        self.states
    }
}

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
        let verifications = Verifications::new(
            local.clone(),
            labels.confirm_key.clone(),
            config.computer(&local).and_then(|c| c.hint.clone()),
        );
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
            portals: BTreeMap::new(),
            portal_window_requests: VecDeque::new(),
            next_portal: 1,
            next_match: 1,
            connected: BTreeSet::new(),
            dialing: BTreeSet::new(),
            found: HashMap::new(),
            search_deadline: None,
            discovery_deadline: None,
            now: Instant::now(),
            leases: LeaseTable::new(first_generation),
            incoming: BTreeMap::new(),
            last_pointer_summary: None,
            device_states: HashMap::new(),
            injector_session: None,
            jobs: HashMap::new(),
            next_job: 1,
            verifications,
            next_prompt: 1,
            pending_hand_backs: BTreeSet::new(),
            control_requests: BTreeMap::new(),
            unsent_control: BTreeSet::new(),
            control_matches: BTreeMap::new(),
            matches: PortalMatches::default(),
            next_control_match: 1,
            display_evaluation: None,
            trusted: BTreeSet::new(),
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
        self.close_all_portals(EndReason::SourceClosed);
        self.stop_discovery();
        let sessions: Vec<_> = self.incoming.keys().copied().collect();
        for session in sessions {
            self.end_incoming(session, EndReason::TargetDisconnected);
        }
        for device in self.verifications.hand_backs() {
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
        push_notice(&mut self.notices, text);
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
            InputEvent::Scroll { dx, dy, phase } => InputEvent::Scroll {
                dx: dx * speed,
                dy: dy * speed,
                phase,
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
        list_displays(
            &*self.ports.displays,
            &mut self.display_cache,
            &mut self.notices,
        )
    }

    /// Runs `f` on the destination verifications. Then it ends each target
    /// session that VER-8 stopped, with `VerifyFailed`.
    fn verify<R>(
        &mut self,
        f: impl FnOnce(&mut Verifications, &mut dyn verification::Env) -> R,
    ) -> R {
        let mut env = VerificationEnv {
            ports: &mut self.ports,
            display_cache: &mut self.display_cache,
            notices: &mut self.notices,
            states: &mut self.device_states,
        };
        let result = f(&mut self.verifications, &mut env);
        for session in self.verifications.take_displaced() {
            self.end_incoming(session, EndReason::VerifyFailed);
        }
        result
    }

    /// Delivers an event of a verifier to its owner.
    fn deliver(&mut self, owner: Owner, event: VerifierEvent, now: Instant) {
        match owner {
            Owner::Session(session) => {
                let actions = self.verifier_actions(session, event);
                self.run_target_actions_at(session, actions, now);
            }
            Owner::HandBack(_) => {
                self.verify(|v, env| v.on_event(&owner, event, env));
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

    // ----- Timers -----

    /// The main loop calls this often. It runs the timers and sends the heartbeat
    /// to each connected peer once a second, from this thread, so that a hung app
    /// stops its heartbeat and the other peer ends the sessions (ADR 0007, AC-18).
    pub fn tick(&mut self, now: Instant) {
        self.now = now;
        if let Some(injector) = self.ports.injector.as_mut()
            && let Err(e) = injector.tick(now)
        {
            // Not a notice: the next tick can fail again.
            eprintln!("telescreen: injection timer: {e}");
        }
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
        for (owner, event) in self.verifications.tick(now) {
            self.deliver(owner, event, now);
        }
        self.update_clipboard();
        self.sync_clipboard();
    }

    // ----- Network -----

    pub fn on_net(&mut self, event: NetEvent, now: Instant) {
        match event {
            NetEvent::Connected { peer } => {
                self.dialing.remove(&peer);
                self.connected.insert(peer.clone());
                if !self.candidate_to_search() {
                    self.stop_search();
                }
                self.send_control_requests(&peer);
                for id in self.portal_ids() {
                    self.start_pending_handoff_in(id);
                }
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
        for id in self.portals_to(peer) {
            let actions = self
                .source_session_mut(id)
                .map(SourceSession::on_peer_lost)
                .unwrap_or_default();
            self.run_source_actions(id, actions);
        }
        let sessions: Vec<_> = self
            .targets()
            .filter(|(_, t)| t.source() == peer)
            .map(|(s, _)| s)
            .collect();
        for session in sessions {
            if let Some(target) = self.target_mut(session) {
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
                self.hand_back_result(peer, device.clone(), *outcome);
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
            | Message::PortalHidden { session }
            | Message::PortalShown { session }
            | Message::PortalCursor { session, .. }
            | Message::End { session, .. } => {
                let session = *session;
                let Some(Incoming { target, .. }) = self.incoming.get_mut(&session) else {
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
                self.display_switched(peer, device.clone());
            }
            Message::ControlRequest { device } => self.control_request(peer, device.clone()),
            Message::ControlRefused { device, reason } => {
                self.control_refused(&peer, device, *reason);
            }
        }
    }

    /// A message from a target goes to each portal with a session to it. Each
    /// session takes only the messages of its device and session id (FLOW-25).
    fn source_message(&mut self, peer: &ComputerId, message: &Message, now: Instant) {
        for id in self.portals_to(peer) {
            self.portal_message(id, peer, message, now);
        }
    }

    fn portal_message(&mut self, id: PortalId, peer: &ComputerId, message: &Message, now: Instant) {
        let Some(session) = self.source_session_mut(id) else {
            return;
        };
        let refused = matches!(message, Message::SessionRefused { device, .. }
            if device == session.device() && session.phase() == SourcePhase::Requesting);
        let hand_back_device =
            matches!(message, Message::End { session: ended, reason: EndReason::HandBack }
            if session.session() == Some(*ended) && !session.is_finished())
            .then(|| session.device().clone());
        let actions = session.on_message(message, now);
        if let Some(device) = hand_back_device {
            self.stop_hand_back_verification(&device);
            self.pending_hand_backs.insert((peer.clone(), device));
        }
        if let Message::SessionRefused { reason, .. } = message
            && refused
            && let Some(portal) = self.portals.get_mut(&id)
        {
            portal.notice = Some(refuse_text(*reason));
            // ACT-5: the portal closes with its session, so keep the reason.
            let text = format!(
                "{} refused the session: {}",
                self.label_of(peer),
                refuse_text(*reason)
            );
            self.notice(text);
        }
        self.run_source_actions(id, actions);
    }

    // ----- Jobs -----

    pub fn on_job_done(&mut self, id: JobId, output: JobOutput) {
        let Some(pending) = self.jobs.remove(&id) else {
            return;
        };
        match pending {
            Pending::PortalMatch(ticket) => {
                let failed = !matches!(output, JobOutput::Operation(Ok(Outcome::Matched(_))));
                let Some(owner) = self.matches.record(ticket, &output) else {
                    return;
                };
                match owner {
                    MatchOwner::Portal(portal) => self.finish_portal_match(portal),
                    MatchOwner::Display(token) => self.display_match_done(&token),
                    MatchOwner::Control(request) => {
                        if failed {
                            eprintln!("telescreen: portal-match for a control request: {output:?}");
                        }
                        self.control_match_done(request);
                    }
                }
            }
            Pending::InputTarget {
                session,
                peer,
                device,
            } => self.input_target_done(session, peer, device, output),
            Pending::Handoff { portal, session } => {
                let JobOutput::Handoff { result, error } = output else {
                    return;
                };
                if let Some(error) = error {
                    self.notice(format!("Handoff failed: {error}"));
                }
                let current = self.source_session(portal).and_then(|s| s.session());
                if current != session || session.is_none() {
                    return;
                }
                let actions = self
                    .source_session_mut(portal)
                    .map(|s| s.on_handoff_finished(result))
                    .unwrap_or_default();
                self.run_source_actions(portal, actions);
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
            Pending::HandBackInputTarget { prompt, device } => {
                let owner = Owner::HandBack(device.clone());
                if self.verifications.is_current(&owner, prompt) {
                    self.hand_back_input_target_done(device, output);
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
            Intent::Refresh { display } => self.refresh_display(&display),
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
            Intent::EndSession { portal } => {
                if let Some(id) = portal.or_else(|| self.main_portal()) {
                    self.end_portal_session(id, EndReason::SourceClosed);
                }
            }
            Intent::HandBack { session } => {
                if self
                    .target(session)
                    .is_some_and(|target| self.hand_back_cause(target).is_none())
                    && let Some(target) = self.target_mut(session)
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
            Intent::SourceHandBack { portal } => {
                if let Some(id) = portal.or_else(|| self.main_portal()) {
                    self.portal_left_fullscreen(id);
                }
            }
        }
    }

    /// The window layer could not show a prompt that the presenter accepted.
    pub fn prompt_failed(&mut self, prompt: PromptId) {
        if let Some(session) = self.verify(|v, env| v.prompt_failed(prompt, env)) {
            self.end_incoming(session, EndReason::PromptFailed);
        }
    }

    /// The local display list changed (FLOW-4, FLOW-19, and FLOW-10 on the target).
    pub fn displays_changed(&mut self) {
        let displays = self.displays();
        let gone: Vec<_> = self
            .targets()
            .filter(|(_, t)| t.phase() == TargetPhase::Forwarding)
            .filter(|(_, t)| !displays.iter().any(|d| &d.token == t.display()))
            .map(|(s, _)| s)
            .collect();
        for session in gone {
            self.end_incoming(session, EndReason::DisplayGone);
        }
        if self.displays_differ(&displays) {
            self.evaluate_displays();
        }
        for id in self.portal_ids() {
            let Some(token) = self.portals.get(&id).map(|p| p.display.token.clone()) else {
                continue;
            };
            match displays.iter().find(|d| d.token == token).cloned() {
                None => self.close_portal(id, EndReason::PortalGone),
                Some(display) => {
                    let changed = self.portals.get(&id).is_some_and(|p| p.display != display);
                    if changed {
                        self.portal_display_changed(id, display);
                    }
                }
            }
        }
    }
}
