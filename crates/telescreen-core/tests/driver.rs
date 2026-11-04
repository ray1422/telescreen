//! The `exec` and `command` drivers and the search path: docs/config.md
//! "Operations and Drivers" and "The `command` Driver (protocol 2)". Unix only.
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use serde_json::Value;
use telescreen_core::config::{Driver, Operation, Program};
use telescreen_core::driver::{
    Context, DeviceRef, DriverError, OperationKind, Outcome, Request, SearchPath, run,
};
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

fn op(program: &Path, timeout_ms: u64) -> Operation {
    Operation {
        driver: Driver::Command(Program {
            argv: vec![program.to_string_lossy().into_owned(), "arg one".into()],
            cwd: None,
            env: BTreeMap::from([("TELESCREEN_TEST".into(), "x".into())]),
        }),
        timeout: Duration::from_millis(timeout_ms),
    }
}

/// The tests use the `PATH` of the test process, so that the helpers find `cat`.
fn context() -> Context {
    Context {
        default_cwd: dir(),
        search_path: SearchPath::from_env(),
    }
}

fn exec_op(argv: &[&str], env: &[(&str, &str)]) -> Operation {
    Operation {
        driver: Driver::Exec(Program {
            argv: argv.iter().map(|a| (*a).to_owned()).collect(),
            cwd: None,
            env: env
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        }),
        timeout: Duration::from_secs(2),
    }
}

fn path_str(path: &Path) -> &str {
    path.to_str().unwrap()
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

fn call(body: &str, kind: OperationKind) -> Result<Outcome, DriverError> {
    run(&op(&helper("h", body), 2000), kind, &request(), &context())
}

#[test]
fn results_by_operation() {
    assert_eq!(
        call(r#"echo '{"matched":true}'"#, OperationKind::PortalMatch).unwrap(),
        Outcome::Matched(true)
    );
    assert_eq!(
        call(r#"echo '{"matched":false}'"#, OperationKind::Verify).unwrap(),
        Outcome::Matched(false)
    );
    assert_eq!(
        call(r#"echo '{"ok":true}'"#, OperationKind::Handoff).unwrap(),
        Outcome::Done
    );
    assert_eq!(
        call(
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
        let result = call(body, kind);
        assert!(
            matches!(result, Err(DriverError::BadResult(_))),
            "{body}: {result:?}"
        );
    }
}

#[test]
fn nonzero_exit_is_always_an_error() {
    let result = call(
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
    assert_eq!(call(&body, OperationKind::Handoff).unwrap(), Outcome::Done);
    let request: Value = serde_json::from_slice(&std::fs::read(&out).unwrap()).unwrap();
    assert_eq!(request["protocol"], 2);
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
        &context(),
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
        call(body, OperationKind::Handoff),
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
    let result = run(&operation, OperationKind::Handoff, &request(), &context());
    assert!(matches!(result, Err(DriverError::Unsupported(name)) if name == "lab.http"));
}

#[test]
fn missing_program() {
    let result = run(
        &op(Path::new("/nonexistent/helper"), 1000),
        OperationKind::Handoff,
        &request(),
        &context(),
    );
    assert!(matches!(result, Err(DriverError::Spawn { .. })));
}

#[test]
fn exec_reads_only_the_exit_code() {
    let yes = helper("yes", "exit 0");
    // stdout is not read: this JSON does not change the result.
    let no = helper("no", r#"echo '{"matched":true}'; exit 3"#);
    let fails = helper("fails", "echo boom >&2; exit 2");
    let run_exec = |program: &Path, kind| {
        run(
            &exec_op(&[path_str(program)], &[]),
            kind,
            &request(),
            &context(),
        )
    };
    for kind in [OperationKind::PortalMatch, OperationKind::Verify] {
        assert_eq!(run_exec(&yes, kind).unwrap(), Outcome::Matched(true));
        assert_eq!(run_exec(&no, kind).unwrap(), Outcome::Matched(false));
    }
    assert_eq!(
        run_exec(&yes, OperationKind::Handoff).unwrap(),
        Outcome::Done
    );
    match run_exec(&fails, OperationKind::Handoff) {
        Err(DriverError::Exit { stderr, .. }) => assert_eq!(stderr, "boom"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn exec_has_empty_stdin_and_the_arguments() {
    let check = helper(
        "check",
        r#"[ -z "$(cat)" ] && [ "$1" = "arg one" ] && [ "$2" = b ]"#,
    );
    let result = run(
        &exec_op(&[path_str(&check), "arg one", "b"], &[]),
        OperationKind::PortalMatch,
        &request(),
        &context(),
    );
    assert_eq!(result.unwrap(), Outcome::Matched(true));
}

#[test]
fn exec_cannot_resolve_an_input_target() {
    let yes = helper("yes", "exit 0");
    let result = run(
        &exec_op(&[path_str(&yes)], &[]),
        OperationKind::InputTarget,
        &request(),
        &context(),
    );
    assert!(
        matches!(result, Err(DriverError::BadResult(_))),
        "{result:?}"
    );
}

#[test]
fn exec_timeout() {
    let slow = helper("slow", "sleep 30");
    let mut operation = exec_op(&[path_str(&slow)], &[]);
    operation.timeout = Duration::from_millis(200);
    let result = run(&operation, OperationKind::Handoff, &request(), &context());
    assert!(matches!(result, Err(DriverError::Timeout(_))), "{result:?}");
}

/// Two directories that both have a program `tool`: in `first` it exits 0, in
/// `second` it exits 1.
fn two_tools() -> (PathBuf, PathBuf) {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    let base = dir().join(format!("search-{}", NEXT.fetch_add(1, Ordering::Relaxed)));
    let first = base.join("first");
    let second = base.join("second");
    for (directory, code) in [(&first, 0), (&second, 1)] {
        std::fs::create_dir_all(directory).unwrap();
        let program = helper("tool", &format!("exit {code}"));
        std::fs::rename(program, directory.join("tool")).unwrap();
    }
    (first, second)
}

#[test]
fn a_program_name_is_found_in_the_search_path_in_sequence() {
    let (first, second) = two_tools();
    // A file that is not executable does not count.
    let plain = first.parent().unwrap().join("plain");
    std::fs::create_dir_all(&plain).unwrap();
    std::fs::write(plain.join("tool"), "#!/bin/sh\nexit 1\n").unwrap();
    for (search, expected) in [
        (vec![plain.clone(), first.clone(), second.clone()], true),
        (vec![second.clone(), first.clone()], false),
    ] {
        let search_path = SearchPath(search);
        assert_eq!(
            search_path.find("tool"),
            Some(if expected {
                first.join("tool")
            } else {
                second.join("tool")
            })
        );
        let context = Context {
            default_cwd: dir(),
            search_path,
        };
        let result = run(
            &exec_op(&["tool"], &[]),
            OperationKind::PortalMatch,
            &request(),
            &context,
        );
        assert_eq!(result.unwrap(), Outcome::Matched(expected));
    }
}

#[test]
fn a_program_with_a_path_is_not_searched() {
    let (first, _) = two_tools();
    let search_path = SearchPath(vec![first.clone()]);
    let absolute = first.join("tool");
    assert_eq!(search_path.find(path_str(&absolute)), Some(absolute));
    assert_eq!(
        search_path.find("first/tool"),
        Some(PathBuf::from("first/tool"))
    );
}

#[test]
fn a_missing_program_name_is_not_found() {
    let context = Context {
        default_cwd: dir(),
        search_path: SearchPath(vec![dir().join("empty")]),
    };
    let result = run(
        &exec_op(&["telescreen-no-such-tool"], &[]),
        OperationKind::Handoff,
        &request(),
        &context,
    );
    assert!(
        matches!(&result, Err(DriverError::NotFound(name)) if name == "telescreen-no-such-tool"),
        "{result:?}"
    );
}

#[test]
fn the_program_gets_the_search_path_as_path() {
    let check = helper("path", r#"[ "$PATH" = "$1" ]"#);
    let context = Context {
        default_cwd: dir(),
        search_path: SearchPath(vec!["/telescreen/a".into(), "/telescreen/b".into()]),
    };
    let result = run(
        &exec_op(&[path_str(&check), "/telescreen/a:/telescreen/b"], &[]),
        OperationKind::PortalMatch,
        &request(),
        &context,
    );
    assert_eq!(result.unwrap(), Outcome::Matched(true));
    // `env` replaces it.
    let result = run(
        &exec_op(&[path_str(&check), "/x"], &[("PATH", "/x")]),
        OperationKind::PortalMatch,
        &request(),
        &context,
    );
    assert_eq!(result.unwrap(), Outcome::Matched(true));
}

#[test]
fn the_command_driver_uses_the_search_path() {
    let (first, _) = two_tools();
    std::fs::rename(
        helper("json", r#"cat > /dev/null; echo '{"ok":true}'"#),
        first.join("json-tool"),
    )
    .unwrap();
    let context = Context {
        default_cwd: dir(),
        search_path: SearchPath(vec![first]),
    };
    let operation = Operation {
        driver: Driver::Command(Program {
            argv: vec!["json-tool".into()],
            cwd: None,
            env: BTreeMap::new(),
        }),
        timeout: Duration::from_secs(2),
    };
    let result = run(&operation, OperationKind::Handoff, &request(), &context);
    assert_eq!(result.unwrap(), Outcome::Done);
}

#[test]
fn the_search_path_from_the_login_shell() {
    // A login shell can print text before the command runs.
    let shell = helper(
        "shell",
        r#"echo 'Last login: today'; [ "$1" = -l ] && [ "$2" = -c ] || exit 9; PATH=/telescreen/a:/telescreen/b; eval "$3""#,
    );
    let search_path = SearchPath::from_shell(&shell, Duration::from_secs(2)).unwrap();
    assert_eq!(
        search_path,
        SearchPath(vec!["/telescreen/a".into(), "/telescreen/b".into()])
    );
}

#[test]
fn a_failed_login_shell_is_an_error() {
    let broken = helper("broken", "exit 1");
    assert!(SearchPath::from_shell(&broken, Duration::from_secs(2)).is_err());
    let slow = helper("slow-shell", "sleep 30");
    let start = Instant::now();
    assert!(SearchPath::from_shell(&slow, Duration::from_millis(200)).is_err());
    assert!(start.elapsed() < Duration::from_secs(5));
}
