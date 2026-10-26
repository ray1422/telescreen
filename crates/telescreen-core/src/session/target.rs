//! The session state machine of the target (FLOW-5, FLOW-7, FLOW-10, FLOW-16, IN-4).

use crate::ids::{ComputerId, DeviceId, DisplayToken, SessionId};
use crate::input::{HeldInputs, InputEvent};
use crate::pointer::Normalized;
use crate::wire::{EndReason, Message, PointerDatagram, PointerOrder};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetPhase {
    /// The lease is granted; waiting for `HandoffDone`.
    AwaitingHandoff,
    /// `ShowPrompt` is returned; waiting for [`TargetSession::on_prompt_shown`].
    ShowingPrompt,
    /// The prompt is visible; waiting for `Confirm`.
    Prompting,
    /// `Confirm` arrived; waiting for [`TargetSession::on_display_checked`].
    CheckingDisplay,
    /// Input is injected.
    Forwarding,
    /// Terminal.
    Ended(EndReason),
}

/// An action for the app.
#[derive(Debug, Clone, PartialEq)]
pub enum TargetAction {
    /// Send on the reliable channel to the source.
    Send(Message),
    /// Show the prompt on the input-target display, then call
    /// [`TargetSession::on_prompt_shown`].
    ShowPrompt,
    HidePrompt,
    /// Check that the input-target display token is still valid, then call
    /// [`TargetSession::on_display_checked`].
    CheckDisplay,
    Inject(InputEvent),
    MovePointer(Normalized),
    /// Release the device lease of this session.
    ReleaseLease,
}

#[derive(Debug)]
pub struct TargetSession {
    session: SessionId,
    source: ComputerId,
    device: DeviceId,
    display: DisplayToken,
    phase: TargetPhase,
    held: HeldInputs,
    pointer_order: PointerOrder,
}

impl TargetSession {
    /// Makes a session after the app acquired the lease and resolved the
    /// input-target display (FLOW-5). Returns `[Send(SessionGranted { session, device })]`.
    pub fn new(
        session: SessionId,
        source: ComputerId,
        device: DeviceId,
        display: DisplayToken,
    ) -> (Self, Vec<TargetAction>) {
        let actions = vec![TargetAction::Send(Message::SessionGranted {
            session,
            device: device.clone(),
        })];
        let target = Self {
            session,
            source,
            device,
            display,
            phase: TargetPhase::AwaitingHandoff,
            held: HeldInputs::default(),
            pointer_order: PointerOrder::default(),
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

    pub fn display(&self) -> &DisplayToken {
        &self.display
    }

    pub fn phase(&self) -> TargetPhase {
        self.phase
    }

    pub fn is_finished(&self) -> bool {
        matches!(self.phase, TargetPhase::Ended(_))
    }

    /// Handles a message from the source. A message with a different session ID is
    /// stale: no effect, no actions. A message that does not match the phase has
    /// no effect and returns no actions.
    pub fn on_message(&mut self, message: &Message) -> Vec<TargetAction> {
        let session = match message {
            Message::HandoffDone { session }
            | Message::Confirm { session }
            | Message::Input { session, .. }
            | Message::ReleaseAll { session }
            | Message::End { session, .. } => session,
            _ => return Vec::new(),
        };
        if *session != self.session {
            return Vec::new();
        }
        match message {
            Message::HandoffDone { .. } if self.phase == TargetPhase::AwaitingHandoff => {
                self.phase = TargetPhase::ShowingPrompt;
                vec![TargetAction::ShowPrompt]
            }
            Message::Confirm { .. } if self.phase == TargetPhase::Prompting => {
                self.phase = TargetPhase::CheckingDisplay;
                vec![TargetAction::CheckDisplay]
            }
            Message::Input { event, .. } if self.phase == TargetPhase::Forwarding => {
                if self.held.record(event) {
                    vec![TargetAction::Inject(*event)]
                } else {
                    Vec::new()
                }
            }
            Message::ReleaseAll { .. } if self.phase == TargetPhase::Forwarding => {
                self.release_held()
            }
            Message::End { reason, .. } => {
                self.finish(*reason, false, self.prompt_can_be_visible())
            }
            _ => Vec::new(),
        }
    }

    /// Reports the result of `ShowPrompt`.
    pub fn on_prompt_shown(&mut self, shown: bool) -> Vec<TargetAction> {
        if self.phase != TargetPhase::ShowingPrompt {
            return self.unexpected_callback();
        }
        if !shown {
            // The app confirmed that the prompt did not show, so there is nothing to hide.
            return self.finish(EndReason::PromptFailed, true, false);
        }
        self.phase = TargetPhase::Prompting;
        vec![TargetAction::Send(Message::PromptShown {
            session: self.session,
        })]
    }

    /// Reports the result of `CheckDisplay` (FLOW-10).
    pub fn on_display_checked(&mut self, valid: bool) -> Vec<TargetAction> {
        if self.phase != TargetPhase::CheckingDisplay {
            return self.unexpected_callback();
        }
        if !valid {
            return self.end(EndReason::DisplayGone);
        }
        self.phase = TargetPhase::Forwarding;
        vec![
            TargetAction::HidePrompt,
            TargetAction::Send(Message::Started {
                session: self.session,
            }),
        ]
    }

    /// A pointer datagram. Returns `MovePointer` only while forwarding, only for this
    /// session, and only for a datagram newer than the last one used (NET-2).
    pub fn on_pointer(&mut self, datagram: &PointerDatagram) -> Option<TargetAction> {
        if self.phase == TargetPhase::Forwarding
            && datagram.session == self.session
            && self.pointer_order.accept(datagram)
        {
            Some(TargetAction::MovePointer(datagram.position))
        } else {
            None
        }
    }

    /// Ends the session on the target: "disconnect", a request for control
    /// (FLOW-17), a lost display, or a lost permission.
    ///
    /// The actions of each end, in this order: `Inject` up events for all held
    /// input, `HidePrompt` if the prompt can be visible (`ShowingPrompt`, `Prompting`,
    /// `CheckingDisplay`), `Send(End)` (only for an end on this peer), `ReleaseLease`.
    /// An end of a finished session returns no actions.
    pub fn end(&mut self, reason: EndReason) -> Vec<TargetAction> {
        self.finish(reason, true, self.prompt_can_be_visible())
    }

    /// The connection to the source closed or its heartbeat timed out. The target
    /// releases all held input by itself (IN-4).
    pub fn on_peer_lost(&mut self) -> Vec<TargetAction> {
        self.finish(EndReason::PeerLost, false, self.prompt_can_be_visible())
    }

    fn release_held(&mut self) -> Vec<TargetAction> {
        self.held
            .release_all()
            .into_iter()
            .map(TargetAction::Inject)
            .collect()
    }

    fn prompt_can_be_visible(&self) -> bool {
        matches!(
            self.phase,
            TargetPhase::ShowingPrompt | TargetPhase::Prompting | TargetPhase::CheckingDisplay
        )
    }

    fn unexpected_callback(&mut self) -> Vec<TargetAction> {
        self.finish(
            EndReason::ProtocolError,
            false,
            self.prompt_can_be_visible(),
        )
    }

    fn finish(
        &mut self,
        reason: EndReason,
        notify_source: bool,
        hide_prompt: bool,
    ) -> Vec<TargetAction> {
        if self.is_finished() {
            return Vec::new();
        }
        let mut actions = self.release_held();
        if hide_prompt {
            actions.push(TargetAction::HidePrompt);
        }
        if notify_source {
            actions.push(TargetAction::Send(Message::End {
                session: self.session,
                reason,
            }));
        }
        actions.push(TargetAction::ReleaseLease);
        self.phase = TargetPhase::Ended(reason);
        actions
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::Button;
    use crate::keys::Key;

    const SESSION: SessionId = SessionId(42);

    fn at_phase(phase: TargetPhase) -> TargetSession {
        let (mut target, _) = TargetSession::new(
            SESSION,
            ComputerId::from("source"),
            DeviceId::from("display"),
            DisplayToken::from("target-display"),
        );
        if phase == TargetPhase::AwaitingHandoff {
            return target;
        }
        target.on_message(&Message::HandoffDone { session: SESSION });
        if phase == TargetPhase::ShowingPrompt {
            return target;
        }
        target.on_prompt_shown(true);
        if phase == TargetPhase::Prompting {
            return target;
        }
        target.on_message(&Message::Confirm { session: SESSION });
        if phase == TargetPhase::CheckingDisplay {
            return target;
        }
        target.on_display_checked(true);
        target
    }

    #[test]
    fn unexpected_callbacks_end_without_sending_and_release_held_input() {
        for phase in [
            TargetPhase::AwaitingHandoff,
            TargetPhase::ShowingPrompt,
            TargetPhase::Prompting,
            TargetPhase::CheckingDisplay,
            TargetPhase::Forwarding,
        ] {
            for prompt_callback in [false, true] {
                if (prompt_callback && phase == TargetPhase::ShowingPrompt)
                    || (!prompt_callback && phase == TargetPhase::CheckingDisplay)
                {
                    continue;
                }
                for result in [false, true] {
                    let mut target = at_phase(phase);
                    let key = Key(30);
                    target.on_message(&Message::Input {
                        session: SESSION,
                        event: InputEvent::Key { key, down: true },
                    });
                    let mut expected = Vec::new();
                    if phase == TargetPhase::Forwarding {
                        expected.push(TargetAction::Inject(InputEvent::Key { key, down: false }));
                    }
                    if matches!(
                        phase,
                        TargetPhase::ShowingPrompt
                            | TargetPhase::Prompting
                            | TargetPhase::CheckingDisplay
                    ) {
                        expected.push(TargetAction::HidePrompt);
                    }
                    expected.push(TargetAction::ReleaseLease);
                    let actions = if prompt_callback {
                        target.on_prompt_shown(result)
                    } else {
                        target.on_display_checked(result)
                    };
                    assert_eq!(actions, expected);
                    assert_eq!(target.phase(), TargetPhase::Ended(EndReason::ProtocolError));
                    assert!(
                        target
                            .on_message(&Message::Input {
                                session: SESSION,
                                event: InputEvent::Key { key, down: true },
                            })
                            .is_empty()
                    );
                }
            }
        }
    }

    #[test]
    fn callbacks_cannot_restart_a_finished_session() {
        let mut target = at_phase(TargetPhase::ShowingPrompt);
        target.on_peer_lost();
        for result in [false, true] {
            assert!(target.on_prompt_shown(result).is_empty());
            assert!(target.on_display_checked(result).is_empty());
        }
        assert_eq!(target.phase(), TargetPhase::Ended(EndReason::PeerLost));
    }

    #[test]
    fn ignored_datagrams_do_not_advance_pointer_order() {
        let mut target = at_phase(TargetPhase::CheckingDisplay);
        let mut datagram = PointerDatagram {
            session: SESSION,
            seq: 100,
            position: Normalized { x: 0.5, y: 0.5 },
        };
        assert_eq!(target.on_pointer(&datagram), None);
        target.on_display_checked(true);
        datagram.session = SessionId(43);
        assert_eq!(target.on_pointer(&datagram), None);
        datagram.session = SESSION;
        datagram.seq = 0;
        assert_eq!(
            target.on_pointer(&datagram),
            Some(TargetAction::MovePointer(datagram.position))
        );
    }

    #[test]
    fn button_pairing_and_scroll_use_held_inputs() {
        let mut target = at_phase(TargetPhase::Forwarding);
        let down = InputEvent::Button {
            button: Button::Left,
            down: true,
        };
        let up = InputEvent::Button {
            button: Button::Left,
            down: false,
        };
        let scroll = InputEvent::Scroll { dx: 1.0, dy: -2.0 };
        let input = |event| Message::Input {
            session: SESSION,
            event,
        };
        assert!(target.on_message(&input(up)).is_empty());
        assert_eq!(
            target.on_message(&input(down)),
            vec![TargetAction::Inject(down)]
        );
        assert!(target.on_message(&input(down)).is_empty());
        assert_eq!(
            target.on_message(&input(scroll)),
            vec![TargetAction::Inject(scroll)]
        );
        assert_eq!(
            target.on_message(&input(up)),
            vec![TargetAction::Inject(up)]
        );
        assert_eq!(target.on_peer_lost(), vec![TargetAction::ReleaseLease]);
    }
}
