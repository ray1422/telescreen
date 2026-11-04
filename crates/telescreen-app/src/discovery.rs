//! Which search results the app uses (NET-16).

use std::str::FromStr;
use telescreen_core::ids::ComputerId;
use telescreen_core::platform::FoundPeer;
use telescreen_transport::{DialTarget, Fingerprint, TrustStore};

/// The computers that a search result is for, and their addresses to dial.
pub fn accepted(found: &FoundPeer, trust: &TrustStore) -> Vec<(ComputerId, Vec<DialTarget>)> {
    let Ok(fingerprint) = Fingerprint::from_str(&found.fingerprint) else {
        return Vec::new();
    };
    let targets: Vec<_> = found
        .addresses
        .iter()
        .filter_map(|address| match address {
            std::net::IpAddr::V4(address) if address.is_private() || address.is_link_local() => {
                Some(DialTarget {
                    host: address.to_string(),
                    port: found.port,
                })
            }
            _ => None,
        })
        .collect();
    if targets.is_empty() {
        return Vec::new();
    }
    trust
        .computers_for(&fingerprint)
        .into_iter()
        .map(|peer| (peer.clone(), targets.clone()))
        .collect()
}
