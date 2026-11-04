//! Verifiers: docs/config.md "`verify` in JSON lines", spec VER-3 and VER-5. Unix only.
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.
#![cfg(unix)]

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use telescreen_core::config::Program;
use telescreen_core::driver::{Context, DeviceRef, Request, SearchPath};
use telescreen_core::ids::ComputerId;
use telescreen_core::verifier::{
    PROMPT_TIMEOUT, PromptVerifier, VerifierEvent, VerifierLine, VerifierProcess, parse_line,
};

fn dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("telescreen-verifier-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn helper(body: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = dir().join(format!("v-{}", NEXT.fetch_add(1, Ordering::Relaxed)));
    // A child process writes the file. If this process wrote it, a child that a
    // parallel test forks could keep the write descriptor open until its exec.
    // Then the exec of this file fails with ETXTBSY (seen on Linux).
    let status = std::process::Command::new("/bin/sh")
        .args([
            "-c",
            r#"printf '#!/bin/sh\n%s\n' "$1" > "$2" && chmod 755 "$2""#,
        ])
        .args(["sh", body])
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success());
    path
}

fn request() -> Request {
    Request {
        computer_id: ComputerId::from("pc1"),
        device: DeviceRef {
            id: "shared-lg".into(),
            alias: "Monitor1".into(),
        },
        source_computer_id: ComputerId::from("mac"),
        target_computer_id: Some(ComputerId::from("pc1")),
        session_id: None,
        portal: None,
        displays: Vec::new(),
    }
}

fn context() -> Context {
    Context {
        default_cwd: dir(),
        search_path: SearchPath::from_env(),
    }
}

/// Starts a verifier script. The events arrive on the returned channel.
fn start(body: &str, timeout: Duration) -> (VerifierProcess, mpsc::Receiver<VerifierEvent>) {
    let program = Program {
        argv: vec![helper(body).to_string_lossy().into_owned()],
        cwd: None,
        env: Default::default(),
    };
    let (sender, events) = mpsc::channel();
    let process = VerifierProcess::spawn(&program, &request(), &context(), timeout, move |e| {
        let _ = sender.send(e);
    })
    .unwrap();
    (process, events)
}

fn next(events: &mpsc::Receiver<VerifierEvent>) -> VerifierEvent {
    events.recv_timeout(Duration::from_secs(5)).unwrap()
}

#[test]
fn ver3_lines() {
    assert_eq!(
        parse_line(r#"{"show":"Press the button"}"#),
        Ok(VerifierLine::Show("Press the button".into()))
    );
    assert_eq!(parse_line(r#"{"want":"ack"}"#), Ok(VerifierLine::WantAck));
    assert_eq!(parse_line(r#"{"ok":true}"#), Ok(VerifierLine::Result(true)));
    assert_eq!(
        parse_line(r#"{"ok":false}"#),
        Ok(VerifierLine::Result(false))
    );
    for bad in [
        "",
        "not json",
        r#"{"show":""}"#,
        r#"{"show":1}"#,
        r#"{"want":"key"}"#,
        r#"{"ok":"yes"}"#,
        r#"{"ok":true,"show":"x"}"#,
        r#"{"other":1}"#,
        "[]",
    ] {
        assert!(parse_line(bad).is_err(), "{bad:?}");
    }
    assert!(parse_line(&format!(r#"{{"show":"{}"}}"#, "x".repeat(500))).is_ok());
    assert!(parse_line(&format!(r#"{{"show":"{}"}}"#, "x".repeat(501))).is_err());
}

#[test]
fn ver3_show_want_ack_ok() {
    // The first stdin line is the request; then the core writes one ack line.
    let (process, events) = start(
        r#"read request
case "$request" in *'"operation":"verify"'*'"protocol":2'*|*'"protocol":2'*'"operation":"verify"'*) ;; *) exit 7;; esac
echo '{"show":"Is it here?"}'
echo '{"want":"ack"}'
read ack
[ "$ack" = '{"ack":true}' ] || exit 8
echo '{"ok":true}'"#,
        Duration::from_secs(5),
    );
    assert_eq!(next(&events), VerifierEvent::Show("Is it here?".into()));
    assert_eq!(next(&events), VerifierEvent::WantAck);
    process.ack();
    assert_eq!(next(&events), VerifierEvent::Done(true));
}

#[test]
fn ver3_a_verifier_with_no_user() {
    let (_process, events) = start(
        r#"read request; echo '{"ok":false}'"#,
        Duration::from_secs(5),
    );
    assert_eq!(next(&events), VerifierEvent::Done(false));
}

#[test]
fn ver3_a_verifier_can_ignore_the_request() {
    // stdin closes before the app writes the request; the write fails with EPIPE.
    let (_process, events) = start(
        r#"exec 0<&-; sleep 0.3; echo '{"ok":true}'"#,
        Duration::from_secs(5),
    );
    assert_eq!(next(&events), VerifierEvent::Done(true));
}

#[test]
fn ver3_errors_fail_the_verification() {
    for body in [
        // No result.
        "read request; exit 0",
        // A non-zero exit, also after a result.
        r#"read request; echo '{"ok":true}'; exit 1"#,
        // A line that is not in the protocol.
        r#"read request; echo 'hello'; sleep 30"#,
    ] {
        let (_process, events) = start(body, Duration::from_secs(5));
        assert!(matches!(next(&events), VerifierEvent::Failed(_)), "{body}");
    }
}

#[test]
fn ver3_timeout_stops_the_process_tree() {
    let marker = dir().join("verifier-child-alive");
    let _ = std::fs::remove_file(&marker);
    let start_time = Instant::now();
    let (_process, events) = start(
        &format!("(sleep 1; touch '{}') & sleep 30", marker.display()),
        Duration::from_millis(200),
    );
    assert!(matches!(next(&events), VerifierEvent::Failed(_)));
    assert!(start_time.elapsed() < Duration::from_secs(5));
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!marker.exists(), "the child of the verifier still ran");
}

#[test]
fn stop_ends_the_verifier_with_no_event() {
    let marker = dir().join("verifier-stopped");
    let _ = std::fs::remove_file(&marker);
    let (process, events) = start(
        &format!(
            "read request; echo '{{\"want\":\"ack\"}}'; sleep 1; touch '{}'",
            marker.display()
        ),
        Duration::from_secs(10),
    );
    assert_eq!(next(&events), VerifierEvent::WantAck);
    process.stop();
    std::thread::sleep(Duration::from_millis(1500));
    assert!(!marker.exists(), "the verifier still ran");
    assert!(events.try_recv().is_err(), "no event after stop");
}

#[test]
fn a_missing_verifier_program_is_an_error() {
    let program = Program {
        argv: vec!["telescreen-no-such-verifier".into()],
        cwd: None,
        env: Default::default(),
    };
    let result = VerifierProcess::spawn(
        &program,
        &request(),
        &Context {
            default_cwd: dir(),
            search_path: SearchPath(vec![dir().join("empty")]),
        },
        Duration::from_secs(1),
        |_| {},
    );
    assert!(result.is_err());
}

#[test]
fn ver5_the_prompt_verifier() {
    let now = Instant::now();
    let (mut prompt, events) = PromptVerifier::start(Some("Copy: Ctrl+C".into()), now);
    assert_eq!(
        events,
        [
            VerifierEvent::Show("Copy: Ctrl+C".into()),
            VerifierEvent::WantAck
        ]
    );
    assert_eq!(
        prompt.tick(now + PROMPT_TIMEOUT - Duration::from_millis(1)),
        None
    );
    assert_eq!(prompt.ack(), Some(VerifierEvent::Done(true)));
    // One result only.
    assert_eq!(prompt.ack(), None);
    assert_eq!(prompt.tick(now + PROMPT_TIMEOUT), None);
}

#[test]
fn ver5_the_prompt_verifier_times_out() {
    let now = Instant::now();
    let (mut prompt, events) = PromptVerifier::start(None, now);
    assert_eq!(events, [VerifierEvent::WantAck], "no hint, no show");
    assert_eq!(PROMPT_TIMEOUT, Duration::from_secs(60));
    assert_eq!(
        prompt.tick(now + PROMPT_TIMEOUT),
        Some(VerifierEvent::Done(false))
    );
    assert_eq!(prompt.ack(), None);
}
