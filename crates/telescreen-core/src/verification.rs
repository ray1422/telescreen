//! Pure rules of verification on the destination (VER-3, VER-4).
//!
//! Shared by a target session and the destination of a hand back. The caller
//! runs the verifier and presents its window, then reports the resulting events.

use crate::verifier::VerifierEvent;

/// An action for the caller to perform, in the returned order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VerificationAction {
    /// Show or update the verifier window, then report the result with `on_shown`.
    Show {
        text: Option<String>,
        ack_wanted: bool,
    },
    Hide,
    /// Send an acknowledgement to the verifier.
    Ack,
    /// Stop the verifier so it sends no more events.
    Stop,
    /// Announce that the visible window wants an acknowledgement.
    AckWanted,
    Finished(VerificationResult),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VerificationResult {
    Ok,
    NotOk,
    WindowFailed,
}

/// The state of one verification, independent of a session and of I/O.
#[derive(Debug)]
pub struct Verification {
    running: bool,
    visible: bool,
    text: Option<String>,
    want: bool,
    want_announced: bool,
}

impl Default for Verification {
    fn default() -> Self {
        Self::new()
    }
}

impl Verification {
    /// Starts with a running verifier, no window, and no acknowledgement wanted.
    pub fn new() -> Self {
        Self {
            running: true,
            visible: false,
            text: None,
            want: false,
            want_announced: false,
        }
    }

    /// Reports an event of the verifier. Finished or stopped verifiers ignore it.
    pub fn on_verifier(&mut self, event: VerifierEvent) -> Vec<VerificationAction> {
        if !self.running {
            return Vec::new();
        }
        match event {
            VerifierEvent::Show(text) => {
                self.text = Some(text);
                vec![self.show()]
            }
            VerifierEvent::WantAck => {
                self.want = true;
                self.want_announced = false;
                vec![self.show()]
            }
            VerifierEvent::Done(true) => {
                self.running = false;
                vec![VerificationAction::Finished(VerificationResult::Ok)]
            }
            VerifierEvent::Done(false) | VerifierEvent::Failed(_) => {
                self.running = false;
                let mut actions = self.hide();
                actions.push(VerificationAction::Finished(VerificationResult::NotOk));
                actions
            }
        }
    }

    /// Reports whether the requested window was shown or updated successfully.
    pub fn on_shown(&mut self, shown: bool) -> Vec<VerificationAction> {
        if !self.running {
            return Vec::new();
        }
        if !shown {
            let mut actions = self.stop();
            actions.push(VerificationAction::Finished(
                VerificationResult::WindowFailed,
            ));
            return actions;
        }
        self.visible = true;
        if self.want && !self.want_announced {
            self.want_announced = true;
            return vec![VerificationAction::AckWanted];
        }
        Vec::new()
    }

    /// True only after the want was announced and before it was acknowledged.
    pub fn ack_wanted(&self) -> bool {
        self.running && self.want && self.want_announced
    }

    /// Accepts one acknowledgement per announced want.
    pub fn acknowledge(&mut self) -> Vec<VerificationAction> {
        if !self.ack_wanted() {
            return Vec::new();
        }
        self.want = false;
        self.want_announced = false;
        vec![VerificationAction::Ack]
    }

    /// Hides a visible window, including after a successful verification.
    pub fn hide(&mut self) -> Vec<VerificationAction> {
        if self.visible {
            self.visible = false;
            vec![VerificationAction::Hide]
        } else {
            Vec::new()
        }
    }

    /// Stops a running verifier and hides its window. Repeated calls do nothing.
    pub fn stop(&mut self) -> Vec<VerificationAction> {
        let mut actions = Vec::new();
        if self.running {
            self.running = false;
            actions.push(VerificationAction::Stop);
        }
        actions.extend(self.hide());
        actions
    }

    fn show(&self) -> VerificationAction {
        VerificationAction::Show {
            text: self.text.clone(),
            ack_wanted: self.want,
        }
    }
}
