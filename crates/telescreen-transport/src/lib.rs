//! QUIC peer endpoint of Telescreen (ADR 0001, ADR 0004, NET-1..NET-3).
//!
//! The transport moves [`Message`]s and [`PointerDatagram`]s between peers. It has
//! no session rules. The API is message-level ([`NetCommand`], [`NetEvent`]).

pub mod fingerprint;
pub mod identity;
pub mod network;
mod tls;
pub mod trust;

use telescreen_core::config::Endpoint;
use telescreen_core::ids::ComputerId;
use telescreen_core::wire::{Message, PointerDatagram};

pub use fingerprint::Fingerprint;
pub use identity::Identity;
pub use network::{NetConfig, Network};
pub use trust::TrustStore;

/// A request to the transport.
#[derive(Debug, Clone, PartialEq)]
pub enum NetCommand {
    /// Connect to a peer. On success the transport sends [`NetEvent::Connected`].
    Dial {
        peer: ComputerId,
        endpoint: Endpoint,
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
    /// An incoming connection was refused (AC-21).
    Refused {
        remote: String,
        reason: String,
    },
}
