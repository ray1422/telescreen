//! QUIC peer endpoint of Telescreen (ADR 0001, ADR 0004, NET-1..NET-3).
//!
//! The transport moves [`Message`]s and [`PointerDatagram`]s between peers. It has
//! no session rules. The API is message-level ([`NetCommand`], [`NetEvent`]).

pub mod fingerprint;
pub mod identity;
pub mod network;
mod tls;
pub mod trust;

use telescreen_core::ids::ComputerId;
use telescreen_core::wire::{Message, PointerDatagram};

pub use fingerprint::Fingerprint;
pub use identity::Identity;
pub use network::{NetConfig, Network};
pub use trust::TrustStore;

/// One address to try for a peer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialTarget {
    pub host: String,
    pub port: u16,
}

/// A request to the transport.
#[derive(Debug, Clone, PartialEq)]
pub enum NetCommand {
    /// Connect to a peer through the first target that answers (NET-11). On success
    /// the transport sends [`NetEvent::Connected`].
    Dial {
        peer: ComputerId,
        targets: Vec<DialTarget>,
    },
    /// Send on the reliable channel. A message to a peer with no connection is lost.
    Send { peer: ComputerId, message: Message },
    /// Send on the unreliable channel.
    SendPointer {
        peer: ComputerId,
        datagram: PointerDatagram,
    },
    /// Close the connection to a peer.
    Close { peer: ComputerId },
    /// Send one Wake-on-LAN magic packet ([`magic_packet`]) as a UDP broadcast
    /// to `broadcast:port` (ACT-7). It needs no connection to the peer. The
    /// transport sends [`NetEvent::WakeSent`] or [`NetEvent::WakeFailed`].
    Wake {
        peer: ComputerId,
        mac: [u8; 6],
        broadcast: std::net::Ipv4Addr,
        port: u16,
    },
}

/// The Wake-on-LAN magic packet: 6 bytes `0xFF`, then 16 copies of the MAC
/// address (ACT-7).
pub fn magic_packet(mac: [u8; 6]) -> [u8; 102] {
    let mut packet = [0xff; 102];
    for copy in packet[6..].as_chunks_mut::<6>().0 {
        copy.copy_from_slice(&mac);
    }
    packet
}

/// A report from the transport.
#[derive(Debug, Clone, PartialEq)]
pub enum NetEvent {
    /// An authenticated connection to the peer is open (both sides pinned, Hello done).
    Connected {
        peer: ComputerId,
    },
    Message {
        peer: ComputerId,
        message: Message,
    },
    Pointer {
        peer: ComputerId,
        datagram: PointerDatagram,
    },
    /// The connection closed or its heartbeat timed out (ADR 0007).
    Disconnected {
        peer: ComputerId,
        reason: String,
    },
    DialFailed {
        peer: ComputerId,
        reason: String,
    },
    /// The magic packet of [`NetCommand::Wake`] was sent (ACT-7).
    WakeSent {
        peer: ComputerId,
    },
    /// The magic packet of [`NetCommand::Wake`] could not be sent (ACT-7).
    WakeFailed {
        peer: ComputerId,
        reason: String,
    },
    /// An incoming connection was refused (AC-21).
    Refused {
        remote: String,
        reason: String,
    },
}
