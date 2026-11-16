//! `telescreen run --stage login` (STG-2): the checks before the stage starts.
//! Each case stops before the process uses the network, the display, or uinput.
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const CONFIG: &str = "\
version: 1
computers:
  mac:
    endpoint: {host: mac.test, port: 24800}
    devices:
      Desktop: {id: shared}
  linux:
    endpoint: {host: linux.test, port: 24800}
    stages:
      login: {from: [mac]}
    devices:
      Desktop: {id: shared}
";

/// A new directory for one test, in the directory for test results of cargo.
fn test_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("stage_cli")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs `telescreen run --config <the test config> <args>` with an empty data
/// directory, so that the test does not use the identity of the user.
fn run(name: &str, args: &[&str]) -> Output {
    let dir = test_dir(name);
    let config = dir.join("config.yaml");
    std::fs::write(&config, CONFIG).unwrap();
    Command::new(env!("CARGO_BIN_EXE_telescreen"))
        .arg("run")
        .arg("--config")
        .arg(&config)
        .args(args)
        .env("TELESCREEN_DATA_DIR", dir.join("data"))
        .output()
        .expect("run telescreen")
}

fn failure(output: &Output) -> String {
    assert!(!output.status.success(), "the process must fail");
    String::from_utf8_lossy(&output.stderr).into_owned()
}

#[test]
fn stg2_an_unknown_stage_name_is_an_error() {
    let output = run("unknown", &["--stage", "boot", "--computer", "linux"]);
    let text = failure(&output);
    assert!(text.contains("unknown stage boot"), "{text}");
}

#[cfg(not(target_os = "linux"))]
#[test]
fn stg2_the_login_stage_runs_only_on_linux() {
    let output = run("platform", &["--stage", "login", "--computer", "linux"]);
    let text = failure(&output);
    assert!(text.contains("only on Linux"), "{text}");
}

#[cfg(target_os = "linux")]
#[test]
fn stg2_the_stage_needs_the_computer_id() {
    // No `--computer`, and no saved ID in the empty data directory. A stage
    // process has no window to ask in.
    let output = run("no-computer", &["--stage", "login"]);
    let text = failure(&output);
    assert!(text.contains("--computer"), "{text}");
}

#[cfg(target_os = "linux")]
#[test]
fn stg2_the_stage_needs_a_computer_of_the_config() {
    let output = run("absent", &["--stage", "login", "--computer", "absent"]);
    let text = failure(&output);
    assert!(text.contains("no computer absent"), "{text}");
}

#[cfg(target_os = "linux")]
#[test]
fn stg2_the_stage_needs_stages_login_in_the_section_of_this_computer() {
    let output = run("no-stage", &["--stage", "login", "--computer", "mac"]);
    let text = failure(&output);
    assert!(text.contains("no stages.login"), "{text}");
}
