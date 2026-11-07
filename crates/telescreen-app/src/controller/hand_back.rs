//! A hand back without a session: the route back and its verification on this
//! computer (spec section 4.6, ADR 0010).

use std::time::Instant;

use telescreen_core::driver::{DeviceRef, OperationKind, Outcome, Request};
use telescreen_core::ids::{ComputerId, DeviceId};
use telescreen_core::session::VideoInput;
use telescreen_core::session::target::TargetSession;
use telescreen_core::wire::{HandBackOutcome, Message};
use telescreen_transport::NetCommand;
use telescreen_view::PromptId;

use super::verification::{Owner, Started};
use super::{Controller, Job, JobOutput, Pending};

#[derive(Debug, thiserror::Error)]
enum HandBackVerificationError {
    #[error("the local device is gone (input-target)")]
    DeviceGone,
    #[error("{0} has no input-target")]
    NoInputTarget(String),
    #[error("input-target failed: {0}")]
    InputTargetFailed(String),
    #[error("input-target did not return a display")]
    InvalidInputTarget,
    #[error("the input-target display is gone")]
    DisplayGone,
}

impl Controller {
    pub(super) fn hand_back_cause(&self, target: &TargetSession) -> Option<String> {
        let has_route = self
            .local_device(target.device())
            .is_some_and(|(_, device)| device.handoff.contains_key(target.source()));
        if !has_route {
            Some(format!(
                "No route back to {} in this config",
                self.label_of(target.source())
            ))
        } else if !target.can_hand_back() {
            Some("The source still switches the display".into())
        } else {
            None
        }
    }

    pub(super) fn send_hand_back_result(
        &mut self,
        peer: ComputerId,
        device: DeviceId,
        outcome: HandBackOutcome,
    ) {
        self.ports.net.send(NetCommand::Send {
            peer,
            message: Message::HandBackResult { device, outcome },
        });
    }

    pub(super) fn run_hand_back(&mut self, source: ComputerId, device_id: DeviceId) {
        self.stop_hand_back_verification(&device_id);
        let route = self
            .local_device(&device_id)
            .and_then(|(alias, mut device)| {
                device.handoff.remove(&source).map(|route| (alias, route))
            });
        let Some((alias, route)) = route else {
            self.notice(format!(
                "Switch the display by hand: no route back to {} in this config.",
                self.label_of(&source)
            ));
            self.send_hand_back_result(source, device_id, HandBackOutcome::NoRoute);
            return;
        };
        let displays = self.displays();
        let request = Request::on_source(
            &self.local,
            DeviceRef {
                id: device_id.clone(),
                alias,
            },
            Some(source.clone()),
            displays,
        );
        self.spawn(
            Pending::HandBack {
                peer: source,
                device: device_id,
            },
            Job::Handoff {
                steps: route.steps,
                request,
            },
        );
    }

    pub(super) fn hand_back_result(
        &mut self,
        peer: ComputerId,
        device: DeviceId,
        outcome: HandBackOutcome,
    ) {
        if !self
            .pending_hand_backs
            .remove(&(peer.clone(), device.clone()))
        {
            return;
        }
        self.stop_hand_back_verification(&device);
        match outcome {
            HandBackOutcome::NoRoute => self.notice(format!(
                "Switch the display by hand: {} has no route back to this computer.",
                self.label_of(&peer)
            )),
            HandBackOutcome::Failed { partial } => {
                self.notice(format!(
                    "The hand back from {} failed.",
                    self.label_of(&peer)
                ));
                if partial {
                    self.device_states.insert(device, VideoInput::Unknown);
                }
            }
            HandBackOutcome::Done => self.verify_as_destination(peer, device),
        }
    }

    /// Verifies as the destination with no session: after a hand back (HB-1) and
    /// after "Switch display only" (ACT-2).
    pub(super) fn verify_as_destination(&mut self, peer: ComputerId, device: DeviceId) {
        let prompt = self.allocate_prompt();
        let owner = Owner::HandBack(device.clone());
        self.verify(|v, env| {
            v.begin(owner, device.clone(), peer.clone(), None, prompt, None, env);
        });
        self.start_hand_back_input_target(peer, device, prompt);
    }

    fn hand_back_verification_failed(
        &mut self,
        device: &DeviceId,
        error: HandBackVerificationError,
    ) {
        self.notice(format!("Cannot verify the hand back: {error}."));
        self.stop_hand_back_verification(device);
        self.device_states
            .insert(device.clone(), VideoInput::Unknown);
    }

    fn start_hand_back_input_target(
        &mut self,
        peer: ComputerId,
        device_id: DeviceId,
        prompt: PromptId,
    ) {
        let Some((alias, device)) = self.local_device(&device_id) else {
            self.hand_back_verification_failed(&device_id, HandBackVerificationError::DeviceGone);
            return;
        };
        let Some(operation) = device.input_target else {
            self.hand_back_verification_failed(
                &device_id,
                HandBackVerificationError::NoInputTarget(alias),
            );
            return;
        };
        let displays = self.displays();
        let request = Request::on_destination(
            &self.local,
            DeviceRef {
                id: device_id.clone(),
                alias,
            },
            peer.clone(),
            displays,
        );
        self.spawn(
            Pending::HandBackInputTarget {
                prompt,
                device: device_id,
            },
            Job::Operation {
                operation,
                kind: OperationKind::InputTarget,
                request,
            },
        );
    }

    pub(super) fn hand_back_input_target_done(&mut self, device_id: DeviceId, output: JobOutput) {
        let result = match output {
            JobOutput::Operation(Ok(Outcome::Display(token))) => self
                .displays()
                .into_iter()
                .find(|display| display.token == token)
                .ok_or(HandBackVerificationError::DisplayGone),
            JobOutput::Operation(Err(error)) => {
                Err(HandBackVerificationError::InputTargetFailed(error))
            }
            _ => Err(HandBackVerificationError::InvalidInputTarget),
        };
        let display = match result {
            Ok(display) => display,
            Err(error) => {
                self.hand_back_verification_failed(&device_id, error);
                return;
            }
        };
        let owner = Owner::HandBack(device_id.clone());
        self.verifications.set_display(&owner, display.token);
        let Some((alias, device)) = self.local_device(&device_id) else {
            self.hand_back_verification_failed(&device_id, HandBackVerificationError::DeviceGone);
            return;
        };
        self.verify(
            |v, env| match v.start(&owner, alias, device.verify, Instant::now(), env) {
                Started::NoVerify => v.stop(&owner, env),
                Started::Events(events) => {
                    for event in events {
                        v.on_event(&owner, event, env);
                    }
                }
            },
        );
    }

    pub(super) fn stop_hand_back_verification(&mut self, device: &DeviceId) {
        let owner = Owner::HandBack(device.clone());
        self.verify(|v, env| v.stop(&owner, env));
    }

    /// True when a hand back verifier wants an acknowledgement on this destination.
    pub fn awaiting_local_ack(&self) -> bool {
        self.verifications.awaiting_local_ack()
    }
}
