//! The built-in `display` driver: docs/config.md "The `display` Driver".
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.

use std::time::Duration;

use serde_json::{Value, json};
use telescreen_core::config::{Config, Driver, Operation};
use telescreen_core::driver::{
    Context, DeviceRef, DriverError, OperationKind, Outcome, PortalInfo, Request, SearchPath, run,
};
use telescreen_core::ids::{ComputerId, DisplayToken};
use telescreen_core::platform::DisplayInfo;

fn display(token: &str, metadata: Value) -> DisplayInfo {
    DisplayInfo {
        token: DisplayToken::from(token),
        bounds: None,
        scale: 1.0,
        metadata: metadata.as_object().unwrap().clone(),
    }
}

fn displays() -> Vec<DisplayInfo> {
    vec![
        display(
            "cg-1",
            json!({"vendor": 1552, "model": 41042, "builtin": true}),
        ),
        display(
            "cg-2",
            json!({"vendor": 7789, "model": 30457, "builtin": false, "name": "LG"}),
        ),
        display(
            "cg-3",
            json!({"vendor": 7789, "model": 1, "builtin": false}),
        ),
    ]
}

fn operation(matches: Value) -> Operation {
    Operation {
        driver: Driver::Display {
            matches: matches.as_object().unwrap().clone(),
        },
        timeout: Duration::from_secs(1),
    }
}

fn request(portal_on: Option<&str>) -> Request {
    let mut request = Request::on_source(
        &ComputerId::from("mac"),
        DeviceRef {
            id: "shared-lg".into(),
            alias: "LG".into(),
        },
        None,
        displays(),
    );
    request.portal = portal_on.map(|token| PortalInfo {
        id: "portal-1".into(),
        bounds: None,
        coordinate_space: "local-desktop-logical",
        display_token: Some(DisplayToken::from(token)),
    });
    request
}

fn context() -> Context {
    Context {
        default_cwd: std::env::temp_dir(),
        search_path: SearchPath(Vec::new()),
    }
}

fn outcome(matches: Value, kind: OperationKind, portal_on: Option<&str>) -> Outcome {
    run(&operation(matches), kind, &request(portal_on), &context()).unwrap()
}

#[test]
fn portal_match_is_true_when_the_portal_display_has_each_value() {
    let lg = json!({"vendor": 7789, "model": 30457});
    assert_eq!(
        outcome(lg.clone(), OperationKind::PortalMatch, Some("cg-2")),
        Outcome::Matched(true)
    );
    assert_eq!(
        outcome(lg, OperationKind::PortalMatch, Some("cg-3")),
        Outcome::Matched(false)
    );
    assert_eq!(
        outcome(
            json!({"builtin": true}),
            OperationKind::PortalMatch,
            Some("cg-1")
        ),
        Outcome::Matched(true)
    );
}

#[test]
fn portal_match_compares_the_json_type() {
    // A string is not the number of the metadata.
    assert_eq!(
        outcome(
            json!({"vendor": "7789"}),
            OperationKind::PortalMatch,
            Some("cg-2")
        ),
        Outcome::Matched(false)
    );
}

#[test]
fn portal_match_is_false_for_a_missing_key_or_display() {
    assert_eq!(
        outcome(
            json!({"serial": 5}),
            OperationKind::PortalMatch,
            Some("cg-2")
        ),
        Outcome::Matched(false)
    );
    assert_eq!(
        outcome(
            json!({"vendor": 7789}),
            OperationKind::PortalMatch,
            Some("cg-9")
        ),
        Outcome::Matched(false)
    );
    assert_eq!(
        outcome(json!({"vendor": 7789}), OperationKind::PortalMatch, None),
        Outcome::Matched(false)
    );
}

#[test]
fn input_target_gives_the_one_matching_display() {
    assert_eq!(
        outcome(json!({"name": "LG"}), OperationKind::InputTarget, None),
        Outcome::Display(DisplayToken::from("cg-2"))
    );
}

#[test]
fn ac9_input_target_fails_with_no_match_or_more_than_one() {
    for matches in [json!({"vendor": 7789}), json!({"vendor": 1})] {
        let result = run(
            &operation(matches),
            OperationKind::InputTarget,
            &request(None),
            &context(),
        );
        assert!(
            matches!(result, Err(DriverError::BadResult(_))),
            "{result:?}"
        );
    }
}

#[test]
fn display_is_not_a_handoff_step_or_a_verifier() {
    for kind in [OperationKind::Handoff, OperationKind::Verify] {
        let result = run(
            &operation(json!({"vendor": 7789})),
            kind,
            &request(None),
            &context(),
        );
        assert!(
            matches!(result, Err(DriverError::BadResult(_))),
            "{result:?}"
        );
    }
}

#[test]
fn the_config_reads_the_display_driver() {
    let config = Config::parse(
        r#"
version: 1
computers:
  mac:
    endpoint: {host: mac.test, port: 24800}
    devices:
      LG:
        id: shared-lg
        portal-match: {driver: display, match: {vendor: 7789, model: 30457}}
        input-target: {driver: display, match: {name: HDMI-A-2}}
        handoff:
          linux:
            steps: [{driver: exec, command: "true"}]
  linux:
    endpoint: {host: linux.test, port: 24800}
    devices:
      LG:
        id: shared-lg
"#,
    )
    .unwrap();
    let mac = &config.computers[&ComputerId::from("mac")];
    let lg = &mac.devices["LG"];
    assert_eq!(
        lg.portal_match.as_ref().unwrap().driver,
        Driver::Display {
            matches: json!({"vendor": 7789, "model": 30457})
                .as_object()
                .unwrap()
                .clone()
        }
    );
}
