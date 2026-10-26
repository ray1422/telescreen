//! Session rules (FLOW-5..FLOW-19, NET-6), with no I/O.
//!
//! The state machines take events and return actions. The app runs the actions:
//! it sends messages, runs drivers on a worker thread, starts and stops capture,
//! shows the prompt, and injects input. Time is an argument, so tests control it.
//! The protocol is tentative: `docs/adr/0007-session-protocol.md`.

pub mod lease;
pub mod source;
pub mod target;

use std::time::Duration;

/// FLOW-11: the confirmation timeout. Fixed in v0.1.0.
pub const CONFIRM_TIMEOUT: Duration = Duration::from_secs(60);

/// What the source knows about the video input of the shared display (UI-4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoInput {
    /// No handoff ran in this session.
    Unchanged,
    /// The handoff steps completed, and there is no `verify` and no user confirmation yet.
    Unverified,
    /// `verify` returned true, or the user confirmed on the source (ADR 0002).
    Confirmed,
    /// The state is not known: a partial handoff, a false `verify`, a timeout, or an
    /// end before the confirmation (FLOW-11, FLOW-12, FLOW-16).
    Unknown,
}
