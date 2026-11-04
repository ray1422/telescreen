//! The session state machine of the source (FLOW-5..FLOW-19).

use std::time::Instant;

use crate::ids::{ComputerId, DeviceId, SessionId};
use crate::input::{HeldInputs, InputEvent};
use crate::pointer::Normalized;
use crate::wire::{EndReason, Message, PointerDatagram, RefuseReason};

use super::VideoInput;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourcePhase {
    /// `RequestSession` is sent; waiting for the lease (FLOW-5).
    Requesting,
    /// The lease is granted; the handoff steps run (FLOW-6).
    HandingOff,
    /// `HandoffDone` is sent; the destination verifies until `Started` (FLOW-10).
    Verifying,
    /// Input is captured, suppressed, and forwarded (FLOW-13).
    Forwarding,
    /// The target refused the session. Terminal.
    Refused(RefuseReason),
    /// Terminal.
    Ended(EndReason),
}

/// An action for the app.
#[derive(Debug, Clone, PartialEq)]
pub enum SourceAction {
    /// Send on the reliable channel to the target.
    Send(Message),
    /// Run the handoff steps for this target on a
    /// worker, then call [`SourceSession::on_handoff_finished`].
    RunHandoff,
    /// Start to capture, suppress, and forward input.
    StartForwarding,
    /// Stop suppression. Local input and the cursor come back.
    StopForwarding,
}

/// The result of the handoff steps. The destination verifies separately.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffResult {
    /// All steps completed.
    Done,
    /// A step failed after `completed_steps` steps completed.
    Failed { completed_steps: usize },
}

#[derive(Debug)]
pub struct SourceSession {
    target: ComputerId,
    device: DeviceId,
    session: Option<SessionId>,
    phase: SourcePhase,
    video: VideoInput,
    ack_wanted: bool,
    keyboard_in_portal: bool,
    held: HeldInputs,
    seq: u64,
    /// The position of the last pointer datagram (NET-2).
    last_pointer: Option<Normalized>,
}

impl SourceSession {
    /// Starts a session request to `target` for the shared device `device`.
    /// Returns `[Send(RequestSession { device })]`. The phase is `Requesting`, the
    /// video input is `Unchanged`.
    pub fn start(target: ComputerId, device: DeviceId) -> (Self, Vec<SourceAction>) {
        let actions = vec![SourceAction::Send(Message::RequestSession {
            device: device.clone(),
        })];
        let source = Self {
            target,
            device,
            session: None,
            phase: SourcePhase::Requesting,
            video: VideoInput::Unchanged,
            ack_wanted: false,
            keyboard_in_portal: true,
            held: HeldInputs::default(),
            seq: 0,
            last_pointer: None,
        };
        (source, actions)
    }

    pub fn target(&self) -> &ComputerId {
        &self.target
    }

    pub fn device(&self) -> &DeviceId {
        &self.device
    }

    /// The session ID, after the target granted the lease.
    pub fn session(&self) -> Option<SessionId> {
        self.session
    }

    pub fn phase(&self) -> SourcePhase {
        self.phase
    }

    pub fn video_input(&self) -> VideoInput {
        self.video
    }

    /// Whether the verifier of the target waits for an acknowledgement.
    pub fn ack_wanted(&self) -> bool {
        self.ack_wanted
    }

    pub fn is_finished(&self) -> bool {
        matches!(self.phase, SourcePhase::Refused(_) | SourcePhase::Ended(_))
    }

    /// Handles a message from the target. A message with a different session ID
    /// is stale: it has no effect and returns no actions. The one exception: a
    /// `SessionGranted` for this device after the session finished returns
    /// `Send(End { session, reason: SourceClosed })`, so that the target releases
    /// the lease.
    pub fn on_message(&mut self, message: &Message, _now: Instant) -> Vec<SourceAction> {
        match *message {
            Message::SessionGranted {
                session,
                ref device,
            } if *device == self.device => {
                if self.is_finished() {
                    return vec![SourceAction::Send(Message::End {
                        session,
                        reason: EndReason::SourceClosed,
                    })];
                }
                if self.phase == SourcePhase::Requesting {
                    self.session = Some(session);
                    self.phase = SourcePhase::HandingOff;
                    return vec![SourceAction::RunHandoff];
                }
            }
            Message::SessionRefused { ref device, reason }
                if *device == self.device && self.phase == SourcePhase::Requesting =>
            {
                self.phase = SourcePhase::Refused(reason);
            }
            Message::AckWanted { session }
                if self.session == Some(session) && self.phase == SourcePhase::Verifying =>
            {
                self.ack_wanted = true;
            }
            Message::Started { session, verified }
                if self.session == Some(session) && self.phase == SourcePhase::Verifying =>
            {
                self.phase = SourcePhase::Forwarding;
                self.ack_wanted = false;
                if verified {
                    self.video = VideoInput::Confirmed;
                }
                self.last_pointer = None;
                self.keyboard_in_portal = true;
                return vec![SourceAction::StartForwarding];
            }
            Message::End { session, reason } if self.session == Some(session) => {
                return self.finish(reason, false);
            }
            _ => {}
        }
        Vec::new()
    }

    /// Reports the result of `RunHandoff`.
    pub fn on_handoff_finished(&mut self, result: HandoffResult) -> Vec<SourceAction> {
        if self.phase != SourcePhase::HandingOff {
            return Vec::new();
        }
        let Some(session) = self.session else {
            return Vec::new();
        };
        match result {
            HandoffResult::Failed { completed_steps } => {
                if completed_steps > 0 {
                    self.video = VideoInput::Unknown;
                    self.end(EndReason::HandoffPartial)
                } else {
                    self.end(EndReason::HandoffFailed)
                }
            }
            HandoffResult::Done => {
                self.video = VideoInput::Unverified;
                self.phase = SourcePhase::Verifying;
                vec![SourceAction::Send(Message::HandoffDone { session })]
            }
        }
    }

    /// The user pressed Return or clicked in the portal (FLOW-8). Accepts one
    /// acknowledgement per `AckWanted`, only while verifying with portal focus.
    /// The app suppresses an accepted acknowledgement (FLOW-9). Otherwise this
    /// returns `None`, and the key is a normal key.
    pub fn acknowledge(&mut self, portal_focused: bool) -> Option<Vec<SourceAction>> {
        if self.phase != SourcePhase::Verifying || !self.ack_wanted || !portal_focused {
            return None;
        }
        let session = self.session?;
        self.ack_wanted = false;
        Some(vec![SourceAction::Send(Message::Ack { session })])
    }

    /// Reports a keyboard focus change while forwarding (FLOW-22, FLOW-23).
    /// The keyboard starts in the portal when forwarding starts.
    pub fn on_keyboard(&mut self, in_portal: bool) -> Vec<SourceAction> {
        if self.phase != SourcePhase::Forwarding || self.keyboard_in_portal == in_portal {
            return Vec::new();
        }
        let Some(session) = self.session else {
            return Vec::new();
        };
        self.keyboard_in_portal = in_portal;
        let message = if in_portal {
            Message::KeyboardBack { session }
        } else {
            Message::KeyboardAway { session }
        };
        vec![SourceAction::Send(message)]
    }

    /// Releases held input when the capture can no longer see up events (IN-13).
    /// The session stays in forwarding (FLOW-14).
    pub fn on_input_lost(&mut self) -> Vec<SourceAction> {
        if self.phase != SourcePhase::Forwarding {
            return Vec::new();
        }
        let Some(session) = self.session else {
            return Vec::new();
        };
        let _ = self.held.release_all();
        vec![SourceAction::Send(Message::ReleaseAll { session })]
    }

    /// A captured key, button, or scroll event. It goes to the target only while
    /// forwarding (IN-3).
    pub fn on_input(&mut self, event: InputEvent) -> Vec<SourceAction> {
        if self.phase != SourcePhase::Forwarding {
            return Vec::new();
        }
        let Some(session) = self.session else {
            return Vec::new();
        };
        if self.held.record(&event) {
            vec![SourceAction::Send(Message::Input { session, event })]
        } else {
            Vec::new()
        }
    }

    /// The pointer position in the portal. Returns a datagram only while forwarding,
    /// and only if the position is different from the last datagram (NET-2).
    pub fn pointer(&mut self, position: Normalized) -> Option<PointerDatagram> {
        if self.phase != SourcePhase::Forwarding || self.last_pointer == Some(position) {
            return None;
        }
        let session = self.session?;
        self.last_pointer = Some(position);
        self.seq = self.seq.checked_add(1)?;
        Some(PointerDatagram {
            session,
            seq: self.seq,
            position,
        })
    }

    /// The next `pointer` call returns a datagram also for the same position (NET-2).
    pub fn forget_pointer(&mut self) {
        self.last_pointer = None;
    }

    /// Ends the session on the source (FLOW-16): the release shortcut, the safety
    /// exit, the UI, a lost portal display, a queue overflow, or a lost permission.
    ///
    /// The actions of each end, in this order: `StopForwarding` if the phase is
    /// `Forwarding`, then `Send(End)` if the target granted a session and the end is
    /// on this peer. An end of a finished session returns no actions.
    pub fn end(&mut self, reason: EndReason) -> Vec<SourceAction> {
        self.finish(reason, true)
    }

    /// The connection to the target closed or its heartbeat timed out.
    pub fn on_peer_lost(&mut self) -> Vec<SourceAction> {
        self.finish(EndReason::PeerLost, false)
    }

    fn finish(&mut self, reason: EndReason, notify_target: bool) -> Vec<SourceAction> {
        if self.is_finished() {
            return Vec::new();
        }
        let mut actions = Vec::new();
        if self.phase == SourcePhase::Forwarding {
            actions.push(SourceAction::StopForwarding);
        }
        if notify_target && let Some(session) = self.session {
            actions.push(SourceAction::Send(Message::End { session, reason }));
        }
        let _ = self.held.release_all();
        self.ack_wanted = false;
        // An unverified switch is unknown after the end. An unchanged input stays
        // unchanged, and a confirmed input stays confirmed (FLOW-16).
        if self.video == VideoInput::Unverified {
            self.video = VideoInput::Unknown;
        }
        self.phase = SourcePhase::Ended(reason);
        actions
    }
}
