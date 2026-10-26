//! The session state machine of the source (FLOW-5..FLOW-19).

use std::time::Instant;

use crate::ids::{ComputerId, DeviceId, SessionId};
use crate::input::{HeldInputs, InputEvent};
use crate::pointer::Normalized;
use crate::wire::{EndReason, Message, PointerDatagram, RefuseReason};

use super::{CONFIRM_TIMEOUT, VideoInput};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourcePhase {
    /// `RequestSession` is sent; waiting for the lease (FLOW-5).
    Requesting,
    /// The lease is granted; the handoff steps run (FLOW-6).
    HandingOff,
    /// `HandoffDone` is sent; waiting for `PromptShown` (FLOW-7).
    AwaitingPrompt,
    /// The prompt is visible on the target; waiting for the user (FLOW-8).
    AwaitingConfirm,
    /// `Confirm` is sent; waiting for `Started` (FLOW-10).
    Starting,
    /// Input is captured, suppressed, and forwarded (FLOW-13).
    Forwarding,
    /// The pointer is out of the portal. The session stays (FLOW-14).
    Paused,
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
    /// Run the handoff steps (and `verify`, if present) for this target on a
    /// worker, then call [`SourceSession::on_handoff_finished`].
    RunHandoff,
    /// Start to capture, suppress, and forward input.
    StartForwarding,
    /// Stop suppression. Local input and the cursor come back.
    StopForwarding,
}

/// The result of the handoff steps and `verify`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HandoffResult {
    /// All steps completed. `verified` is the `verify` result, or `None` if the
    /// handoff has no `verify`.
    Done { verified: Option<bool> },
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
    confirm_deadline: Option<Instant>,
    pointer_inside: bool,
    held: HeldInputs,
    seq: u64,
}

impl SourceSession {
    /// Starts a session request to `target` for the shared device `device`.
    /// Returns `[Send(RequestSession { device })]`. The phase is `Requesting`, the
    /// video input is `Unchanged`, and the pointer is not in the portal.
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
            confirm_deadline: None,
            pointer_inside: false,
            held: HeldInputs::default(),
            seq: 0,
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

    pub fn is_finished(&self) -> bool {
        matches!(self.phase, SourcePhase::Refused(_) | SourcePhase::Ended(_))
    }

    /// Handles a message from the target. A message with a different session ID
    /// is stale: it has no effect and returns no actions. The one exception: a
    /// `SessionGranted` for this device after the session finished returns
    /// `Send(End { session, reason: SourceClosed })`, so that the target releases
    /// the lease.
    pub fn on_message(&mut self, message: &Message, now: Instant) -> Vec<SourceAction> {
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
            Message::PromptShown { session }
                if self.session == Some(session) && self.phase == SourcePhase::AwaitingPrompt =>
            {
                let Some(deadline) = now.checked_add(CONFIRM_TIMEOUT) else {
                    return self.finish(EndReason::ProtocolError, false);
                };
                self.confirm_deadline = Some(deadline);
                self.phase = SourcePhase::AwaitingConfirm;
            }
            Message::Started { session }
                if self.session == Some(session) && self.phase == SourcePhase::Starting =>
            {
                self.phase = SourcePhase::Paused;
                return self.set_pointer_inside(self.pointer_inside);
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
            HandoffResult::Done {
                verified: Some(false),
            } => {
                self.video = VideoInput::Unknown;
                self.end(EndReason::HandoffFailed)
            }
            HandoffResult::Failed { completed_steps } => {
                if completed_steps > 0 {
                    self.video = VideoInput::Unknown;
                }
                self.end(EndReason::HandoffFailed)
            }
            HandoffResult::Done { verified } => {
                self.video = if verified == Some(true) {
                    VideoInput::Confirmed
                } else {
                    VideoInput::Unverified
                };
                self.phase = SourcePhase::AwaitingPrompt;
                vec![SourceAction::Send(Message::HandoffDone { session })]
            }
        }
    }

    /// The user pressed the confirmation key or clicked in the portal (FLOW-8).
    /// Returns `None` if this is not a confirmation: then the key is a normal key.
    /// It is a confirmation only in `AwaitingConfirm` and only if the portal has
    /// the window focus. The app suppresses a confirmation (FLOW-9).
    pub fn confirm(&mut self, portal_focused: bool) -> Option<Vec<SourceAction>> {
        if self.phase != SourcePhase::AwaitingConfirm || !portal_focused {
            return None;
        }
        let session = self.session?;
        self.phase = SourcePhase::Starting;
        self.video = VideoInput::Confirmed;
        self.confirm_deadline = None;
        Some(vec![SourceAction::Send(Message::Confirm { session })])
    }

    /// The pointer entered (`true`) or left (`false`) the portal (FLOW-13, FLOW-14).
    pub fn set_pointer_inside(&mut self, inside: bool) -> Vec<SourceAction> {
        self.pointer_inside = inside;
        match (self.phase, inside) {
            (SourcePhase::Forwarding, false) => {
                self.phase = SourcePhase::Paused;
                let _ = self.held.release_all();
                let mut actions = vec![SourceAction::StopForwarding];
                if let Some(session) = self.session {
                    actions.push(SourceAction::Send(Message::ReleaseAll { session }));
                }
                actions
            }
            (SourcePhase::Paused, true) => {
                self.phase = SourcePhase::Forwarding;
                vec![SourceAction::StartForwarding]
            }
            _ => Vec::new(),
        }
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

    /// The pointer position in the portal. Returns a datagram only while forwarding.
    pub fn pointer(&mut self, position: Normalized) -> Option<PointerDatagram> {
        if self.phase != SourcePhase::Forwarding {
            return None;
        }
        let session = self.session?;
        self.seq = self.seq.checked_add(1)?;
        Some(PointerDatagram {
            session,
            seq: self.seq,
            position,
        })
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

    /// Checks the timers (FLOW-11).
    pub fn tick(&mut self, now: Instant) -> Vec<SourceAction> {
        if self.phase == SourcePhase::AwaitingConfirm
            && self
                .confirm_deadline
                .is_some_and(|deadline| now >= deadline)
        {
            self.video = VideoInput::Unknown;
            return self.end(EndReason::ConfirmTimeout);
        }
        Vec::new()
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
        self.confirm_deadline = None;
        // An unverified switch is unknown after the end. An unchanged input stays
        // unchanged, and a confirmed input stays confirmed (FLOW-16).
        if self.video == VideoInput::Unverified {
            self.video = VideoInput::Unknown;
        }
        self.phase = SourcePhase::Ended(reason);
        actions
    }
}
