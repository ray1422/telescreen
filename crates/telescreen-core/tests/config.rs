//! Config contract: parity with `tools/test_config.py` (docs/config.md, "Validation Layers").
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.

use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use telescreen_core::config::{
    Config, ConfigError, Driver, Program, Quirk, SpecialKeyAction, yaml,
};
use telescreen_core::ids::{ComputerId, DeviceId};
use telescreen_core::keys::SpecialKeyGroup;

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
    assert_eq!(
        mac.path.as_deref().unwrap()[0],
        Path::new("/opt/homebrew/bin")
    );
    let pc1 = &desktop.handoff[&ComputerId::from("pc1")];
    assert_eq!(pc1.steps.len(), 1);
    assert_eq!(pc1.steps[0].timeout, Duration::from_millis(3000));
    // A `command` string is split by shell rules (docs/config.md).
    assert_eq!(
        pc1.steps[0].driver,
        Driver::Exec(Program {
            argv: vec!["lg-helper".into(), "select".into(), "dp".into()],
            cwd: None,
            env: Default::default(),
        })
    );
    // The default timeout applies when `timeout-ms` is absent.
    let portal_match = desktop.portal_match.as_ref().unwrap();
    assert_eq!(portal_match.timeout, Duration::from_millis(5000));
    assert!(
        matches!(&portal_match.driver, Driver::Exec(p) if p.argv == ["lg-helper", "is-portal-display"])
    );
    match &desktop.input_target.as_ref().unwrap().driver {
        Driver::Command(program) => assert_eq!(program.argv, ["lg-helper", "input-target"]),
        other => panic!("expected command, got {other:?}"),
    }
    // `verify` is on the device of the destination (ADR 0009); its default
    // timeout is longer, because a verifier can wait for the user.
    let verify = desktop.verify.as_ref().unwrap();
    assert_eq!(verify.driver, Driver::Prompt);
    assert_eq!(verify.timeout, Duration::from_millis(60_000));
    let pc2 = config.computer(&ComputerId::from("pc2")).unwrap();
    assert_eq!(
        pc2.devices["MyScreen"].verify.as_ref().unwrap().timeout,
        Duration::from_millis(90_000)
    );
    // Quotes keep one argument.
    let pc1_computer = config.computer(&ComputerId::from("pc1")).unwrap();
    let to_mac = &pc1_computer.devices["Monitor1"].handoff[&ComputerId::from("mac")];
    assert!(
        matches!(&to_mac.steps[0].driver, Driver::Exec(p) if p.argv[0] == "ddcutil" && p.argv.len() == 8)
    );
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
    /// The typed model splits a `command` string.
    Model,
    Semantic,
}

type Mutation = (&'static str, Layer, fn(&mut Value));
type Case = (&'static str, fn(&mut Value));

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
        ("empty command list", Layer::Schema, |c| {
            device(c)["portal-match"]["command"] = json!([])
        }),
        ("empty command string", Layer::Schema, |c| {
            device(c)["portal-match"]["command"] = json!("")
        }),
        ("blank command string", Layer::Model, |c| {
            device(c)["portal-match"]["command"] = json!("   ")
        }),
        ("unbalanced quote", Layer::Model, |c| {
            device(c)["portal-match"]["command"] = json!("a 'b")
        }),
        ("relative program with a path", Layer::Semantic, |c| {
            device(c)["portal-match"]["command"] = json!(["bin/helper"])
        }),
        ("relative cwd", Layer::Semantic, |c| {
            device(c)["portal-match"]["cwd"] = json!("work")
        }),
        ("relative search path", Layer::Semantic, |c| {
            c["computers"]["mac"]["path"] = json!(["bin"])
        }),
        ("empty search path", Layer::Schema, |c| {
            c["computers"]["mac"]["path"] = json!([])
        }),
        ("unknown command field", Layer::Schema, |c| {
            device(c)["portal-match"]["shell"] = json!(true)
        }),
        ("options on a built-in driver", Layer::Schema, |c| {
            device(c)["portal-match"]["options"] = json!({})
        }),
        ("command missing", Layer::Schema, |c| {
            device(c)["portal-match"]
                .as_object_mut()
                .unwrap()
                .remove("command");
        }),
        ("prompt out of verify", Layer::Schema, |c| {
            device(c)["portal-match"] = json!({"driver": "prompt"})
        }),
        ("exec for input-target", Layer::Schema, |c| {
            device(c)["input-target"]["driver"] = json!("exec")
        }),
        ("prompt with a field", Layer::Schema, |c| {
            device(c)["verify"] = json!({"driver": "prompt", "timeout-ms": 1000})
        }),
        ("verify on a route", Layer::Schema, |c| {
            device(c)["handoff"]["pc1"]["verify"] = json!({"driver": "prompt"})
        }),
        ("unknown special key group", Layer::Schema, |c| {
            c["computers"]["mac"]["special-keys"] = json!({"send-to-target": ["power"]})
        }),
        ("duplicate special key group", Layer::Schema, |c| {
            c["computers"]["mac"]["special-keys"] =
                json!({"send-to-target": ["brightness", "brightness"]})
        }),
        ("special key group in both lists", Layer::Semantic, |c| {
            c["computers"]["mac"]["special-keys"] =
                json!({"send-to-target": ["media"], "keep-on-source": ["media"]})
        }),
        ("unknown special keys field", Layer::Schema, |c| {
            c["computers"]["mac"]["special-keys"] = json!({"forward": ["brightness"]})
        }),
        ("non-list special key groups", Layer::Schema, |c| {
            c["computers"]["mac"]["special-keys"] = json!({"keep-on-source": "volume"})
        }),
        ("unknown built-in driver", Layer::Schema, |c| {
            device(c)["portal-match"]["driver"] = json!("shell")
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
            (Layer::Model, Err(ConfigError::Model(_))) => {}
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
fn the_lg_example_is_valid() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contrib/example-lg.yaml");
    let text = std::fs::read_to_string(path).unwrap();
    Config::parse(&text).expect("contrib/example-lg.yaml is valid");
}

#[test]
fn valid_commands() {
    let cases: Vec<Case> = vec![
        ("program name with no path", |c| {
            device(c)["portal-match"]["command"] = json!("helper")
        }),
        ("command verifier", |c| {
            device(c)["verify"] = json!({"driver": "command", "command": ["v"]})
        }),
        ("exec verifier", |c| {
            device(c)["verify"] = json!({"driver": "exec", "command": "v"})
        }),
        ("extension verifier", |c| {
            device(c)["verify"] = json!({"driver": "lab.http", "options": {}})
        }),
        ("no verify", |c| {
            device(c).as_object_mut().unwrap().remove("verify");
        }),
    ];
    for (name, mutate) in cases {
        let mut value = example_value();
        mutate(&mut value);
        assert!(Config::from_value(value).is_ok(), "{name}");
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
        device(&mut value)["portal-match"]["command"] = json!([path]);
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

#[test]
fn in14_the_scroll_speed_defaults_to_one() {
    let mut value = example_value();
    let computers = value["computers"].as_object_mut().unwrap();
    for computer in computers.values_mut() {
        computer.as_object_mut().unwrap().remove("scroll-speed");
    }
    computers["pc1"]["scroll-speed"] = json!(0.4);
    let config: Config = serde_json::from_value(value).unwrap();
    assert_eq!(
        config.computers[&ComputerId("pc1".into())].scroll_speed,
        0.4
    );
    assert_eq!(
        config.computers[&ComputerId("mac".into())].scroll_speed,
        1.0
    );
}

#[test]
fn in16_each_quirk_is_on_unless_the_list_has_it() {
    let mut value = example_value();
    let computers = value["computers"].as_object_mut().unwrap();
    for computer in computers.values_mut() {
        computer.as_object_mut().unwrap().remove("disabled-quirks");
    }
    computers["pc1"]["disabled-quirks"] = json!(["kwin-button-scroll"]);
    let config: Config = serde_json::from_value(value).unwrap();
    assert!(!config.computers[&ComputerId("pc1".into())].uses_quirk(Quirk::KwinButtonScroll));
    assert!(config.computers[&ComputerId("mac".into())].uses_quirk(Quirk::KwinButtonScroll));
}

#[test]
fn in20_each_special_key_group_is_suppressed_unless_a_list_has_it() {
    let mut value = example_value();
    let computers = value["computers"].as_object_mut().unwrap();
    for computer in computers.values_mut() {
        computer.as_object_mut().unwrap().remove("special-keys");
    }
    computers["mac"]["special-keys"] =
        json!({"send-to-target": ["brightness"], "keep-on-source": ["volume", "media"]});
    computers["pc1"]["special-keys"] = json!({});
    let config = Config::from_value(value).unwrap();
    let action = |id: &str, group| {
        config
            .computer(&ComputerId::from(id))
            .unwrap()
            .special_keys
            .action(group)
    };
    assert_eq!(
        action("mac", SpecialKeyGroup::Brightness),
        SpecialKeyAction::SendToTarget
    );
    assert_eq!(
        action("mac", SpecialKeyGroup::Volume),
        SpecialKeyAction::KeepOnSource
    );
    assert_eq!(
        action("mac", SpecialKeyGroup::Media),
        SpecialKeyAction::KeepOnSource
    );
    assert_eq!(
        action("mac", SpecialKeyGroup::KeyboardLight),
        SpecialKeyAction::Suppress
    );
    for group in [
        SpecialKeyGroup::Brightness,
        SpecialKeyGroup::Volume,
        SpecialKeyGroup::Media,
        SpecialKeyGroup::KeyboardLight,
    ] {
        assert_eq!(
            action("pc1", group),
            SpecialKeyAction::Suppress,
            "an empty key"
        );
        assert_eq!(action("pc2", group), SpecialKeyAction::Suppress, "no key");
    }
}

#[test]
fn in20_a_special_key_group_in_both_lists_names_the_computer() {
    let mut value = example_value();
    value["computers"]["mac"]["special-keys"] =
        json!({"send-to-target": ["media"], "keep-on-source": ["media"]});
    match Config::from_value(value) {
        Err(ConfigError::Semantic(errors)) => {
            assert!(
                errors
                    .iter()
                    .any(|e| e.contains("mac.special-keys") && e.contains("media")),
                "{errors:?}"
            );
        }
        other => panic!("wrong result {other:?}"),
    }
}

#[test]
fn clip1_the_clipboard_switch_starts_on_by_default() {
    let mut value = example_value();
    let computers = value["computers"].as_object_mut().unwrap();
    for computer in computers.values_mut() {
        computer.as_object_mut().unwrap().remove("clipboard");
    }
    computers["pc1"]["clipboard"] = json!({"text": false});
    computers["mac"]["clipboard"] = json!({});
    let config = Config::from_value(value).unwrap();
    let text = |id: &str| {
        config
            .computer(&ComputerId::from(id))
            .unwrap()
            .clipboard
            .text
    };
    assert!(text("mac"), "an empty clipboard key");
    assert!(!text("pc1"), "the user turned it off");
    let other = config
        .computers
        .keys()
        .find(|id| id.0 != "mac" && id.0 != "pc1")
        .unwrap()
        .clone();
    assert!(text(&other.0), "no clipboard key");
}

fn endpoint_of(yaml_endpoint: &str) -> Result<telescreen_core::config::Endpoint, String> {
    let text = format!(
        "version: 1\ncomputers:\n  mac:\n    endpoint: {yaml_endpoint}\n    devices:\n      D: {{id: d}}\n"
    );
    Config::parse(&text)
        .map(|c| {
            c.computer(&ComputerId::from("mac"))
                .unwrap()
                .endpoint
                .clone()
        })
        .map_err(|e| e.to_string())
}

#[test]
fn net11_host_is_one_host_or_a_list() {
    assert_eq!(
        endpoint_of("{host: a.test, port: 1}").unwrap().hosts,
        ["a.test"]
    );
    assert_eq!(
        endpoint_of("{host: [a.test, 192.0.2.1], port: 1}")
            .unwrap()
            .hosts,
        ["a.test", "192.0.2.1"]
    );
    assert!(endpoint_of("{host: [], port: 1}").is_err());
}

#[test]
fn net13_host_is_optional_only_with_discovery() {
    let endpoint = endpoint_of("{port: 1, discovery: mdns}").unwrap();
    assert!(endpoint.hosts.is_empty());
    assert_eq!(
        endpoint.discovery,
        Some(telescreen_core::config::Discovery::Mdns)
    );
    assert!(endpoint_of("{port: 1}").is_err());
    assert!(endpoint_of("{host: a.test, port: 1, discovery: dns}").is_err());
}
