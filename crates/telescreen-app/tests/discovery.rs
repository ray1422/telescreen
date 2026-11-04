//! Which search results the app uses (NET-16, AC-47).
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.

use telescreen_app::discovery::accepted;
use telescreen_core::ids::ComputerId;
use telescreen_core::platform::FoundPeer;
use telescreen_transport::{DialTarget, Identity, TrustStore};

fn found(fingerprint: String, addresses: &[&str]) -> FoundPeer {
    FoundPeer {
        fingerprint,
        addresses: addresses.iter().map(|a| a.parse().unwrap()).collect(),
        port: 24801,
    }
}

fn trusted() -> (TrustStore, String) {
    let fingerprint = Identity::generate().unwrap().fingerprint();
    let mut store = TrustStore::empty();
    store.trust(ComputerId::from("pc1"), fingerprint);
    (store, fingerprint.to_string())
}

#[test]
fn ac47_a_trusted_fingerprint_gives_its_computer_and_private_addresses() {
    let (store, fp) = trusted();
    let result = accepted(
        &found(
            fp,
            &["192.168.0.9", "10.0.0.2", "172.16.0.1", "169.254.1.1"],
        ),
        &store,
    );
    let target = |host: &str| DialTarget {
        host: host.into(),
        port: 24801,
    };
    assert_eq!(
        result,
        [(
            ComputerId::from("pc1"),
            vec![
                target("192.168.0.9"),
                target("10.0.0.2"),
                target("172.16.0.1"),
                target("169.254.1.1"),
            ]
        )]
    );
}

#[test]
fn ac47_an_unknown_or_invalid_fingerprint_gives_nothing() {
    let (store, _) = trusted();
    let other = Identity::generate().unwrap().fingerprint().to_string();
    assert!(accepted(&found(other, &["192.168.0.9"]), &store).is_empty());
    assert!(accepted(&found("nonsense".into(), &["192.168.0.9"]), &store).is_empty());
}

#[test]
fn ac47_public_loopback_and_ipv6_addresses_are_not_used() {
    let (store, fp) = trusted();
    assert!(
        accepted(
            &found(
                fp.clone(),
                &["8.8.8.8", "127.0.0.1", "fe80::1", "172.32.0.1"]
            ),
            &store
        )
        .is_empty()
    );
    let result = accepted(&found(fp, &["8.8.8.8", "192.168.0.9"]), &store);
    assert_eq!(result[0].1.len(), 1);
    assert_eq!(result[0].1[0].host, "192.168.0.9");
}
