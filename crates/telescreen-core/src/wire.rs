//! Wire messages between two peers (NET-1, NET-2, NET-6) and their encoding.
//!
//! The reliable channel carries [`Message`] frames. The unreliable channel carries
//! [`PointerDatagram`]s. Both use postcard. A reliable frame is a 4-byte
//! little-endian length and the payload.
//!
//! Protocol (tentative, `docs/adr/0007-session-protocol.md`):
//!
//! ```text
//! source                         target
//! Hello  ───────────────────────▶          (each side sends Hello first)
//! RequestSession{device} ───────▶          lease + input-target (FLOW-5)
//!        ◀─────────────────────── SessionGranted{session} | SessionRefused
//! (run handoff steps, FLOW-6)
//! HandoffDone{session} ─────────▶          show prompt (FLOW-7)
//!        ◀─────────────────────── PromptShown{session}
//! (user confirms, FLOW-8)
//! Confirm{session} ─────────────▶          check display, remove prompt (FLOW-10)
//!        ◀─────────────────────── Started{session}
//! Input / ReleaseAll / datagrams ▶         inject
//! KeyboardAway{session} ────────▶          hold keyboard focus (FLOW-23)
//! KeyboardBack{session} ────────▶          release keyboard focus holder
//! ClipboardText{session, text} ◀▶          sync text while forwarding (CLIP-2)
//! End{session, reason}  ◀──────▶  End{session, reason}
//! Heartbeat ◀──────────────────▶ Heartbeat (each second; the transport owns it)
//! ```

use serde::{Deserialize, Serialize};

use crate::ids::{ComputerId, DeviceId, SessionId};
use crate::input::InputEvent;
use crate::pointer::Normalized;

/// The wire protocol version. A peer refuses a different version.
pub const PROTOCOL_VERSION: u32 = 3;

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
    Confirm {
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
    /// Plain text in both directions while forwarding (CLIP-2).
    ClipboardText {
        session: SessionId,
        text: String,
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
    PromptShown {
        session: SessionId,
    },
    Started {
        session: SessionId,
    },

    // Both directions.
    End {
        session: SessionId,
        reason: EndReason,
    },
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
    /// The release shortcut on the source (IN-7).
    ReleaseShortcut,
    /// The safety exit on the source (IN-8).
    SafetyExit,
    /// The user ended the session in the UI of the source.
    SourceClosed,
    /// The target app quit (AC-18).
    TargetDisconnected,
    /// The user selected "Stop this session" on the target (FLOW-17).
    TargetStopped,
    /// The target requests control back (FLOW-17).
    TargetRequestedControl,
    /// No confirmation in the timeout (FLOW-11).
    ConfirmTimeout,
    /// A handoff step failed or `verify` was false (FLOW-12).
    HandoffFailed,
    /// The prompt could not show on the target.
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
}

/// The latest pointer position, on the unreliable channel (NET-2).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PointerDatagram {
    pub session: SessionId,
    /// Increases by one for each datagram of a session. The target drops a datagram
    /// with a sequence number that is not larger than the last one that it used.
    pub seq: u64,
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
