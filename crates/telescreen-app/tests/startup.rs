#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.
//! PEER-3, PEER-5, UI-8: the config path, the saved computer ID, and the start decision.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use telescreen_app::paths::{default_config_in, save_computer, saved_computer};
use telescreen_app::startup::{Startup, resolve};
use telescreen_core::ids::ComputerId;

const CONFIG: &str = "\
version: 1
computers:
  mac:
    endpoint: {host: mac.test, port: 24800}
    devices:
      Desktop: {id: shared}
  linux:
    endpoint: {host: linux.test, port: 24800}
    devices:
      Desktop: {id: shared}
";

/// A new directory for one test. The test process removes nothing: it is in the
/// directory for test results of cargo.
fn test_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("startup")
        .join(name);
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn config_file(name: &str, text: &str) -> PathBuf {
    let path = test_dir(name).join("config.yaml");
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn peer5_default_config_uses_xdg_config_home_when_absolute() {
    let home = Path::new("/home/u");
    assert_eq!(
        default_config_in(None, home),
        Path::new("/home/u/.config/telescreen/config.yaml")
    );
    assert_eq!(
        default_config_in(Some(OsStr::new("/x")), home),
        Path::new("/x/telescreen/config.yaml")
    );
    assert_eq!(
        default_config_in(Some(OsStr::new("relative")), home),
        Path::new("/home/u/.config/telescreen/config.yaml")
    );
}

#[test]
fn peer3_saved_computer_round_trip() {
    let file = test_dir("saved").join("data").join("computer");
    assert_eq!(saved_computer(&file), None);
    save_computer(&file, "mac").unwrap();
    assert_eq!(saved_computer(&file).as_deref(), Some("mac"));
    std::fs::write(&file, " \n").unwrap();
    assert_eq!(saved_computer(&file), None);
}

#[test]
fn ui8_missing_config_is_a_problem_that_names_the_path() {
    let path = test_dir("missing").join("config.yaml");
    let Startup::Problem(text) = resolve(&path, None, Some("mac")) else {
        panic!("expected a problem");
    };
    assert!(text.contains(&path.display().to_string()), "{text}");
}

#[test]
fn ui8_invalid_config_is_a_problem() {
    let path = config_file("invalid", "version: 99\n");
    assert!(matches!(resolve(&path, None, None), Startup::Problem(_)));
}

#[test]
fn peer3_no_saved_computer_asks_also_with_one_computer() {
    let path = config_file("ask", CONFIG);
    assert!(matches!(
        resolve(&path, None, None),
        Startup::Choose { notice: None, .. }
    ));
}

#[test]
fn peer3_saved_computer_runs() {
    let path = config_file("runs", CONFIG);
    let Startup::Run { computer, .. } = resolve(&path, None, Some("linux")) else {
        panic!("expected run");
    };
    assert_eq!(computer, ComputerId::from("linux"));
}

#[test]
fn peer3_saved_computer_that_left_the_config_asks_again() {
    let path = config_file("left", CONFIG);
    let Startup::Choose { notice, .. } = resolve(&path, None, Some("old")) else {
        panic!("expected choose");
    };
    assert!(notice.unwrap().contains("old"));
}

#[test]
fn peer3_argument_has_priority_over_saved_computer() {
    let path = config_file("priority", CONFIG);
    let Startup::Run { computer, .. } = resolve(&path, Some("mac"), Some("linux")) else {
        panic!("expected run");
    };
    assert_eq!(computer, ComputerId::from("mac"));
}

#[test]
fn peer3_unknown_argument_is_a_problem_also_with_a_saved_computer() {
    let path = config_file("unknown", CONFIG);
    assert!(matches!(
        resolve(&path, Some("nope"), Some("mac")),
        Startup::Problem(_)
    ));
}
