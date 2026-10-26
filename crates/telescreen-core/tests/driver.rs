//! `command` driver: docs/config.md "Driver I/O Contract (protocol 1)". Unix only.
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;
use telescreen_core::config::{CommandOptions, Driver, Operation};
use telescreen_core::driver::{DeviceRef, DriverError, OperationKind, Outcome, Request, run};
use telescreen_core::ids::{ComputerId, DisplayToken};
use telescreen_core::platform::DisplayInfo;

fn dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("telescreen-driver-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn helper(name: &str, body: &str) -> PathBuf {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let path = dir().join(format!("{name}-{}", NEXT.fetch_add(1, Ordering::Relaxed)));
    // Write and rename, so that no test runs a file that is open for writing (ETXTBSY).
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, format!("#!/bin/sh\n{body}\n")).unwrap();
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755)).unwrap();
    std::fs::rename(&tmp, &path).unwrap();
    path
}

fn op(program: &Path, timeout_ms: u64) -> Operation {
    Operation {
        driver: Driver::Command(CommandOptions {
            argv: vec![program.to_string_lossy().into_owned(), "arg one".into()],
            cwd: None,
            env: BTreeMap::from([("TELESCREEN_TEST".into(), "x".into())]),
        }),
        timeout: Duration::from_millis(timeout_ms),
    }
}

fn request() -> Request {
    Request {
        computer_id: ComputerId::from("mac"),
        device: DeviceRef {
            id: "shared-lg".into(),
            alias: "Desktop".into(),
        },
        source_computer_id: ComputerId::from("mac"),
        target_computer_id: Some(ComputerId::from("pc1")),
        session_id: None,
        portal: None,
        displays: vec![DisplayInfo {
            token: DisplayToken::from("d-1"),
            bounds: None,
            scale: 2.0,
            metadata: Default::default(),
        }],
    }
}

fn exec(body: &str, kind: OperationKind) -> Result<Outcome, DriverError> {
    run(&op(&helper("h", body), 2000), kind, &request(), &dir())
}

#[test]
fn results_by_operation() {
    assert_eq!(
        exec(r#"echo '{"matched":true}'"#, OperationKind::PortalMatch).unwrap(),
        Outcome::Matched(true)
    );
    assert_eq!(
        exec(r#"echo '{"matched":false}'"#, OperationKind::Verify).unwrap(),
        Outcome::Matched(false)
    );
    assert_eq!(
        exec(r#"echo '{"ok":true}'"#, OperationKind::Handoff).unwrap(),
        Outcome::Done
    );
    assert_eq!(
        exec(
            r#"echo '{"display_token":"d-1"}'"#,
            OperationKind::InputTarget
        )
        .unwrap(),
        Outcome::Display(DisplayToken::from("d-1"))
    );
}

#[test]
fn bad_results_are_errors() {
    for (body, kind) in [
        (
            r#"echo '{"display_token":"stale"}'"#,
            OperationKind::InputTarget,
        ),
        (r#"echo '{"ok":false}'"#, OperationKind::Handoff),
        (r#"echo '{"matched":"yes"}'"#, OperationKind::PortalMatch),
        (
            r#"echo '{"matched":true,"x":1}'"#,
            OperationKind::PortalMatch,
        ),
        (r#"echo 'not json'"#, OperationKind::PortalMatch),
    ] {
        assert!(
            matches!(exec(body, kind), Err(DriverError::BadResult(_))),
            "{body}"
        );
    }
}

#[test]
fn nonzero_exit_is_always_an_error() {
    let result = exec(
        r#"echo '{"matched":false}'; echo boom >&2; exit 1"#,
        OperationKind::PortalMatch,
    );
    match result {
        Err(DriverError::Exit { stderr, .. }) => assert_eq!(stderr, "boom"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn request_on_stdin_argv_env_and_cwd() {
    let out = dir().join("request.json");
    let body = format!(
        r#"cat > '{}'; printf '%s|%s|%s' "$1" "$TELESCREEN_TEST" "$(pwd -P)" > '{}.args'; echo '{{"ok":true}}'"#,
        out.display(),
        out.display()
    );
    assert_eq!(exec(&body, OperationKind::Handoff).unwrap(), Outcome::Done);
    let request: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(request["protocol"], 1);
    assert_eq!(request["operation"], "handoff");
    assert_eq!(request["computer_id"], "mac");
    assert_eq!(request["device"]["alias"], "Desktop");
    assert_eq!(request["target_computer_id"], "pc1");
    assert_eq!(request["session_id"], Value::Null);
    assert_eq!(request["portal"], Value::Null);
    assert_eq!(request["displays"][0]["token"], "d-1");
    let args = std::fs::read_to_string(format!("{}.args", out.display())).unwrap();
    let cwd = dir().canonicalize().unwrap();
    assert_eq!(args, format!("arg one|x|{}", cwd.display()));
}

#[test]
fn timeout_kills_the_process_tree() {
    let marker = dir().join("child-alive");
    let _ = std::fs::remove_file(&marker);
    let body = format!("(sleep 1; touch '{}') & sleep 30", marker.display());
    let start = Instant::now();
    let result = run(
        &op(&helper("slow", &body), 200),
        OperationKind::Handoff,
        &request(),
        &dir(),
    );
    assert!(matches!(result, Err(DriverError::Timeout(_))), "{result:?}");
    assert!(start.elapsed() < Duration::from_secs(5));
    std::thread::sleep(Duration::from_millis(1500));
    assert!(
        !marker.exists(),
        "the background child of the helper still ran"
    );
}

#[test]
fn output_limit() {
    let body = "head -c 70000 /dev/zero | tr '\\0' 'a'";
    assert!(matches!(
        exec(body, OperationKind::Handoff),
        Err(DriverError::OutputTooLarge("stdout"))
    ));
}

#[test]
fn extension_driver_is_unsupported() {
    let operation = Operation {
        driver: Driver::Extension {
            name: "lab.http".into(),
            options: Value::Null,
        },
        timeout: Duration::from_secs(1),
    };
    let result = run(&operation, OperationKind::Handoff, &request(), &dir());
    assert!(matches!(result, Err(DriverError::Unsupported(name)) if name == "lab.http"));
}

#[test]
fn missing_program() {
    let result = run(
        &op(Path::new("/nonexistent/helper"), 1000),
        OperationKind::Handoff,
        &request(),
        &dir(),
    );
    assert!(matches!(result, Err(DriverError::Spawn { .. })));
}
