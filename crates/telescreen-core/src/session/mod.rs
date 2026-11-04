//! Session rules (FLOW-5..FLOW-19, NET-6), with no I/O.
//!
//! The state machines take events and return actions. The app runs the actions:
//! it sends messages, runs drivers on a worker thread, starts and stops capture,
//! runs the verifier, shows its window, and injects input.
//! The destination verifies: `docs/adr/0009-destination-verifies-a-handoff.md`.

pub mod lease;
pub mod source;
pub mod target;

/// What the source knows about the video input of the shared display (UI-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoInput {
    /// No handoff ran in this session.
    Unchanged,
    /// The handoff steps completed; verification is pending or there is no `verify`.
    Unverified,
    /// A verifier of the destination answered ok (VER-6).
    Confirmed,
    /// The state is not known: a partial handoff, a failed verification, or an
    /// end while unverified (FLOW-11, FLOW-12, FLOW-16).
    Unknown,
}
