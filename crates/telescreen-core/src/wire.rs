//! Wire messages between two peers (NET-1, NET-2, NET-6) and their encoding.
//!
//! The reliable channel carries [`Message`] frames. The unreliable channel carries
//! [`PointerDatagram`]s. Both use postcard. A reliable frame is a 4-byte
//! little-endian length and the payload.
//!
//! Protocol (ADRs 0009 and 0010):
//!
//! ```text
//! source                         target
//! Hello  ───────────────────────▶          (each side sends Hello first)
//! RequestSession{device} ───────▶          lease + input-target (FLOW-5)
//!        ◀─────────────────────── SessionGranted{session} | SessionRefused
//! (run handoff steps, FLOW-6)
//! HandoffDone{session} ─────────▶          start destination verifier (VER-1)
//!        ◀─────────────────────── AckWanted{session} (if requested, window visible)
//! (user acknowledges in the portal, FLOW-8)
//! Ack{session} ────────────────▶           acknowledge to verifier (may repeat)
//!                                         verifier ok, or no verify (VER-2):
//!                                         check display, hide verifier (FLOW-10)
//!        ◀─────────────────────── Started{session, verified}
//! Input / ReleaseAll / datagrams ▶         inject
//! KeyboardAway{session} ────────▶          hold keyboard focus (FLOW-23)
//! KeyboardBack{session} ────────▶          release keyboard focus holder
//! PortalHidden{session} ────────▶          show the portal warning (FLOW-26)
//! PortalShown{session} ─────────▶          hide the portal warning
//! ClipboardText{session, text} ◀▶          sync text while forwarding (CLIP-2)
//! ClipboardClear{session}     ◀▶          the sent concealed text went away (CLIP-7)
//! End{session, reason}  ◀──────▶  End{session, reason}
//! HandBackRequest{session} ─────▶          source ends; target ends, runs route back
//! (or target sends End{reason: HandBack}, ends, runs route back)
//!        ◀─────────────────────── HandBackResult{device, outcome} (no session)
//! (on Done, old source verifies as the destination, VER-4)
//! Attach (ACT-3): RequestSession, SessionGranted, then HandoffDone at once.
//! DisplaySwitched{device} ──────▶          after "Switch display only": verify
//!                                         as the destination (ACT-6)
//! ControlRequest{device} ───────▶          the receiver attaches to the sender
//!        ◀─────────────────────── ControlRefused{device, reason} (ACT-4)
//! Heartbeat ◀──────────────────▶ Heartbeat (each second; the transport owns it)
//! ```

use serde::{Deserialize, Serialize};

use crate::ids::{ComputerId, DeviceId, SessionId};
use crate::input::InputEvent;
use crate::pointer::Normalized;

/// The wire protocol version. A peer refuses a different version.
pub const PROTOCOL_VERSION: u32 = 6;

/// The maximum size of one reliable frame payload.
pub const MAX_FRAME: usize = crate::clipboard::MAX_TEXT + 4 * 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Message {
    /// The first message in each direction. The transport checks that `computer`
    /// is the computer ID that the pinned certificate belongs to (NET-3).
    Hello {
        protocol: u32,
        computer: ComputerId,
    },
    Heartbeat,

    // Source to target.
    RequestSession {
        device: DeviceId,
    },
    HandoffDone {
        session: SessionId,
    },
    /// The source ends its active session and asks the target to run its route
    /// back to the source (HB-3).
    HandBackRequest {
        session: SessionId,
    },
    /// The user acknowledged in the portal (FLOW-8).
    Ack {
        session: SessionId,
    },
    Input {
        session: SessionId,
        event: InputEvent,
    },
    /// Release all held keys and buttons after input was lost (IN-13).
    ReleaseAll {
        session: SessionId,
    },
    /// The portal lost the keyboard focus (FLOW-23).
    KeyboardAway {
        session: SessionId,
    },
    /// The portal got the keyboard focus (FLOW-23).
    KeyboardBack {
        session: SessionId,
    },
    /// The portal is not on the screen of the source (FLOW-26).
    PortalHidden {
        session: SessionId,
    },
    /// The portal is on the screen of the source again (FLOW-26).
    PortalShown {
        session: SessionId,
    },
    /// Plain text in both directions while forwarding (CLIP-2). `concealed` is the
    /// concealed mark of a password manager (CLIP-7).
    ClipboardText {
        session: SessionId,
        text: String,
        concealed: bool,
    },
    /// The concealed text that the sender sent went off its clipboard (CLIP-7).
    ClipboardClear {
        session: SessionId,
    },

    // Target to source.
    SessionGranted {
        session: SessionId,
        device: DeviceId,
    },
    SessionRefused {
        device: DeviceId,
        reason: RefuseReason,
    },
    /// The verifier of the target waits for an acknowledgement (FLOW-8).
    AckWanted {
        session: SessionId,
    },
    Started {
        session: SessionId,
        /// True if the verifier answered ok; false if there is no `verify` (VER-2).
        verified: bool,
    },
    /// The old target reports its route back to the old source (HB-1, HB-4).
    /// This result is not part of a session.
    HandBackResult {
        device: DeviceId,
        outcome: HandBackOutcome,
    },

    // Both directions.
    End {
        session: SessionId,
        reason: EndReason,
    },

    // Not part of a session (ACT-6).
    /// "Switch display only" ran its steps to the receiver (ACT-2). The receiver
    /// verifies as the destination.
    DisplaySwitched {
        device: DeviceId,
    },
    /// The sender asks the receiver to control it on this device (ACT-4).
    ControlRequest {
        device: DeviceId,
    },
    /// The receiver of a `ControlRequest` does not attach (ACT-6).
    ControlRefused {
        device: DeviceId,
        reason: ControlRefuseReason,
    },
}

/// Why the receiver of a control request does not attach (ACT-6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ControlRefuseReason {
    /// The receiver has no device with this ID.
    UnknownDevice,
    /// The receiver has no `handoff.<sender>` route for this device.
    NoRoute,
    /// No display of the receiver matches the device.
    NoDisplay,
    /// More than one display of the receiver matches the device.
    AmbiguousDisplay,
    /// `portal-match` failed on a display of the receiver (AC-9).
    MatchFailed,
    /// The receiver cannot capture or suppress input.
    NoCapture,
    /// The receiver already is the source of a session on the matching display.
    Busy,
}

/// The result of the old target's route back to the old source.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum HandBackOutcome {
    /// The steps succeeded; the old source verifies as the destination (HB-1).
    Done,
    /// The target has no `handoff.<source>` route (HB-4).
    NoRoute,
    /// A step failed. `partial` is true when other steps completed (FLOW-12).
    Failed { partial: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum RefuseReason {
    /// The target has no device with this ID.
    UnknownDevice,
    /// The config of the target has no handoff route from the source to the target
    /// for this device.
    NoRoute,
    /// The device has no `input-target`, or it failed (FLOW-5).
    NoInputTarget,
    /// The target cannot inject input (permission or backend).
    NoInjection,
    /// The target has no focus holder (UI-7, AC-29).
    NoFocusHold,
    /// A different session holds the lease of the device (AC-20).
    Busy,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EndReason {
    /// The session ended for a hand back (HB-1).
    HandBack,
    /// The safety exit on the source (IN-8).
    SafetyExit,
    /// The user ended the session in the UI of the source.
    SourceClosed,
    /// The target app quit (AC-18).
    TargetDisconnected,
    /// The user selected "Disconnect" on the target (FLOW-17).
    TargetStopped,
    /// The target requests control back (FLOW-17).
    TargetRequestedControl,
    /// The verifier of the destination answered not ok, or failed (FLOW-11).
    VerifyFailed,
    /// A handoff step failed with no completed step (FLOW-12).
    HandoffFailed,
    /// The window of the verifier could not show on the target.
    PromptFailed,
    /// The input-target display is no longer valid (FLOW-10).
    DisplayGone,
    /// The portal display disappeared on the source (FLOW-19).
    PortalGone,
    /// The input queue overflowed (IN-3).
    QueueOverflow,
    /// A permission was revoked, or the OS blocks capture or injection.
    PermissionLost,
    /// The connection closed or the heartbeat timed out.
    PeerLost,
    /// A message did not match the session state.
    ProtocolError,
    /// A new handoff of the same device ended this session (FLOW-6).
    NewHandoff,
    /// A handoff step failed after other steps completed; the device state is
    /// unknown (FLOW-12).
    HandoffPartial,
}

/// The latest pointer position, on the unreliable channel (NET-2).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PointerDatagram {
    pub session: SessionId,
    /// Increases by one for each datagram of a session. The target drops a datagram
    /// with a sequence number that is not larger than the last one that it used.
    pub seq: u64,
    /// Microseconds since the first pointer datagram of this session on the
    /// monotonic clock of the source (NET-8).
    pub sent_us: u64,
    /// The position in the portal, which the target maps to its input-target display.
    pub position: Normalized,
}

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error("encode: {0}")]
    Encode(postcard::Error),
    #[error("decode: {0}")]
    Decode(postcard::Error),
    #[error("frame of {0} bytes is larger than the limit")]
    TooLarge(usize),
}

/// Encodes a message payload, with no length prefix.
pub fn encode_message(message: &Message) -> Result<Vec<u8>, WireError> {
    let bytes = postcard::to_stdvec(message).map_err(WireError::Encode)?;
    if bytes.len() > MAX_FRAME {
        return Err(WireError::TooLarge(bytes.len()));
    }
    Ok(bytes)
}

pub fn decode_message(bytes: &[u8]) -> Result<Message, WireError> {
    if bytes.len() > MAX_FRAME {
        return Err(WireError::TooLarge(bytes.len()));
    }
    postcard::from_bytes(bytes).map_err(WireError::Decode)
}

pub fn encode_pointer(datagram: &PointerDatagram) -> Result<Vec<u8>, WireError> {
    postcard::to_stdvec(datagram).map_err(WireError::Encode)
}

pub fn decode_pointer(bytes: &[u8]) -> Result<PointerDatagram, WireError> {
    postcard::from_bytes(bytes).map_err(WireError::Decode)
}

/// Keeps the newest pointer sequence number of a session (NET-2).
#[derive(Debug, Default, Clone)]
pub struct PointerOrder {
    last: Option<(SessionId, u64)>,
}

impl PointerOrder {
    /// Returns true if the datagram is newer than all datagrams before it in its
    /// session. A datagram of a newer session starts a new sequence.
    pub fn accept(&mut self, datagram: &PointerDatagram) -> bool {
        let newer = match self.last {
            Some((session, seq)) if session == datagram.session => datagram.seq > seq,
            Some((session, _)) => datagram.session > session,
            None => true,
        };
        if newer {
            self.last = Some((datagram.session, datagram.seq));
        }
        newer
    }
}
