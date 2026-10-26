---
status: accepted
---

# Transport is an abstraction; v1 is a direct LAN connection

The computers connect to the same physical display with video cables. Thus they are usually at one location and on one LAN. We do not make NAT traversal a v1 requirement. The core uses a transport abstraction with two channel types. The reliable, ordered channel sends control messages, key and button events, and clipboard data. The unreliable channel sends only the latest pointer position, and a lost or old position is not sent again. The v1 backend is a direct connection on the LAN. Keys pinned at pairing authenticate the peer. Encryption is necessary.

## Considered Options

- **WebRTC (ICE + DataChannel).** It gives NAT traversal and an unreliable channel mode. It also needs a signaling channel, a TURN server when traversal fails, and a large Rust dependency (webrtc-rs or str0m). It is not part of v1. We can add it later as a backend.
- **SSH only.** It gives authentication, encryption, and jump hosts from the user's existing setup. A single TCP stream has head-of-line blocking, so pointer motion waits behind lost packets. It is a candidate second backend (agent on stdio through `ssh`). It is not the only backend.

## Consequences

- The v1 config keeps `endpoint: {host, port}`. A different backend needs a change to the config contract (`docs/config.md`, the schema, the validator, and the tests).
- The choice between QUIC and TLS over TCP is not decided. Decide it in the transport prototype.
- Key and button down/up events must use the reliable channel. Do not send them on the unreliable channel.
