//! Controllers with fake ports and an in-memory network: the user flow of
//! `docs/spec.md` section 4 across two peers, and three peers for FLOW-24.
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::{Duration, Instant};

use serde_json::json;
use telescreen_app::controller::{
    Controller, Job, JobId, JobOutput, JobPort, Labels, NetPort, Ports, VerifierId, VerifierPort,
};
use telescreen_app::pointer_mailbox::ReceivedPointer;
use telescreen_core::clipboard::{ClipboardChange, ClipboardItem};
use telescreen_core::config::{Config, Driver, Operation};
use telescreen_core::driver::{OperationKind, Outcome, Request};
use telescreen_core::ids::{ComputerId, DeviceId, DisplayToken};
use telescreen_core::input::{InputEvent, ScrollPhase};
use telescreen_core::keys::Key;
use telescreen_core::platform::{
    Capability, CapturedEvent, Clipboard, Discovery, DisplayInfo, Displays, FocusHolder, Injector,
    InputCapabilities, InputCapture, PlatformError,
};
use telescreen_core::pointer::{Normalized, Point, Rect};
use telescreen_core::session::VideoInput;
use telescreen_core::session::source::HandoffResult;
use telescreen_core::verifier::VerifierEvent;
use telescreen_core::wire::{EndReason, HandBackOutcome, Message, PointerDatagram, RefuseReason};
use telescreen_transport::{DialTarget, NetCommand, NetEvent};
use telescreen_view::{Intent, Matching, PromptId, PromptPresenter, PromptScreen, SessionPhase};

/// The config of the tests. pc1 and the Mac verify with the `prompt` verifier.
fn config() -> Config {
    let op = |name: &str| json!({"driver": "command", "command": [format!("/bin/{name}")]});
    serde_json::from_value(json!({
        "version": 1,
        "computers": {
            "mac": {
                "label": "MacBook",
                "clipboard": {"text": true},
                "endpoint": {"host": "mac.test", "port": 24800},
                "devices": {
                    "Desktop": {
                        "id": "shared-lg",
                        "portal-match": op("match"),
                        "input-target": op("target"),
                        "verify": {"driver": "prompt"},
                        "handoff": {"pc1": {"steps": [op("to-pc1")]}}
                    }
                }
            },
            "pc1": {
                "label": "Linux",
                "hint": "Copy: Ctrl+C",
                "clipboard": {"text": true},
                "scroll-speed": 0.5,
                "endpoint": {"host": "pc1.test", "port": 24800},
                "devices": {
                    "Monitor": {
                        "id": "shared-lg",
                        "portal-match": op("match"),
                        "input-target": op("target"),
                        "verify": {"driver": "prompt"},
                        "handoff": {"mac": {"steps": [op("to-mac")]}}
                    }
                }
            },
            "pc2": {
                "endpoint": {"host": "pc2.test", "port": 24800},
                "devices": {"Screen": {"id": "shared-lg", "input-target": op("target")}}
            },
            "pc3": {
                "endpoint": {"host": "pc3.test", "port": 24800},
                "devices": {
                    "Screen": {
                        "id": "shared-lg",
                        "handoff": {"pc1": {"steps": [op("to-pc1")]}}
                    }
                }
            },
            "pc4": {
                "label": "Desk PC",
                "endpoint": {"host": "pc4.test", "port": 24800},
                "devices": {
                    "Screen": {
                        "id": "shared-lg",
                        "portal-match": op("match"),
                        "handoff": {"mac": {"steps": [op("to-mac")]}}
                    }
                }
            }
        }
    }))
    .unwrap()
}

#[derive(Default)]
struct Log {
    /// The calls of the discovery port, in sequence.
    discovery: Vec<String>,
    /// The next `announce` and `search` calls fail.
    discovery_fails: bool,
    net: VecDeque<NetCommand>,
    jobs: VecDeque<(JobId, Job)>,
    capture: Vec<String>,
    injected: Vec<InputEvent>,
    pointer: Vec<Normalized>,
    prompts: Vec<String>,
    prompt_visible: bool,
    /// The cursor positions on the prompt (FLOW-26).
    prompt_cursor: Vec<Option<Normalized>>,
    /// Peers of each `NetCommand::Close`.
    closed: Vec<ComputerId>,
    /// The number of `NetPort::shutdown` calls.
    net_shutdowns: usize,
    /// Injected events and focus holder calls, in sequence (FLOW-23).
    target: Vec<String>,
    /// Local clipboard changes that the next `changed` call reports.
    clip_pending: Vec<ClipboardChange>,
    /// Texts and concealed marks that the controller put on the clipboard.
    clip_set: Vec<(String, bool)>,
    /// The clipboard is between `open` and `close`.
    clip_open: bool,
    /// The clipboard still holds the last text from `set_text` (CLIP-11).
    clip_own: bool,
    /// `open`, `close`, and `clear` (a `clear_if_owned` that removed a text).
    clip_calls: Vec<&'static str>,
    /// Calls of the verifier port: "start <id> <driver>", "ack <id>", "stop <id>".
    verifier_calls: Vec<String>,
}

type Shared = Rc<RefCell<Log>>;

struct Net(Shared);
impl NetPort for Net {
    fn send(&mut self, command: NetCommand) {
        self.0.borrow_mut().net.push_back(command);
    }
    fn shutdown(&mut self) {
        self.0.borrow_mut().net_shutdowns += 1;
    }
}

struct Jobs(Shared);
impl JobPort for Jobs {
    fn spawn(&mut self, id: JobId, job: Job) {
        let mut log = self.0.borrow_mut();
        if let Job::Handoff { steps, .. } = &job {
            let first = match &steps[0].driver {
                Driver::Command(program) | Driver::Exec(program) => program.argv[0].clone(),
                _ => "other".into(),
            };
            log.target.push(format!("handoff {first}"));
        }
        log.jobs.push_back((id, job));
    }
}

struct FakeDisplays(Vec<DisplayInfo>);
impl Displays for FakeDisplays {
    fn displays(&self) -> Result<Vec<DisplayInfo>, PlatformError> {
        Ok(self.0.clone())
    }
}

struct Capture(Shared);
impl InputCapture for Capture {
    fn arm(&mut self, portal: Rect) -> Result<(), PlatformError> {
        self.0
            .borrow_mut()
            .capture
            .push(format!("arm {}", portal.width));
        Ok(())
    }
    fn forward(&mut self) -> Result<(), PlatformError> {
        self.0.borrow_mut().capture.push("forward".into());
        Ok(())
    }
    fn disarm(&mut self) {
        self.0.borrow_mut().capture.push("disarm".into());
    }
    fn guard(&mut self, on: bool) -> Result<(), PlatformError> {
        let text = if on { "guard on" } else { "guard off" };
        self.0.borrow_mut().capture.push(text.into());
        Ok(())
    }
    fn set_portal_visible(&mut self, visible: bool) {
        let text = if visible { "visible" } else { "hidden" };
        self.0.borrow_mut().capture.push(text.into());
    }
    fn set_portal_focus(&mut self, focused: bool) {
        self.0.borrow_mut().capture.push(format!("focus {focused}"));
    }
}

struct Inject(Shared);
impl Injector for Inject {
    fn set_display(&mut self, _: &DisplayInfo) -> Result<(), PlatformError> {
        Ok(())
    }
    fn pointer(&mut self, at: Normalized) -> Result<(), PlatformError> {
        self.0.borrow_mut().pointer.push(at);
        Ok(())
    }
    fn input(&mut self, event: &InputEvent) -> Result<(), PlatformError> {
        let mut log = self.0.borrow_mut();
        log.injected.push(*event);
        log.target.push(format!("inject {event:?}"));
        Ok(())
    }
}

struct Clip(Shared);
impl Clipboard for Clip {
    fn open(&mut self) -> Result<(), PlatformError> {
        let mut log = self.0.borrow_mut();
        assert!(!log.clip_open, "open twice");
        log.clip_open = true;
        // The current content is the baseline (CLIP-3).
        log.clip_pending.clear();
        log.clip_calls.push("open");
        Ok(())
    }
    fn close(&mut self) {
        let mut log = self.0.borrow_mut();
        assert!(log.clip_open, "close while closed");
        log.clip_open = false;
        log.clip_calls.push("close");
    }
    fn changed(&mut self) -> Option<ClipboardChange> {
        let mut log = self.0.borrow_mut();
        assert!(log.clip_open, "a read while closed (CLIP-1)");
        // A real clipboard reports only its latest content.
        log.clip_pending.drain(..).next_back()
    }
    fn set_text(&mut self, text: &str, concealed: bool) -> Result<(), PlatformError> {
        let mut log = self.0.borrow_mut();
        assert!(log.clip_open, "a write while closed");
        log.clip_set.push((text.into(), concealed));
        log.clip_own = true;
        // The platform then reports the change that the app made (CLIP-4).
        log.clip_pending.push(ClipboardChange::Text(ClipboardItem {
            text: text.into(),
            concealed,
        }));
        Ok(())
    }
    fn clear_if_owned(&mut self) {
        let mut log = self.0.borrow_mut();
        assert!(log.clip_open, "a clear while closed");
        if std::mem::take(&mut log.clip_own) {
            log.clip_calls.push("clear");
            log.clip_pending.push(ClipboardChange::Cleared);
        }
    }
}

struct Holder(Shared);
impl FocusHolder for Holder {
    fn hold(&mut self, display: &DisplayInfo) -> Result<(), PlatformError> {
        self.0
            .borrow_mut()
            .target
            .push(format!("hold {}", display.token.0));
        Ok(())
    }
    fn release(&mut self) {
        self.0.borrow_mut().target.push("release".into());
    }
}

struct Verifiers(Shared);
impl VerifierPort for Verifiers {
    fn start(&mut self, id: VerifierId, operation: Operation, request: Request) {
        let driver = match operation.driver {
            Driver::Exec(_) => "exec",
            Driver::Command(_) => "command",
            _ => "other",
        };
        // The destination verifies its own device for the source (ADR 0009).
        assert_eq!(request.computer_id, ComputerId::from("pc1"));
        assert_eq!(request.source_computer_id, ComputerId::from("mac"));
        assert!(request.session_id.is_some());
        self.0
            .borrow_mut()
            .verifier_calls
            .push(format!("start {} {driver}", id.0));
    }
    fn ack(&mut self, id: VerifierId) {
        self.0
            .borrow_mut()
            .verifier_calls
            .push(format!("ack {}", id.0));
    }
    fn stop(&mut self, id: VerifierId) {
        self.0
            .borrow_mut()
            .verifier_calls
            .push(format!("stop {}", id.0));
    }
}

struct Prompt(Shared);
impl PromptPresenter for Prompt {
    fn show(&mut self, _: PromptId, _: &DisplayInfo, screen: &PromptScreen) -> Result<(), String> {
        let mut log = self.0.borrow_mut();
        log.prompts.push(format!(
            "{} | {}",
            screen.title().unwrap_or_default(),
            screen.text.clone().unwrap_or_default()
        ));
        log.prompt_visible = true;
        Ok(())
    }
    fn hide(&mut self, _: PromptId) {
        self.0.borrow_mut().prompt_visible = false;
    }
    fn cursor(&mut self, _: PromptId, at: Option<Normalized>) {
        self.0.borrow_mut().prompt_cursor.push(at);
    }
}

fn display(token: &str, x: f64, width: f64) -> DisplayInfo {
    DisplayInfo {
        token: DisplayToken::from(token),
        bounds: Some(Rect {
            x,
            y: 0.0,
            width,
            height: 1000.0,
        }),
        scale: 2.0,
        metadata: Default::default(),
    }
}

struct Peer {
    id: ComputerId,
    controller: Controller,
    log: Shared,
}

struct FakeDiscovery(Shared);

impl FakeDiscovery {
    fn call(&self, text: String) -> Result<(), PlatformError> {
        let mut log = self.0.borrow_mut();
        if log.discovery_fails {
            return Err(PlatformError::Unsupported("no responder".into()));
        }
        log.discovery.push(text);
        Ok(())
    }
}

impl Discovery for FakeDiscovery {
    fn announce(&mut self, port: u16, fingerprint: &str) -> Result<(), PlatformError> {
        self.call(format!("announce {port} {fingerprint}"))
    }

    fn stop_announce(&mut self) {
        self.0.borrow_mut().discovery.push("stop_announce".into());
    }

    fn search(&mut self) -> Result<(), PlatformError> {
        self.call("search".into())
    }

    fn stop_search(&mut self) {
        self.0.borrow_mut().discovery.push("stop_search".into());
    }
}

fn peer(id: &str, displays: Vec<DisplayInfo>) -> Peer {
    let capabilities = InputCapabilities {
        capture: Capability::Available,
        suppress: Capability::Available,
        inject: Capability::Available,
    };
    peer_with(id, displays, capabilities)
}

fn peer_with(id: &str, displays: Vec<DisplayInfo>, capabilities: InputCapabilities) -> Peer {
    peer_full(id, displays, capabilities, true)
}

fn peer_full(
    id: &str,
    displays: Vec<DisplayInfo>,
    capabilities: InputCapabilities,
    holder: bool,
) -> Peer {
    peer_config(id, displays, capabilities, holder, config())
}

fn peer_config(
    id: &str,
    displays: Vec<DisplayInfo>,
    capabilities: InputCapabilities,
    holder: bool,
    config: Config,
) -> Peer {
    let log: Shared = Rc::default();
    let focus_holder: Option<Box<dyn FocusHolder>> = if holder {
        Some(Box::new(Holder(log.clone())))
    } else {
        None
    };
    let ports = Ports {
        net: Box::new(Net(log.clone())),
        jobs: Box::new(Jobs(log.clone())),
        displays: Box::new(FakeDisplays(displays)),
        capture: Box::new(Capture(log.clone())),
        injector: Some(Box::new(Inject(log.clone()))),
        prompt: Box::new(Prompt(log.clone())),
        focus_holder,
        clipboard: Some(Box::new(Clip(log.clone()))),
        verifiers: Box::new(Verifiers(log.clone())),
        discovery: Box::new(FakeDiscovery(log.clone())),
        capabilities,
    };
    let labels = Labels {
        confirm_key: "Return".into(),
        hand_back_shortcut: "Ctrl+Opt+Cmd+Esc".into(),
    };
    let controller = Controller::new(
        config,
        ComputerId::from(id),
        std::env::temp_dir(),
        labels,
        "sha256:00".into(),
        1000,
        ports,
    );
    Peer {
        id: ComputerId::from(id),
        controller,
        log,
    }
}

/// Two different peers of the slice.
fn pair<'a>(peers: &'a mut [&mut Peer], i: usize, j: usize) -> (&'a mut Peer, &'a mut Peer) {
    assert_ne!(i, j, "a peer sends to itself");
    if i < j {
        let (left, right) = peers.split_at_mut(j);
        (&mut *left[i], &mut *right[0])
    } else {
        let (left, right) = peers.split_at_mut(i);
        (&mut *right[0], &mut *left[j])
    }
}

struct World {
    mac: Peer,
    pc1: Peer,
    /// More peers for FLOW-24.
    extra: Vec<Peer>,
    now: Instant,
    /// The result of each handoff job.
    handoff: HandoffResult,
}

impl World {
    fn new() -> Self {
        Self {
            mac: peer(
                "mac",
                vec![
                    display("builtin", -1500.0, 1500.0),
                    display("lg", 0.0, 3400.0),
                ],
            ),
            pc1: peer("pc1", vec![display("HDMI-A-2", 0.0, 2048.0)]),
            extra: Vec::new(),
            now: Instant::now(),
            handoff: HandoffResult::Done,
        }
    }

    /// Delivers network commands and completes jobs until nothing is left.
    fn pump(&mut self) {
        loop {
            let mut progress = false;
            let mut peers: Vec<&mut Peer> = [&mut self.mac, &mut self.pc1]
                .into_iter()
                .chain(self.extra.iter_mut())
                .collect();
            for i in 0..peers.len() {
                loop {
                    // Take the item first: the controller writes to the log while it runs.
                    let next = peers[i].log.borrow_mut().net.pop_front();
                    let Some(command) = next else { break };
                    progress = true;
                    let to = match &command {
                        NetCommand::Dial { peer, .. }
                        | NetCommand::Send { peer, .. }
                        | NetCommand::SendPointer { peer, .. }
                        | NetCommand::Close { peer } => peer.clone(),
                    };
                    let j = peers
                        .iter()
                        .position(|p| p.id == to)
                        .unwrap_or_else(|| panic!("no peer {to:?} in the world"));
                    let (a, b) = pair(&mut peers, i, j);
                    match command {
                        NetCommand::Dial { .. } => {
                            a.controller
                                .on_net(NetEvent::Connected { peer: b.id.clone() }, self.now);
                            b.controller
                                .on_net(NetEvent::Connected { peer: a.id.clone() }, self.now);
                        }
                        NetCommand::Send { message, .. } => {
                            b.controller.on_net(
                                NetEvent::Message {
                                    peer: a.id.clone(),
                                    message,
                                },
                                self.now,
                            );
                        }
                        NetCommand::SendPointer { datagram, .. } => {
                            b.controller.on_net(
                                NetEvent::Pointer {
                                    peer: a.id.clone(),
                                    datagram,
                                },
                                self.now,
                            );
                        }
                        NetCommand::Close { peer } => {
                            a.log.borrow_mut().closed.push(peer);
                            let reason = || "closed".to_string();
                            let (a_id, b_id) = (a.id.clone(), b.id.clone());
                            a.controller.on_net(
                                NetEvent::Disconnected {
                                    peer: b_id,
                                    reason: reason(),
                                },
                                self.now,
                            );
                            b.controller.on_net(
                                NetEvent::Disconnected {
                                    peer: a_id,
                                    reason: reason(),
                                },
                                self.now,
                            );
                        }
                    }
                }
                let a = &mut *peers[i];
                loop {
                    let next = a.log.borrow_mut().jobs.pop_front();
                    let Some((id, job)) = next else { break };
                    progress = true;
                    let output = match job {
                        Job::Operation {
                            kind: OperationKind::PortalMatch,
                            request,
                            ..
                        } => {
                            // Only the external display matches. "broken" fails.
                            let token = request.portal.unwrap().display_token.unwrap();
                            if token.0 == "broken" {
                                JobOutput::Operation(Err("no EDID".into()))
                            } else {
                                JobOutput::Operation(Ok(Outcome::Matched(
                                    token.0 == "lg" || token.0 == "HDMI-A-2",
                                )))
                            }
                        }
                        Job::Operation {
                            kind: OperationKind::InputTarget,
                            request,
                            ..
                        } => {
                            // The external display of the receiving computer.
                            let token = request
                                .displays
                                .iter()
                                .map(|d| d.token.clone())
                                .find(|t| t.0 == "lg" || t.0 == "HDMI-A-2")
                                .unwrap();
                            JobOutput::Operation(Ok(Outcome::Display(token)))
                        }
                        Job::Operation { .. } => JobOutput::Operation(Err("unexpected".into())),
                        Job::Handoff { .. } => JobOutput::Handoff {
                            result: self.handoff,
                            error: None,
                        },
                    };
                    a.controller.on_job_done(id, output);
                }
            }
            if !progress {
                return;
            }
        }
    }

    fn phase(&self) -> Option<SessionPhase> {
        Some(self.mac.controller.portal_screen()?.session?.phase)
    }

    /// ACT-5: the portal closed with its session, and a notice has the reason.
    fn assert_ended(&self, reason: &str) {
        assert!(!self.mac.controller.has_portal(), "ACT-5");
        assert!(
            has_notice(&self.mac, &format!("ended: {reason}")),
            "{:?}",
            self.mac.controller.control_screen().notices
        );
    }

    /// Completes the portal match and delivers no network command.
    fn pump_until_dial(&mut self) {
        let job = self.mac.log.borrow_mut().jobs.pop_front();
        let (id, _) = job.unwrap();
        self.mac
            .controller
            .on_job_done(id, JobOutput::Operation(Ok(Outcome::Matched(true))));
    }

    fn open_portal(&mut self) {
        self.mac.controller.open_portal(display("lg", 0.0, 3400.0));
        self.pump();
    }

    fn advance_to_confirmation(&mut self) {
        self.open_portal();
        self.mac.controller.start_handoff(ComputerId::from("pc1"));
        self.pump();
    }

    fn advance_to_forwarding(&mut self) {
        self.advance_to_confirmation();
        self.advance_to_forwarding_from_confirmation();
    }

    fn advance_to_forwarding_from_confirmation(&mut self) {
        assert!(self.mac.controller.acknowledge());
        self.mac
            .controller
            .on_captured(CapturedEvent::PointerEntered {
                at: Point { x: 10.0, y: 10.0 },
            });
        self.pump();
        assert_eq!(self.phase(), Some(SessionPhase::Forwarding));
    }
}

fn key(code: u16, down: bool) -> InputEvent {
    InputEvent::Key {
        key: Key(code),
        down,
    }
}

#[test]
fn in14_the_target_multiplies_a_scroll_by_its_speed() {
    let mut w = World::new();
    w.advance_to_forwarding();
    w.mac
        .controller
        .on_captured(CapturedEvent::Input(InputEvent::Scroll {
            dx: -2.0,
            dy: 8.0,
            phase: ScrollPhase::None,
        }));
    w.mac
        .controller
        .on_captured(CapturedEvent::Input(key(30, true)));
    w.pump();
    assert_eq!(
        w.pc1.log.borrow().injected,
        [
            InputEvent::Scroll {
                dx: -1.0,
                dy: 4.0,
                phase: ScrollPhase::None
            },
            key(30, true)
        ]
    );
}

#[test]
fn flow2_flow3_portal_match_and_candidates() {
    let mut w = World::new();
    w.open_portal();
    let screen = w.mac.controller.portal_screen().unwrap();
    assert!(matches!(screen.matching, Matching::Matched(ref d) if d.alias == "Desktop"));
    assert_eq!(screen.candidates.len(), 1, "pc2 has no route from the Mac");
    assert_eq!(screen.candidates[0].target, ComputerId::from("pc1"));
    assert_eq!(
        screen.candidates[0].unavailable, None,
        "connected after the dial"
    );
}

#[test]
fn flow3_no_handoff_without_suppress() {
    // Hardware test 2026-10-27: Input Monitoring without Accessibility gives
    // capture but no suppress. The handoff must not switch the display.
    let mut w = World::new();
    w.mac = peer_with(
        "mac",
        vec![
            display("builtin", -1500.0, 1500.0),
            display("lg", 0.0, 3400.0),
        ],
        InputCapabilities {
            capture: Capability::Available,
            suppress: Capability::Missing("grant Accessibility".into()),
            inject: Capability::Available,
        },
    );
    w.open_portal();
    let screen = w.mac.controller.portal_screen().unwrap();
    let reason = screen.candidates[0].unavailable.clone().unwrap_or_default();
    assert!(reason.contains("suppress"), "unavailable: {reason:?}");
    w.mac.controller.start_handoff(ComputerId::from("pc1"));
    w.pump();
    assert_eq!(w.phase(), None, "no session starts");
}

#[test]
fn flow2_ac9_no_match_on_other_display() {
    let mut w = World::new();
    w.mac
        .controller
        .open_portal(display("builtin", -1500.0, 1500.0));
    w.pump();
    let screen = w.mac.controller.portal_screen().unwrap();
    assert_eq!(screen.matching, Matching::NoMatch);
    assert!(screen.candidates.is_empty());
}

#[test]
fn flow7_ac3_prompt_with_hint_on_target() {
    let mut w = World::new();
    w.advance_to_confirmation();
    let log = w.pc1.log.borrow();
    // The prompt verifier shows the hint, then asks for an acknowledgement (VER-5).
    assert_eq!(
        log.prompts,
        [
            " | Copy: Ctrl+C",
            "Press Return on MacBook to control this computer | Copy: Ctrl+C"
        ]
    );
    assert!(log.prompt_visible);
    drop(log);
    assert_eq!(
        w.phase(),
        Some(SessionPhase::WaitingForAck {
            key: "Return".into()
        })
    );
    let video = w
        .mac
        .controller
        .portal_screen()
        .unwrap()
        .session
        .unwrap()
        .video_input;
    assert_eq!(video, VideoInput::Unverified);
    assert_eq!(
        w.mac.log.borrow().capture,
        ["arm 3400", "focus true", "guard on"],
        "no forwarding before the confirmation; only the guard (FLOW-21)"
    );
}

#[test]
fn ac6_confirmation_needs_prompt_and_focus() {
    let mut w = World::new();
    w.open_portal();
    assert!(!w.mac.controller.acknowledge(), "no session: a normal key");
    w.mac.controller.start_handoff(ComputerId::from("pc1"));
    w.pump();
    w.mac.controller.set_portal_focus(false);
    assert!(!w.mac.controller.acknowledge(), "no focus: a normal key");
    w.mac.controller.set_portal_focus(true);
    assert!(w.mac.controller.acknowledge());
}

#[test]
fn flow10_ac5_forwarding_after_confirmation() {
    let mut w = World::new();
    w.advance_to_forwarding();
    assert!(!w.pc1.log.borrow().prompt_visible, "the prompt goes away");
    assert_eq!(
        w.mac.log.borrow().capture,
        // FLOW-22, FLOW-26: forwarding starts from the native portal state.
        [
            "arm 3400",
            "focus true",
            "guard on",
            "guard off",
            "forward",
            "focus true",
            "visible"
        ]
    );
    let video = w
        .mac
        .controller
        .portal_screen()
        .unwrap()
        .session
        .unwrap()
        .video_input;
    assert_eq!(video, VideoInput::Confirmed);

    w.mac
        .controller
        .on_captured(CapturedEvent::Input(key(30, true)));
    w.mac.controller.on_captured(CapturedEvent::PointerMoved {
        at: Point {
            x: 1699.0,
            y: 500.0,
        },
    });
    w.pump();
    let log = w.pc1.log.borrow();
    assert_eq!(log.injected, [key(30, true)]);
    let last = *log.pointer.last().unwrap();
    assert!((last.x - 1699.0 / 3399.0).abs() < 1e-4, "{last:?}");
}

#[test]
fn net2_m10_keys_with_no_motion_send_no_pointer_datagram() {
    let mut w = World::new();
    w.advance_to_forwarding();
    let sent = w.pc1.log.borrow().pointer.len();
    assert!(sent > 0, "forwarding starts with a pointer datagram");
    // The macOS capture reports the cursor position with each key event.
    let at = Point { x: 10.0, y: 10.0 };
    for down in [true, false] {
        w.mac
            .controller
            .on_captured(CapturedEvent::Input(key(56, down)));
        w.mac
            .controller
            .on_captured(CapturedEvent::PointerMoved { at });
    }
    w.pump();
    assert_eq!(w.pc1.log.borrow().injected, [key(56, true), key(56, false)]);
    assert_eq!(w.pc1.log.borrow().pointer.len(), sent);

    // The pointer enters the portal again at the same position.
    w.mac.controller.on_captured(CapturedEvent::PointerLeft);
    w.mac
        .controller
        .on_captured(CapturedEvent::PointerEntered { at });
    w.pump();
    assert_eq!(w.pc1.log.borrow().pointer.len(), sent + 1);
}

#[test]
fn net9_the_target_keeps_pointer_statistics_after_the_session() {
    let mut w = World::new();
    assert!(w.pc1.controller.pointer_stats().is_none());
    w.advance_to_forwarding();
    for x in [10.0, 20.0, 30.0] {
        w.mac.controller.on_captured(CapturedEvent::PointerMoved {
            at: Point { x, y: 10.0 },
        });
    }
    w.pump();
    let session = w.pc1.controller.control_screen().incoming[0].session;
    let sent = w.pc1.log.borrow().pointer.len() as u64;
    let stats = w.pc1.controller.pointer_stats().unwrap();
    assert_eq!(stats.session, session);
    assert_eq!(stats.received, sent);
    assert_eq!(stats.lost, 0);
    assert!(
        w.mac.controller.pointer_stats().is_none(),
        "the source has no incoming session"
    );

    w.pc1.controller.on_intent(Intent::StopSession { session });
    w.pump();
    let stats = w.pc1.controller.pointer_stats().unwrap();
    assert_eq!(stats.session, session, "the last session");
    assert_eq!(stats.received, sent);
}

#[test]
fn net2_the_target_injects_only_the_newest_waiting_position() {
    let mut w = World::new();
    w.advance_to_forwarding();
    let session = w.pc1.controller.control_screen().incoming[0].session;
    let injected = w.pc1.log.borrow().pointer.len();
    let before = w.pc1.controller.pointer_stats().unwrap();

    let start = w.now;
    let pointers = (0..3u64)
        .map(|i| ReceivedPointer {
            peer: ComputerId::from("mac"),
            datagram: PointerDatagram {
                session,
                seq: 1000 + i,
                sent_us: 1_000_000 + i * 8000,
                position: Normalized {
                    x: 0.1 * (i + 1) as f32,
                    y: 0.5,
                },
            },
            received: start + Duration::from_millis(8 * i),
        })
        .collect();
    w.pc1
        .controller
        .on_pointers(pointers, start + Duration::from_millis(30));

    let log = w.pc1.log.borrow();
    assert_eq!(
        log.pointer.len(),
        injected + 1,
        "one injection for the batch"
    );
    let last = *log.pointer.last().unwrap();
    assert!((last.x - 0.3).abs() < 1e-4, "the newest position: {last:?}");
    let stats = w.pc1.controller.pointer_stats().unwrap();
    assert_eq!(stats.received, before.received + 3);
    assert_eq!(stats.used, before.used + 3);
    assert_eq!(stats.injected, before.injected + 1);
}

#[test]
fn net2_a_batch_from_another_peer_is_ignored() {
    let mut w = World::new();
    w.advance_to_forwarding();
    let session = w.pc1.controller.control_screen().incoming[0].session;
    let injected = w.pc1.log.borrow().pointer.len();
    let before = w.pc1.controller.pointer_stats().unwrap();
    let pointer = ReceivedPointer {
        peer: ComputerId::from("pc4"),
        datagram: PointerDatagram {
            session,
            seq: 2000,
            sent_us: 0,
            position: Normalized { x: 0.9, y: 0.9 },
        },
        received: w.now,
    };
    w.pc1.controller.on_pointers(vec![pointer], w.now);
    assert_eq!(w.pc1.log.borrow().pointer.len(), injected);
    assert_eq!(w.pc1.controller.pointer_stats().unwrap(), before);
}

#[test]
fn flow14_ac11_leaving_the_portal_keeps_forwarding() {
    let mut w = World::new();
    w.advance_to_forwarding();
    w.mac
        .controller
        .on_captured(CapturedEvent::Input(key(42, true)));
    // The free cursor leaves the portal (FLOW-20). There is no pause state.
    w.mac.controller.on_captured(CapturedEvent::PointerLeft);
    w.pump();
    assert_eq!(w.phase(), Some(SessionPhase::Forwarding));
    assert!(!w.mac.log.borrow().capture.contains(&"disarm".to_string()));
    // The keyboard is still in the portal, so the capture forwards the up (IN-3).
    w.mac
        .controller
        .on_captured(CapturedEvent::Input(key(42, false)));
    w.pump();
    assert_eq!(w.pc1.log.borrow().injected, [key(42, true), key(42, false)]);

    w.mac.controller.on_captured(CapturedEvent::PointerEntered {
        at: Point { x: 0.0, y: 300.0 },
    });
    w.pump();
    assert_eq!(w.phase(), Some(SessionPhase::Forwarding));
    let last = *w.pc1.log.borrow().pointer.last().unwrap();
    assert_eq!(
        (last.x, (last.y - 300.0 / 999.0).abs() < 1e-4),
        (0.0, true),
        "the entry point goes to the target: {last:?}"
    );
}

#[test]
fn flow23_ac27_one_visible_keyboard_focus() {
    let mut w = World::new();
    w.advance_to_forwarding();
    assert!(
        w.pc1.log.borrow().target.is_empty(),
        "no holder at the start"
    );

    // A click on a different window of the source.
    w.mac.controller.on_captured(CapturedEvent::KeyboardLeft);
    w.pump();
    assert_eq!(w.pc1.log.borrow().target, ["hold HDMI-A-2"]);

    // A click in the portal: the keyboard comes back before the click.
    let click = InputEvent::Button {
        button: telescreen_core::input::Button::Left,
        down: true,
    };
    w.mac.controller.on_captured(CapturedEvent::KeyboardEntered);
    w.mac.controller.on_captured(CapturedEvent::Input(click));
    w.pump();
    assert_eq!(
        w.pc1.log.borrow().target,
        [
            "hold HDMI-A-2".to_string(),
            "release".into(),
            format!("inject {click:?}")
        ]
    );
    assert_eq!(w.phase(), Some(SessionPhase::Forwarding));
}

#[test]
fn flow22_a_click_in_the_portal_asks_for_the_native_focus() {
    let mut w = World::new();
    w.advance_to_forwarding();
    w.mac.controller.set_portal_focus(false);
    assert!(!w.mac.controller.take_portal_focus_request());

    // IN-17: macOS suppresses the click, so the window layer asks for the focus.
    w.mac.controller.on_captured(CapturedEvent::FocusWanted);
    assert!(w.mac.controller.take_portal_focus_request());
    assert!(!w.mac.controller.take_portal_focus_request(), "once");

    // The native focus comes before the window layer asks: no request.
    w.mac.controller.on_captured(CapturedEvent::FocusWanted);
    w.mac.controller.set_portal_focus(true);
    assert!(!w.mac.controller.take_portal_focus_request());
}

#[test]
fn flow22_act5_the_next_session_has_a_new_portal() {
    let mut w = World::new();
    w.advance_to_forwarding();
    w.mac.controller.on_captured(CapturedEvent::SafetyExit);
    w.pump();
    assert!(!w.mac.controller.has_portal(), "ACT-5");
    w.mac.log.borrow_mut().capture.clear();
    w.advance_to_confirmation();
    // The new portal window gets the native focus (FLOW-22).
    w.mac.controller.set_portal_focus(true);
    w.advance_to_forwarding_from_confirmation();
    let capture = w.mac.log.borrow().capture.clone();
    assert!(capture.contains(&"focus true".to_string()), "{capture:?}");
}

#[test]
fn flow26_ac51_the_target_warns_while_the_portal_is_hidden() {
    let mut w = World::new();
    w.advance_to_forwarding();
    let shown = w.pc1.log.borrow().prompts.len();

    // Mission Control opens on the Mac.
    w.mac.controller.set_portal_visible(false);
    w.pump();
    // The portal keeps the native focus, but the keys stay on the Mac.
    assert!(
        w.mac
            .log
            .borrow()
            .capture
            .ends_with(&["hidden".into(), "focus false".into()])
    );
    w.mac.controller.set_portal_focus(true);
    assert_eq!(w.mac.log.borrow().capture.last().unwrap(), "focus false");
    assert!(w.mac.controller.portal_hidden());
    {
        let log = w.pc1.log.borrow();
        assert_eq!(
            log.prompts[shown..],
            ["MacBook does not show its portal | Input is paused"]
        );
        assert!(log.prompt_visible);
    }
    w.mac.controller.set_portal_visible(false);
    w.pump();
    assert_eq!(w.pc1.log.borrow().prompts.len(), shown + 1, "no change");

    w.mac.controller.set_portal_visible(true);
    w.pump();
    assert!(
        w.mac
            .log
            .borrow()
            .capture
            .ends_with(&["visible".into(), "focus true".into()])
    );
    assert!(!w.mac.controller.portal_hidden());
    assert!(!w.pc1.log.borrow().prompt_visible);
}

#[test]
fn flow26_the_warning_shows_the_cursor_of_the_source() {
    let mut w = World::new();
    w.advance_to_forwarding();
    w.mac.controller.set_portal_visible(false);
    w.pump();
    let datagrams = w.pc1.log.borrow().pointer.len();
    let moved = |w: &mut World, x: f64| {
        w.mac.controller.on_captured(CapturedEvent::PointerMoved {
            at: Point { x, y: 500.0 },
        });
        w.pump();
    };
    moved(&mut w, 1699.0);
    {
        let log = w.pc1.log.borrow();
        let at = log.prompt_cursor.last().unwrap().unwrap();
        assert!((at.x - 1699.0 / 3399.0).abs() < 1e-4, "{at:?}");
        assert_eq!(log.pointer.len(), datagrams, "the target pointer stays");
    }
    moved(&mut w, 1699.0);
    assert_eq!(w.pc1.log.borrow().prompt_cursor.len(), 1, "no change");
    // Out of the portal area: the cursor image goes away.
    moved(&mut w, -100.0);
    moved(&mut w, -200.0);
    assert_eq!(w.pc1.log.borrow().prompt_cursor[1..], [None]);

    moved(&mut w, 100.0);
    w.mac.controller.set_portal_visible(true);
    w.pump();
    assert_eq!(*w.pc1.log.borrow().prompt_cursor.last().unwrap(), None);
    let count = w.pc1.log.borrow().prompt_cursor.len();
    moved(&mut w, 200.0);
    assert_eq!(w.pc1.log.borrow().prompt_cursor.len(), count);
    assert!(
        w.pc1.log.borrow().pointer.len() > datagrams,
        "forwarding again"
    );
}

#[test]
fn flow26_the_warning_goes_away_with_the_session() {
    let mut w = World::new();
    w.advance_to_forwarding();
    w.mac.controller.set_portal_visible(false);
    w.pump();
    assert!(w.pc1.log.borrow().prompt_visible);
    w.mac.controller.on_captured(CapturedEvent::SafetyExit);
    w.pump();
    assert!(!w.pc1.log.borrow().prompt_visible);
}

#[test]
fn flow26_a_portal_hidden_before_forwarding_warns_at_the_start() {
    let mut w = World::new();
    w.advance_to_confirmation();
    w.mac.controller.set_portal_visible(false);
    w.pump();
    assert!(!w.mac.controller.portal_hidden(), "not forwarding yet");
    w.mac.controller.set_portal_visible(true);
    w.pump();
    w.mac.controller.set_portal_visible(false);
    w.advance_to_forwarding_from_confirmation();
    assert!(w.mac.controller.portal_hidden());
    assert!(
        w.pc1
            .log
            .borrow()
            .prompts
            .iter()
            .any(|p| p.starts_with("MacBook does not show its portal"))
    );
}

#[test]
fn flow23_session_end_removes_the_holder() {
    let mut w = World::new();
    w.advance_to_forwarding();
    w.mac.controller.on_captured(CapturedEvent::KeyboardLeft);
    w.mac
        .controller
        .on_captured(CapturedEvent::HandBackShortcut);
    w.pump();
    // The holder goes before the route back runs (HB-1).
    assert_eq!(
        w.pc1.log.borrow().target,
        ["hold HDMI-A-2", "release", "handoff /bin/to-mac"]
    );
}

#[test]
fn in13_ac28_input_lost_releases_on_the_target() {
    let mut w = World::new();
    w.advance_to_forwarding();
    w.mac
        .controller
        .on_captured(CapturedEvent::Input(key(42, true)));
    w.mac.controller.on_captured(CapturedEvent::InputLost);
    w.pump();
    assert_eq!(w.pc1.log.borrow().injected, [key(42, true), key(42, false)]);
    assert_eq!(w.phase(), Some(SessionPhase::Forwarding));
}

#[test]
fn ac29_target_without_focus_holder_refuses() {
    let caps = InputCapabilities {
        capture: Capability::Available,
        suppress: Capability::Available,
        inject: Capability::Available,
    };
    let mut pc1 = peer_full("pc1", vec![display("HDMI-A-2", 0.0, 2048.0)], caps, false);
    pc1.controller.on_net(
        NetEvent::Message {
            peer: ComputerId::from("mac"),
            message: Message::RequestSession {
                device: "shared-lg".into(),
            },
        },
        Instant::now(),
    );
    let sent = pc1.log.borrow_mut().net.pop_front().unwrap();
    assert!(
        matches!(
            sent,
            NetCommand::Send {
                message: Message::SessionRefused {
                    reason: RefuseReason::NoFocusHold,
                    ..
                },
                ..
            }
        ),
        "{sent:?}"
    );
}

#[test]
fn flow16_in8_the_safety_exit_ends_both_sides() {
    let mut w = World::new();
    w.advance_to_forwarding();
    w.mac
        .controller
        .on_captured(CapturedEvent::Input(key(29, true)));
    w.mac.controller.on_captured(CapturedEvent::SafetyExit);
    w.pump();
    w.assert_ended("safety exit");
    assert_eq!(w.mac.log.borrow().capture.last().unwrap(), "disarm");
    assert_eq!(w.pc1.log.borrow().injected.last(), Some(&key(29, false)));
    assert!(
        w.pc1.controller.control_screen().incoming.is_empty(),
        "the lease is released"
    );
    // A new session can start in a new portal: the lease is free.
    w.advance_to_confirmation();
    assert!(matches!(
        w.phase(),
        Some(SessionPhase::WaitingForAck { .. })
    ));
}

#[test]
fn flow11_ac7_timeout_removes_prompt() {
    let mut w = World::new();
    w.advance_to_confirmation();
    w.pc1
        .controller
        .tick(w.now + std::time::Duration::from_secs(61));
    w.pump();
    assert!(!w.pc1.log.borrow().prompt_visible);
    assert!(!w.mac.controller.has_portal(), "ACT-5");
    assert!(has_notice(&w.mac, "The display state is unknown."));
}

#[test]
fn flow21_ac26_guard_from_handoff_to_confirmation() {
    let mut w = World::new();
    w.advance_to_confirmation();
    assert_eq!(
        w.mac.log.borrow().capture,
        ["arm 3400", "focus true", "guard on"]
    );
    assert!(w.mac.controller.guarded());
}

#[test]
fn flow21_guard_ends_with_the_timeout() {
    let mut w = World::new();
    w.advance_to_confirmation();
    w.pc1
        .controller
        .tick(w.now + std::time::Duration::from_secs(61));
    w.pump();
    assert!(!w.mac.controller.guarded());
    assert!(
        w.mac
            .log
            .borrow()
            .capture
            .iter()
            .any(|c| c == "guard off" || c == "disarm"),
        "{:?}",
        w.mac.log.borrow().capture
    );
}

#[test]
fn flow21_no_guard_after_a_failed_handoff() {
    let mut w = World::new();
    w.handoff = HandoffResult::Failed { completed_steps: 0 };
    w.advance_to_confirmation();
    assert!(!w.mac.controller.guarded());
}

#[test]
fn flow12_ac10_failed_handoff_does_not_reach_the_prompt() {
    let mut w = World::new();
    w.handoff = HandoffResult::Failed { completed_steps: 0 };
    w.advance_to_confirmation();
    assert!(w.pc1.log.borrow().prompts.is_empty());
    assert!(
        w.pc1.controller.control_screen().incoming.is_empty(),
        "the target released the lease"
    );
    w.assert_ended("the handoff failed");
}

#[test]
fn ac23_stop_this_session() {
    let mut w = World::new();
    w.advance_to_forwarding();
    w.mac
        .controller
        .on_captured(CapturedEvent::Input(key(56, true)));
    w.pump();
    let session = w.pc1.controller.control_screen().incoming[0].session;
    w.pc1.controller.on_intent(Intent::StopSession { session });
    w.pump();
    assert_eq!(w.pc1.log.borrow().injected.last(), Some(&key(56, false)));
    w.assert_ended("the target stopped the session");
    assert_eq!(w.mac.log.borrow().capture.last().unwrap(), "disarm");
    assert!(
        w.pc1.controller.portal_screen().is_none(),
        "no reverse session"
    );
}

#[test]
fn flow17_ac23_control_from_here_starts_the_reverse_session() {
    let mut w = World::new();
    w.advance_to_forwarding();
    w.mac
        .controller
        .on_captured(CapturedEvent::Input(key(56, true)));
    w.pump();
    let session = w.pc1.controller.control_screen().incoming[0].session;
    w.pc1
        .controller
        .on_intent(Intent::ControlFromHere { session });
    w.pump();
    assert_eq!(w.pc1.log.borrow().injected.last(), Some(&key(56, false)));
    // FLOW-24: the Mac grants the new session and closes its portal on the display.
    assert!(!w.mac.controller.has_portal());
    assert_eq!(w.mac.log.borrow().capture.last().unwrap(), "disarm");
    // pc1 is now the source: a portal on the display of the old session, and a
    // handoff to the old source that waits for the confirmation.
    let portal = w.pc1.controller.portal_screen().unwrap();
    assert_eq!(
        portal.session.unwrap().phase,
        SessionPhase::WaitingForAck {
            key: "Return".into()
        }
    );
    assert_eq!(w.mac.controller.control_screen().incoming.len(), 1);
}

#[test]
fn flow6_reverse_handoff_ends_the_old_session() {
    // Hardware test 2026-10-27: Mac to Linux forwarded, then a click in the Linux
    // portal started Linux to Mac. Both sessions forwarded, and the input looped.
    let mut w = World::new();
    w.advance_to_forwarding();
    w.mac
        .controller
        .on_captured(CapturedEvent::Input(key(56, true)));
    w.pump();
    w.pc1
        .controller
        .open_portal(display("HDMI-A-2", 0.0, 2048.0));
    w.pump();
    w.pc1.controller.start_handoff(ComputerId::from("mac"));
    w.pump();
    assert!(!w.mac.controller.has_portal(), "FLOW-24");
    assert!(
        !w.mac
            .controller
            .control_screen()
            .notices
            .iter()
            .any(|n| n.contains("took this display")),
        "with two peers, the old session ended before the request (FLOW-24)"
    );
    assert!(
        w.mac.log.borrow().capture.contains(&"disarm".to_string()),
        "the Mac stops capture"
    );
    assert_eq!(
        w.pc1.log.borrow().injected.last(),
        Some(&key(56, false)),
        "Linux releases the held key"
    );
    assert!(w.pc1.controller.control_screen().incoming.is_empty());
    assert_eq!(w.mac.controller.control_screen().incoming.len(), 1);
}

#[test]
fn ac20_second_source_gets_busy() {
    let mut w = World::new();
    w.advance_to_forwarding();
    let pc3 = ComputerId::from("pc3");
    w.pc1.controller.on_net(
        NetEvent::Message {
            peer: pc3.clone(),
            message: Message::RequestSession {
                device: DeviceId::from("shared-lg"),
            },
        },
        w.now,
    );
    let refused = w.pc1.log.borrow().net.iter().any(|c| {
        matches!(c, NetCommand::Send { peer, message: Message::SessionRefused { reason: RefuseReason::Busy, .. } } if *peer == pc3)
    });
    assert!(refused, "pc3 gets Busy");
    assert_eq!(
        w.phase(),
        Some(SessionPhase::Forwarding),
        "the Mac keeps its session"
    );
}

#[test]
fn ac18_source_exit_ends_the_session_and_closes() {
    let mut w = World::new();
    w.advance_to_forwarding();
    w.mac
        .controller
        .on_captured(CapturedEvent::Input(key(56, true)));
    w.pump();
    w.mac.controller.shutdown();
    w.mac.controller.shutdown();
    w.pump();
    assert_eq!(w.pc1.log.borrow().injected.last(), Some(&key(56, false)));
    assert!(w.pc1.controller.control_screen().incoming.is_empty());
    assert_eq!(w.mac.log.borrow().capture.last().unwrap(), "disarm");
    assert_eq!(w.mac.log.borrow().closed, vec![ComputerId::from("pc1")]);
    assert_eq!(
        w.mac.log.borrow().net_shutdowns,
        1,
        "one transport shutdown"
    );
}

#[test]
fn ac18_target_exit_ends_the_session_and_closes() {
    let mut w = World::new();
    w.advance_to_forwarding();
    w.mac
        .controller
        .on_captured(CapturedEvent::Input(key(56, true)));
    w.pump();
    w.pc1.controller.shutdown();
    w.pump();
    assert_eq!(w.pc1.log.borrow().injected.last(), Some(&key(56, false)));
    w.assert_ended("disconnected on the target");
    assert_eq!(w.mac.log.borrow().capture.last().unwrap(), "disarm");
    assert_eq!(w.pc1.log.borrow().closed, vec![ComputerId::from("mac")]);
    assert_eq!(w.pc1.log.borrow().net_shutdowns, 1);
}

#[test]
fn ac18_peer_lost_releases_both_sides() {
    let mut w = World::new();
    w.advance_to_forwarding();
    w.mac
        .controller
        .on_captured(CapturedEvent::Input(key(30, true)));
    w.pump();
    w.pc1.controller.on_net(
        NetEvent::Disconnected {
            peer: ComputerId::from("mac"),
            reason: "timeout".into(),
        },
        w.now,
    );
    w.mac.controller.on_net(
        NetEvent::Disconnected {
            peer: ComputerId::from("pc1"),
            reason: "timeout".into(),
        },
        w.now,
    );
    assert_eq!(w.pc1.log.borrow().injected.last(), Some(&key(30, false)));
    assert_eq!(w.mac.log.borrow().capture.last().unwrap(), "disarm");
    w.assert_ended("connection lost");
}

#[test]
fn flow19_ac19_portal_display_gone() {
    let mut w = World::new();
    w.advance_to_forwarding();
    w.mac.controller.close_portal(EndReason::PortalGone);
    w.pump();
    assert!(w.mac.controller.portal_screen().is_none());
    assert_eq!(w.mac.log.borrow().capture.last().unwrap(), "disarm");
    assert!(w.pc1.controller.control_screen().incoming.is_empty());
}

#[test]
fn net4_session_without_route_is_refused() {
    // pc2 has no handoff route to pc1, so pc1 refuses its request.
    let mut pc1 = peer("pc1", vec![display("HDMI-A-2", 0.0, 2048.0)]);
    pc1.controller.on_net(
        NetEvent::Message {
            peer: ComputerId::from("pc2"),
            message: telescreen_core::wire::Message::RequestSession {
                device: "shared-lg".into(),
            },
        },
        Instant::now(),
    );
    let sent = pc1.log.borrow_mut().net.pop_front().unwrap();
    assert!(matches!(
        sent,
        NetCommand::Send {
            message: telescreen_core::wire::Message::SessionRefused {
                reason: telescreen_core::wire::RefuseReason::NoRoute,
                ..
            },
            ..
        }
    ));
}

#[test]
fn flow4_ac8_portal_moves_to_other_display() {
    let mut w = World::new();
    w.advance_to_forwarding();
    w.mac
        .controller
        .portal_display_changed(display("builtin", -1500.0, 1500.0));
    w.pump();
    // The old device binding does not apply on the new display, so the
    // session ends, and the portal closes with it (ACT-5).
    w.assert_ended("the portal display is gone");
    assert_eq!(w.mac.log.borrow().capture.last().unwrap(), "disarm");
    assert!(w.pc1.controller.control_screen().incoming.is_empty());
}

#[test]
fn adr0007_controller_sends_the_heartbeat() {
    let mut w = World::new();
    w.open_portal();
    let t0 = w.now;
    w.mac.controller.tick(t0);
    let sent = |w: &World| {
        w.mac
            .log
            .borrow()
            .net
            .iter()
            .filter(|c| {
                matches!(
                    c,
                    NetCommand::Send {
                        message: telescreen_core::wire::Message::Heartbeat,
                        ..
                    }
                )
            })
            .count()
    };
    assert_eq!(sent(&w), 1);
    w.mac
        .controller
        .tick(t0 + std::time::Duration::from_millis(500));
    assert_eq!(sent(&w), 1, "once a second");
    w.mac
        .controller
        .tick(t0 + std::time::Duration::from_millis(1000));
    assert_eq!(sent(&w), 2);
}

/// The user copies on this peer.
fn copy(peer: &Peer, text: &str, concealed: bool) {
    let mut log = peer.log.borrow_mut();
    log.clip_own = false;
    log.clip_pending.push(ClipboardChange::Text(ClipboardItem {
        text: text.into(),
        concealed,
    }));
}

/// A password manager on this peer clears its clipboard.
fn clear(peer: &Peer) {
    let mut log = peer.log.borrow_mut();
    log.clip_own = false;
    log.clip_pending.push(ClipboardChange::Cleared);
}

/// Runs both peers once and delivers the messages.
fn settle(w: &mut World) {
    w.mac.controller.tick(w.now);
    w.pump();
    w.pc1.controller.tick(w.now);
    w.pump();
}

#[test]
fn clip1_the_control_window_shows_the_switch() {
    let mut w = World::new();
    assert_eq!(w.mac.controller.control_screen().clipboard, Some(true));
    w.mac
        .controller
        .on_intent(Intent::SetClipboard { on: false });
    assert_eq!(w.mac.controller.control_screen().clipboard, Some(false));
}

#[test]
fn clip2_ac30_text_goes_in_both_directions() {
    let mut w = World::new();
    w.advance_to_forwarding();
    copy(&w.mac, "hello", false);
    w.mac.controller.tick(w.now);
    w.pump();
    assert_eq!(w.pc1.log.borrow().clip_set, [("hello".into(), false)]);
    copy(&w.pc1, "back", false);
    w.pc1.controller.tick(w.now);
    w.pump();
    assert_eq!(w.mac.log.borrow().clip_set, [("back".into(), false)]);
}

#[test]
fn clip3_ac31_text_from_before_the_session_is_not_sent() {
    let mut w = World::new();
    copy(&w.mac, "old", false);
    w.advance_to_forwarding();
    w.mac.controller.tick(w.now);
    w.pump();
    assert!(w.pc1.log.borrow().clip_set.is_empty());
}

#[test]
fn clip2_ac32_no_sync_with_a_switch_off() {
    let mut w = World::new();
    w.advance_to_forwarding();
    w.mac
        .controller
        .on_intent(Intent::SetClipboard { on: false });
    copy(&w.mac, "a", false);
    w.mac.controller.tick(w.now);
    w.pump();
    assert!(w.pc1.log.borrow().clip_set.is_empty(), "the sender is off");

    w.mac
        .controller
        .on_intent(Intent::SetClipboard { on: true });
    w.pc1
        .controller
        .on_intent(Intent::SetClipboard { on: false });
    copy(&w.mac, "b", false);
    w.mac.controller.tick(w.now);
    w.pump();
    assert!(
        w.pc1.log.borrow().clip_set.is_empty(),
        "the receiver is off"
    );
}

#[test]
fn clip5_ac32_no_sync_after_the_session_end() {
    let mut w = World::new();
    w.advance_to_forwarding();
    w.mac.controller.on_captured(CapturedEvent::SafetyExit);
    w.pump();
    copy(&w.mac, "late", false);
    w.mac.controller.tick(w.now);
    w.pump();
    assert!(w.pc1.log.borrow().clip_set.is_empty());
}

#[test]
fn clip4_ac33_no_echo() {
    let mut w = World::new();
    w.advance_to_forwarding();
    copy(&w.mac, "hello", false);
    w.mac.controller.tick(w.now);
    w.pump();
    w.pc1.controller.tick(w.now);
    w.pump();
    assert!(w.mac.log.borrow().clip_set.is_empty(), "pc1 does not echo");
}

#[test]
fn clip7_ac34_a_concealed_text_is_sent_with_the_mark() {
    let mut w = World::new();
    w.advance_to_forwarding();
    copy(&w.mac, "hunter2", true);
    settle(&mut w);
    assert_eq!(w.pc1.log.borrow().clip_set, [("hunter2".into(), true)]);
}

#[test]
fn clip7_ac34_the_other_peer_clears_when_the_source_clears() {
    let mut w = World::new();
    w.advance_to_forwarding();
    copy(&w.mac, "hunter2", true);
    settle(&mut w);
    clear(&w.mac);
    settle(&mut w);
    assert_eq!(w.pc1.log.borrow().clip_calls, ["open", "clear"]);
    assert!(w.pc1.log.borrow().clip_open, "the session continues");
}

#[test]
fn clip7_a_copy_by_the_user_is_not_cleared() {
    let mut w = World::new();
    w.advance_to_forwarding();
    copy(&w.mac, "hunter2", true);
    settle(&mut w);
    copy(&w.pc1, "mine", false);
    clear(&w.mac);
    settle(&mut w);
    assert!(!w.pc1.log.borrow().clip_calls.contains(&"clear"));
}

#[test]
fn clip7_no_clear_for_a_text_without_the_mark() {
    let mut w = World::new();
    w.advance_to_forwarding();
    copy(&w.mac, "a", false);
    settle(&mut w);
    clear(&w.mac);
    settle(&mut w);
    assert!(!w.pc1.log.borrow().clip_calls.contains(&"clear"));
}

#[test]
fn clip11_ac35_the_session_end_clears_the_received_text() {
    for end in ["safety exit", "stop on the target", "switch off"] {
        let mut w = World::new();
        w.advance_to_forwarding();
        copy(&w.mac, "a", false);
        settle(&mut w);
        match end {
            "safety exit" => w.mac.controller.on_captured(CapturedEvent::SafetyExit),
            "stop on the target" => {
                let session = w.pc1.controller.control_screen().incoming[0].session;
                w.pc1.controller.on_intent(Intent::StopSession { session });
            }
            _ => w
                .pc1
                .controller
                .on_intent(Intent::SetClipboard { on: false }),
        }
        settle(&mut w);
        assert_eq!(
            w.pc1.log.borrow().clip_calls,
            ["open", "clear", "close"],
            "{end}"
        );
        assert!(!w.pc1.log.borrow().clip_open, "{end}");
    }
}

#[test]
fn clip11_the_source_also_clears_and_closes() {
    let mut w = World::new();
    w.advance_to_forwarding();
    copy(&w.pc1, "from pc1", false);
    settle(&mut w);
    assert_eq!(w.mac.log.borrow().clip_set, [("from pc1".into(), false)]);
    w.mac.controller.on_intent(Intent::EndSession);
    settle(&mut w);
    assert_eq!(w.mac.log.borrow().clip_calls, ["open", "clear", "close"]);
}

#[test]
fn clip11_ac35_a_copy_by_the_user_stays_at_the_end() {
    let mut w = World::new();
    w.advance_to_forwarding();
    copy(&w.mac, "a", false);
    settle(&mut w);
    copy(&w.pc1, "mine", false);
    w.mac.controller.on_captured(CapturedEvent::SafetyExit);
    settle(&mut w);
    assert_eq!(w.pc1.log.borrow().clip_calls, ["open", "close"]);
}

#[test]
fn clip11_shutdown_clears_with_no_tick() {
    let mut w = World::new();
    w.advance_to_forwarding();
    copy(&w.mac, "a", false);
    settle(&mut w);
    w.pc1.controller.shutdown();
    assert_eq!(w.pc1.log.borrow().clip_calls, ["open", "clear", "close"]);
}

#[test]
fn clip2_a_text_that_arrives_before_a_tick_is_applied() {
    let mut w = World::new();
    w.advance_to_forwarding();
    // pc1 did not tick since forwarding started.
    copy(&w.mac, "early", false);
    w.mac.controller.tick(w.now);
    w.pump();
    assert_eq!(w.pc1.log.borrow().clip_set, [("early".into(), false)]);
}

#[test]
fn clip6_large_text_gives_a_notice() {
    let mut w = World::new();
    w.advance_to_forwarding();
    let big = "x".repeat(telescreen_core::clipboard::MAX_TEXT + 1);
    copy(&w.mac, &big, false);
    w.mac.controller.tick(w.now);
    w.pump();
    assert!(w.pc1.log.borrow().clip_set.is_empty());
    let notices = w.mac.controller.control_screen().notices;
    assert!(
        notices.iter().any(|n| n.contains("clipboard")),
        "{notices:?}"
    );
}

#[test]
fn clip1_no_clipboard_read_without_the_switch_or_a_session() {
    let mut w = World::new();
    copy(&w.mac, "before", false);
    w.mac.controller.tick(w.now);
    assert!(w.mac.log.borrow().clip_calls.is_empty(), "no session");

    w.advance_to_forwarding();
    w.mac.controller.tick(w.now);
    assert_eq!(w.mac.log.borrow().clip_calls, ["open"]);
    w.mac
        .controller
        .on_intent(Intent::SetClipboard { on: false });
    w.mac.controller.tick(w.now);
    assert_eq!(w.mac.log.borrow().clip_calls, ["open", "close"]);
    copy(&w.mac, "off", false);
    w.mac.controller.tick(w.now);
    assert!(!w.mac.log.borrow().clip_open, "the switch is off");

    w.mac
        .controller
        .on_intent(Intent::SetClipboard { on: true });
    w.mac.controller.tick(w.now);
    assert_eq!(w.mac.log.borrow().clip_calls, ["open", "close", "open"]);
    w.pump();
    assert!(
        w.pc1.log.borrow().clip_set.is_empty(),
        "the copy while off is the new baseline (CLIP-3)"
    );
}

/// A world where `verify` of pc1 (the destination of the Mac) is `verify`, or absent.
fn world_with_pc1_verify(verify: Option<serde_json::Value>) -> World {
    let mut config = config();
    let device = config
        .computers
        .get_mut(&ComputerId::from("pc1"))
        .unwrap()
        .devices
        .get_mut("Monitor")
        .unwrap();
    device.verify = verify.map(|v| {
        let wrapped: telescreen_core::config::Device =
            serde_json::from_value(json!({"id": "shared-lg", "verify": v})).unwrap();
        wrapped.verify.unwrap()
    });
    let mut w = World::new();
    w.pc1 = peer_config(
        "pc1",
        vec![display("HDMI-A-2", 0.0, 2048.0)],
        InputCapabilities {
            capture: Capability::Available,
            suppress: Capability::Available,
            inject: Capability::Available,
        },
        true,
        config,
    );
    w
}

#[test]
fn ver2_ac39_no_verify_starts_forwarding_unverified() {
    let mut w = world_with_pc1_verify(None);
    w.advance_to_confirmation();
    assert!(w.pc1.log.borrow().prompts.is_empty(), "no prompt");
    let session = w.mac.controller.portal_screen().unwrap().session.unwrap();
    assert_eq!(session.phase, SessionPhase::Forwarding);
    assert_eq!(session.video_input, VideoInput::Unverified);
    assert!(!w.mac.controller.acknowledge(), "Return is a normal key");
}

#[test]
fn ver3_a_process_verifier_runs_on_the_destination() {
    let mut w = world_with_pc1_verify(Some(json!({"driver": "command", "command": "check"})));
    w.advance_to_confirmation();
    assert_eq!(w.pc1.log.borrow().verifier_calls, ["start 1 command"]);
    assert!(w.mac.log.borrow().verifier_calls.is_empty());
    let session = w.mac.controller.portal_screen().unwrap().session.unwrap();
    assert_eq!(session.phase, SessionPhase::Verifying);

    // The verifier shows a text and asks for an acknowledgement (VER-3, VER-4).
    w.pc1
        .controller
        .on_verifier_event(VerifierId(1), VerifierEvent::Show("Is it here?".into()));
    w.pc1
        .controller
        .on_verifier_event(VerifierId(1), VerifierEvent::WantAck);
    w.pump();
    assert!(w.pc1.log.borrow().prompt_visible);
    assert_eq!(
        w.pc1.log.borrow().prompts.last().unwrap(),
        "Press Return on MacBook to control this computer | Is it here?"
    );
    assert!(w.mac.controller.acknowledge());
    w.pump();
    assert_eq!(
        w.pc1.log.borrow().verifier_calls,
        ["start 1 command", "ack 1"]
    );

    w.pc1
        .controller
        .on_verifier_event(VerifierId(1), VerifierEvent::Done(true));
    w.mac.controller.on_captured(CapturedEvent::PointerEntered {
        at: Point { x: 10.0, y: 10.0 },
    });
    w.pump();
    assert!(!w.pc1.log.borrow().prompt_visible);
    let session = w.mac.controller.portal_screen().unwrap().session.unwrap();
    assert_eq!(session.phase, SessionPhase::Forwarding);
    assert_eq!(session.video_input, VideoInput::Confirmed);
}

#[test]
fn ver3_an_exec_verifier_with_no_user() {
    let mut w = world_with_pc1_verify(Some(json!({"driver": "exec", "command": "check"})));
    w.advance_to_confirmation();
    assert_eq!(w.pc1.log.borrow().verifier_calls, ["start 1 exec"]);
    w.pc1
        .controller
        .on_verifier_event(VerifierId(1), VerifierEvent::Done(true));
    w.pump();
    assert!(w.pc1.log.borrow().prompts.is_empty(), "no window");
    let session = w.mac.controller.portal_screen().unwrap().session.unwrap();
    assert_eq!(session.phase, SessionPhase::Forwarding);
    assert_eq!(session.video_input, VideoInput::Confirmed);
}

#[test]
fn flow11_ac10_a_failed_verification_ends_the_session() {
    let mut w = world_with_pc1_verify(Some(json!({"driver": "command", "command": "check"})));
    w.advance_to_confirmation();
    w.pc1
        .controller
        .on_verifier_event(VerifierId(1), VerifierEvent::Failed("timeout".into()));
    w.pump();
    assert!(!w.mac.controller.has_portal(), "ACT-5");
    assert!(has_notice(&w.mac, "The display state is unknown."));
    assert!(!w.mac.controller.guarded(), "the guard ends (FLOW-21)");
    assert!(w.pc1.controller.control_screen().incoming.is_empty());
}

#[test]
fn flow16_an_end_while_verifying_stops_the_verifier() {
    let mut w = world_with_pc1_verify(Some(json!({"driver": "command", "command": "check"})));
    w.advance_to_confirmation();
    w.mac.controller.on_intent(Intent::EndSession);
    w.pump();
    assert_eq!(
        w.pc1.log.borrow().verifier_calls,
        ["start 1 command", "stop 1"]
    );
    // A late event of the stopped verifier has no effect.
    w.pc1
        .controller
        .on_verifier_event(VerifierId(1), VerifierEvent::Done(true));
    w.pump();
    assert!(w.pc1.controller.control_screen().incoming.is_empty());
}

/// The device state of the shared display on pc1, the destination (VER-6).
fn pc1_state(w: &World) -> VideoInput {
    let screen = w.pc1.controller.control_screen();
    let row = screen
        .devices
        .iter()
        .find(|row| row.alias == "Monitor")
        .expect("the device list has each local device");
    assert_eq!(row.device, DeviceId::from("shared-lg"));
    row.state
}

#[test]
fn ver6_no_state_before_a_handoff() {
    let w = World::new();
    assert_eq!(pc1_state(&w), VideoInput::Unchanged);
}

#[test]
fn ver6_ac5_an_acknowledged_prompt_confirms_the_device() {
    let mut w = World::new();
    w.advance_to_confirmation();
    assert_eq!(pc1_state(&w), VideoInput::Unverified, "the steps succeeded");
    w.advance_to_forwarding_from_confirmation();
    assert_eq!(pc1_state(&w), VideoInput::Confirmed);
    // The device state does not depend on the session.
    w.mac.controller.on_intent(Intent::EndSession);
    w.pump();
    assert_eq!(pc1_state(&w), VideoInput::Confirmed);
}

#[test]
fn ver2_ac39_no_verify_leaves_the_device_unverified() {
    let mut w = world_with_pc1_verify(None);
    w.advance_to_confirmation();
    assert_eq!(pc1_state(&w), VideoInput::Unverified);
    w.mac.controller.on_intent(Intent::EndSession);
    w.pump();
    assert_eq!(
        pc1_state(&w),
        VideoInput::Unverified,
        "the end does not change it"
    );
}

#[test]
fn ac40_a_timeout_makes_the_device_unknown_and_it_is_back_confirms_it() {
    let mut w = World::new();
    w.advance_to_confirmation();
    w.pc1
        .controller
        .tick(w.now + std::time::Duration::from_secs(61));
    w.pump();
    assert_eq!(pc1_state(&w), VideoInput::Unknown);
    w.pc1.controller.on_intent(Intent::DeviceIsBack {
        device: DeviceId::from("shared-lg"),
    });
    assert_eq!(pc1_state(&w), VideoInput::Confirmed);
}

#[test]
fn ver6_it_is_back_needs_an_unknown_device() {
    let mut w = world_with_pc1_verify(None);
    w.advance_to_confirmation();
    w.pc1.controller.on_intent(Intent::DeviceIsBack {
        device: DeviceId::from("shared-lg"),
    });
    assert_eq!(pc1_state(&w), VideoInput::Unverified);
}

#[test]
fn ac40_a_failed_process_verifier_makes_the_device_unknown() {
    let mut w = world_with_pc1_verify(Some(json!({"driver": "exec", "command": "check"})));
    w.advance_to_confirmation();
    w.pc1
        .controller
        .on_verifier_event(VerifierId(1), VerifierEvent::Done(false));
    w.pump();
    assert_eq!(pc1_state(&w), VideoInput::Unknown);
}

#[test]
fn flow12_a_partial_handoff_makes_the_device_unknown_on_the_destination() {
    let mut w = World::new();
    w.handoff = HandoffResult::Failed { completed_steps: 1 };
    w.advance_to_confirmation();
    assert_eq!(pc1_state(&w), VideoInput::Unknown);
    assert!(w.pc1.controller.control_screen().incoming.is_empty());

    let mut w = World::new();
    w.handoff = HandoffResult::Failed { completed_steps: 0 };
    w.advance_to_confirmation();
    assert_eq!(pc1_state(&w), VideoInput::Unchanged, "nothing switched");
}

/// The device state of the shared display on the Mac (VER-6).
fn mac_state(w: &World) -> VideoInput {
    let screen = w.mac.controller.control_screen();
    screen
        .devices
        .iter()
        .find(|row| row.alias == "Desktop")
        .unwrap()
        .state
}

const LOCAL_PROMPT: &str = "Press Return or click here to confirm this display | ";

fn hand_back_on_pc1(w: &mut World) {
    let row = w.pc1.controller.control_screen().incoming[0].clone();
    assert_eq!(row.hand_back, None, "Hand back is available");
    w.pc1.controller.on_intent(Intent::HandBack {
        session: row.session,
    });
    w.pump();
}

#[test]
fn hb1_hb2_ac36_the_target_hands_back() {
    let mut w = World::new();
    w.advance_to_forwarding();
    w.mac
        .controller
        .on_captured(CapturedEvent::Input(key(42, true)));
    w.pump();
    hand_back_on_pc1(&mut w);
    // The end comes first: no key stays held across the switch.
    assert_eq!(
        w.pc1.log.borrow().target[1..],
        [
            format!("inject {:?}", key(42, false)),
            "handoff /bin/to-mac".into()
        ]
    );
    assert!(w.pc1.controller.control_screen().incoming.is_empty());
    assert!(!w.mac.controller.has_portal(), "ACT-5");
    assert_eq!(w.mac.log.borrow().capture.last().unwrap(), "disarm");
    // VER-6: the device left pc1, so pc1 shows no state.
    assert_eq!(pc1_state(&w), VideoInput::Unchanged);

    // The Mac verifies as the destination, with its own prompt (HB-1, VER-4).
    assert!(w.mac.log.borrow().prompt_visible);
    assert_eq!(w.mac.log.borrow().prompts.last().unwrap(), LOCAL_PROMPT);
    assert_eq!(mac_state(&w), VideoInput::Unverified);
    assert!(w.mac.controller.awaiting_local_ack());
    assert!(
        !w.mac.controller.acknowledge(),
        "the portal has no session to ack"
    );
    assert!(w.mac.controller.acknowledge_local());
    assert!(!w.mac.controller.awaiting_local_ack());
    assert!(!w.mac.log.borrow().prompt_visible);
    assert_eq!(mac_state(&w), VideoInput::Confirmed);
    assert!(!w.mac.controller.acknowledge_local(), "one ack");
}

#[test]
fn hb3_ac15_the_hand_back_shortcut() {
    let mut w = World::new();
    w.advance_to_forwarding();
    w.mac
        .controller
        .on_captured(CapturedEvent::HandBackShortcut);
    w.pump();
    assert_eq!(
        w.pc1.log.borrow().target.last().unwrap(),
        "handoff /bin/to-mac"
    );
    assert_eq!(w.mac.log.borrow().capture.last().unwrap(), "disarm");
    assert_eq!(w.mac.log.borrow().prompts.last().unwrap(), LOCAL_PROMPT);
    assert!(w.mac.controller.acknowledge_local());
    assert_eq!(mac_state(&w), VideoInput::Confirmed);
}

#[test]
fn hb3_ui3_ac38_the_portal_left_fullscreen() {
    let mut w = World::new();
    w.advance_to_forwarding();
    w.mac.controller.portal_left_fullscreen();
    w.pump();
    assert_eq!(
        w.pc1.log.borrow().target.last().unwrap(),
        "handoff /bin/to-mac"
    );
    assert!(w.mac.controller.awaiting_local_ack());
}

#[test]
fn ver5_the_local_prompt_times_out() {
    let mut w = World::new();
    w.advance_to_forwarding();
    hand_back_on_pc1(&mut w);
    w.mac
        .controller
        .tick(w.now + std::time::Duration::from_secs(61));
    w.pump();
    assert!(!w.mac.log.borrow().prompt_visible);
    assert_eq!(mac_state(&w), VideoInput::Unknown);
    w.mac.controller.on_intent(Intent::DeviceIsBack {
        device: DeviceId::from("shared-lg"),
    });
    assert_eq!(mac_state(&w), VideoInput::Confirmed);
}

#[test]
fn hb5_in8_the_safety_exit_does_not_hand_back() {
    let mut w = World::new();
    w.advance_to_forwarding();
    w.mac.controller.on_captured(CapturedEvent::SafetyExit);
    w.pump();
    assert!(
        !w.pc1
            .log
            .borrow()
            .target
            .iter()
            .any(|t| t.starts_with("handoff")),
        "no route back"
    );
    assert!(!w.mac.log.borrow().prompt_visible);
    assert!(!w.mac.controller.awaiting_local_ack());
}

/// A world where pc1 has no route back to the Mac.
fn world_with_no_route_back() -> World {
    let mut config = config();
    config
        .computers
        .get_mut(&ComputerId::from("pc1"))
        .unwrap()
        .devices
        .get_mut("Monitor")
        .unwrap()
        .handoff
        .clear();
    let mut w = World::new();
    w.pc1 = peer_config(
        "pc1",
        vec![display("HDMI-A-2", 0.0, 2048.0)],
        InputCapabilities {
            capture: Capability::Available,
            suppress: Capability::Available,
            inject: Capability::Available,
        },
        true,
        config,
    );
    w
}

fn has_notice(peer: &Peer, text: &str) -> bool {
    peer.controller
        .control_screen()
        .notices
        .iter()
        .any(|n| n.contains(text))
}

#[test]
fn hb2_hb4_ac37_no_route_back() {
    let mut w = world_with_no_route_back();
    w.advance_to_forwarding();
    let row = w.pc1.controller.control_screen().incoming[0].clone();
    assert_eq!(
        row.hand_back.as_deref(),
        Some("No route back to MacBook in this config")
    );
    // The disabled action has no effect.
    w.pc1.controller.on_intent(Intent::HandBack {
        session: row.session,
    });
    w.pump();
    assert_eq!(w.phase(), Some(SessionPhase::Forwarding));

    w.mac
        .controller
        .on_captured(CapturedEvent::HandBackShortcut);
    w.pump();
    assert!(w.pc1.controller.control_screen().incoming.is_empty());
    assert!(has_notice(&w.pc1, "by hand"));
    assert!(has_notice(&w.mac, "by hand"));
    assert!(!w.mac.controller.awaiting_local_ack());
    assert_eq!(mac_state(&w), VideoInput::Unchanged);
    assert_eq!(
        pc1_state(&w),
        VideoInput::Confirmed,
        "no route: nothing switched"
    );
}

#[test]
fn flow12_a_partial_hand_back_makes_the_device_unknown() {
    let mut w = World::new();
    w.advance_to_forwarding();
    w.handoff = HandoffResult::Failed { completed_steps: 1 };
    hand_back_on_pc1(&mut w);
    assert!(!w.mac.log.borrow().prompt_visible, "no verification");
    assert_eq!(mac_state(&w), VideoInput::Unknown);
    assert_eq!(pc1_state(&w), VideoInput::Unknown, "VER-6");
    assert!(has_notice(&w.mac, "hand back"));
    assert!(has_notice(&w.pc1, "hand back"));

    let mut w = World::new();
    w.advance_to_forwarding();
    w.handoff = HandoffResult::Failed { completed_steps: 0 };
    hand_back_on_pc1(&mut w);
    assert_eq!(mac_state(&w), VideoInput::Unchanged, "nothing switched");
    assert_eq!(pc1_state(&w), VideoInput::Confirmed, "nothing switched");
    assert!(has_notice(&w.mac, "hand back"));
}

#[test]
fn hb3_a_result_with_no_hand_back_has_no_effect() {
    let mut w = World::new();
    w.advance_to_forwarding();
    for outcome in [
        HandBackOutcome::Done,
        HandBackOutcome::Failed { partial: true },
    ] {
        w.mac.controller.on_net(
            NetEvent::Message {
                peer: ComputerId::from("pc1"),
                message: Message::HandBackResult {
                    device: DeviceId::from("shared-lg"),
                    outcome,
                },
            },
            w.now,
        );
    }
    assert!(w.mac.log.borrow().jobs.is_empty());
    assert!(!w.mac.controller.awaiting_local_ack());
    assert_eq!(mac_state(&w), VideoInput::Unchanged);
    assert_eq!(w.phase(), Some(SessionPhase::Forwarding));
}

#[test]
fn hb3_a_request_from_another_peer_has_no_effect() {
    let mut w = World::new();
    w.advance_to_forwarding();
    let session = w.pc1.controller.control_screen().incoming[0].session;
    w.pc1.controller.on_net(
        NetEvent::Message {
            peer: ComputerId::from("pc2"),
            message: Message::HandBackRequest { session },
        },
        w.now,
    );
    w.pump();
    assert_eq!(w.phase(), Some(SessionPhase::Forwarding));
    assert!(
        !w.pc1
            .log
            .borrow()
            .target
            .iter()
            .any(|t| t.starts_with("handoff"))
    );
}

#[test]
fn hb1_no_input_target_on_the_destination_leaves_the_device_unknown() {
    let mut config = config();
    config
        .computers
        .get_mut(&ComputerId::from("mac"))
        .unwrap()
        .devices
        .get_mut("Desktop")
        .unwrap()
        .input_target = None;
    let mut w = World::new();
    w.mac = peer_config(
        "mac",
        vec![
            display("builtin", -1500.0, 1500.0),
            display("lg", 0.0, 3400.0),
        ],
        InputCapabilities {
            capture: Capability::Available,
            suppress: Capability::Available,
            inject: Capability::Available,
        },
        true,
        config,
    );
    w.advance_to_forwarding();
    hand_back_on_pc1(&mut w);
    assert!(!w.mac.log.borrow().prompt_visible);
    assert_eq!(mac_state(&w), VideoInput::Unknown);
    assert!(has_notice(&w.mac, "input-target"));
}

// ----- FLOW-24: the target closes its portal on the display -----

fn pc4() -> Peer {
    peer("pc4", vec![display("HDMI-A-2", 0.0, 2048.0)])
}

/// pc4 opens a portal on the shared display and requests a session to the Mac.
fn pc4_takes_the_display(w: &mut World) {
    w.extra[0]
        .controller
        .open_portal(display("HDMI-A-2", 0.0, 2048.0));
    w.pump();
    w.extra[0].controller.start_handoff(ComputerId::from("mac"));
    w.pump();
}

#[test]
fn flow24_ac41_a_third_peer_takes_the_display() {
    let mut w = World::new();
    w.extra.push(pc4());
    w.advance_to_forwarding();
    w.mac
        .controller
        .on_captured(CapturedEvent::Input(key(56, true)));
    w.pump();
    // The user switches the display to pc4 by hand.
    pc4_takes_the_display(&mut w);
    assert!(!w.mac.controller.has_portal(), "the Mac closes its portal");
    assert_eq!(w.mac.log.borrow().capture.last().unwrap(), "disarm");
    assert_eq!(
        w.pc1.log.borrow().injected.last(),
        Some(&key(56, false)),
        "the session Mac to pc1 ends and releases the held key"
    );
    assert!(w.pc1.controller.control_screen().incoming.is_empty());
    let control = w.mac.controller.control_screen();
    assert!(
        control
            .notices
            .iter()
            .any(|n| n == "Session ended: Desk PC took this display"),
        "{:?}",
        control.notices
    );
    assert_eq!(control.incoming.len(), 1);
    assert_eq!(control.incoming[0].source, "Desk PC");
}

#[test]
fn flow24_a_portal_on_another_display_stays_open() {
    let mut w = World::new();
    w.extra.push(pc4());
    w.mac
        .controller
        .open_portal(display("builtin", -1500.0, 1500.0));
    w.pump();
    pc4_takes_the_display(&mut w);
    assert!(w.mac.controller.has_portal());
    assert_eq!(w.mac.controller.control_screen().incoming.len(), 1);
}

#[test]
fn net10_ac42_refresh_dials_an_offline_candidate_one_time() {
    let mut w = World::new();
    let pc1 = ComputerId::from("pc1");
    let dials = |w: &World| {
        w.mac
            .log
            .borrow()
            .net
            .iter()
            .filter(|c| matches!(c, NetCommand::Dial { .. }))
            .count()
    };
    w.open_portal();
    let candidate = |w: &World| {
        let screen = w.mac.controller.portal_screen().unwrap();
        (screen.candidates[0].unavailable.clone(), screen.can_refresh)
    };
    assert_eq!(candidate(&w), (None, false), "connected");
    w.mac.controller.on_intent(Intent::Refresh);
    assert_eq!(dials(&w), 0, "a connected candidate needs no dial");

    w.mac.controller.on_net(
        NetEvent::Disconnected {
            peer: pc1.clone(),
            reason: "test".into(),
        },
        w.now,
    );
    assert_eq!(candidate(&w), (Some("offline".into()), true));
    assert_eq!(dials(&w), 0, "no automatic dial");

    w.mac.controller.on_intent(Intent::Refresh);
    assert_eq!(dials(&w), 1);
    assert_eq!(candidate(&w), (Some("connecting".into()), false));
    w.mac.controller.on_intent(Intent::Refresh);
    assert_eq!(dials(&w), 1, "one dial at a time");

    w.mac.controller.on_net(
        NetEvent::DialFailed {
            peer: pc1.clone(),
            reason: "test".into(),
        },
        w.now,
    );
    assert_eq!(candidate(&w), (Some("offline".into()), true));
    w.mac.log.borrow_mut().net.clear();

    w.mac.controller.on_intent(Intent::Refresh);
    assert_eq!(dials(&w), 1, "the user can try again");
    w.pump();
    assert_eq!(candidate(&w), (None, false));
}

#[test]
fn net10_refresh_without_a_portal_does_not_dial() {
    let mut w = World::new();
    w.mac.controller.on_intent(Intent::Refresh);
    assert!(w.mac.log.borrow().net.is_empty());
}

const PC1: &str = "pc1";

/// A world where `change` edited the endpoint of pc1 and the config of `id` for both peers.
fn world_with(change: impl Fn(&mut Config)) -> World {
    let capabilities = || InputCapabilities {
        capture: Capability::Available,
        suppress: Capability::Available,
        inject: Capability::Available,
    };
    let mut changed = config();
    change(&mut changed);
    let mut w = World::new();
    w.mac = peer_config(
        "mac",
        vec![
            display("builtin", -1500.0, 1500.0),
            display("lg", 0.0, 3400.0),
        ],
        capabilities(),
        true,
        changed.clone(),
    );
    w.pc1 = peer_config(
        "pc1",
        vec![display("HDMI-A-2", 0.0, 2048.0)],
        capabilities(),
        true,
        changed,
    );
    w
}

fn endpoint_of<'a>(config: &'a mut Config, id: &str) -> &'a mut telescreen_core::config::Endpoint {
    &mut config
        .computers
        .get_mut(&ComputerId::from(id))
        .unwrap()
        .endpoint
}

fn mdns_world(hosts: &[&str]) -> World {
    let hosts: Vec<String> = hosts.iter().map(|h| (*h).to_owned()).collect();
    world_with(move |config| {
        let endpoint = endpoint_of(config, PC1);
        endpoint.hosts = hosts.clone();
        endpoint.discovery = Some(telescreen_core::config::Discovery::Mdns);
    })
}

/// Opens the portal with pc1 offline. No dial is pending at the end.
fn offline_portal(w: &mut World) {
    let pc1 = ComputerId::from(PC1);
    w.mac.controller.tick(w.now);
    w.mac.controller.open_portal(display("lg", 0.0, 3400.0));
    w.pump();
    // The fake network connected the dial, if any. Make pc1 offline.
    w.mac.controller.on_net(
        NetEvent::Disconnected {
            peer: pc1,
            reason: "test".into(),
        },
        w.now,
    );
    w.mac.log.borrow_mut().net.clear();
    // Let the search of the portal end.
    w.now += Duration::from_secs(31);
    w.mac.controller.tick(w.now);
    w.mac.log.borrow_mut().discovery.clear();
}

fn dials(w: &World) -> Vec<Vec<DialTarget>> {
    w.mac
        .log
        .borrow()
        .net
        .iter()
        .filter_map(|c| match c {
            NetCommand::Dial { targets, .. } => Some(targets.clone()),
            _ => None,
        })
        .collect()
}

fn dial_target(host: &str) -> DialTarget {
    DialTarget {
        host: host.into(),
        port: 24800,
    }
}

fn status(w: &World) -> Option<String> {
    w.mac.controller.portal_screen().unwrap().candidates[0]
        .unavailable
        .clone()
}

fn discovery_calls(w: &World) -> Vec<String> {
    w.mac.log.borrow().discovery.clone()
}

fn fail_dial(w: &mut World) {
    w.mac.log.borrow_mut().net.clear();
    w.mac.controller.on_net(
        NetEvent::DialFailed {
            peer: ComputerId::from(PC1),
            reason: "test".into(),
        },
        w.now,
    );
}

#[test]
fn net11_a_dial_has_all_hosts_in_sequence() {
    let mut w = world_with(|config| {
        endpoint_of(config, PC1).hosts = vec!["a.test".into(), "b.test".into()];
    });
    offline_portal(&mut w);
    w.mac.controller.on_intent(Intent::Refresh);
    assert_eq!(
        dials(&w),
        [vec![dial_target("a.test"), dial_target("b.test")]]
    );
}

#[test]
fn net15_refresh_with_no_discovery_key_does_not_search() {
    let mut w = World::new();
    offline_portal(&mut w);
    w.mac.controller.on_intent(Intent::Refresh);
    assert!(discovery_calls(&w).is_empty());
    assert_eq!(status(&w), Some("connecting".into()));
}

#[test]
fn net15_opening_a_portal_dials_and_searches_at_the_same_time() {
    let mut w = mdns_world(&["pc1.test"]);
    w.mac.controller.open_portal(display("lg", 0.0, 3400.0));
    w.pump_until_dial();
    assert_eq!(dials(&w).len(), 1);
    assert_eq!(discovery_calls(&w), ["search"]);
    assert_eq!(status(&w), Some("connecting".into()));
    let screen = w.mac.controller.portal_screen().unwrap();
    assert!(screen.show_refresh);
    assert!(!screen.can_refresh, "the dial and the search run");
}

#[test]
fn net15_a_connection_ends_the_search() {
    let mut w = mdns_world(&["pc1.test"]);
    w.open_portal();
    assert_eq!(discovery_calls(&w), ["search", "stop_search"]);
    assert_eq!(status(&w), None);
    assert!(!w.mac.controller.portal_screen().unwrap().show_refresh);
}

#[test]
fn net15_a_portal_with_no_discovery_candidate_does_not_search() {
    let mut w = World::new();
    w.open_portal();
    assert!(discovery_calls(&w).is_empty());
}

#[test]
fn net15_refresh_during_a_dial_starts_a_search() {
    let mut w = mdns_world(&["pc1.test"]);
    offline_portal(&mut w);
    w.mac.controller.on_intent(Intent::Refresh);
    w.mac.log.borrow_mut().net.clear();
    w.mac.controller.tick(w.now + Duration::from_secs(31));
    assert_eq!(discovery_calls(&w), ["search", "stop_search"]);
    assert_eq!(status(&w), Some("connecting".into()));
    assert!(w.mac.controller.portal_screen().unwrap().can_refresh);
    w.mac.controller.on_intent(Intent::Refresh);
    assert!(dials(&w).is_empty(), "one dial at a time");
    assert_eq!(discovery_calls(&w), ["search", "stop_search", "search"]);
}

#[test]
fn net10_refresh_is_shown_but_not_available_during_a_dial_with_no_discovery() {
    let mut w = World::new();
    offline_portal(&mut w);
    w.mac.controller.on_intent(Intent::Refresh);
    let screen = w.mac.controller.portal_screen().unwrap();
    assert!(screen.show_refresh);
    assert!(!screen.can_refresh);
}

#[test]
fn net15_refresh_searches_for_30_seconds_and_the_candidate_shows_it() {
    let mut w = mdns_world(&[]);
    offline_portal(&mut w);
    assert_eq!(status(&w), Some("offline".into()));
    assert!(w.mac.controller.portal_screen().unwrap().can_refresh);

    w.mac.controller.on_intent(Intent::Refresh);
    assert_eq!(discovery_calls(&w), ["search"]);
    assert!(dials(&w).is_empty(), "no host and no known address");
    assert_eq!(status(&w), Some("searching".into()));
    assert!(!w.mac.controller.portal_screen().unwrap().can_refresh);
    w.mac.controller.on_intent(Intent::Refresh);
    assert_eq!(discovery_calls(&w), ["search"], "one search at a time");

    w.mac.controller.tick(w.now + Duration::from_secs(29));
    assert_eq!(discovery_calls(&w), ["search"]);
    w.mac.controller.tick(w.now + Duration::from_secs(31));
    assert_eq!(discovery_calls(&w), ["search", "stop_search"]);
    assert_eq!(status(&w), Some("offline".into()));
    assert!(w.mac.controller.portal_screen().unwrap().can_refresh);
}

#[test]
fn net16_a_result_dials_the_found_address_first() {
    let mut w = mdns_world(&["pc1.test"]);
    offline_portal(&mut w);
    w.mac.controller.on_intent(Intent::Refresh);
    fail_dial(&mut w);
    w.mac
        .controller
        .on_discovered(ComputerId::from(PC1), vec![dial_target("192.168.0.9")]);
    assert_eq!(
        dials(&w),
        [vec![dial_target("192.168.0.9"), dial_target("pc1.test")]]
    );
    assert_eq!(status(&w), Some("connecting".into()));
}

#[test]
fn net16_a_result_during_a_dial_joins_its_race() {
    let mut w = mdns_world(&["pc1.test"]);
    offline_portal(&mut w);
    w.mac.controller.on_intent(Intent::Refresh);
    w.mac.log.borrow_mut().net.clear();
    w.mac
        .controller
        .on_discovered(ComputerId::from(PC1), vec![dial_target("192.168.0.9")]);
    assert_eq!(
        dials(&w),
        [vec![dial_target("192.168.0.9"), dial_target("pc1.test")]]
    );
    assert_eq!(status(&w), Some("connecting".into()));
}

#[test]
fn net16_a_lost_connection_does_not_dial_again() {
    let mut w = mdns_world(&["pc1.test"]);
    offline_portal(&mut w);
    w.mac.controller.on_intent(Intent::Refresh);
    w.mac
        .controller
        .on_discovered(ComputerId::from(PC1), vec![dial_target("192.168.0.9")]);
    w.mac.log.borrow_mut().net.clear();
    w.mac.controller.on_net(
        NetEvent::Connected {
            peer: ComputerId::from(PC1),
        },
        w.now,
    );
    assert!(dials(&w).is_empty());
    w.mac.controller.on_net(
        NetEvent::Disconnected {
            peer: ComputerId::from(PC1),
            reason: "test".into(),
        },
        w.now,
    );
    assert!(dials(&w).is_empty(), "no automatic dial (NET-10)");
}

#[test]
fn net16_a_new_result_replaces_the_previous_one_and_later_dials_use_it() {
    let mut w = mdns_world(&["pc1.test"]);
    offline_portal(&mut w);
    let pc1 = ComputerId::from(PC1);
    w.mac.controller.on_intent(Intent::Refresh);
    fail_dial(&mut w);
    w.mac
        .controller
        .on_discovered(pc1.clone(), vec![dial_target("192.168.0.8")]);
    fail_dial(&mut w);
    w.mac
        .controller
        .on_discovered(pc1, vec![dial_target("192.168.0.9")]);
    fail_dial(&mut w);
    w.mac.controller.tick(w.now + Duration::from_secs(31));
    w.mac.controller.on_intent(Intent::Refresh);
    assert_eq!(
        dials(&w),
        [vec![dial_target("192.168.0.9"), dial_target("pc1.test")]]
    );
}

#[test]
fn net16_a_result_that_is_not_an_offline_candidate_does_not_dial() {
    let mut w = mdns_world(&["pc1.test"]);
    w.mac
        .controller
        .on_discovered(ComputerId::from(PC1), vec![dial_target("192.168.0.9")]);
    assert!(dials(&w).is_empty(), "no portal");
    w.open_portal();
    w.mac.log.borrow_mut().net.clear();
    w.mac
        .controller
        .on_discovered(ComputerId::from(PC1), vec![dial_target("192.168.0.9")]);
    assert!(dials(&w).is_empty(), "connected");
    w.mac
        .controller
        .on_discovered(ComputerId::from("pc2"), vec![dial_target("192.168.0.7")]);
    w.mac.controller.on_discovered(
        ComputerId::from("unknown"),
        vec![dial_target("192.168.0.7")],
    );
    assert!(dials(&w).is_empty(), "no route to these computers");
}

#[test]
fn net15_closing_the_portal_stops_the_search() {
    let mut w = mdns_world(&[]);
    offline_portal(&mut w);
    w.mac.controller.on_intent(Intent::Refresh);
    w.mac.controller.close_portal(EndReason::SourceClosed);
    assert_eq!(discovery_calls(&w), ["search", "stop_search"]);
}

#[test]
fn net12_ac48_a_search_error_gives_a_notice_and_the_dial_still_runs() {
    let mut w = mdns_world(&["pc1.test"]);
    offline_portal(&mut w);
    w.mac.log.borrow_mut().discovery_fails = true;
    w.mac.controller.on_intent(Intent::Refresh);
    assert_eq!(dials(&w).len(), 1);
    assert_eq!(status(&w), Some("connecting".into()));
    assert!(
        w.mac
            .controller
            .control_screen()
            .notices
            .iter()
            .any(|n| n.contains("no responder"))
    );
}

#[test]
fn net12_a_search_that_stops_with_an_error_ends_the_searching_state() {
    let mut w = mdns_world(&[]);
    offline_portal(&mut w);
    w.mac.controller.on_intent(Intent::Refresh);
    w.mac.controller.on_discovery_failed("gone".into());
    assert_eq!(status(&w), Some("offline".into()));
    assert!(
        w.mac
            .controller
            .control_screen()
            .notices
            .iter()
            .any(|n| n.contains("gone"))
    );
}

#[test]
fn net14_allow_discovery_needs_the_key_of_this_computer() {
    let mut w = World::new();
    assert_eq!(w.mac.controller.control_screen().discovery, None);
    w.mac.controller.on_intent(Intent::AllowDiscovery);
    assert!(discovery_calls(&w).is_empty());
    // The key of a different computer does not count.
    let mut w = mdns_world(&["pc1.test"]);
    assert_eq!(w.mac.controller.control_screen().discovery, None);
    w.mac.controller.on_intent(Intent::AllowDiscovery);
    assert!(discovery_calls(&w).is_empty());
}

fn discoverable_mac() -> World {
    world_with(|config| {
        endpoint_of(config, "mac").discovery = Some(telescreen_core::config::Discovery::Mdns);
    })
}

fn remaining(w: &World) -> Option<u64> {
    w.mac
        .controller
        .control_screen()
        .discovery
        .unwrap()
        .remaining_seconds
}

#[test]
fn net14_ac46_allow_discovery_ends_after_10_minutes() {
    let mut w = discoverable_mac();
    w.mac.controller.tick(w.now);
    assert_eq!(remaining(&w), None);
    w.mac.controller.on_intent(Intent::AllowDiscovery);
    let calls = discovery_calls(&w);
    assert_eq!(calls.len(), 1);
    assert!(calls[0].starts_with("announce 24800 "), "{calls:?}");
    assert_eq!(remaining(&w), Some(600));

    w.mac.controller.tick(w.now + Duration::from_secs(599));
    assert_eq!(remaining(&w), Some(1));
    assert_eq!(discovery_calls(&w).len(), 1);
    w.mac.controller.tick(w.now + Duration::from_secs(601));
    assert_eq!(remaining(&w), None);
    assert_eq!(discovery_calls(&w)[1], "stop_announce");
    w.mac.controller.tick(w.now + Duration::from_secs(602));
    assert_eq!(discovery_calls(&w).len(), 2, "one stop");
}

#[test]
fn net14_ac46_the_user_can_stop_it() {
    let mut w = discoverable_mac();
    w.mac.controller.tick(w.now);
    w.mac.controller.on_intent(Intent::AllowDiscovery);
    w.mac.controller.on_intent(Intent::StopDiscovery);
    assert_eq!(remaining(&w), None);
    assert_eq!(discovery_calls(&w)[1], "stop_announce");
    w.mac.controller.tick(w.now + Duration::from_secs(601));
    assert_eq!(discovery_calls(&w).len(), 2);
}

#[test]
fn net14_selecting_it_again_restarts_the_time_with_one_record() {
    let mut w = discoverable_mac();
    w.mac.controller.tick(w.now);
    w.mac.controller.on_intent(Intent::AllowDiscovery);
    w.mac.controller.tick(w.now + Duration::from_secs(100));
    w.mac.controller.on_intent(Intent::AllowDiscovery);
    assert_eq!(remaining(&w), Some(600));
    assert_eq!(discovery_calls(&w).len(), 1);
}

#[test]
fn net12_ac48_an_announce_error_gives_a_notice() {
    let mut w = discoverable_mac();
    w.mac.log.borrow_mut().discovery_fails = true;
    w.mac.controller.on_intent(Intent::AllowDiscovery);
    assert_eq!(remaining(&w), None);
    assert!(
        w.mac
            .controller
            .control_screen()
            .notices
            .iter()
            .any(|n| n.contains("no responder"))
    );
}

// ----- ACT-1 to ACT-6: the three actions and the control request -----

fn lg() -> DisplayToken {
    DisplayToken::from("lg")
}

fn shared() -> DeviceId {
    DeviceId::from("shared-lg")
}

/// Connects the Mac and pc1 with no portal and no session.
fn connect(w: &mut World) {
    w.open_portal();
    w.mac.controller.close_portal(EndReason::SourceClosed);
    w.pump();
    assert!(!w.mac.controller.has_portal());
}

fn act(w: &mut World, intent: Intent) {
    w.mac.controller.on_intent(intent);
    w.pump();
}

fn handoff_jobs(peer: &Peer) -> usize {
    peer.log
        .borrow()
        .target
        .iter()
        .filter(|t| t.starts_with("handoff"))
        .count()
}

fn armed(peer: &Peer) -> bool {
    peer.log
        .borrow()
        .capture
        .iter()
        .any(|c| c.starts_with("arm"))
}

fn waiting_for_ack() -> Option<SessionPhase> {
    Some(SessionPhase::WaitingForAck {
        key: "Return".into(),
    })
}

#[test]
fn act1_act2_switch_display_only_runs_the_steps_with_no_session() {
    let mut w = World::new();
    connect(&mut w);
    act(
        &mut w,
        Intent::SwitchDisplayOnly {
            display: lg(),
            device: shared(),
            target: ComputerId::from("pc1"),
        },
    );
    assert_eq!(handoff_jobs(&w.mac), 1, "the steps of handoff.pc1");
    assert!(!w.mac.controller.has_portal(), "ACT-6: no portal");
    assert!(!armed(&w.mac), "ACT-6: no capture");
    assert!(w.pc1.controller.control_screen().incoming.is_empty());
    // The destination verifies, as after a hand back (ACT-2, VER-4).
    assert!(w.pc1.controller.awaiting_local_ack());
    assert_eq!(pc1_state(&w), VideoInput::Unverified);
    assert!(w.pc1.controller.acknowledge_local());
    assert_eq!(pc1_state(&w), VideoInput::Confirmed);
    // VER-6: the device left the Mac.
    assert_eq!(mac_state(&w), VideoInput::Unchanged);
}

#[test]
fn act2_switch_display_only_to_a_destination_that_is_not_connected() {
    let mut w = World::new();
    act(
        &mut w,
        Intent::SwitchDisplayOnly {
            display: lg(),
            device: shared(),
            target: ComputerId::from("pc1"),
        },
    );
    assert_eq!(
        handoff_jobs(&w.mac),
        1,
        "ACT-1: no Telescreen on the destination is necessary"
    );
    assert!(!w.pc1.controller.awaiting_local_ack());
    assert!(has_notice(&w.mac, "not verified"));
    assert!(
        !w.mac.log.borrow().closed.contains(&ComputerId::from("pc1")),
        "no dial and no close"
    );
}

#[test]
fn act6_switch_display_only_needs_no_capture_permission() {
    let mut w = World::new();
    w.mac = peer_with(
        "mac",
        vec![display("lg", 0.0, 3400.0)],
        InputCapabilities {
            capture: Capability::Missing("grant Input Monitoring".into()),
            suppress: Capability::Missing("grant Accessibility".into()),
            inject: Capability::Available,
        },
    );
    act(
        &mut w,
        Intent::SwitchDisplayOnly {
            display: lg(),
            device: shared(),
            target: ComputerId::from("pc1"),
        },
    );
    assert_eq!(handoff_jobs(&w.mac), 1);
}

#[test]
fn act6_switch_display_only_needs_a_route() {
    let mut w = World::new();
    act(
        &mut w,
        Intent::SwitchDisplayOnly {
            display: lg(),
            device: shared(),
            target: ComputerId::from("pc2"),
        },
    );
    assert_eq!(handoff_jobs(&w.mac), 0);
    assert!(has_notice(&w.mac, "no handoff route"));
}

#[test]
fn flow12_a_partial_switch_display_only_makes_the_device_unknown() {
    let mut w = World::new();
    w.handoff = HandoffResult::Failed { completed_steps: 1 };
    connect(&mut w);
    act(
        &mut w,
        Intent::SwitchDisplayOnly {
            display: lg(),
            device: shared(),
            target: ComputerId::from("pc1"),
        },
    );
    assert_eq!(mac_state(&w), VideoInput::Unknown);
    assert!(
        !w.pc1.controller.awaiting_local_ack(),
        "no DisplaySwitched after a failure"
    );
}

#[test]
fn act6_display_switched_from_a_peer_with_no_route_is_ignored() {
    let mut w = World::new();
    // pc2 has no handoff.mac route for the device.
    w.mac.controller.on_net(
        NetEvent::Message {
            peer: ComputerId::from("pc2"),
            message: Message::DisplaySwitched { device: shared() },
        },
        w.now,
    );
    w.pump();
    assert!(!w.mac.controller.awaiting_local_ack());
    assert_eq!(mac_state(&w), VideoInput::Unchanged);
}

#[test]
fn act6_flow6_display_switched_ends_the_sessions_of_the_device() {
    let mut w = World::new();
    w.advance_to_forwarding();
    act(
        &mut w,
        Intent::SwitchDisplayOnly {
            display: lg(),
            device: shared(),
            target: ComputerId::from("pc1"),
        },
    );
    assert!(matches!(w.phase(), Some(SessionPhase::Ended { .. }) | None));
    assert_eq!(w.mac.log.borrow().capture.last().unwrap(), "disarm");
    assert!(w.pc1.controller.control_screen().incoming.is_empty());
    assert!(w.pc1.controller.awaiting_local_ack());
}

#[test]
fn act3_control_only_attaches_with_no_steps() {
    let mut w = World::new();
    act(
        &mut w,
        Intent::ControlOnly {
            display: lg(),
            device: shared(),
            target: ComputerId::from("pc1"),
        },
    );
    assert_eq!(w.mac.controller.take_portal_window_request(), Some(lg()));
    assert_eq!(handoff_jobs(&w.mac), 0, "ACT-3: no steps");
    assert_eq!(w.phase(), waiting_for_ack());
    // FLOW-21: the portal guards the keyboard while it waits.
    assert!(armed(&w.mac));
    assert!(w.mac.log.borrow().capture.contains(&"guard on".to_string()));
    w.advance_to_forwarding_from_confirmation();
    assert_eq!(w.phase(), Some(SessionPhase::Forwarding));
    assert_eq!(pc1_state(&w), VideoInput::Confirmed);
}

#[test]
fn act3_flow3_control_only_with_an_offline_target_does_nothing() {
    let mut w = World::new();
    // The dial does not connect: drop it.
    w.mac.controller.on_intent(Intent::ControlOnly {
        display: lg(),
        device: shared(),
        target: ComputerId::from("pc2"),
    });
    w.pump();
    assert_eq!(w.phase(), None);
    assert!(has_notice(&w.mac, "Cannot"));
}

#[test]
fn act5_switch_and_control_opens_the_portal_and_closes_it_at_the_end() {
    let mut w = World::new();
    act(
        &mut w,
        Intent::SwitchAndControl {
            display: lg(),
            device: shared(),
            target: ComputerId::from("pc1"),
        },
    );
    assert_eq!(w.mac.controller.take_portal_window_request(), Some(lg()));
    assert_eq!(handoff_jobs(&w.mac), 1);
    assert_eq!(w.phase(), waiting_for_ack());
    w.advance_to_forwarding_from_confirmation();
    act(&mut w, Intent::EndSession);
    assert!(
        !w.mac.controller.has_portal(),
        "ACT-5: the portal exists only with its session"
    );
    assert!(w.pc1.controller.control_screen().incoming.is_empty());
}

#[test]
fn act5_a_refused_session_closes_the_portal() {
    let mut w = world_with_pc1_verify(None);
    w.pc1 = peer_full(
        "pc1",
        vec![display("HDMI-A-2", 0.0, 2048.0)],
        InputCapabilities {
            capture: Capability::Available,
            suppress: Capability::Available,
            inject: Capability::Available,
        },
        false,
    );
    act(
        &mut w,
        Intent::ControlOnly {
            display: lg(),
            device: shared(),
            target: ComputerId::from("pc1"),
        },
    );
    assert!(!w.mac.controller.has_portal());
    assert!(!w.mac.controller.control_screen().notices.is_empty());
}

#[test]
fn act5_every_portal_closes_with_its_session() {
    let mut w = World::new();
    w.advance_to_forwarding();
    act(&mut w, Intent::EndSession);
    assert!(!w.mac.controller.has_portal());
    assert!(
        w.mac.controller.control_screen().notices.is_empty(),
        "no notice for an end on this computer"
    );
}

#[test]
fn act5_flow17_the_portal_of_control_from_here_closes_after_a_hand_back() {
    // Hardware test 2026-11-07: the portal stayed after "Hand back".
    let mut w = World::new();
    w.advance_to_forwarding();
    let session = w.pc1.controller.control_screen().incoming[0].session;
    w.pc1
        .controller
        .on_intent(Intent::ControlFromHere { session });
    w.pump();
    assert!(w.pc1.controller.portal_screen().is_some());
    assert!(w.pc1.controller.acknowledge());
    w.pc1.controller.on_captured(CapturedEvent::PointerEntered {
        at: Point { x: 10.0, y: 10.0 },
    });
    w.pump();
    w.pc1.controller.on_intent(Intent::SourceHandBack);
    w.pump();
    assert!(!w.pc1.controller.has_portal(), "ACT-5");
}

#[test]
fn act6_an_action_on_a_display_that_does_not_match_the_device() {
    let mut w = World::new();
    act(
        &mut w,
        Intent::ControlOnly {
            display: DisplayToken::from("builtin"),
            device: shared(),
            target: ComputerId::from("pc1"),
        },
    );
    assert_eq!(w.phase(), None);
    assert!(!w.mac.controller.has_portal());
    assert!(has_notice(&w.mac, "does not match"));
}

fn let_control(w: &mut World) {
    w.pc1.controller.on_intent(Intent::LetControl {
        peer: ComputerId::from("mac"),
        device: shared(),
    });
    w.pump();
}

#[test]
fn act4_a_control_request_makes_the_receiver_attach() {
    let mut w = World::new();
    let_control(&mut w);
    // The Mac opens a portal on the matching display and attaches (ACT-4).
    assert_eq!(w.mac.controller.take_portal_window_request(), Some(lg()));
    assert_eq!(handoff_jobs(&w.mac), 0);
    assert_eq!(w.phase(), waiting_for_ack());
    assert!(
        !w.pc1
            .controller
            .control_request_pending(&ComputerId::from("mac"), &shared()),
        "the attach request ends the wait"
    );
    // The prompt shows on pc1, the asking peer, and the Mac keyboard acknowledges.
    assert!(w.pc1.log.borrow().prompt_visible);
    w.advance_to_forwarding_from_confirmation();
    assert_eq!(w.phase(), Some(SessionPhase::Forwarding));
    assert_eq!(w.pc1.controller.control_screen().incoming.len(), 1);
}

#[test]
fn act6_a_control_request_waits_15_seconds() {
    let mut w = World::new();
    w.pc1.controller.on_intent(Intent::LetControl {
        peer: ComputerId::from("mac"),
        device: shared(),
    });
    assert!(
        w.pc1
            .controller
            .control_request_pending(&ComputerId::from("mac"), &shared())
    );
    // The Mac never answers.
    w.pc1.log.borrow_mut().net.clear();
    w.now += Duration::from_secs(14);
    w.pc1.controller.tick(w.now);
    assert!(
        w.pc1
            .controller
            .control_request_pending(&ComputerId::from("mac"), &shared())
    );
    w.now += Duration::from_secs(2);
    w.pc1.controller.tick(w.now);
    assert!(
        !w.pc1
            .controller
            .control_request_pending(&ComputerId::from("mac"), &shared())
    );
    assert!(has_notice(&w.pc1, "did not answer"));
}

/// The asking peer got `ControlRefused` and shows a notice with this text.
fn refused(w: &World, text: &str) {
    assert!(
        !w.pc1
            .controller
            .control_request_pending(&ComputerId::from("mac"), &shared())
    );
    assert!(
        has_notice(&w.pc1, text),
        "{:?}",
        w.pc1.controller.control_screen().notices
    );
    assert!(!w.mac.controller.has_portal());
}

#[test]
fn act6_a_control_request_with_no_capture_is_refused() {
    let mut w = World::new();
    w.mac = peer_with(
        "mac",
        vec![display("lg", 0.0, 3400.0)],
        InputCapabilities {
            capture: Capability::Available,
            suppress: Capability::Missing("grant Accessibility".into()),
            inject: Capability::Available,
        },
    );
    let_control(&mut w);
    refused(&w, "cannot capture");
}

#[test]
fn act6_a_control_request_with_two_matching_displays_is_refused() {
    let mut w = World::new();
    w.mac = peer(
        "mac",
        vec![
            display("lg", 0.0, 3400.0),
            display("HDMI-A-2", 3400.0, 2048.0),
        ],
    );
    let_control(&mut w);
    refused(&w, "more than one display");
}

#[test]
fn act6_a_control_request_with_no_matching_display_is_refused() {
    let mut w = World::new();
    w.mac = peer("mac", vec![display("builtin", -1500.0, 1500.0)]);
    let_control(&mut w);
    refused(&w, "no display");
}

#[test]
fn act6_a_control_request_while_the_receiver_is_a_source_is_refused() {
    let mut w = World::new();
    w.advance_to_forwarding();
    let_control(&mut w);
    assert!(has_notice(&w.pc1, "already controls"));
    assert_eq!(
        w.phase(),
        Some(SessionPhase::Forwarding),
        "the session stays"
    );
}

#[test]
fn act6_a_control_request_with_no_route_is_refused() {
    let mut w = World::new();
    w.extra.push(pc4());
    // The Mac has no handoff.pc4 route for the device.
    w.extra[0].controller.on_intent(Intent::LetControl {
        peer: ComputerId::from("mac"),
        device: shared(),
    });
    w.pump();
    assert!(has_notice(&w.extra[0], "no route"));
    assert!(!w.mac.controller.has_portal());
}

#[test]
fn act6_a_control_request_for_an_unknown_device_is_refused() {
    let mut w = World::new();
    w.mac.controller.on_net(
        NetEvent::Message {
            peer: ComputerId::from("pc1"),
            message: Message::ControlRequest {
                device: DeviceId::from("nope"),
            },
        },
        w.now,
    );
    let sent = w.mac.log.borrow().net.iter().any(|c| {
        matches!(
            c,
            NetCommand::Send {
                message: Message::ControlRefused {
                    reason: telescreen_core::wire::ControlRefuseReason::UnknownDevice,
                    ..
                },
                ..
            }
        )
    });
    assert!(sent);
}

#[test]
fn act6_ac9_a_portal_match_error_refuses_the_control_request() {
    let mut w = World::new();
    w.mac = peer(
        "mac",
        vec![
            display("broken", -1500.0, 1500.0),
            display("lg", 0.0, 3400.0),
        ],
    );
    let_control(&mut w);
    refused(&w, "portal-match failed");
}

#[test]
fn act6_an_action_dials_when_the_display_matches_two_devices() {
    let mut w = world_with(|config| {
        let mac = config.computers.get_mut(&ComputerId::from("mac")).unwrap();
        let usb: telescreen_core::config::Device = serde_json::from_value(json!({
            "id": "usb-switch",
            "portal-match": {"driver": "command", "command": ["/bin/match"]}
        }))
        .unwrap();
        mac.devices.insert("Usb".into(), usb);
    });
    act(
        &mut w,
        Intent::ControlOnly {
            display: lg(),
            device: shared(),
            target: ComputerId::from("pc1"),
        },
    );
    assert_eq!(w.phase(), waiting_for_ack());
}

#[test]
fn act4_in8_the_safety_exit_ends_the_wait_of_a_control_request() {
    let mut w = World::new();
    let_control(&mut w);
    assert_eq!(w.phase(), waiting_for_ack());
    w.mac.controller.on_captured(CapturedEvent::SafetyExit);
    w.pump();
    assert!(!w.mac.controller.has_portal(), "ACT-5");
    assert_eq!(w.mac.log.borrow().capture.last().unwrap(), "disarm");
    assert!(!w.pc1.log.borrow().prompt_visible);
    assert!(w.pc1.controller.control_screen().incoming.is_empty());
}

#[test]
fn act4_ver5_the_prompt_timeout_ends_the_wait_of_a_control_request() {
    let mut w = World::new();
    let_control(&mut w);
    w.pc1.controller.tick(w.now + Duration::from_secs(61));
    w.pump();
    assert!(!w.pc1.log.borrow().prompt_visible);
    assert!(!w.mac.controller.has_portal(), "ACT-5");
    assert_eq!(w.mac.log.borrow().capture.last().unwrap(), "disarm");
}

// ----- UI-18: the menu view model -----

fn evaluated() -> World {
    let mut w = World::new();
    w.mac.controller.evaluate_displays();
    w.pump();
    w
}

fn lg_row(peer: &Peer) -> telescreen_view::MenuDisplay {
    peer.controller
        .menu_screen()
        .displays
        .into_iter()
        .find(|d| d.token == lg())
        .expect("a row for the LG display")
}

fn pc1_candidate(peer: &Peer) -> telescreen_view::MenuCandidate {
    let row = lg_row(peer);
    assert_eq!(row.devices.len(), 1);
    row.devices[0]
        .candidates
        .iter()
        .find(|c| c.target == ComputerId::from("pc1"))
        .cloned()
        .expect("pc1 is a candidate")
}

#[test]
fn ui18_no_display_rows_before_the_first_evaluation() {
    let w = World::new();
    assert!(w.mac.controller.menu_screen().displays.is_empty());
    assert!(
        w.mac.log.borrow().jobs.is_empty(),
        "no portal-match at the start"
    );
}

#[test]
fn ui18_each_display_shows_its_matching_devices_and_candidates() {
    let w = evaluated();
    let menu = w.mac.controller.menu_screen();
    let tokens: Vec<_> = menu.displays.iter().map(|d| d.token.0.as_str()).collect();
    assert_eq!(tokens, ["builtin", "lg"], "stable ids: the display tokens");
    let builtin = &menu.displays[0];
    assert_eq!(builtin.matching, telescreen_view::DisplayMatching::Done);
    assert!(builtin.devices.is_empty());
    let lg = lg_row(&w.mac);
    assert_eq!(lg.devices[0].device, shared());
    assert_eq!(lg.devices[0].alias, "Desktop");
    // NET-10: the evaluation dialed the candidate.
    let candidate = pc1_candidate(&w.mac);
    assert_eq!(candidate.label, "Linux");
    assert_eq!(candidate.status, "online");
    assert_eq!(candidate.switch_and_control, None);
    assert_eq!(candidate.switch_display_only, None);
    assert_eq!(candidate.control_only, None);
}

#[test]
fn ui18_an_offline_target_leaves_only_switch_display_only() {
    let mut w = world_with(|config| endpoint_of(config, "pc1").hosts.clear());
    w.mac.controller.evaluate_displays();
    w.pump();
    let candidate = pc1_candidate(&w.mac);
    assert_eq!(candidate.status, "offline");
    assert_eq!(candidate.switch_and_control.as_deref(), Some("offline"));
    assert_eq!(candidate.control_only.as_deref(), Some("offline"));
    assert_eq!(candidate.switch_display_only, None, "ACT-6");
}

#[test]
fn ui18_missing_capture_leaves_only_switch_display_only() {
    let mut w = World::new();
    w.mac = peer_with(
        "mac",
        vec![display("lg", 0.0, 3400.0)],
        InputCapabilities {
            capture: Capability::Missing("grant Input Monitoring".into()),
            suppress: Capability::Available,
            inject: Capability::Available,
        },
    );
    w.mac.controller.evaluate_displays();
    w.pump();
    let candidate = pc1_candidate(&w.mac);
    assert!(
        candidate
            .switch_and_control
            .unwrap()
            .contains("input capture")
    );
    assert!(candidate.control_only.unwrap().contains("input capture"));
    assert_eq!(candidate.switch_display_only, None);
}

#[test]
fn ui18_a_stale_portal_match_result_is_dropped() {
    let mut w = World::new();
    w.mac.controller.evaluate_displays();
    // The first job is the built-in display. Keep it, and evaluate again.
    let (old, _) = w.mac.log.borrow_mut().jobs.pop_front().unwrap();
    w.mac.controller.evaluate_displays();
    w.pump();
    w.mac
        .controller
        .on_job_done(old, JobOutput::Operation(Ok(Outcome::Matched(true))));
    let menu = w.mac.controller.menu_screen();
    assert!(
        menu.displays[0].devices.is_empty(),
        "the old result has no effect"
    );
}

#[test]
fn ui18_refresh_evaluates_again_only_after_the_first_evaluation() {
    let mut w = evaluated();
    assert!(w.mac.log.borrow().jobs.is_empty());
    w.mac.controller.on_intent(Intent::Refresh);
    assert!(!w.mac.log.borrow().jobs.is_empty());
}

#[test]
fn ui18_the_session_rows_and_the_glyph() {
    let mut w = World::new();
    assert_eq!(
        w.mac.controller.menu_screen().glyph,
        telescreen_view::Glyph::Idle
    );
    w.advance_to_forwarding();
    let mac = w.mac.controller.menu_screen();
    assert_eq!(mac.glyph, telescreen_view::Glyph::Controlling);
    assert_eq!(mac.sessions.len(), 1);
    assert_eq!(mac.sessions[0].id, telescreen_view::MenuSessionId::Portal);
    assert_eq!(mac.sessions[0].peer, ComputerId::from("pc1"));
    assert!(matches!(
        mac.sessions[0].role,
        telescreen_view::MenuRole::Source {
            phase: SessionPhase::Forwarding,
            ..
        }
    ));
    let pc1 = w.pc1.controller.menu_screen();
    assert_eq!(pc1.glyph, telescreen_view::Glyph::Controlled);
    let session = w.pc1.controller.control_screen().incoming[0].session;
    assert_eq!(
        pc1.sessions[0].id,
        telescreen_view::MenuSessionId::Incoming { session }
    );
    assert!(matches!(
        pc1.sessions[0].role,
        telescreen_view::MenuRole::Target {
            hand_back: None,
            ..
        }
    ));
    act(&mut w, Intent::EndSession);
    assert_eq!(
        w.mac.controller.menu_screen().glyph,
        telescreen_view::Glyph::Idle
    );
    assert!(w.mac.controller.menu_screen().sessions.is_empty());
}

#[test]
fn ui9_hb3_hand_back_from_the_session_part_of_the_source() {
    let mut w = World::new();
    w.advance_to_forwarding();
    act(&mut w, Intent::SourceHandBack);
    assert!(w.pc1.controller.control_screen().incoming.is_empty());
    assert!(
        w.mac.controller.awaiting_local_ack(),
        "HB-1: the Mac verifies"
    );
}

fn request_rows(peer: &Peer) -> Vec<(String, bool)> {
    peer.controller
        .menu_screen()
        .control_requests
        .into_iter()
        .map(|r| (r.peer.0, r.waiting))
        .collect()
}

#[test]
fn ui18_let_control_rows_and_the_waiting_state() {
    let mut w = World::new();
    // The Mac and pc3 have handoff.pc1 for the device.
    assert_eq!(
        request_rows(&w.pc1),
        [("mac".to_string(), false), ("pc3".to_string(), false)]
    );
    w.pc1.controller.on_intent(Intent::LetControl {
        peer: ComputerId::from("mac"),
        device: shared(),
    });
    w.pc1.log.borrow_mut().net.clear();
    assert_eq!(request_rows(&w.pc1)[0], ("mac".to_string(), true));
    w.pc1.controller.tick(w.now + Duration::from_secs(16));
    assert_eq!(request_rows(&w.pc1)[0], ("mac".to_string(), false));
}

#[test]
fn ui18_a_refusal_ends_the_waiting_state() {
    let mut w = World::new();
    w.mac = peer("mac", vec![display("builtin", -1500.0, 1500.0)]);
    let_control(&mut w);
    assert_eq!(request_rows(&w.pc1)[0], ("mac".to_string(), false));
}

#[test]
fn ui13_the_main_window_lists_the_computers_and_their_trust() {
    let mut w = World::new();
    let row = |w: &World, id: &str| {
        w.mac
            .controller
            .main_screen()
            .computers
            .into_iter()
            .find(|c| c.id == ComputerId::from(id))
            .unwrap()
    };
    assert!(row(&w, "mac").this_computer);
    assert!(!row(&w, "pc1").trusted, "the app sets the trust state");
    assert!(!row(&w, "pc1").connected);
    w.mac
        .controller
        .set_trusted([ComputerId::from("pc1")].into_iter().collect());
    w.open_portal();
    assert!(row(&w, "pc1").trusted);
    assert!(row(&w, "pc1").connected);
    assert_eq!(w.mac.controller.main_screen().devices.len(), 1);
}

#[test]
fn ui18_the_same_display_list_does_not_evaluate_again() {
    // The window layer calls `displays_changed` every 2 seconds.
    let mut w = evaluated();
    w.mac.controller.displays_changed();
    assert!(
        w.mac.log.borrow().jobs.is_empty(),
        "no portal-match helper runs"
    );
}
