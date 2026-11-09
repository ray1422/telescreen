//! Two peers on one machine (M3): mutual pinning (NET-3, AC-21), both channels
//! (NET-1, NET-2), heartbeat timeout (ADR 0007).
#![allow(clippy::unwrap_used, clippy::expect_used)] // Tests may panic on a failed setup.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use telescreen_core::ids::{ComputerId, DeviceId, SessionId};
use telescreen_core::pointer::Normalized;
use telescreen_core::wire::{Message, PointerDatagram};
use telescreen_transport::network::{Timing, TrustSource};
use telescreen_transport::{
    DialTarget, Identity, NetCommand, NetConfig, NetEvent, Network, TrustStore,
};

const WAIT: Duration = Duration::from_secs(10);

struct Peer {
    id: ComputerId,
    identity: Identity,
    network: Network,
    events: Receiver<NetEvent>,
}

fn timing() -> Timing {
    Timing {
        heartbeat: Duration::from_millis(100),
        timeout: Duration::from_millis(1000),
        auto_heartbeat: true,
    }
}

fn start(id: &str, identity: Identity, trust: TrustStore) -> Peer {
    start_with(id, identity, trust, timing())
}

fn start_with(id: &str, identity: Identity, trust: TrustStore, timing: Timing) -> Peer {
    let (tx, events) = channel();
    let config = NetConfig {
        local: ComputerId::from(id),
        listen: "127.0.0.1:0".parse().unwrap(),
        identity: identity.clone(),
        trust: TrustSource::Fixed(Arc::new(trust)),
        timing,
    };
    let network = Network::start(config, move |e| {
        let _ = tx.send(e);
    })
    .unwrap();
    Peer {
        id: ComputerId::from(id),
        identity,
        network,
        events,
    }
}

fn trusting(pairs: &[(&str, &Identity)]) -> TrustStore {
    let mut store = TrustStore::empty();
    for (id, identity) in pairs {
        store.trust(ComputerId::from(*id), identity.fingerprint());
    }
    store
}

fn endpoint(peer: &Peer) -> Vec<DialTarget> {
    vec![DialTarget {
        host: "127.0.0.1".into(),
        port: peer.network.local_addr().port(),
    }]
}

fn next(peer: &Peer) -> NetEvent {
    peer.events.recv_timeout(WAIT).expect("an event")
}

/// The next event that is not a heartbeat-level detail.
fn connected_pair() -> (Peer, Peer) {
    let mac_id = Identity::generate().unwrap();
    let pc1_id = Identity::generate().unwrap();
    let mac = start("mac", mac_id.clone(), trusting(&[("pc1", &pc1_id)]));
    let pc1 = start("pc1", pc1_id.clone(), trusting(&[("mac", &mac_id)]));
    mac.network.send(NetCommand::Dial {
        peer: pc1.id.clone(),
        targets: endpoint(&pc1),
    });
    assert_eq!(
        next(&mac),
        NetEvent::Connected {
            peer: pc1.id.clone()
        }
    );
    assert_eq!(
        next(&pc1),
        NetEvent::Connected {
            peer: mac.id.clone()
        }
    );
    (mac, pc1)
}

#[test]
fn net1_net2_messages_and_datagrams_both_ways() {
    let (mac, pc1) = connected_pair();
    let request = Message::RequestSession {
        device: DeviceId::from("shared-lg"),
    };
    mac.network.send(NetCommand::Send {
        peer: pc1.id.clone(),
        message: request.clone(),
    });
    assert_eq!(
        next(&pc1),
        NetEvent::Message {
            peer: mac.id.clone(),
            message: request
        }
    );

    let grant = Message::SessionGranted {
        session: SessionId(9),
        device: DeviceId::from("shared-lg"),
    };
    pc1.network.send(NetCommand::Send {
        peer: mac.id.clone(),
        message: grant.clone(),
    });
    assert_eq!(
        next(&mac),
        NetEvent::Message {
            peer: pc1.id.clone(),
            message: grant
        }
    );

    let datagram = PointerDatagram {
        session: SessionId(9),
        seq: 1,
        sent_us: 0,
        position: Normalized { x: 0.5, y: 0.25 },
    };
    mac.network.send(NetCommand::SendPointer {
        peer: pc1.id.clone(),
        datagram,
    });
    assert_eq!(
        next(&pc1),
        NetEvent::Pointer {
            peer: mac.id.clone(),
            datagram
        }
    );
}

#[test]
fn reliable_messages_keep_their_order() {
    let (mac, pc1) = connected_pair();
    for i in 0..200 {
        let message = Message::End {
            session: SessionId(i),
            reason: telescreen_core::wire::EndReason::PeerLost,
        };
        mac.network.send(NetCommand::Send {
            peer: pc1.id.clone(),
            message,
        });
    }
    for i in 0..200 {
        match next(&pc1) {
            NetEvent::Message {
                message: Message::End { session, .. },
                ..
            } => assert_eq!(session, SessionId(i)),
            other => panic!("{other:?}"),
        }
    }
}

#[test]
fn dial_when_connected_reports_connected() {
    let (mac, pc1) = connected_pair();
    mac.network.send(NetCommand::Dial {
        peer: pc1.id.clone(),
        targets: endpoint(&pc1),
    });
    assert_eq!(
        next(&mac),
        NetEvent::Connected {
            peer: pc1.id.clone()
        }
    );
}

#[test]
fn close_reports_disconnect_on_both_sides() {
    let (mac, pc1) = connected_pair();
    mac.network.send(NetCommand::Close {
        peer: pc1.id.clone(),
    });
    assert!(matches!(next(&mac), NetEvent::Disconnected { .. }));
    assert!(matches!(next(&pc1), NetEvent::Disconnected { .. }));
}

#[test]
fn ac18_peer_that_stops_is_lost() {
    let (mac, pc1) = connected_pair();
    drop(pc1);
    assert!(matches!(next(&mac), NetEvent::Disconnected { .. }));
}

#[test]
fn ac21_unknown_dialer_is_refused_on_both_sides() {
    let mac_id = Identity::generate().unwrap();
    let pc1_id = Identity::generate().unwrap();
    // The Mac trusts pc1, but pc1 does not trust the Mac.
    let mac = start("mac", mac_id, trusting(&[("pc1", &pc1_id)]));
    let pc1 = start("pc1", pc1_id, TrustStore::empty());
    mac.network.send(NetCommand::Dial {
        peer: pc1.id.clone(),
        targets: endpoint(&pc1),
    });
    assert!(matches!(next(&mac), NetEvent::DialFailed { .. }));
    assert!(matches!(next(&pc1), NetEvent::Refused { .. }));
}

#[test]
fn ac21_unknown_listener_is_refused_on_both_sides() {
    let mac_id = Identity::generate().unwrap();
    let pc1_id = Identity::generate().unwrap();
    let other = Identity::generate().unwrap();
    // The Mac pins a different key for pc1.
    let mac = start("mac", mac_id.clone(), trusting(&[("pc1", &other)]));
    let pc1 = start("pc1", pc1_id, trusting(&[("mac", &mac_id)]));
    mac.network.send(NetCommand::Dial {
        peer: pc1.id.clone(),
        targets: endpoint(&pc1),
    });
    assert!(matches!(next(&mac), NetEvent::DialFailed { .. }));
    assert!(matches!(next(&pc1), NetEvent::Refused { .. }));
}

#[test]
fn net3_trusted_key_with_the_wrong_computer_id_is_refused() {
    let mac_id = Identity::generate().unwrap();
    let pc1_id = Identity::generate().unwrap();
    // pc1 trusts this key only as "pc2". The dialer says it is "mac".
    let mac = start("mac", mac_id.clone(), trusting(&[("pc1", &pc1_id)]));
    let pc1 = start("pc1", pc1_id, trusting(&[("pc2", &mac_id)]));
    mac.network.send(NetCommand::Dial {
        peer: pc1.id.clone(),
        targets: endpoint(&pc1),
    });
    assert!(matches!(next(&pc1), NetEvent::Refused { .. }));
    assert!(!matches!(next(&mac), NetEvent::Connected { .. }));
}

#[test]
fn net3_listener_that_names_a_different_computer_is_refused() {
    let mac_id = Identity::generate().unwrap();
    let pc1_id = Identity::generate().unwrap();
    // The Mac dials "pc2" at the address of pc1. pc1 has the trusted key, but its
    // Hello says "pc1".
    let mac = start("mac", mac_id.clone(), trusting(&[("pc2", &pc1_id)]));
    let pc1 = start("pc1", pc1_id, trusting(&[("mac", &mac_id)]));
    mac.network.send(NetCommand::Dial {
        peer: ComputerId::from("pc2"),
        targets: endpoint(&pc1),
    });
    assert!(matches!(next(&mac), NetEvent::DialFailed { .. }));
    let _ = pc1.identity.fingerprint();
}

#[test]
fn trust_store_file_round_trip() {
    let dir = std::env::temp_dir().join(format!("telescreen-trust-{}", std::process::id()));
    let path = dir.join("trust.json");
    assert_eq!(
        TrustStore::load(&path).unwrap(),
        TrustStore::empty(),
        "a missing file is empty"
    );
    let id = Identity::generate().unwrap();
    let mut store = TrustStore::empty();
    assert!(store.trust(ComputerId::from("pc1"), id.fingerprint()));
    assert!(!store.trust(ComputerId::from("pc1"), id.fingerprint()));
    store.save(&path).unwrap();
    assert_eq!(TrustStore::load(&path).unwrap(), store);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn identity_is_kept_on_disk() {
    let dir = std::env::temp_dir().join(format!("telescreen-identity-{}", std::process::id()));
    let a = Identity::load_or_create(&dir).unwrap();
    let b = Identity::load_or_create(&dir).unwrap();
    assert_eq!(a.fingerprint(), b.fingerprint());
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(dir.join("private-key.der"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn fingerprint_text_forms() {
    let id = Identity::generate().unwrap();
    let fp = id.fingerprint();
    let text = fp.to_string();
    assert!(text.starts_with("sha256:") && text.len() == 71, "{text}");
    assert_eq!(
        text.parse::<telescreen_transport::Fingerprint>().unwrap(),
        fp
    );
    assert_eq!(
        text[7..]
            .to_uppercase()
            .parse::<telescreen_transport::Fingerprint>()
            .unwrap(),
        fp
    );
    assert!(
        "sha256:00"
            .parse::<telescreen_transport::Fingerprint>()
            .is_err()
    );
}

#[test]
fn ac18_hung_peer_times_out() {
    // The Mac sends no heartbeat (as an app with a hung main loop). pc1 closes the
    // connection after its timeout; both sides report it.
    let mac_id = Identity::generate().unwrap();
    let pc1_id = Identity::generate().unwrap();
    let silent = Timing {
        auto_heartbeat: false,
        ..timing()
    };
    let mac = start_with("mac", mac_id.clone(), trusting(&[("pc1", &pc1_id)]), silent);
    let pc1 = start("pc1", pc1_id, trusting(&[("mac", &mac_id)]));
    mac.network.send(NetCommand::Dial {
        peer: pc1.id.clone(),
        targets: endpoint(&pc1),
    });
    assert_eq!(
        next(&mac),
        NetEvent::Connected {
            peer: pc1.id.clone()
        }
    );
    assert_eq!(
        next(&pc1),
        NetEvent::Connected {
            peer: mac.id.clone()
        }
    );
    match next(&pc1) {
        NetEvent::Disconnected { reason, .. } => assert_eq!(reason, "heartbeat timeout"),
        other => panic!("{other:?}"),
    }
    assert!(matches!(next(&mac), NetEvent::Disconnected { .. }));
}

fn target(host: &str, port: u16) -> DialTarget {
    DialTarget {
        host: host.into(),
        port,
    }
}

fn trusted_pair() -> (Peer, Peer) {
    let mac_id = Identity::generate().unwrap();
    let pc1_id = Identity::generate().unwrap();
    let mac = start("mac", mac_id.clone(), trusting(&[("pc1", &pc1_id)]));
    let pc1 = start("pc1", pc1_id, trusting(&[("mac", &mac_id)]));
    (mac, pc1)
}

/// The events of a peer for a short time. A losing attempt can give `Refused`.
fn drain(peer: &Peer) -> Vec<NetEvent> {
    let mut events = Vec::new();
    while let Ok(event) = peer.events.recv_timeout(Duration::from_millis(600)) {
        if !matches!(event, NetEvent::Refused { .. }) {
            events.push(event);
        }
    }
    events
}

#[test]
fn net11_ac43_a_later_address_connects_with_no_wait_for_the_timeout() {
    let (mac, pc1) = trusted_pair();
    let port = pc1.network.local_addr().port();
    let start = Instant::now();
    mac.network.send(NetCommand::Dial {
        peer: pc1.id.clone(),
        targets: vec![target("192.0.2.1", port), target("127.0.0.1", port)],
    });
    assert_eq!(
        next(&mac),
        NetEvent::Connected {
            peer: pc1.id.clone()
        }
    );
    assert!(start.elapsed() < timing().timeout);
}

#[test]
fn net11_an_address_of_a_different_computer_does_not_stop_the_dial() {
    let (mac, pc1) = trusted_pair();
    let other = start("pc2", Identity::generate().unwrap(), TrustStore::empty());
    mac.network.send(NetCommand::Dial {
        peer: pc1.id.clone(),
        targets: vec![
            target("127.0.0.1", other.network.local_addr().port()),
            target("127.0.0.1", pc1.network.local_addr().port()),
        ],
    });
    assert_eq!(
        next(&mac),
        NetEvent::Connected {
            peer: pc1.id.clone()
        }
    );
}

#[test]
fn net11_ac43_two_addresses_of_one_peer_make_one_connection() {
    let (mac, pc1) = trusted_pair();
    let port = pc1.network.local_addr().port();
    mac.network.send(NetCommand::Dial {
        peer: pc1.id.clone(),
        targets: vec![target("127.0.0.1", port), target("localhost", port)],
    });
    assert_eq!(
        drain(&mac),
        [NetEvent::Connected {
            peer: pc1.id.clone()
        }]
    );
    assert_eq!(
        drain(&pc1),
        [NetEvent::Connected {
            peer: mac.id.clone()
        }]
    );
}

#[test]
fn net11_ac44_a_failure_gives_the_reason_of_each_address() {
    let (mac, pc1) = trusted_pair();
    let port = pc1.network.local_addr().port();
    mac.network.send(NetCommand::Dial {
        peer: pc1.id.clone(),
        targets: vec![
            target("first.invalid", port),
            target("second.invalid", port),
        ],
    });
    let NetEvent::DialFailed { reason, .. } = next(&mac) else {
        panic!("a dial failure");
    };
    assert!(reason.contains("first.invalid"), "{reason}");
    assert!(reason.contains("second.invalid"), "{reason}");
}

#[test]
fn net11_a_dial_with_no_address_fails() {
    let (mac, pc1) = trusted_pair();
    mac.network.send(NetCommand::Dial {
        peer: pc1.id.clone(),
        targets: Vec::new(),
    });
    assert!(matches!(next(&mac), NetEvent::DialFailed { .. }));
}

#[test]
fn net11_a_new_target_joins_the_running_dial() {
    let (mac, pc1) = trusted_pair();
    let port = pc1.network.local_addr().port();
    let start = Instant::now();
    mac.network.send(NetCommand::Dial {
        peer: pc1.id.clone(),
        targets: vec![target("192.0.2.1", port)],
    });
    std::thread::sleep(Duration::from_millis(100));
    mac.network.send(NetCommand::Dial {
        peer: pc1.id.clone(),
        targets: vec![target("192.0.2.1", port), target("127.0.0.1", port)],
    });
    assert_eq!(
        drain(&mac),
        [NetEvent::Connected {
            peer: pc1.id.clone()
        }]
    );
    assert!(start.elapsed() < timing().timeout + Duration::from_millis(600));
    assert_eq!(
        drain(&pc1),
        [NetEvent::Connected {
            peer: mac.id.clone()
        }]
    );
}

#[test]
fn net11_a_joined_target_is_in_the_one_failure() {
    let (mac, pc1) = trusted_pair();
    let port = pc1.network.local_addr().port();
    mac.network.send(NetCommand::Dial {
        peer: pc1.id.clone(),
        targets: vec![target("192.0.2.1", port)],
    });
    mac.network.send(NetCommand::Dial {
        peer: pc1.id.clone(),
        targets: vec![target("second.invalid", port)],
    });
    let NetEvent::DialFailed { reason, .. } = next(&mac) else {
        panic!("a dial failure");
    };
    assert!(reason.contains("192.0.2.1"), "{reason}");
    assert!(reason.contains("second.invalid"), "{reason}");
    assert_eq!(drain(&mac), [], "one failure");
}

#[test]
fn act7_the_magic_packet_has_six_ff_bytes_and_16_copies_of_the_mac() {
    let mac = [0xaa, 0xbb, 0xcc, 0x01, 0x02, 0x03];
    let packet = telescreen_transport::magic_packet(mac);
    assert_eq!(&packet[..6], &[0xff; 6]);
    for copy in packet[6..].chunks(6) {
        assert_eq!(copy, &mac);
    }
    assert_eq!(packet[6..].len(), 16 * 6);
}

#[test]
fn act7_wake_sends_one_datagram_with_no_connection() {
    let peer = start("mac", Identity::generate().unwrap(), TrustStore::empty());
    let receiver = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
    receiver.set_read_timeout(Some(WAIT)).unwrap();
    let mac = [0x04, 0x05, 0x06, 0x07, 0x08, 0x09];
    // A unicast address on loopback stands for the broadcast address.
    peer.network.send(NetCommand::Wake {
        peer: ComputerId::from("pc1"),
        mac,
        broadcast: "127.0.0.1".parse().unwrap(),
        port: receiver.local_addr().unwrap().port(),
    });
    let mut buffer = [0u8; 200];
    let (size, _) = receiver.recv_from(&mut buffer).unwrap();
    assert_eq!(
        &buffer[..size],
        &telescreen_transport::magic_packet(mac)[..]
    );
    assert_eq!(
        next(&peer),
        NetEvent::WakeSent {
            peer: ComputerId::from("pc1")
        }
    );
}
