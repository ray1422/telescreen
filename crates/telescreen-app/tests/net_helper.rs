//! ADR 0005: the network helper process carries `NetCommand` and `NetEvent`
//! between the app and the network. Unix only.
#![cfg(unix)]
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.

use std::sync::mpsc::channel;
use std::time::Duration;

use telescreen_app::controller::NetPort;
use telescreen_app::net_helper::HelperSocket;
use telescreen_core::config::Endpoint;
use telescreen_core::ids::ComputerId;
use telescreen_transport::{NetCommand, NetEvent};

#[test]
fn helper_reports_events_for_commands() {
    let data = std::env::temp_dir().join(format!("telescreen-helper-test-{}", std::process::id()));
    let socket = HelperSocket::bind().unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_telescreen"))
        .args(["net-helper", "--socket"])
        .arg(&socket.path)
        .args(["--computer", "mac", "--port", "0"])
        .env("TELESCREEN_DATA_DIR", &data)
        .spawn()
        .unwrap();
    let (tx, rx) = channel();
    let (stop_tx, stop_rx) = channel();
    let mut net = socket
        .accept(
            Duration::from_secs(10),
            move |event| {
                let _ = tx.send(event);
            },
            move || {
                let _ = stop_tx.send(());
            },
        )
        .unwrap();
    // Nothing listens on port 9 of the loopback address: the dial fails.
    net.send(NetCommand::Dial {
        peer: ComputerId::from("pc1"),
        endpoint: Endpoint {
            host: "127.0.0.1".into(),
            port: 9,
        },
    });
    let event = rx.recv_timeout(Duration::from_secs(15)).unwrap();
    assert!(
        matches!(event, NetEvent::DialFailed { ref peer, .. } if peer.0 == "pc1"),
        "{event:?}"
    );
    drop(net);
    let status = child.wait().unwrap();
    assert!(
        status.success(),
        "the helper stops when the app closes the socket"
    );
    stop_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the app reports the stopped helper");
    let _ = std::fs::remove_dir_all(data);
}

#[test]
fn ac18_killed_helper_is_reported() {
    let data = std::env::temp_dir().join(format!("telescreen-helper-kill-{}", std::process::id()));
    let socket = HelperSocket::bind().unwrap();
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_telescreen"))
        .args(["net-helper", "--socket"])
        .arg(&socket.path)
        .args(["--computer", "mac", "--port", "0"])
        .env("TELESCREEN_DATA_DIR", &data)
        .spawn()
        .unwrap();
    let (stop_tx, stop_rx) = channel();
    let _net = socket
        .accept(
            Duration::from_secs(10),
            |_| {},
            move || {
                let _ = stop_tx.send(());
            },
        )
        .unwrap();
    child.kill().unwrap();
    let _ = child.wait();
    stop_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("the app learns that the helper stopped");
    let _ = std::fs::remove_dir_all(data);
}
