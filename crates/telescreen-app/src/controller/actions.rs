//! The three actions of a display and the control request (ACT-1 to ACT-6).
//!
//! "Switch & control" and "Control only" open a portal that closes with its
//! session (ACT-5). "Switch display only" runs the handoff steps with no session.
//! A control request makes the receiving peer attach to the peer that asked.

use std::time::{Duration, Instant};

use telescreen_core::config::Device;
use telescreen_core::driver::{DeviceRef, OperationKind, Outcome, PortalInfo, Request};
use telescreen_core::ids::{ComputerId, DeviceId, DisplayToken};
use telescreen_core::platform::{Capability, DisplayInfo};
use telescreen_core::session::VideoInput;
use telescreen_core::session::source::{HandoffResult, SourcePhase, SourceSession};
use telescreen_core::wire::{ControlRefuseReason, EndReason, Message};
use telescreen_transport::NetCommand;
use telescreen_view::{DeviceChoice, Matching};

use super::{Controller, Job, JobOutput, Pending};

/// How long the asking peer waits for an answer to a control request (ACT-6).
const CONTROL_REQUEST_WAIT: Duration = Duration::from_secs(15);

/// A session that starts when `portal-match` of the portal finishes.
pub(super) struct PendingStart {
    pub(super) target: ComputerId,
    /// The device that the action names (ACT-6). `None` needs exactly one
    /// matching device (FLOW-17).
    pub(super) device: Option<DeviceId>,
    /// An attach: no handoff steps (ACT-3).
    pub(super) attach: bool,
}

/// `portal-match` of each display for one received control request (ACT-4).
pub(super) struct ControlMatch {
    peer: ComputerId,
    device: DeviceId,
    pending: usize,
    matched: Vec<DisplayInfo>,
    /// A `portal-match` error: the receiver cannot exclude that display (AC-9).
    failed: bool,
}

impl Controller {
    fn local_device(&self, id: &DeviceId) -> Option<(String, Device)> {
        self.config
            .computer(&self.local)?
            .device_by_id(id)
            .map(|(alias, device)| (alias.to_owned(), device.clone()))
    }

    fn display_portal_info(id: String, display: &DisplayInfo) -> PortalInfo {
        PortalInfo {
            id,
            bounds: display.bounds,
            coordinate_space: "local-desktop-logical",
            display_token: Some(display.token.clone()),
        }
    }

    /// ACT-5: the portal exists only with its session. Closes the portal of a
    /// session that ended or did not start. An end that the user did not make
    /// on this computer gets a notice, because the portal and its status line
    /// go away (FLOW-16).
    pub(super) fn close_session_portal(&mut self) {
        let Some(portal) = self.portal.as_ref() else {
            return;
        };
        if portal.session.as_ref().is_some_and(|s| !s.is_finished()) {
            // The end runs `run_source_actions`, which calls this function again.
            self.close_portal(EndReason::SourceClosed);
            return;
        }
        if let Some(session) = &portal.session
            && let SourcePhase::Ended(reason) = session.phase()
            && !matches!(reason, EndReason::SourceClosed | EndReason::HandBack)
        {
            let mut text = format!(
                "The session with {} ended: {}.",
                self.label_of(session.target()),
                super::end_text(reason)
            );
            if session.video_input() == VideoInput::Unknown {
                text.push_str(" The display state is unknown.");
            }
            self.notice(text);
        }
        self.stop_search();
        self.portal_window_request = None;
        if self.portal.take().is_some() {
            self.ports.capture.disarm();
        }
    }

    // ----- "Switch & control" and "Control only" (ACT-1, ACT-3, ACT-5) -----

    /// Opens a portal on the display that closes with its session (ACT-5), and
    /// starts the session when `portal-match` confirms the device (ACT-6).
    pub(super) fn start_on_display(
        &mut self,
        token: &DisplayToken,
        device: DeviceId,
        target: ComputerId,
        attach: bool,
    ) {
        let Some(display) = self.displays().into_iter().find(|d| &d.token == token) else {
            self.notice(format!("Cannot start: the display {token} is gone."));
            return;
        };
        self.open_portal(display);
        let Some(portal) = self.portal.as_mut() else {
            return;
        };
        portal.pending_start = Some(PendingStart {
            target,
            device: Some(device),
            attach,
        });
        self.portal_window_request = Some(token.clone());
        // A portal with no `portal-match` job is already finished.
        self.start_pending_handoff();
    }

    /// Starts the session of the portal after `portal-match` finished.
    pub(super) fn start_pending(&mut self, pending: PendingStart) {
        let Some(portal) = self.portal.as_ref() else {
            return;
        };
        match &pending.device {
            None => {
                if !matches!(portal.matching, Matching::Matched(_)) {
                    self.notice(
                        "Cannot control from here: the portal needs one matching device.".into(),
                    );
                    return;
                }
            }
            Some(device) => {
                let local = self.local_device(device);
                let matched = portal.match_error.is_none()
                    && local
                        .as_ref()
                        .is_some_and(|(alias, _)| portal.matched.contains(alias));
                let Some((alias, local)) = local.filter(|_| matched) else {
                    let display = super::display_name(&portal.display);
                    self.notice(format!(
                        "Cannot start: the display {display} does not match the device {device}."
                    ));
                    self.close_session_portal();
                    return;
                };
                let label = local.label.clone().unwrap_or_else(|| alias.clone());
                if let Some(portal) = self.portal.as_mut() {
                    portal.device = Some(alias.clone());
                    portal.matching = Matching::Matched(DeviceChoice { alias, label });
                }
            }
        }
        // `portal-match` dials the candidates only when it selects one device.
        let route = self
            .portal_device()
            .is_some_and(|d| d.handoff.contains_key(&pending.target));
        if route && !self.connected.contains(&pending.target) {
            self.dial_peer(pending.target.clone());
        }
        // Wait for the dial.
        if self.dialing.contains(&pending.target) && !self.connected.contains(&pending.target) {
            if let Some(portal) = self.portal.as_mut() {
                portal.pending_start = Some(pending);
            }
            return;
        }
        if pending.attach {
            self.start_attach(pending.target);
        } else {
            self.start_handoff(pending.target);
        }
        if self.portal.as_ref().is_some_and(|p| p.session.is_none()) {
            self.close_session_portal();
        }
    }

    /// A session with no handoff steps (ACT-3).
    fn start_attach(&mut self, target: ComputerId) {
        if let Some(reason) = self.candidate_unavailable(&target) {
            self.notice(format!(
                "Cannot control {}: {reason}",
                self.label_of(&target)
            ));
            return;
        }
        let Some(device) = self.portal_device() else {
            return;
        };
        // FLOW-6: stop each old input stream of this device first, in both roles.
        self.end_device_sessions(&device.id);
        let (session, actions) = SourceSession::attach(target, device.id);
        let Some(portal) = self.portal.as_mut() else {
            return;
        };
        portal.session = Some(session);
        portal.notice = None;
        let (bounds, focused) = (portal.bounds, portal.focused);
        // FLOW-21 applies from the start, because no steps run (ACT-5).
        if let Err(e) = self.ports.capture.arm(bounds) {
            self.notice(format!("Input capture failed: {e}"));
            self.end_portal_session(EndReason::PermissionLost);
            return;
        }
        self.ports.capture.set_portal_focus(focused);
        self.set_guard(true);
        self.run_source_actions(actions);
    }

    /// The dial of a pending start failed: its session cannot start.
    pub(super) fn dial_failed(&mut self, peer: &ComputerId) {
        let waits = self
            .portal
            .as_ref()
            .and_then(|p| p.pending_start.as_ref())
            .is_some_and(|s| &s.target == peer);
        if waits {
            if let Some(portal) = self.portal.as_mut() {
                portal.pending_start = None;
            }
            self.close_session_portal();
        }
    }

    // ----- "Switch display only" (ACT-1, ACT-2, ACT-6) -----

    /// Runs the handoff steps with no session, no portal, and no capture.
    pub(super) fn switch_display_only(
        &mut self,
        token: &DisplayToken,
        device_id: DeviceId,
        target: ComputerId,
    ) {
        let displays = self.displays();
        let Some(display) = displays.iter().find(|d| &d.token == token).cloned() else {
            self.notice(format!("Cannot switch: the display {token} is gone."));
            return;
        };
        let Some((alias, device)) = self.local_device(&device_id) else {
            self.notice(format!(
                "Cannot switch: this computer has no device {device_id}."
            ));
            return;
        };
        let Some(handoff) = device.handoff.get(&target).cloned() else {
            self.notice(format!(
                "Cannot switch to {}: no handoff route",
                self.label_of(&target)
            ));
            return;
        };
        // FLOW-6: the display does not show the old sessions after the steps.
        self.end_device_sessions(&device_id);
        let request = Request {
            computer_id: self.local.clone(),
            device: DeviceRef {
                id: device_id.clone(),
                alias,
            },
            source_computer_id: self.local.clone(),
            target_computer_id: Some(target.clone()),
            session_id: None,
            portal: Some(Self::display_portal_info(
                "switch-display-only".into(),
                &display,
            )),
            displays,
        };
        self.spawn(
            Pending::SwitchOnly {
                target,
                device: device_id,
            },
            Job::Handoff {
                steps: handoff.steps,
                request,
            },
        );
    }

    /// VER-6: the computer that ran the steps changes its own state, as after
    /// a route back. The destination verifies if it is connected (ACT-2).
    pub(super) fn switch_only_done(
        &mut self,
        target: ComputerId,
        device: DeviceId,
        output: JobOutput,
    ) {
        let label = self.label_of(&target);
        match output {
            JobOutput::Handoff {
                result: HandoffResult::Done,
                ..
            } => {
                self.device_states.remove(&device);
                if self.connected.contains(&target) {
                    self.ports.net.send(NetCommand::Send {
                        peer: target,
                        message: Message::DisplaySwitched { device },
                    });
                } else {
                    self.notice(format!(
                        "Switched to {label}, not verified: {label} is not connected"
                    ));
                }
            }
            JobOutput::Handoff {
                result: HandoffResult::Failed { completed_steps },
                error,
            } => {
                if completed_steps > 0 {
                    self.device_states.insert(device, VideoInput::Unknown);
                }
                self.notice(format!(
                    "The switch to {label} failed: {}",
                    error.as_deref().unwrap_or("the handoff steps failed")
                ));
            }
            other => self.notice(format!(
                "The switch to {label} failed: unexpected result {other:?}"
            )),
        }
    }

    /// The destination of "Switch display only" (ACT-6).
    pub(super) fn display_switched(&mut self, peer: ComputerId, device: DeviceId, now: Instant) {
        // NET-4: the sender needs a route to this computer for the device.
        let route = self
            .config
            .computer(&peer)
            .and_then(|c| c.device_by_id(&device))
            .is_some_and(|(_, d)| d.handoff.contains_key(&self.local));
        if !route || self.local_device(&device).is_none() {
            eprintln!("telescreen: ignored DisplaySwitched from {peer}: no route for {device}");
            return;
        }
        self.end_device_sessions(&device);
        self.verify_as_destination(peer, device, now);
    }

    // ----- Control request, asking peer (ACT-4, ACT-6) -----

    /// "Let <peer> control this display".
    pub(super) fn let_control(&mut self, peer: ComputerId, device: DeviceId) {
        let key = (peer.clone(), device.clone());
        self.control_requests
            .insert(key.clone(), self.now + CONTROL_REQUEST_WAIT);
        if self.connected.contains(&peer) {
            self.ports.net.send(NetCommand::Send {
                peer,
                message: Message::ControlRequest { device },
            });
        } else {
            self.unsent_control.insert(key);
            self.dial_peer(peer);
        }
    }

    /// Sends the control requests that waited for this peer to connect.
    pub(super) fn send_control_requests(&mut self, peer: &ComputerId) {
        let keys: Vec<_> = self
            .unsent_control
            .iter()
            .filter(|(p, _)| p == peer)
            .cloned()
            .collect();
        for key in keys {
            self.unsent_control.remove(&key);
            if self.control_requests.contains_key(&key) {
                let (peer, device) = key;
                self.ports.net.send(NetCommand::Send {
                    peer,
                    message: Message::ControlRequest { device },
                });
            }
        }
    }

    pub(super) fn expire_control_requests(&mut self, now: Instant) {
        let expired: Vec<_> = self
            .control_requests
            .iter()
            .filter(|(_, deadline)| now >= **deadline)
            .map(|(key, _)| key.clone())
            .collect();
        for key in expired {
            self.control_requests.remove(&key);
            self.unsent_control.remove(&key);
            self.notice(format!("{} did not answer", self.label_of(&key.0)));
        }
    }

    pub(super) fn control_refused(
        &mut self,
        peer: &ComputerId,
        device: &DeviceId,
        reason: ControlRefuseReason,
    ) {
        if self
            .control_requests
            .remove(&(peer.clone(), device.clone()))
            .is_some()
        {
            self.notice(format!(
                "{} cannot control this display: {}",
                self.label_of(peer),
                control_refuse_text(reason)
            ));
        }
    }

    // ----- Control request, receiving peer (ACT-4, ACT-6) -----

    fn refuse_control(&mut self, peer: ComputerId, device: DeviceId, reason: ControlRefuseReason) {
        self.ports.net.send(NetCommand::Send {
            peer,
            message: Message::ControlRefused { device, reason },
        });
    }

    fn source_session_active(&self) -> bool {
        self.portal
            .as_ref()
            .and_then(|p| p.session.as_ref())
            .is_some_and(|s| !s.is_finished())
    }

    /// Checks the request, then runs `portal-match` of the device on each display.
    pub(super) fn control_request(&mut self, peer: ComputerId, device: DeviceId) {
        let Some((alias, local)) = self.local_device(&device) else {
            return self.refuse_control(peer, device, ControlRefuseReason::UnknownDevice);
        };
        if !local.handoff.contains_key(&peer) {
            return self.refuse_control(peer, device, ControlRefuseReason::NoRoute);
        }
        let caps = &self.ports.capabilities;
        if matches!(caps.capture, Capability::Missing(_))
            || matches!(caps.suppress, Capability::Missing(_))
        {
            return self.refuse_control(peer, device, ControlRefuseReason::NoCapture);
        }
        // One portal at a time: a session on any display counts (ACT-6).
        if self.source_session_active() {
            return self.refuse_control(peer, device, ControlRefuseReason::Busy);
        }
        let Some(operation) = local.portal_match.clone() else {
            return self.refuse_control(peer, device, ControlRefuseReason::NoDisplay);
        };
        let all = self.displays();
        let displays: Vec<_> = all.iter().filter(|d| d.bounds.is_some()).cloned().collect();
        if displays.is_empty() {
            return self.refuse_control(peer, device, ControlRefuseReason::NoDisplay);
        }
        let request_id = self.next_control_match;
        self.next_control_match += 1;
        self.control_matches.insert(
            request_id,
            ControlMatch {
                peer,
                device: device.clone(),
                pending: displays.len(),
                matched: Vec::new(),
                failed: false,
            },
        );
        for display in displays {
            let request = Request {
                computer_id: self.local.clone(),
                device: DeviceRef {
                    id: device.clone(),
                    alias: alias.clone(),
                },
                source_computer_id: self.local.clone(),
                target_computer_id: None,
                session_id: None,
                portal: Some(Self::display_portal_info(
                    format!("control-request-{request_id}"),
                    &display,
                )),
                displays: all.clone(),
            };
            self.spawn(
                Pending::ControlMatch {
                    request: request_id,
                    display,
                },
                Job::Operation {
                    operation: operation.clone(),
                    kind: OperationKind::PortalMatch,
                    request,
                },
            );
        }
    }

    /// AC-9: an error or more than one match never selects a display.
    pub(super) fn control_match_done(
        &mut self,
        request: u64,
        display: DisplayInfo,
        output: JobOutput,
    ) {
        let Some(m) = self.control_matches.get_mut(&request) else {
            return;
        };
        m.pending = m.pending.saturating_sub(1);
        match output {
            JobOutput::Operation(Ok(Outcome::Matched(true))) => m.matched.push(display),
            JobOutput::Operation(Ok(Outcome::Matched(false))) => {}
            other => {
                eprintln!("telescreen: portal-match for a control request: {other:?}");
                m.failed = true;
            }
        }
        if m.pending > 0 {
            return;
        }
        let Some(m) = self.control_matches.remove(&request) else {
            return;
        };
        if m.failed {
            return self.refuse_control(m.peer, m.device, ControlRefuseReason::MatchFailed);
        }
        match m.matched.as_slice() {
            [] => self.refuse_control(m.peer, m.device, ControlRefuseReason::NoDisplay),
            [display] => {
                if self.source_session_active() {
                    return self.refuse_control(m.peer, m.device, ControlRefuseReason::Busy);
                }
                let token = display.token.clone();
                self.start_on_display(&token, m.device, m.peer, true);
            }
            _ => self.refuse_control(m.peer, m.device, ControlRefuseReason::AmbiguousDisplay),
        }
    }
}

fn control_refuse_text(reason: ControlRefuseReason) -> &'static str {
    match reason {
        ControlRefuseReason::UnknownDevice => "it does not know this device",
        ControlRefuseReason::NoRoute => "its config has no route to this computer",
        ControlRefuseReason::NoDisplay => "no display there matches this device",
        ControlRefuseReason::AmbiguousDisplay => "more than one display there matches this device",
        ControlRefuseReason::MatchFailed => "its portal-match failed",
        ControlRefuseReason::NoCapture => "it cannot capture input",
        ControlRefuseReason::Busy => "it already controls a computer",
    }
}
