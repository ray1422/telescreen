//! The session state machine of the target (FLOW-5, FLOW-7, FLOW-10, FLOW-16, IN-4).

use crate::ids::{ComputerId, DeviceId, DisplayToken, SessionId};
use crate::input::{HeldInputs, InputEvent};
use crate::pointer::Normalized;
use crate::verification::{Verification, VerificationAction, VerificationResult};
use crate::verifier::VerifierEvent;
use crate::wire::{EndReason, Message, PointerDatagram, PointerOrder};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TargetPhase {
    /// The lease is granted; waiting for `HandoffDone`.
    AwaitingHandoff,
    /// The destination verifies the handoff, with optional text and acknowledgements.
    Verifying,
    /// Verification finished; waiting for [`TargetSession::on_display_checked`].
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
    /// Start the verifier of the local device, then report its events with
    /// [`TargetSession::on_verifier`], or call [`TargetSession::on_no_verify`]
    /// if the device has no `verify`.
    StartVerifier,
    /// Stop the verifier; it sends no more events.
    StopVerifier,
    /// Show the window of the verifier on the input-target display, or update
    /// it in place, then call [`TargetSession::on_verifier_shown`].
    ShowVerifier {
        text: Option<String>,
        ack_wanted: bool,
    },
    /// Remove the window of the verifier.
    HideVerifier,
    /// Send the acknowledgement to the verifier.
    AckVerifier,
    /// Check that the input-target display token is still valid, then call
    /// [`TargetSession::on_display_checked`].
    CheckDisplay,
    Inject(InputEvent),
    MovePointer(Normalized),
    /// Takes keyboard focus from target applications (FLOW-23, UI-7).
    HoldKeyboard,
    /// Removes the focus holder (FLOW-23, UI-7).
    ReleaseKeyboard,
    /// Shows or removes the warning that the source does not show its portal
    /// (FLOW-26). It takes no input.
    PortalWarning(bool),
    /// Draws the cursor of the source on the warning, or removes it (FLOW-26).
    PortalCursor(Option<Normalized>),
    /// Release the device lease of this session.
    ReleaseLease,
    /// Run this computer's route back to the source for the device (HB-1).
    /// Always follows all session end actions and is the last action.
    RunHandBack,
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
    keyboard_held: bool,
    /// The warning of FLOW-26 is shown.
    portal_warning: bool,
    verification: Option<Verification>,
    verified: bool,
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
            keyboard_held: false,
            portal_warning: false,
            verification: None,
            verified: false,
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
            | Message::HandBackRequest { session }
            | Message::Ack { session }
            | Message::Input { session, .. }
            | Message::ReleaseAll { session }
            | Message::KeyboardAway { session }
            | Message::KeyboardBack { session }
            | Message::PortalHidden { session }
            | Message::PortalShown { session }
            | Message::PortalCursor { session, .. }
            | Message::End { session, .. } => session,
            _ => return Vec::new(),
        };
        if *session != self.session {
            return Vec::new();
        }
        match message {
            Message::HandoffDone { .. } if self.phase == TargetPhase::AwaitingHandoff => {
                self.phase = TargetPhase::Verifying;
                self.verification = Some(Verification::new());
                vec![TargetAction::StartVerifier]
            }
            Message::HandBackRequest { .. } if self.can_hand_back() => {
                let mut actions = self.finish(EndReason::HandBack, false);
                actions.push(TargetAction::RunHandBack);
                actions
            }
            Message::Ack { .. } if self.phase == TargetPhase::Verifying => {
                let actions = self
                    .verification
                    .as_mut()
                    .map(Verification::acknowledge)
                    .unwrap_or_default();
                self.apply_verification(actions)
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
            Message::KeyboardAway { .. }
                if self.phase == TargetPhase::Forwarding && !self.keyboard_held =>
            {
                self.keyboard_held = true;
                vec![TargetAction::HoldKeyboard]
            }
            Message::KeyboardBack { .. }
                if self.phase == TargetPhase::Forwarding && self.keyboard_held =>
            {
                self.keyboard_held = false;
                vec![TargetAction::ReleaseKeyboard]
            }
            Message::PortalHidden { .. }
                if self.phase == TargetPhase::Forwarding && !self.portal_warning =>
            {
                self.portal_warning = true;
                vec![TargetAction::PortalWarning(true)]
            }
            Message::PortalShown { .. } if self.portal_warning => {
                self.portal_warning = false;
                vec![TargetAction::PortalWarning(false)]
            }
            Message::PortalCursor { at, .. } if self.portal_warning => {
                vec![TargetAction::PortalCursor(*at)]
            }
            Message::End { reason, .. } => self.finish(*reason, false),
            _ => Vec::new(),
        }
    }

    /// Reports an event of the destination verifier. Late events have no effect.
    pub fn on_verifier(&mut self, event: VerifierEvent) -> Vec<TargetAction> {
        if self.phase != TargetPhase::Verifying {
            return Vec::new();
        }
        let actions = self
            .verification
            .as_mut()
            .map(|verification| verification.on_verifier(event))
            .unwrap_or_default();
        self.apply_verification(actions)
    }

    /// The destination has no `verify`: continue with an unverified result (VER-2).
    pub fn on_no_verify(&mut self) -> Vec<TargetAction> {
        if self.phase != TargetPhase::Verifying {
            return Vec::new();
        }
        // Complete the shared lifecycle without claiming a verified result.
        let actions = self.on_verifier(VerifierEvent::Done(true));
        self.verified = false;
        actions
    }

    /// Reports the synchronous result of `ShowVerifier`. Only a visible window
    /// can request an acknowledgement from the source (FLOW-8).
    pub fn on_verifier_shown(&mut self, shown: bool) -> Vec<TargetAction> {
        if self.phase != TargetPhase::Verifying {
            return self.unexpected_callback();
        }
        let actions = self
            .verification
            .as_mut()
            .map(|verification| verification.on_shown(shown))
            .unwrap_or_default();
        self.apply_verification(actions)
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
        let verification_actions = self
            .verification
            .as_mut()
            .map(Verification::hide)
            .unwrap_or_default();
        let mut actions = self.apply_verification(verification_actions);
        actions.push(TargetAction::Send(Message::Started {
            session: self.session,
            verified: self.verified,
        }));
        actions
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
    /// input, `ReleaseKeyboard` if the focus holder is held (FLOW-23),
    /// `StopVerifier` if it runs, `HideVerifier` if its window is visible,
    /// `Send(End)` (only for an end on this peer), `ReleaseLease`.
    /// An end of a finished session returns no actions.
    pub fn end(&mut self, reason: EndReason) -> Vec<TargetAction> {
        self.finish(reason, true)
    }

    /// Whether the source finished its handoff steps, so a route back cannot
    /// race them (HB-1).
    pub fn can_hand_back(&self) -> bool {
        matches!(
            self.phase,
            TargetPhase::Verifying | TargetPhase::CheckingDisplay | TargetPhase::Forwarding
        )
    }

    /// Ends the session, then runs the route back to its source (HB-2).
    pub fn hand_back(&mut self) -> Vec<TargetAction> {
        if !self.can_hand_back() {
            return Vec::new();
        }
        let mut actions = self.end(EndReason::HandBack);
        actions.push(TargetAction::RunHandBack);
        actions
    }

    /// The connection to the source closed or its heartbeat timed out. The target
    /// releases all held input by itself (IN-4).
    pub fn on_peer_lost(&mut self) -> Vec<TargetAction> {
        self.finish(EndReason::PeerLost, false)
    }

    fn release_held(&mut self) -> Vec<TargetAction> {
        self.held
            .release_all()
            .into_iter()
            .map(TargetAction::Inject)
            .collect()
    }

    fn apply_verification(&mut self, actions: Vec<VerificationAction>) -> Vec<TargetAction> {
        let mut target_actions = Vec::new();
        for action in actions {
            match action {
                VerificationAction::Show { text, ack_wanted } => {
                    target_actions.push(TargetAction::ShowVerifier { text, ack_wanted });
                }
                VerificationAction::Hide => target_actions.push(TargetAction::HideVerifier),
                VerificationAction::Ack => target_actions.push(TargetAction::AckVerifier),
                VerificationAction::Stop => target_actions.push(TargetAction::StopVerifier),
                VerificationAction::AckWanted => {
                    target_actions.push(TargetAction::Send(Message::AckWanted {
                        session: self.session,
                    }));
                }
                VerificationAction::Finished(result) => target_actions.extend(match result {
                    VerificationResult::Ok => self.check_display(true),
                    VerificationResult::NotOk => self.end(EndReason::VerifyFailed),
                    VerificationResult::WindowFailed => self.end(EndReason::PromptFailed),
                }),
            }
        }
        target_actions
    }

    fn check_display(&mut self, verified: bool) -> Vec<TargetAction> {
        self.verified = verified;
        self.phase = TargetPhase::CheckingDisplay;
        vec![TargetAction::CheckDisplay]
    }

    fn unexpected_callback(&mut self) -> Vec<TargetAction> {
        self.finish(EndReason::ProtocolError, false)
    }

    fn finish(&mut self, reason: EndReason, notify_source: bool) -> Vec<TargetAction> {
        if self.is_finished() {
            return Vec::new();
        }
        let mut actions = self.release_held();
        if self.keyboard_held {
            self.keyboard_held = false;
            actions.push(TargetAction::ReleaseKeyboard);
        }
        if self.portal_warning {
            self.portal_warning = false;
            actions.push(TargetAction::PortalWarning(false));
        }
        let verification_actions = self
            .verification
            .as_mut()
            .map(Verification::stop)
            .unwrap_or_default();
        actions.extend(self.apply_verification(verification_actions));
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
    use crate::verifier::VerifierEvent;

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
        if phase == TargetPhase::Verifying {
            return target;
        }
        target.on_no_verify();
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
            TargetPhase::Verifying,
            TargetPhase::CheckingDisplay,
            TargetPhase::Forwarding,
        ] {
            for window_callback in [false, true] {
                // The expected callback of each phase.
                if (window_callback && phase == TargetPhase::Verifying)
                    || (!window_callback && phase == TargetPhase::CheckingDisplay)
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
                    if phase == TargetPhase::Verifying {
                        expected.push(TargetAction::StopVerifier);
                    }
                    expected.push(TargetAction::ReleaseLease);
                    let actions = if window_callback {
                        target.on_verifier_shown(result)
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
        let mut target = at_phase(TargetPhase::Verifying);
        target.on_peer_lost();
        for result in [false, true] {
            assert!(target.on_verifier_shown(result).is_empty());
            assert!(target.on_display_checked(result).is_empty());
            assert!(target.on_verifier(VerifierEvent::Done(result)).is_empty());
        }
        assert!(target.on_no_verify().is_empty());
        assert_eq!(target.phase(), TargetPhase::Ended(EndReason::PeerLost));
    }

    #[test]
    fn ignored_datagrams_do_not_advance_pointer_order() {
        let mut target = at_phase(TargetPhase::CheckingDisplay);
        let mut datagram = PointerDatagram {
            session: SESSION,
            seq: 100,
            sent_us: 0,
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
        let scroll = InputEvent::Scroll {
            dx: 1.0,
            dy: -2.0,
            phase: crate::input::ScrollPhase::None,
        };
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
