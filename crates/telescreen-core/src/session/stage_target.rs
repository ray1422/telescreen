//! The session of a target in the login stage (STG-5, ADR 0015), with no I/O.
//!
//! It is a different type from [`super::target::TargetSession`] on purpose: a
//! stage session has no display, no pointer, no verifier, and no keyboard
//! holder, so this type has no action for them. It injects key events only.

use crate::ids::{ComputerId, DeviceId, SessionId};
use crate::input::{HeldInputs, InputEvent};
use crate::wire::{EndReason, Message};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StageTargetPhase {
    /// The session is granted. The source runs its handoff steps.
    AwaitingHandoff,
    /// Key events from the source are injected.
    Forwarding,
    /// Terminal.
    Ended(EndReason),
}

/// An action for the app.
#[derive(Debug, Clone, PartialEq)]
pub enum StageTargetAction {
    /// Send on the reliable channel to the source.
    Send(Message),
    /// Inject a key event. The event is always [`InputEvent::Key`].
    Inject(InputEvent),
}

#[derive(Debug)]
pub struct StageTargetSession {
    session: SessionId,
    source: ComputerId,
    device: DeviceId,
    phase: StageTargetPhase,
    held: HeldInputs,
}

impl StageTargetSession {
    /// Makes a session for `source`. Returns
    /// `[Send(SessionGranted { session, device })]`.
    pub fn new(
        session: SessionId,
        source: ComputerId,
        device: DeviceId,
    ) -> (Self, Vec<StageTargetAction>) {
        let actions = vec![StageTargetAction::Send(Message::SessionGranted {
            session,
            device: device.clone(),
        })];
        let target = Self {
            session,
            source,
            device,
            phase: StageTargetPhase::AwaitingHandoff,
            held: HeldInputs::default(),
        };
        (target, actions)
    }

    pub fn session(&self) -> SessionId {
        self.session
    }

    pub fn source(&self) -> &ComputerId {
        &self.source
    }

    pub fn device(&self) -> &DeviceId {
        &self.device
    }

    pub fn phase(&self) -> StageTargetPhase {
        self.phase
    }

    pub fn is_finished(&self) -> bool {
        matches!(self.phase, StageTargetPhase::Ended(_))
    }

    /// Handles a message from the source. A message with a different session
    /// ID, and a message that does not match the phase, have no effect.
    pub fn on_message(&mut self, message: &Message) -> Vec<StageTargetAction> {
        if self.is_finished() {
            return Vec::new();
        }
        let session = match message {
            Message::HandoffDone { session }
            | Message::Input { session, .. }
            | Message::ReleaseAll { session }
            | Message::End { session, .. } => session,
            _ => return Vec::new(),
        };
        if *session != self.session {
            return Vec::new();
        }
        match message {
            Message::HandoffDone { .. } if self.phase == StageTargetPhase::AwaitingHandoff => {
                self.phase = StageTargetPhase::Forwarding;
                vec![StageTargetAction::Send(Message::Started {
                    session: self.session,
                    verified: false,
                })]
            }
            Message::Input {
                event: event @ InputEvent::Key { .. },
                ..
            } if self.phase == StageTargetPhase::Forwarding => {
                if self.held.record(event) {
                    vec![StageTargetAction::Inject(*event)]
                } else {
                    Vec::new()
                }
            }
            Message::ReleaseAll { .. } if self.phase == StageTargetPhase::Forwarding => {
                self.release_held()
            }
            Message::End { reason, .. } => self.finish(*reason, false),
            _ => Vec::new(),
        }
    }

    /// Ends the session on this target: releases each held key, then sends
    /// `End` with `reason`. No effect after the end.
    pub fn end(&mut self, reason: EndReason) -> Vec<StageTargetAction> {
        self.finish(reason, true)
    }

    /// The connection to the source is gone: releases each held key and ends
    /// with `PeerLost`. It sends nothing.
    pub fn on_peer_lost(&mut self) -> Vec<StageTargetAction> {
        self.finish(EndReason::PeerLost, false)
    }

    fn release_held(&mut self) -> Vec<StageTargetAction> {
        self.held
            .release_all()
            .into_iter()
            .map(StageTargetAction::Inject)
            .collect()
    }

    fn finish(&mut self, reason: EndReason, notify_source: bool) -> Vec<StageTargetAction> {
        if self.is_finished() {
            return Vec::new();
        }
        let mut actions = self.release_held();
        if notify_source {
            actions.push(StageTargetAction::Send(Message::End {
                session: self.session,
                reason,
            }));
        }
        self.phase = StageTargetPhase::Ended(reason);
        actions
    }
}
