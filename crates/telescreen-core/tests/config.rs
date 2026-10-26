//! Config contract: parity with `tools/test_config.py` (docs/config.md, "Validation Layers").
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use telescreen_core::config::{Config, ConfigError, Driver, yaml};
use telescreen_core::ids::{ComputerId, DeviceId};

fn example_text() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/config.yaml");
    std::fs::read_to_string(path).expect("examples/config.yaml")
}

fn example_value() -> Value {
    yaml::to_json(&example_text()).expect("the example is strict YAML")
}

#[test]
fn example_is_valid() {
    let config = Config::parse(&example_text()).expect("the example is valid");
    assert_eq!(config.computers.len(), 3);
}

#[test]
fn example_typed_model() {
    let config = Config::parse(&example_text()).unwrap();
    let mac = config.computer(&ComputerId::from("mac")).unwrap();
    assert_eq!(mac.label.as_deref(), Some("MacBook"));
    assert_eq!(mac.endpoint.port, 24800);
    let desktop = &mac.devices["Desktop"];
    assert_eq!(desktop.id, DeviceId::from("shared-lg"));
    let pc1 = &desktop.handoff[&ComputerId::from("pc1")];
    assert_eq!(pc1.steps.len(), 1);
    assert_eq!(pc1.steps[0].timeout, Duration::from_millis(3000));
    assert!(pc1.verify.is_some());
    // The default timeout applies when `timeout-ms` is absent.
    assert_eq!(
        desktop.portal_match.as_ref().unwrap().timeout,
        Duration::from_millis(5000)
    );
    match &desktop.portal_match.as_ref().unwrap().driver {
        Driver::Command(options) => {
            assert_eq!(options.argv, ["/Users/me/bin/lg-helper", "portal-match"]);
            assert!(options.cwd.is_none());
            assert!(options.env.is_empty());
        }
        other => panic!("expected command, got {other:?}"),
    }
    match &mac.devices["USB"].handoff[&ComputerId::from("pc1")].steps[0].driver {
        Driver::Extension { name, options } => {
            assert_eq!(name, "lab.http");
            assert_eq!(options["json"]["port"], json!(2));
        }
        other => panic!("expected extension, got {other:?}"),
    }
    let pc1 = config.computer(&ComputerId::from("pc1")).unwrap();
    assert!(pc1.hint.as_deref().unwrap().starts_with("Copy/paste"));
}

fn device(config: &mut Value) -> &mut Value {
    &mut config["computers"]["mac"]["devices"]["Desktop"]
}

enum Layer {
    Schema,
    Semantic,
}

type Mutation = (&'static str, Layer, fn(&mut Value));

/// The same mutations as `tools/test_config.py`, with the layer that must reject each.
#[test]
fn invalid_configurations() {
    let mutations: Vec<Mutation> = vec![
        ("empty hint", Layer::Schema, |c| {
            c["computers"]["pc1"]["hint"] = json!("")
        }),
        ("non-string hint", Layer::Schema, |c| {
            c["computers"]["pc1"]["hint"] = json!(["x"])
        }),
        ("long hint", Layer::Schema, |c| {
            c["computers"]["pc1"]["hint"] = json!("x".repeat(501))
        }),
        ("version", Layer::Schema, |c| c["version"] = json!(2)),
        ("unknown field", Layer::Schema, |c| {
            device(c)["portal_match"] = json!({})
        }),
        ("duplicate device id", Layer::Semantic, |c| {
            c["computers"]["mac"]["devices"]["Duplicate"] = json!({"id": "shared-lg"})
        }),
        ("missing computer", Layer::Semantic, |c| {
            let copy = device(c)["handoff"]["pc1"].clone();
            device(c)["handoff"]["absent"] = copy;
        }),
        ("self handoff", Layer::Semantic, |c| {
            let copy = device(c)["handoff"]["pc1"].clone();
            device(c)["handoff"]["mac"] = copy;
        }),
        ("missing connection", Layer::Semantic, |c| {
            c["computers"]["pc1"]["devices"]["Monitor1"]["id"] = json!("different")
        }),
        ("empty steps", Layer::Schema, |c| {
            device(c)["handoff"]["pc1"]["steps"] = json!([])
        }),
        ("bad timeout", Layer::Schema, |c| {
            device(c)["portal-match"]["timeout-ms"] = json!(0)
        }),
        ("empty argv", Layer::Schema, |c| {
            device(c)["portal-match"]["options"]["argv"] = json!([])
        }),
        ("relative executable", Layer::Semantic, |c| {
            device(c)["portal-match"]["options"]["argv"] = json!(["helper"])
        }),
        ("relative cwd", Layer::Semantic, |c| {
            device(c)["portal-match"]["options"]["cwd"] = json!("work")
        }),
        ("unknown command option", Layer::Schema, |c| {
            device(c)["portal-match"]["options"]["shell"] = json!(true)
        }),
    ];
    for (name, layer, mutate) in mutations {
        let mut value = example_value();
        mutate(&mut value);
        let result = Config::from_value(value);
        match (layer, &result) {
            (Layer::Schema, Err(ConfigError::Schema(errors)))
            | (Layer::Semantic, Err(ConfigError::Semantic(errors))) => {
                assert!(!errors.is_empty(), "{name}: empty error list");
            }
            _ => panic!("{name}: wrong result {result:?}"),
        }
    }
}

#[test]
fn semantic_errors_name_the_place() {
    let mut value = example_value();
    let copy = device(&mut value)["handoff"]["pc1"].clone();
    device(&mut value)["handoff"]["absent"] = copy;
    match Config::from_value(value) {
        Err(ConfigError::Semantic(errors)) => {
            assert_eq!(errors.len(), 1, "{errors:?}");
            assert!(errors[0].contains("mac.devices.Desktop"), "{}", errors[0]);
            assert!(errors[0].contains("absent"), "{}", errors[0]);
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn windows_absolute_paths_are_accepted_offline() {
    for path in [
        "C:\\helpers\\lg.exe",
        "D:/helpers/lg.exe",
        "\\\\server\\share\\lg.exe",
    ] {
        let mut value = example_value();
        device(&mut value)["portal-match"]["options"]["argv"] = json!([path]);
        assert!(Config::from_value(value).is_ok(), "{path}");
    }
}

/// `tools/test_config.py` `test_reject_ambiguous_yaml`, and the YAML 1.1 / 1.2
/// differences (docs/config.md, "Validation Layers").
#[test]
fn reject_ambiguous_yaml() {
    for source in [
        "version: 1\nversion: 2",
        "x: &a [1]\ny: *a",
        "x: &a 1",
        "x: {<<: {a: 1}}",
        "1: value",
        "x: !custom value",
        "x: !!str value",
        "x: .nan",
        "x: .inf",
        "x: -.Inf",
        "x: 2026-10-27",
        "x: 2026-10-27T10:00:00Z",
        "x: yes",
        "x: No",
        "x: on",
        "x: OFF",
        "x: y",
        "x: 010",
        "x: 0b101",
        "x: 0o17",
        "0o10: value",
        "x: 1_000",
        "x: 1:30",
        "x: 1e3",
        "x: 1.5e3",
        "x: 1.5e+3",
        "x: .5E3",
        "1.5e3: value",
        "x: =",
        "[1, 2]: value",
        "a: 1\n---\nb: 2",
    ] {
        match yaml::to_json(source) {
            Err(ConfigError::Yaml(_)) => {}
            other => panic!("{source:?}: expected a YAML error, got {other:?}"),
        }
    }
}

#[test]
fn accept_json_compatible_yaml() {
    let cases: [(&str, Value); 13] = [
        ("x: 1", json!({"x": 1})),
        ("x: -12", json!({"x": -12})),
        ("x: 0", json!({"x": 0})),
        ("x: 1.5", json!({"x": 1.5})),
        ("x: 0x1F", json!({"x": 31})),
        ("a: true\nb: False", json!({"a": true, "b": false})),
        (
            "a: null\nb: ~\nc:",
            json!({"a": null, "b": null, "c": null}),
        ),
        // A quoted `y` is a string key; a plain `y` is ambiguous.
        (
            "a: 'yes'\nb: \"2026-10-27\"\n'y': 1",
            json!({"a": "yes", "b": "2026-10-27", "y": 1}),
        ),
        ("x: hello world", json!({"x": "hello world"})),
        ("x: [a, 1, {b: c}]", json!({"x": ["a", 1, {"b": "c"}]})),
        (
            "x:\n  - /usr/bin/a\n  - --flag",
            json!({"x": ["/usr/bin/a", "--flag"]}),
        ),
        ("x: |\n  line\n", json!({"x": "line\n"})),
        ("# comment\nx: \"a: b\"", json!({"x": "a: b"})),
    ];
    for (source, expected) in cases {
        assert_eq!(yaml::to_json(source).unwrap(), expected, "{source:?}");
    }
}

#[test]
fn syntax_error_is_a_yaml_error() {
    assert!(matches!(
        yaml::to_json("x: [1, 2"),
        Err(ConfigError::Yaml(_))
    ));
}
