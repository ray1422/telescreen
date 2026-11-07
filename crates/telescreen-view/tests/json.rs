//! The JSON shape of the menu view model and the intents (UI-18). The KDE applet
//! reads this shape, so a change must change this test.
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.

use serde_json::json;
use telescreen_core::ids::{ComputerId, DeviceId, DisplayToken, SessionId};
use telescreen_core::session::VideoInput;
use telescreen_view::*;

fn menu() -> MenuScreen {
    MenuScreen {
        computer: "MacBook".into(),
        glyph: Glyph::Controlling,
        sessions: vec![
            MenuSession {
                id: MenuSessionId::Portal,
                peer: ComputerId::from("pc1"),
                peer_label: "Linux".into(),
                device: DeviceId::from("shared-lg"),
                role: MenuRole::Source {
                    phase: SessionPhase::WaitingForAck {
                        key: "Return".into(),
                    },
                    video_input: VideoInput::Unverified,
                },
            },
            MenuSession {
                id: MenuSessionId::Incoming {
                    session: SessionId(7),
                },
                peer: ComputerId::from("pc4"),
                peer_label: "Desk PC".into(),
                device: DeviceId::from("usb"),
                role: MenuRole::Target {
                    state: "forwarding".into(),
                    hand_back: Some("No route back".into()),
                },
            },
        ],
        displays: vec![MenuDisplay {
            token: DisplayToken::from("lg"),
            name: "LG".into(),
            matching: DisplayMatching::Done,
            devices: vec![MenuDevice {
                device: DeviceId::from("shared-lg"),
                alias: "Desktop".into(),
                label: "Desktop".into(),
                state: VideoInput::Confirmed,
                candidates: vec![MenuCandidate {
                    target: ComputerId::from("pc1"),
                    label: "Linux".into(),
                    status: "offline".into(),
                    switch_and_control: Some("offline".into()),
                    switch_display_only: None,
                    control_only: Some("offline".into()),
                }],
            }],
        }],
        control_requests: vec![ControlRequestRow {
            peer: ComputerId::from("pc1"),
            peer_label: "Linux".into(),
            device: DeviceId::from("shared-lg"),
            device_label: "Desktop".into(),
            waiting: true,
        }],
        clipboard: Some(true),
    }
}

#[test]
fn ui18_the_menu_json_shape() {
    let value = serde_json::to_value(menu()).unwrap();
    assert_eq!(
        value,
        json!({
            "computer": "MacBook",
            "glyph": "controlling",
            "sessions": [
                {
                    "id": {"kind": "portal"},
                    "peer": "pc1",
                    "peer_label": "Linux",
                    "device": "shared-lg",
                    "role": {
                        "kind": "source",
                        "phase": {"kind": "waiting_for_ack", "key": "Return"},
                        "video_input": "unverified"
                    }
                },
                {
                    "id": {"kind": "incoming", "session": 7},
                    "peer": "pc4",
                    "peer_label": "Desk PC",
                    "device": "usb",
                    "role": {"kind": "target", "state": "forwarding", "hand_back": "No route back"}
                }
            ],
            "displays": [{
                "token": "lg",
                "name": "LG",
                "matching": {"kind": "done"},
                "devices": [{
                    "device": "shared-lg",
                    "alias": "Desktop",
                    "label": "Desktop",
                    "state": "confirmed",
                    "candidates": [{
                        "target": "pc1",
                        "label": "Linux",
                        "status": "offline",
                        "switch_and_control": "offline",
                        "switch_display_only": null,
                        "control_only": "offline"
                    }]
                }]
            }],
            "control_requests": [{
                "peer": "pc1",
                "peer_label": "Linux",
                "device": "shared-lg",
                "device_label": "Desktop",
                "waiting": true
            }],
            "clipboard": true
        })
    );
    let back: MenuScreen = serde_json::from_value(value).unwrap();
    assert_eq!(back, menu());
}

#[test]
fn ui18_each_intent_round_trips_as_json() {
    let lg = || DisplayToken::from("lg");
    let dev = || DeviceId::from("shared-lg");
    let pc1 = || ComputerId::from("pc1");
    let intents = [
        Intent::DeviceIsBack { device: dev() },
        Intent::SetClipboard { on: true },
        Intent::Refresh,
        Intent::AllowDiscovery,
        Intent::StopDiscovery,
        Intent::EndSession,
        Intent::HandBack {
            session: SessionId(1),
        },
        Intent::StopSession {
            session: SessionId(1),
        },
        Intent::ControlFromHere {
            session: SessionId(1),
        },
        Intent::SwitchAndControl {
            display: lg(),
            device: dev(),
            target: pc1(),
        },
        Intent::SwitchDisplayOnly {
            display: lg(),
            device: dev(),
            target: pc1(),
        },
        Intent::ControlOnly {
            display: lg(),
            device: dev(),
            target: pc1(),
        },
        Intent::LetControl {
            peer: pc1(),
            device: dev(),
        },
        Intent::SourceHandBack,
    ];
    for intent in intents {
        let text = serde_json::to_string(&intent).unwrap();
        assert_eq!(
            serde_json::from_str::<Intent>(&text).unwrap(),
            intent,
            "{text}"
        );
    }
}

#[test]
fn ui18_an_intent_from_the_applet() {
    let intent: Intent = serde_json::from_value(json!({
        "kind": "control_only",
        "display": "lg",
        "device": "shared-lg",
        "target": "pc1"
    }))
    .unwrap();
    assert_eq!(
        intent,
        Intent::ControlOnly {
            display: DisplayToken::from("lg"),
            device: DeviceId::from("shared-lg"),
            target: ComputerId::from("pc1"),
        }
    );
}

#[test]
fn ui18_the_publisher_gives_only_changes() {
    let mut publisher = Publisher::new();
    assert_eq!(publisher.update(menu()), Some(&menu()), "the first value");
    assert_eq!(publisher.update(menu()), None, "no change");
    let mut changed = menu();
    changed.glyph = Glyph::Idle;
    assert_eq!(publisher.update(changed.clone()), Some(&changed));
    assert_eq!(publisher.update(changed), None);
}

#[test]
fn ui18_the_main_window_json_shape() {
    let main = MainScreen {
        menu: MenuScreen::default(),
        fingerprint: "sha256:00".into(),
        computers: vec![ComputerRow {
            id: ComputerId::from("pc1"),
            label: "Linux".into(),
            this_computer: false,
            trusted: false,
            connected: true,
        }],
        devices: vec![DeviceRow {
            alias: "Desktop".into(),
            device: DeviceId::from("shared-lg"),
            state: VideoInput::Unknown,
        }],
        discovery: Some(DiscoveryRow {
            remaining_seconds: Some(42),
        }),
        notices: vec!["hello".into()],
    };
    let value = serde_json::to_value(&main).unwrap();
    assert_eq!(value["fingerprint"], "sha256:00");
    assert_eq!(
        value["computers"][0],
        json!({"id": "pc1", "label": "Linux", "this_computer": false, "trusted": false, "connected": true})
    );
    assert_eq!(
        value["devices"][0],
        json!({"alias": "Desktop", "device": "shared-lg", "state": "unknown"})
    );
    assert_eq!(value["discovery"], json!({"remaining_seconds": 42}));
    assert_eq!(value["notices"], json!(["hello"]));
    assert_eq!(value["menu"]["glyph"], "idle");
    let back: MainScreen = serde_json::from_value(value).unwrap();
    assert_eq!(back, main);
}
