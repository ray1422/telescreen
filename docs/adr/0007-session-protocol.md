---
status: tentative
---

# Session protocol, leases, and heartbeat (tentative)

Status: tentative. Claude proposed this protocol for v0.1.0 (2026-10-27). The user decided (2026-10-27) to keep it tentative until the hardware tests measure it (AC-18). Then change the status to accepted, or change the values. It answers the open question "lease arbitration and the recovery protocol (NET-6)" of `docs/spec.md` only for v0.1.0.

## Decision

- The target is the only peer that gives the lease of its device. A source has a session only after `SessionGranted`. Thus the local state of one peer alone does not decide the lease owner (NET-6).
- The session ID is a generation number from the target. The first generation of a run is the start time in milliseconds since the Unix epoch, so that a restart gives newer generations. Each grant increases it by one. Each message has the session ID. A peer ignores a message with a different session ID.
- One lease for each device at one time (AC-20). The target refuses a second request with `Busy`. There is no preemption in v0.1.0: the user ends the first session (on the source, or "disconnect" on the target).
- Messages and their sequence: see the diagram in `crates/telescreen-core/src/wire.rs`. The source waits for `Started` after `Confirm`, because the target checks that the input-target display token is still valid (FLOW-10).
- Heartbeat: the controller sends `Heartbeat` each second to each connected peer, from the main loop of the app. Thus a hung app stops its heartbeat, and the other peer detects the hang (AC-18). If no message arrives for 5 seconds, the transport closes the connection. (The transport can also send the heartbeat itself; the app does not use this.) A closed connection ends all sessions on it, on both peers (`PeerLost`). The target releases all held input and the lease by itself (IN-4). The QUIC idle timeout is also 5 seconds.
- Lease expiry is the heartbeat timeout. There is no separate lease renewal message.
- The prompt has no timeout of its own on the target. The source sends `End { ConfirmTimeout }` after 60 seconds (FLOW-11). If the source is lost, the heartbeat ends the session.

## Consequences

- A source that does not respond for 5 seconds loses its session. The user must start a new handoff.
- If two sources request the same device, the first request that arrives wins. The second source shows "busy".
- Open: preemption of a lease, recovery after a short network loss, and the timeout values. Measure them in the product before a stable release.
