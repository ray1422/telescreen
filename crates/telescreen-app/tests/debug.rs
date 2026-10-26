//! The debug protocol (docs/debug-protocol.md). Run with
//! `cargo test -p telescreen-app --features debug-protocol --test debug`.
#![cfg(all(unix, feature = "debug-protocol"))]
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use serde_json::{Value, json};
use telescreen_app::debug::{self, Request, StateReport, WindowFacts, WindowRole, parse_request};
use telescreen_core::ids::{ComputerId, DisplayToken, SessionId};
use telescreen_view::{ControlScreen, Intent};

fn parse(line: &str) -> Request {
    parse_request(line).unwrap_or_else(|e| panic!("{line}: {e}"))
}

#[test]
fn requests_without_arguments() {
    assert_eq!(parse(r#"{"cmd":"state"}"#), Request::State);
    assert_eq!(parse(r#"{"cmd":"confirm"}"#), Request::Confirm);
    assert_eq!(parse(r#"{"cmd":"quit"}"#), Request::Quit);
}

#[test]
fn intents() {
    let cases = [
        (json!("OpenPortal"), Intent::OpenPortal),
        (json!("ClosePortal"), Intent::ClosePortal),
        (json!("EndSession"), Intent::EndSession),
        (
            json!({"StartHandoff": {"target": "linux"}}),
            Intent::StartHandoff {
                target: ComputerId::from("linux"),
            },
        ),
        (
            json!({"SelectDevice": {"alias": "LG"}}),
            Intent::SelectDevice { alias: "LG".into() },
        ),
        (
            json!({"Disconnect": {"session": 7}}),
            Intent::Disconnect {
                session: SessionId(7),
            },
        ),
        (
            json!({"TakeControl": {"session": 7}}),
            Intent::TakeControl {
                session: SessionId(7),
            },
        ),
    ];
    for (intent, expected) in cases {
        let line = json!({"cmd": "intent", "intent": intent}).to_string();
        assert_eq!(parse(&line), Request::Intent(expected), "{line}");
    }
}

#[test]
fn bad_requests_are_errors() {
    for line in [
        "",
        "state",
        r#"{"cmd":"nope"}"#,
        r#"{"cmd":"intent"}"#,
        r#"{"cmd":"intent","intent":"Nope"}"#,
        r#"{"cmd":"intent","intent":{"StartHandoff":{}}}"#,
        r#"{"cmd":"intent","intent":{"Disconnect":{"session":"x"}}}"#,
        r#"{"cmd":"intent","intent":{"Disconnect":{"session":-1}}}"#,
        r#"["state"]"#,
    ] {
        assert!(parse_request(line).is_err(), "{line:?} must be an error");
    }
}

fn report() -> StateReport {
    StateReport {
        control: ControlScreen {
            computer: "MacBook".into(),
            fingerprint: "sha256:00".into(),
            displays: vec![],
            incoming: vec![],
            notices: vec!["a notice".into()],
        },
        portal: None,
        windows: vec![
            WindowFacts {
                role: WindowRole::Control,
                monitor: Some(DisplayToken::from("cg-1")),
                fullscreen: false,
                focused: true,
                position: Some((10.0, 20.0)),
                size: (560.0, 520.0),
            },
            WindowFacts {
                role: WindowRole::Portal,
                monitor: None,
                fullscreen: true,
                focused: false,
                position: None,
                size: (1706.0, 720.0),
            },
        ],
    }
}

fn line_json(line: &str) -> Value {
    assert!(!line.contains('\n'), "one line: {line:?}");
    serde_json::from_str(line).unwrap_or_else(|e| panic!("{line}: {e}"))
}

#[test]
fn state_response() {
    let v = line_json(&debug::state_response(&report()));
    assert_eq!(v["ok"], json!(true));
    assert!(v["control"].as_str().unwrap().contains("a notice"));
    assert_eq!(v["portal"], Value::Null);
    assert_eq!(
        v["windows"][0],
        json!({"role": "control", "monitor": "cg-1", "fullscreen": false,
               "focused": true, "position": [10.0, 20.0], "size": [560.0, 520.0]})
    );
    assert_eq!(v["windows"][1]["role"], json!("portal"));
    assert_eq!(v["windows"][1]["monitor"], Value::Null);
    assert_eq!(v["windows"][1]["position"], Value::Null);
}

#[test]
fn ok_and_error_responses() {
    assert_eq!(line_json(&debug::ok_response()), json!({"ok": true}));
    let v = line_json(&debug::error_response("bad \"x\"\nnext"));
    assert_eq!(v, json!({"ok": false, "error": "bad \"x\"\nnext"}));
}

fn socket_path(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("telescreen-debug-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir.join("s")
}

fn ask(path: &PathBuf, line: &str) -> Value {
    let mut stream = UnixStream::connect(path).unwrap();
    writeln!(stream, "{line}").unwrap();
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply).unwrap();
    line_json(reply.trim_end())
}

#[test]
fn server_answers_each_line() {
    let path = socket_path("serve");
    let server = debug::serve(&path, |request| match request {
        Request::State => debug::state_response(&report()),
        _ => debug::ok_response(),
    })
    .unwrap();
    let meta = std::fs::metadata(&path).unwrap();
    assert!(meta.file_type().is_socket());
    assert_eq!(meta.permissions().mode() & 0o777, 0o600, "owner only");

    assert_eq!(ask(&path, r#"{"cmd":"quit"}"#), json!({"ok": true}));
    assert_eq!(ask(&path, r#"{"cmd":"state"}"#)["ok"], json!(true));
    let bad = ask(&path, "nope");
    assert_eq!(bad["ok"], json!(false));
    assert!(bad["error"].is_string());

    // Two requests on one connection get two replies in order.
    let mut stream = UnixStream::connect(&path).unwrap();
    writeln!(stream, r#"{{"cmd":"quit"}}"#).unwrap();
    writeln!(stream, r#"{{"cmd":"state"}}"#).unwrap();
    let mut reader = BufReader::new(stream);
    let mut first = String::new();
    let mut second = String::new();
    reader.read_line(&mut first).unwrap();
    reader.read_line(&mut second).unwrap();
    assert_eq!(line_json(first.trim_end()), json!({"ok": true}));
    assert!(line_json(second.trim_end())["windows"].is_array());

    drop(server);
    assert!(!path.exists(), "the server removes its socket");
}

#[test]
fn server_replaces_a_stale_socket() {
    let path = socket_path("stale");
    let old = std::os::unix::net::UnixListener::bind(&path).unwrap();
    drop(old);
    let _server = debug::serve(&path, |_| debug::ok_response()).unwrap();
    assert_eq!(ask(&path, r#"{"cmd":"quit"}"#), json!({"ok": true}));
}

#[test]
fn server_does_not_remove_a_file() {
    let path = socket_path("file");
    std::fs::write(&path, "keep").unwrap();
    assert!(debug::serve(&path, |_| debug::ok_response()).is_err());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep");
}
