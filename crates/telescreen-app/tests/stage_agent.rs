//! The agent of a stage process with fake ports (STG-2 to STG-5, STG-9).
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use serde_json::json;
use telescreen_app::controller::NetPort;
use telescreen_app::stage_agent::{KeyInjector, StageAgent, StageError, StagePorts};
use telescreen_core::config::Config;
use telescreen_core::ids::{ComputerId, DeviceId, SessionId};
use telescreen_core::input::{Button, InputEvent};
use telescreen_core::keys::Key;
use telescreen_core::platform::{Discovery, PlatformError};
use telescreen_core::pointer::Normalized;
use telescreen_core::wire::{EndReason, Message, PointerDatagram, RefuseReason, Stage};
use telescreen_transport::{NetCommand, NetEvent};

const A: Key = Key(30);
const FINGERPRINT: &str = "sha256:00";

#[derive(Default)]
struct Log {
    net: Vec<NetCommand>,
    shutdowns: usize,
    keys: Vec<(Key, bool)>,
    keys_fail: bool,
    discovery: Vec<String>,
    discovery_fails: bool,
}

type Shared = Rc<RefCell<Log>>;

struct Net(Shared);
impl NetPort for Net {
    fn send(&mut self, command: NetCommand) {
        self.0.borrow_mut().net.push(command);
    }
    fn shutdown(&mut self) {
        self.0.borrow_mut().shutdowns += 1;
    }
}

struct Keys(Shared);
impl KeyInjector for Keys {
    fn key(&mut self, key: Key, down: bool) -> Result<(), PlatformError> {
        let mut log = self.0.borrow_mut();
        if log.keys_fail {
            return Err(PlatformError::Permission("/dev/uinput".into()));
        }
        log.keys.push((key, down));
        Ok(())
    }
}

struct Disc(Shared);
impl Discovery for Disc {
    fn announce(&mut self, port: u16, fingerprint: &str) -> Result<(), PlatformError> {
        let mut log = self.0.borrow_mut();
        if log.discovery_fails {
            return Err(PlatformError::Unsupported("no responder".into()));
        }
        log.discovery.push(format!("announce {port} {fingerprint}"));
        Ok(())
    }
    fn stop_announce(&mut self) {
        self.0.borrow_mut().discovery.push("stop_announce".into());
    }
    fn search(&mut self) -> Result<(), PlatformError> {
        self.0.borrow_mut().discovery.push("search".into());
        Ok(())
    }
    fn stop_search(&mut self) {
        self.0.borrow_mut().discovery.push("stop_search".into());
    }
}

/// pc1 has the login stage. The Mac and pc2 can control it; pc3 cannot.
fn config(discovery: bool) -> Config {
    let endpoint = |host: &str| {
        if discovery {
            json!({"host": host, "port": 24800, "discovery": "mdns"})
        } else {
            json!({"host": host, "port": 24800})
        }
    };
    let device = json!({"Screen": {"id": "shared-lg"}});
    serde_json::from_value(json!({
        "version": 1,
        "computers": {
            "mac": {"label": "MacBook", "endpoint": endpoint("mac.test"), "devices": device},
            "pc1": {
                "label": "Linux",
                "endpoint": endpoint("pc1.test"),
                "stages": {"login": {"from": ["mac", "pc2"]}},
                "devices": device
            },
            "pc2": {"endpoint": endpoint("pc2.test"), "devices": device},
            "pc3": {"endpoint": endpoint("pc3.test"), "devices": device}
        }
    }))
    .unwrap()
}

fn agent_with(config: Config, local: &str) -> (Result<StageAgent, StageError>, Shared) {
    let log: Shared = Rc::default();
    let ports = StagePorts {
        net: Box::new(Net(log.clone())),
        keys: Box::new(Keys(log.clone())),
        discovery: Box::new(Disc(log.clone())),
    };
    let agent = StageAgent::new(config, ComputerId::from(local), FINGERPRINT.into(), ports);
    (agent, log)
}

fn agent() -> (StageAgent, Shared) {
    let (agent, log) = agent_with(config(true), "pc1");
    (agent.expect("pc1 has the login stage"), log)
}

fn mac() -> ComputerId {
    ComputerId::from("mac")
}

fn device() -> DeviceId {
    DeviceId::from("shared-lg")
}

fn from(agent: &mut StageAgent, peer: &str, message: Message) {
    agent.on_net(
        NetEvent::Message {
            peer: ComputerId::from(peer),
            message,
        },
        Instant::now(),
    );
}

/// The messages that the agent sent to `peer`, in sequence. It removes them.
fn sent(log: &Shared, peer: &str) -> Vec<Message> {
    let mut log = log.borrow_mut();
    let mut messages = Vec::new();
    log.net.retain(|command| match command {
        NetCommand::Send { peer: to, message } if to.0 == peer => {
            messages.push(message.clone());
            false
        }
        _ => true,
    });
    messages
}

/// Connects the Mac and starts a stage session. Returns the session ID.
fn forwarding(agent: &mut StageAgent, log: &Shared) -> SessionId {
    agent.on_net(
        NetEvent::Connected {
            peer: mac(),
            stage: None,
        },
        Instant::now(),
    );
    from(agent, "mac", Message::RequestSession { device: device() });
    let granted = sent(log, "mac");
    let [Message::SessionGranted { session, device: d }] = granted.as_slice() else {
        panic!("a grant: {granted:?}");
    };
    assert_eq!(d, &device());
    let session = *session;
    from(agent, "mac", Message::HandoffDone { session });
    assert_eq!(
        sent(log, "mac"),
        [Message::Started {
            session,
            verified: false,
        }]
    );
    session
}

fn key(session: SessionId, key: Key, down: bool) -> Message {
    Message::Input {
        session,
        event: InputEvent::Key { key, down },
    }
}

#[test]
fn stg2_the_agent_needs_the_login_stage_in_the_config_of_this_computer() {
    let (agent, _) = agent_with(config(true), "mac");
    assert_eq!(agent.err(), Some(StageError::NoStage(mac())));
    let (agent, _) = agent_with(config(true), "absent");
    assert_eq!(
        agent.err(),
        Some(StageError::UnknownComputer(ComputerId::from("absent")))
    );
    let (agent, _) = agent_with(config(true), "pc1");
    assert!(agent.is_ok());
    assert_eq!(StageAgent::STAGE, Stage::Login);
}

#[test]
fn stg2_the_start_makes_the_computer_discoverable_and_says_so() {
    let (mut agent, log) = agent();
    let lines = agent.start(Instant::now());
    assert_eq!(
        log.borrow().discovery,
        [format!("announce 24800 {FINGERPRINT}")]
    );
    let text = lines.join("\n");
    assert!(text.contains("discoverable for 10 minutes"), "{text}");
    assert!(text.contains("MacBook"), "the sources by label: {text}");
    assert!(text.contains("pc2"), "a source with no label by ID: {text}");
    assert!(text.contains("Control+C"), "how to stop: {text}");
    assert!(!agent.is_finished());
}

#[test]
fn stg2_no_discovery_in_the_config_means_no_record() {
    let (agent, log) = agent_with(config(false), "pc1");
    let mut agent = agent.unwrap();
    let text = agent.start(Instant::now()).join("\n");
    assert!(log.borrow().discovery.is_empty());
    assert!(text.contains("Discovery is off"), "{text}");
}

#[test]
fn stg2_a_failed_discovery_start_is_a_line_and_not_an_error() {
    let (mut agent, log) = agent();
    log.borrow_mut().discovery_fails = true;
    let text = agent.start(Instant::now()).join("\n");
    assert!(text.contains("no responder"), "{text}");
    assert!(!agent.is_finished());
}

#[test]
fn net14_the_discovery_record_ends_after_10_minutes() {
    let (mut agent, log) = agent();
    let start = Instant::now();
    agent.start(start);
    agent.tick(start + Duration::from_secs(599));
    assert_eq!(log.borrow().discovery.len(), 1, "still discoverable");
    agent.tick(start + Duration::from_secs(600));
    assert_eq!(
        log.borrow().discovery.last().map(String::as_str),
        Some("stop_announce")
    );
    agent.tick(start + Duration::from_secs(700));
    assert_eq!(log.borrow().discovery.len(), 2, "one stop");
    assert!(!agent.is_finished(), "STG-4: the stage has no time limit");
}

#[test]
fn stg5_a_permitted_source_gets_a_session_and_its_keys_are_injected() {
    let (mut agent, log) = agent();
    let session = forwarding(&mut agent, &log);
    from(&mut agent, "mac", key(session, A, true));
    from(&mut agent, "mac", key(session, A, false));
    assert_eq!(log.borrow().keys, [(A, true), (A, false)]);
    assert!(sent(&log, "mac").is_empty());
}

#[test]
fn stg5_net4_a_source_that_from_does_not_list_is_refused() {
    let (mut agent, log) = agent();
    from(
        &mut agent,
        "pc3",
        Message::RequestSession { device: device() },
    );
    assert_eq!(
        sent(&log, "pc3"),
        [Message::SessionRefused {
            device: device(),
            reason: RefuseReason::NoRoute,
        }]
    );
    // No session exists: a key of pc3 goes nowhere.
    from(&mut agent, "pc3", key(SessionId(1), A, true));
    assert!(log.borrow().keys.is_empty());
}

#[test]
fn stg5_one_session_at_a_time() {
    let (mut agent, log) = agent();
    let session = forwarding(&mut agent, &log);
    from(
        &mut agent,
        "pc2",
        Message::RequestSession { device: device() },
    );
    assert_eq!(
        sent(&log, "pc2"),
        [Message::SessionRefused {
            device: device(),
            reason: RefuseReason::Busy,
        }]
    );
    // The session of the Mac continues.
    from(&mut agent, "mac", key(session, A, true));
    assert_eq!(log.borrow().keys, [(A, true)]);
}

#[test]
fn stg5_only_the_source_of_the_session_can_send_its_keys() {
    let (mut agent, log) = agent();
    let session = forwarding(&mut agent, &log);
    from(&mut agent, "pc2", key(session, A, true));
    from(
        &mut agent,
        "pc2",
        Message::End {
            session,
            reason: EndReason::SourceClosed,
        },
    );
    assert!(log.borrow().keys.is_empty());
    from(&mut agent, "mac", key(session, A, true));
    assert_eq!(log.borrow().keys, [(A, true)], "the session did not end");
}

#[test]
fn stg5_pointer_buttons_and_other_requests_do_nothing() {
    let (mut agent, log) = agent();
    let session = forwarding(&mut agent, &log);
    agent.on_net(
        NetEvent::Pointer {
            peer: mac(),
            datagram: PointerDatagram {
                session,
                seq: 1,
                sent_us: 0,
                position: Normalized { x: 0.5, y: 0.5 },
            },
        },
        Instant::now(),
    );
    let ignored = [
        Message::Input {
            session,
            event: InputEvent::Button {
                button: Button::Left,
                down: true,
            },
        },
        Message::ClipboardText {
            session,
            text: "secret".into(),
            concealed: false,
        },
        Message::ControlRequest { device: device() },
        Message::DisplaySwitched { device: device() },
        Message::HandBackRequest { session },
        Message::Heartbeat,
    ];
    for message in ignored {
        from(&mut agent, "mac", message);
    }
    assert!(log.borrow().keys.is_empty());
    assert!(sent(&log, "mac").is_empty());
    assert!(log.borrow().net.is_empty(), "{:?}", log.borrow().net);
}

#[test]
fn stg9_a_login_releases_the_keys_tells_the_source_and_finishes() {
    let (mut agent, log) = agent();
    agent.start(Instant::now());
    let session = forwarding(&mut agent, &log);
    from(&mut agent, "mac", key(session, A, true));
    agent.on_login();
    assert_eq!(log.borrow().keys, [(A, true), (A, false)], "IN-4");
    assert_eq!(
        sent(&log, "mac"),
        [Message::End {
            session,
            reason: EndReason::LoginComplete,
        }]
    );
    // STG-4: the connections close, the transport stops, and the record ends.
    assert!(
        log.borrow()
            .net
            .contains(&NetCommand::Close { peer: mac() }),
        "{:?}",
        log.borrow().net
    );
    assert_eq!(log.borrow().shutdowns, 1);
    assert_eq!(
        log.borrow().discovery.last().map(String::as_str),
        Some("stop_announce")
    );
    assert!(agent.is_finished());
    // Nothing more happens after the end.
    from(
        &mut agent,
        "mac",
        Message::RequestSession { device: device() },
    );
    agent.on_login();
    agent.stop();
    assert!(sent(&log, "mac").is_empty());
    assert_eq!(log.borrow().shutdowns, 1);
}

#[test]
fn stg9_a_login_with_no_session_only_finishes() {
    let (mut agent, log) = agent();
    agent.on_login();
    assert!(agent.is_finished());
    assert!(log.borrow().keys.is_empty());
    assert!(
        !log.borrow()
            .net
            .iter()
            .any(|c| matches!(c, NetCommand::Send { .. })),
        "{:?}",
        log.borrow().net
    );
    assert_eq!(log.borrow().shutdowns, 1);
}

#[test]
fn stg4_a_stop_ends_the_session_as_a_target_that_quit() {
    let (mut agent, log) = agent();
    let session = forwarding(&mut agent, &log);
    from(&mut agent, "mac", key(session, A, true));
    agent.stop();
    assert_eq!(log.borrow().keys, [(A, true), (A, false)]);
    assert_eq!(
        sent(&log, "mac"),
        [Message::End {
            session,
            reason: EndReason::TargetDisconnected,
        }]
    );
    assert_eq!(log.borrow().shutdowns, 1);
    assert!(agent.is_finished());
}

#[test]
fn stg10_a_lost_source_releases_the_keys_and_the_stage_continues() {
    let (mut agent, log) = agent();
    let session = forwarding(&mut agent, &log);
    from(&mut agent, "mac", key(session, A, true));
    agent.on_net(
        NetEvent::Disconnected {
            peer: mac(),
            reason: "timeout".into(),
        },
        Instant::now(),
    );
    assert_eq!(log.borrow().keys, [(A, true), (A, false)]);
    assert!(!agent.is_finished());
    assert!(sent(&log, "mac").is_empty());
    // A new session is possible, with a new session ID.
    let next = forwarding(&mut agent, &log);
    assert_ne!(next, session);
}

#[test]
fn stg10_an_end_from_the_source_frees_the_stage_for_the_next_session() {
    let (mut agent, log) = agent();
    let session = forwarding(&mut agent, &log);
    from(
        &mut agent,
        "mac",
        Message::End {
            session,
            reason: EndReason::SourceClosed,
        },
    );
    from(
        &mut agent,
        "pc2",
        Message::RequestSession { device: device() },
    );
    assert!(
        matches!(
            sent(&log, "pc2").as_slice(),
            [Message::SessionGranted { .. }]
        ),
        "the stage is free"
    );
}

#[test]
fn stg5_a_key_that_cannot_be_injected_ends_the_session() {
    let (mut agent, log) = agent();
    let session = forwarding(&mut agent, &log);
    log.borrow_mut().keys_fail = true;
    from(&mut agent, "mac", key(session, A, true));
    assert_eq!(
        sent(&log, "mac"),
        [Message::End {
            session,
            reason: EndReason::PermissionLost,
        }]
    );
    assert!(!agent.is_finished());
}
