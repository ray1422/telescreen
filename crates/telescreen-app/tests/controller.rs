//! Two controllers with fake ports and an in-memory network: the user flow of
//! `docs/spec.md` section 4 across two peers.
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;
use std::time::Instant;

use serde_json::json;
use telescreen_app::controller::{
    Controller, Job, JobId, JobOutput, JobPort, Labels, NetPort, Ports,
};
use telescreen_core::clipboard::{ClipboardChange, ClipboardItem};
use telescreen_core::config::Config;
use telescreen_core::driver::{OperationKind, Outcome};
use telescreen_core::ids::{ComputerId, DeviceId, DisplayToken, SessionId};
use telescreen_core::input::InputEvent;
use telescreen_core::keys::Key;
use telescreen_core::platform::{
    Capability, CapturedEvent, Clipboard, DisplayInfo, Displays, FocusHolder, Injector,
    InputCapabilities, InputCapture, PlatformError,
};
use telescreen_core::pointer::{Normalized, Point, Rect};
use telescreen_core::session::VideoInput;
use telescreen_core::session::source::HandoffResult;
use telescreen_core::wire::{EndReason, Message, RefuseReason};
use telescreen_transport::{NetCommand, NetEvent};
use telescreen_view::{Intent, Matching, PromptPresenter, PromptScreen, SessionPhase};

fn config() -> Config {
    let op =
        |name: &str| json!({"driver": "command", "options": {"argv": [format!("/bin/{name}")]}});
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
                        "handoff": {"pc1": {"steps": [op("to-pc1")]}}
                    }
                }
            },
            "pc1": {
                "label": "Linux",
                "hint": "Copy: Ctrl+C",
                "clipboard": {"text": true},
                "endpoint": {"host": "pc1.test", "port": 24800},
                "devices": {
                    "Monitor": {
                        "id": "shared-lg",
                        "portal-match": op("match"),
                        "input-target": op("target"),
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
            }
        }
    }))
    .unwrap()
}

#[derive(Default)]
struct Log {
    net: VecDeque<NetCommand>,
    jobs: VecDeque<(JobId, Job)>,
    capture: Vec<String>,
    injected: Vec<InputEvent>,
    pointer: Vec<Normalized>,
    prompts: Vec<String>,
    prompt_visible: bool,
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
        self.0.borrow_mut().jobs.push_back((id, job));
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

struct Prompt(Shared);
impl PromptPresenter for Prompt {
    fn show(&mut self, _: SessionId, _: &DisplayInfo, screen: &PromptScreen) -> Result<(), String> {
        let mut log = self.0.borrow_mut();
        log.prompts.push(format!(
            "{} | {}",
            screen.title(),
            screen.hint.clone().unwrap_or_default()
        ));
        log.prompt_visible = true;
        Ok(())
    }
    fn hide(&mut self, _: SessionId) {
        self.0.borrow_mut().prompt_visible = false;
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
        capabilities,
    };
    let labels = Labels {
        confirm_key: "Return".into(),
        release_shortcut: "Ctrl+Opt+Cmd+Esc".into(),
    };
    let controller = Controller::new(
        config(),
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

struct World {
    mac: Peer,
    pc1: Peer,
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
            now: Instant::now(),
            handoff: HandoffResult::Done { verified: None },
        }
    }

    /// Delivers network commands and completes jobs until nothing is left.
    fn pump(&mut self) {
        loop {
            let mut progress = false;
            for mac_first in [true, false] {
                let (a, b) = if mac_first {
                    (&mut self.mac, &mut self.pc1)
                } else {
                    (&mut self.pc1, &mut self.mac)
                };
                loop {
                    // Take the item first: the controller writes to the log while it runs.
                    let next = a.log.borrow_mut().net.pop_front();
                    let Some(command) = next else { break };
                    progress = true;
                    match command {
                        NetCommand::Dial { peer, .. } => {
                            assert_eq!(peer, b.id);
                            a.controller
                                .on_net(NetEvent::Connected { peer: b.id.clone() }, self.now);
                            b.controller
                                .on_net(NetEvent::Connected { peer: a.id.clone() }, self.now);
                        }
                        NetCommand::Send { peer, message } => {
                            assert_eq!(peer, b.id);
                            b.controller.on_net(
                                NetEvent::Message {
                                    peer: a.id.clone(),
                                    message,
                                },
                                self.now,
                            );
                        }
                        NetCommand::SendPointer { peer, datagram } => {
                            assert_eq!(peer, b.id);
                            b.controller.on_net(
                                NetEvent::Pointer {
                                    peer: a.id.clone(),
                                    datagram,
                                },
                                self.now,
                            );
                        }
                        NetCommand::Close { peer } => {
                            assert_eq!(peer, b.id);
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
                            // Only the external display matches.
                            let token = request.portal.unwrap().display_token.unwrap();
                            JobOutput::Operation(Ok(Outcome::Matched(
                                token.0 == "lg" || token.0 == "HDMI-A-2",
                            )))
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
        assert!(self.mac.controller.confirm());
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
    assert_eq!(
        log.prompts,
        ["Press Return on MacBook to control this computer | Copy: Ctrl+C"]
    );
    assert!(log.prompt_visible);
    drop(log);
    assert_eq!(
        w.phase(),
        Some(SessionPhase::WaitingForConfirmation {
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
        ["arm 3400", "guard on"],
        "no forwarding before the confirmation; only the guard (FLOW-21)"
    );
}

#[test]
fn ac6_confirmation_needs_prompt_and_focus() {
    let mut w = World::new();
    w.open_portal();
    assert!(!w.mac.controller.confirm(), "no session: a normal key");
    w.mac.controller.start_handoff(ComputerId::from("pc1"));
    w.pump();
    w.mac.controller.set_portal_focus(false);
    assert!(!w.mac.controller.confirm(), "no focus: a normal key");
    w.mac.controller.set_portal_focus(true);
    assert!(w.mac.controller.confirm());
}

#[test]
fn flow10_ac5_forwarding_after_confirmation() {
    let mut w = World::new();
    w.advance_to_forwarding();
    assert!(!w.pc1.log.borrow().prompt_visible, "the prompt goes away");
    assert_eq!(
        w.mac.log.borrow().capture,
        ["arm 3400", "guard on", "guard off", "forward"]
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
    assert_eq!(w.mac.log.borrow().capture.last().unwrap(), "forward");
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
fn flow23_session_end_removes_the_holder() {
    let mut w = World::new();
    w.advance_to_forwarding();
    w.mac.controller.on_captured(CapturedEvent::KeyboardLeft);
    w.mac.controller.on_captured(CapturedEvent::ReleaseShortcut);
    w.pump();
    assert_eq!(w.pc1.log.borrow().target, ["hold HDMI-A-2", "release"]);
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
fn flow16_ac15_release_shortcut_ends_both_sides() {
    let mut w = World::new();
    w.advance_to_forwarding();
    w.mac
        .controller
        .on_captured(CapturedEvent::Input(key(29, true)));
    w.mac.controller.on_captured(CapturedEvent::ReleaseShortcut);
    w.pump();
    assert_eq!(
        w.phase(),
        Some(SessionPhase::Ended {
            reason: "release shortcut".into()
        })
    );
    assert_eq!(w.mac.log.borrow().capture.last().unwrap(), "disarm");
    assert_eq!(w.pc1.log.borrow().injected.last(), Some(&key(29, false)));
    assert!(
        w.pc1.controller.control_screen().incoming.is_empty(),
        "the lease is released"
    );
    // A new session can start: the lease is free.
    w.mac.controller.start_handoff(ComputerId::from("pc1"));
    w.pump();
    assert!(matches!(
        w.phase(),
        Some(SessionPhase::WaitingForConfirmation { .. })
    ));
}

#[test]
fn flow11_ac7_timeout_removes_prompt() {
    let mut w = World::new();
    w.advance_to_confirmation();
    w.mac
        .controller
        .tick(w.now + std::time::Duration::from_secs(61));
    w.pump();
    assert!(!w.pc1.log.borrow().prompt_visible);
    let session = w.mac.controller.portal_screen().unwrap().session.unwrap();
    assert_eq!(session.video_input, VideoInput::Unknown);
}

#[test]
fn flow21_ac26_guard_from_handoff_to_confirmation() {
    let mut w = World::new();
    w.advance_to_confirmation();
    assert_eq!(w.mac.log.borrow().capture, ["arm 3400", "guard on"]);
    assert!(w.mac.controller.guarded());
}

#[test]
fn flow21_guard_ends_with_the_timeout() {
    let mut w = World::new();
    w.advance_to_confirmation();
    w.mac
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
    assert_eq!(
        w.phase(),
        Some(SessionPhase::Ended {
            reason: "the handoff failed".into()
        })
    );
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
    assert_eq!(
        w.phase(),
        Some(SessionPhase::Ended {
            reason: "the target stopped the session".into()
        })
    );
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
    assert_eq!(
        w.phase(),
        Some(SessionPhase::Ended {
            reason: "the target took control back".into()
        })
    );
    assert_eq!(w.mac.log.borrow().capture.last().unwrap(), "disarm");
    // pc1 is now the source: a portal on the display of the old session, and a
    // handoff to the old source that waits for the confirmation.
    let portal = w.pc1.controller.portal_screen().unwrap();
    assert_eq!(
        portal.session.unwrap().phase,
        SessionPhase::WaitingForConfirmation {
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
    assert_eq!(
        w.phase(),
        Some(SessionPhase::Ended {
            reason: "a new handoff of this display started".into()
        })
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
    assert_eq!(
        w.phase(),
        Some(SessionPhase::Ended {
            reason: "disconnected on the target".into()
        })
    );
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
    assert_eq!(
        w.phase(),
        Some(SessionPhase::Ended {
            reason: "connection lost".into()
        })
    );
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
    let screen = w.mac.controller.portal_screen().unwrap();
    assert_eq!(
        screen.matching,
        Matching::NoMatch,
        "portal-match ran again on the new display"
    );
    assert!(
        screen.candidates.is_empty(),
        "the old device binding does not apply"
    );
    assert_eq!(
        screen.session.unwrap().phase,
        SessionPhase::Ended {
            reason: "the portal display is gone".into()
        }
    );
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
    w.mac.controller.on_captured(CapturedEvent::ReleaseShortcut);
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
    for end in ["release shortcut", "stop on the target", "switch off"] {
        let mut w = World::new();
        w.advance_to_forwarding();
        copy(&w.mac, "a", false);
        settle(&mut w);
        match end {
            "release shortcut" => w.mac.controller.on_captured(CapturedEvent::ReleaseShortcut),
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
    w.mac.controller.on_captured(CapturedEvent::ReleaseShortcut);
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
