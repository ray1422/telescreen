//! FLOW-3 candidates (AC-1, AC-2), AC-20 leases, IN-3 held input.
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.

use std::path::Path;

use telescreen_core::config::Config;
use telescreen_core::ids::{ComputerId, DeviceId};
use telescreen_core::input::{Button, HeldInputs, InputEvent};
use telescreen_core::keys::Key;
use telescreen_core::routes::candidates;
use telescreen_core::session::lease::LeaseTable;

fn example() -> Config {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/config.yaml");
    Config::load(&path).unwrap()
}

#[test]
fn ac1_same_device_id_with_different_aliases() {
    let config = example();
    let mac = config.computer(&ComputerId::from("mac")).unwrap();
    let routes = candidates(&config, &mac.devices["Desktop"]);
    let found: Vec<_> = routes
        .iter()
        .map(|r| (r.target.0.as_str(), r.target_alias))
        .collect();
    assert_eq!(found, [("pc1", "Monitor1"), ("pc2", "MyScreen")]);
}

#[test]
fn ac2_mac_is_not_a_candidate_for_a_device_it_does_not_connect_to() {
    let config = example();
    let pc1 = config.computer(&ComputerId::from("pc1")).unwrap();
    let routes = candidates(&config, &pc1.devices["SideScreen"]);
    let targets: Vec<_> = routes.iter().map(|r| r.target.0.as_str()).collect();
    assert_eq!(targets, ["pc2"]);
    let mac = config.computer(&ComputerId::from("mac")).unwrap();
    assert!(candidates(&config, &mac.devices["BuiltIn"]).is_empty());
}

#[test]
fn ac24_generic_device_has_routes() {
    let config = example();
    let mac = config.computer(&ComputerId::from("mac")).unwrap();
    let routes = candidates(&config, &mac.devices["USB"]);
    assert_eq!(routes.len(), 1);
    assert!(routes[0].target_device.input_target.is_none());
}

#[test]
fn ac20_one_lease_per_device() {
    let mut leases = LeaseTable::new(100);
    let lg = DeviceId::from("shared-lg");
    let a = leases.acquire(&lg, &ComputerId::from("mac")).unwrap();
    let busy = leases.acquire(&lg, &ComputerId::from("pc2")).unwrap_err();
    assert_eq!(busy.session, a);
    assert_eq!(busy.holder, ComputerId::from("mac"));
    let other = leases
        .acquire(&DeviceId::from("usb"), &ComputerId::from("pc2"))
        .unwrap();
    assert!(other > a);
    assert!(leases.release(a));
    assert!(!leases.release(a));
    let b = leases.acquire(&lg, &ComputerId::from("pc2")).unwrap();
    assert!(b > other, "generations only increase");
    assert_eq!(leases.holder(&lg), Some((b, &ComputerId::from("pc2"))));
}

#[test]
fn in3_held_inputs() {
    let mut held = HeldInputs::default();
    let k = |down| InputEvent::Key { key: Key(30), down };
    let b = |down| InputEvent::Button {
        button: Button::Left,
        down,
    };
    assert!(held.record(&k(true)));
    assert!(held.record(&k(true)), "repeat");
    assert!(held.record(&b(true)));
    assert!(!held.record(&b(true)));
    assert!(!held.record(&InputEvent::Key {
        key: Key(31),
        down: false
    }));
    assert_eq!(held.release_all(), vec![b(false), k(false)]);
    assert!(held.is_empty());
    assert!(!held.record(&k(false)));
}
