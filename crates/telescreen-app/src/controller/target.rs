//! The target side: incoming sessions, their actions, injection of received
//! pointer positions, and the portal warning (FLOW-5, FLOW-26, NET-2).

use std::collections::BTreeMap;
use std::time::Instant;

use telescreen_core::driver::{DeviceRef, OperationKind, Outcome, Request};
use telescreen_core::ids::{ComputerId, DeviceId, SessionId};
use telescreen_core::platform::{Capability, DisplayInfo, PlatformError};
use telescreen_core::pointer_stats::PointerStats;
use telescreen_core::session::target::{TargetAction, TargetSession};
use telescreen_core::verifier::VerifierEvent;
use telescreen_core::wire::{EndReason, Message, RefuseReason};
use telescreen_transport::NetCommand;
use telescreen_view::{PromptId, PromptScreen};

use super::actions::PendingStart;
use super::verification::{Owner, Started};
use super::{Controller, Incoming, Job, JobOutput, Pending, VerifierId};
use crate::pointer_mailbox::ReceivedPointer;

enum TargetWork {
    Action(TargetAction),
    Event(VerifierEvent),
}

impl Controller {
    /// Handles waiting datagrams in order, injecting only the newest accepted
    /// position of each incoming session (NET-2).
    pub fn on_pointers(&mut self, pointers: Vec<ReceivedPointer>, now: Instant) {
        let mut newest_actions = BTreeMap::new();
        for ReceivedPointer {
            peer,
            datagram,
            received,
        } in pointers
        {
            let session = datagram.session;
            let Some(incoming) = self.incoming.get_mut(&session) else {
                continue;
            };
            if incoming.target.source() != &peer {
                continue;
            }
            incoming.stats.record(&datagram, received, now);
            if let Some(action) = incoming.target.on_pointer(&datagram) {
                newest_actions.insert(session, action);
            }
        }
        for (session, action) in newest_actions {
            if let Some(incoming) = self.incoming.get_mut(&session) {
                incoming.stats.injected();
            }
            self.run_target_actions(session, vec![action]);
        }
    }

    /// Shows the warning of FLOW-26 for an incoming session, or a blank one.
    fn show_portal_warning(&mut self, session: SessionId, source: &ComputerId, hidden: bool) {
        let Some(warning) = self.incoming.get(&session).map(|i| i.warning) else {
            return;
        };
        let prompt = match warning {
            Some(prompt) => prompt,
            None => {
                let prompt = self.allocate_prompt();
                if let Some(incoming) = self.incoming.get_mut(&session) {
                    incoming.warning = Some(prompt);
                }
                prompt
            }
        };
        let label = self.label_of(source);
        let screen = if hidden {
            PromptScreen::portal_hidden(label)
        } else {
            PromptScreen::portal_shown(label)
        };
        let shown = match self.find_display(session) {
            Some(display) => self.ports.prompt.show(prompt, &display, &screen),
            None => Err("the input-target display is gone".into()),
        };
        if let Err(e) = shown {
            self.notice(format!("Cannot show the portal warning: {e}"));
        }
    }

    fn refuse(&mut self, peer: ComputerId, device: DeviceId, reason: RefuseReason) {
        self.ports.net.send(NetCommand::Send {
            peer,
            message: Message::SessionRefused { device, reason },
        });
    }

    /// Checks the target route, input-target, injection, focus holder, and lease (FLOW-5, UI-7).
    pub(super) fn request_session(&mut self, peer: ComputerId, device_id: DeviceId) {
        let Some((alias, device)) = self.local_device(&device_id) else {
            return self.refuse(peer, device_id, RefuseReason::UnknownDevice);
        };
        // NET-4: an authenticated peer still needs a configured route to this computer.
        let route = self
            .config
            .computer(&peer)
            .and_then(|c| c.device_by_id(&device_id))
            .is_some_and(|(_, d)| d.handoff.contains_key(&self.local));
        if !route {
            return self.refuse(peer, device_id, RefuseReason::NoRoute);
        }
        let Some(operation) = device.input_target.clone() else {
            return self.refuse(peer, device_id, RefuseReason::NoInputTarget);
        };
        if self.ports.injector.is_none()
            || matches!(self.ports.capabilities.inject, Capability::Missing(_))
        {
            return self.refuse(peer, device_id, RefuseReason::NoInjection);
        }
        if self.ports.focus_holder.is_none() {
            return self.refuse(peer, device_id, RefuseReason::NoFocusHold);
        }
        // FLOW-6: a new handoff of this device ends the session of the local
        // portal. A session of a different source keeps its lease (AC-20).
        let taken = self
            .portals
            .values()
            .filter_map(|p| p.session.as_ref())
            .any(|s| !s.is_finished() && s.device() == &device_id);
        self.end_portal_session_of(&device_id);
        if taken {
            // FLOW-24: another computer shows on the display now.
            self.notice(format!(
                "Session ended: {} took this display",
                self.label_of(&peer)
            ));
        }
        let session = match self.leases.acquire(&device_id, &peer) {
            Ok(session) => session,
            Err(_) => return self.refuse(peer, device_id, RefuseReason::Busy),
        };
        self.stop_hand_back_verification(&device_id);
        self.pending_hand_backs.retain(|(_, id)| id != &device_id);
        let displays = self.displays();
        let request = Request::on_destination(
            &self.local,
            DeviceRef {
                id: device_id.clone(),
                alias,
            },
            peer.clone(),
            displays,
        )
        .with_session(Some(session));
        self.spawn(
            Pending::InputTarget {
                session,
                peer,
                device: device_id,
            },
            Job::Operation {
                operation,
                kind: OperationKind::InputTarget,
                request,
            },
        );
    }

    pub(super) fn input_target_done(
        &mut self,
        session: SessionId,
        peer: ComputerId,
        device: DeviceId,
        output: JobOutput,
    ) {
        let token = match output {
            JobOutput::Operation(Ok(Outcome::Display(token))) => token,
            JobOutput::Operation(Err(e)) => {
                self.notice(format!("input-target failed: {e}"));
                self.leases.release(session);
                return self.refuse(peer, device, RefuseReason::NoInputTarget);
            }
            _ => {
                self.leases.release(session);
                return self.refuse(peer, device, RefuseReason::NoInputTarget);
            }
        };
        let on_portal = self.portal_on(&token);
        let (target, actions) = TargetSession::new(session, peer, device, token);
        self.incoming.insert(
            session,
            Incoming {
                target,
                stats: PointerStats::new(session),
                warning: None,
            },
        );
        self.run_target_actions(session, actions);
        // FLOW-24: the display shows this computer as the target, not a second portal.
        if let Some(portal) = on_portal {
            self.close_portal(portal, EndReason::SourceClosed);
        }
    }

    fn find_display(&mut self, session: SessionId) -> Option<DisplayInfo> {
        let token = self.incoming.get(&session)?.target.display().clone();
        self.displays().into_iter().find(|d| d.token == token)
    }

    pub(super) fn run_target_actions(&mut self, session: SessionId, actions: Vec<TargetAction>) {
        self.run_target_actions_at(session, actions, Instant::now());
    }

    /// Delivers events to a running session or hand back process verifier.
    pub fn on_verifier_event(&mut self, id: VerifierId, event: VerifierEvent) {
        if let Some(owner) = self.verifications.owner_of_verifier(id) {
            self.deliver(owner, event, Instant::now());
        }
    }

    pub(super) fn verifier_actions(
        &mut self,
        session: SessionId,
        event: VerifierEvent,
    ) -> Vec<TargetAction> {
        let owner = Owner::Session(session);
        let event = self.verify(|v, env| v.on_event(&owner, event, env));
        match (event, self.target_mut(session)) {
            (Some(event), Some(target)) => target.on_verifier(event),
            _ => Vec::new(),
        }
    }

    pub(super) fn run_target_actions_at(
        &mut self,
        session: SessionId,
        actions: Vec<TargetAction>,
        now: Instant,
    ) {
        let Some(target) = self.target(session) else {
            return;
        };
        // RunHandBack follows ReleaseLease, which removes the session.
        let source = target.source().clone();
        let device_id = target.device().clone();
        let mut queue: std::collections::VecDeque<_> =
            actions.into_iter().map(TargetWork::Action).collect();
        while let Some(work) = queue.pop_front() {
            let action = match work {
                TargetWork::Action(action) => action,
                TargetWork::Event(event) => {
                    let actions = self.verifier_actions(session, event);
                    for action in actions.into_iter().rev() {
                        queue.push_front(TargetWork::Action(action));
                    }
                    continue;
                }
            };
            match action {
                TargetAction::Send(message) => {
                    self.ports.net.send(NetCommand::Send {
                        peer: source.clone(),
                        message,
                    });
                }
                TargetAction::StartVerifier => {
                    let owner = Owner::Session(session);
                    let prompt = self.allocate_prompt();
                    let label = self.label_of(&source);
                    let display = self.target(session).map(|t| t.display().clone());
                    self.verify(|v, env| {
                        v.begin(
                            owner.clone(),
                            device_id.clone(),
                            source.clone(),
                            Some(label),
                            prompt,
                            display,
                            env,
                        );
                    });
                    let Some((alias, device)) = self.local_device(&device_id) else {
                        queue.push_front(TargetWork::Event(VerifierEvent::Failed(
                            "the local device is gone".into(),
                        )));
                        continue;
                    };
                    match self.verify(|v, env| v.start(&owner, alias, device.verify, now, env)) {
                        Started::NoVerify => {
                            let more = self.target_mut(session).map(TargetSession::on_no_verify);
                            queue.extend(
                                more.unwrap_or_default().into_iter().map(TargetWork::Action),
                            );
                        }
                        Started::Events(events) => {
                            for event in events.into_iter().rev() {
                                queue.push_front(TargetWork::Event(event));
                            }
                        }
                    }
                }
                TargetAction::AckVerifier => {
                    let owner = Owner::Session(session);
                    if let Some(event) = self.verify(|v, env| v.ack(&owner, env)) {
                        queue.push_front(TargetWork::Event(event));
                    }
                }
                TargetAction::StopVerifier => {
                    let owner = Owner::Session(session);
                    self.verify(|v, env| v.stop_verifier(&owner, env));
                }
                TargetAction::ShowVerifier { text, ack_wanted } => {
                    let owner = Owner::Session(session);
                    let shown = self.verify(|v, env| v.show(&owner, text, ack_wanted, env));
                    let more = self.target_mut(session).map(|t| t.on_verifier_shown(shown));
                    for action in more.unwrap_or_default().into_iter().rev() {
                        queue.push_front(TargetWork::Action(action));
                    }
                }
                TargetAction::HideVerifier => {
                    let owner = Owner::Session(session);
                    self.verify(|v, env| v.hide(&owner, env));
                }
                TargetAction::HoldKeyboard => {
                    let display = self.find_display(session);
                    let result = match (display, self.ports.focus_holder.as_mut()) {
                        (Some(display), Some(holder)) => holder.hold(&display),
                        (None, _) => Err(PlatformError::Failed(
                            "the input-target display is gone".into(),
                        )),
                        (_, None) => Err(PlatformError::Unsupported(
                            "no focus holder is available".into(),
                        )),
                    };
                    if let Err(e) = result {
                        self.notice(format!("Cannot hold the keyboard focus: {e}"));
                        queue.clear();
                        let more = self
                            .target_mut(session)
                            .map(|t| t.end(EndReason::PermissionLost));
                        queue.extend(more.unwrap_or_default().into_iter().map(TargetWork::Action));
                    }
                }
                TargetAction::PortalWarning(true) => {
                    self.show_portal_warning(session, &source, true)
                }
                TargetAction::PortalWarning(false) => {
                    if let Some(prompt) = self.portal_warning(session) {
                        self.ports.prompt.cursor(prompt, None);
                    }
                    if self.ports.prompt.keeps_blank() {
                        self.show_portal_warning(session, &source, false);
                    } else if let Some(prompt) = self
                        .incoming
                        .get_mut(&session)
                        .and_then(|i| i.warning.take())
                    {
                        self.ports.prompt.hide(prompt);
                    }
                }
                TargetAction::PortalCursor(at) => {
                    if let Some(prompt) = self.portal_warning(session) {
                        self.ports.prompt.cursor(prompt, at);
                    }
                }
                TargetAction::ReleaseKeyboard => {
                    if let Some(holder) = self.ports.focus_holder.as_mut() {
                        holder.release();
                    }
                }
                TargetAction::CheckDisplay => {
                    let display = self.find_display(session);
                    let valid = match (&display, self.ports.injector.as_mut()) {
                        (Some(display), Some(injector)) => match injector.set_display(display) {
                            Ok(()) => true,
                            Err(e) => {
                                self.notice(format!("Cannot inject on the display: {e}"));
                                false
                            }
                        },
                        _ => false,
                    };
                    if valid {
                        self.injector_session = Some(session);
                        // FLOW-26: a blank warning now shows at once later.
                        if self.ports.prompt.keeps_blank() {
                            self.show_portal_warning(session, &source, false);
                        }
                    }
                    let more = self
                        .target_mut(session)
                        .map(|t| t.on_display_checked(valid));
                    queue.extend(more.unwrap_or_default().into_iter().map(TargetWork::Action));
                }
                TargetAction::Inject(event) => {
                    let event = self.scaled(event);
                    if let Some(injector) = self.ports.injector.as_mut()
                        && let Err(e) = injector.input(&event)
                    {
                        self.notice(format!("Injection failed: {e}"));
                    }
                }
                TargetAction::MovePointer(at) => {
                    if self.injector_session != Some(session) {
                        let display = self.find_display(session);
                        if let (Some(display), Some(injector)) =
                            (display, self.ports.injector.as_mut())
                        {
                            let _ = injector.set_display(&display);
                            self.injector_session = Some(session);
                        }
                    }
                    if let Some(injector) = self.ports.injector.as_mut() {
                        let _ = injector.pointer(at);
                    }
                }
                TargetAction::ReleaseLease => self.remove_incoming(session),
                TargetAction::RunHandBack => self.run_hand_back(source.clone(), device_id.clone()),
            }
        }
        // Each end of a target session emits ReleaseLease, which removes it.
        // Keep the cleanup for a handler that drops the queue after the end.
        let finished = self.target(session).is_some_and(TargetSession::is_finished);
        debug_assert!(!finished, "a finished target session stays");
        if finished {
            self.remove_incoming(session);
        }
        self.update_clipboard();
    }

    pub(super) fn targets(&self) -> impl Iterator<Item = (SessionId, &TargetSession)> {
        self.incoming.iter().map(|(s, i)| (*s, &i.target))
    }

    pub(super) fn target(&self, session: SessionId) -> Option<&TargetSession> {
        self.incoming.get(&session).map(|i| &i.target)
    }

    pub(super) fn target_mut(&mut self, session: SessionId) -> Option<&mut TargetSession> {
        self.incoming.get_mut(&session).map(|i| &mut i.target)
    }

    fn portal_warning(&self, session: SessionId) -> Option<PromptId> {
        self.incoming.get(&session).and_then(|i| i.warning)
    }

    /// Removes an incoming session and all of its state: the lease, the pointer
    /// statistics (NET-9), the verification, the portal warning (FLOW-26), and
    /// the injector display.
    pub(super) fn remove_incoming(&mut self, session: SessionId) {
        self.leases.release(session);
        let incoming = self.incoming.remove(&session);
        if let Some(incoming) = &incoming {
            let summary = incoming.stats.summary();
            eprintln!("telescreen: {summary}");
            self.last_pointer_summary = Some(summary);
        }
        self.verifications.remove(&Owner::Session(session));
        if let Some(prompt) = incoming.and_then(|i| i.warning) {
            self.ports.prompt.hide(prompt);
        }
        if self.injector_session == Some(session) {
            self.injector_session = None;
            if let Some(injector) = self.ports.injector.as_mut() {
                injector.end_session();
            }
        }
    }

    /// Ends the incoming session (FLOW-17).
    /// Opens a portal on its input-target display.
    /// Starts a handoff to the old source.
    pub(super) fn control_from_here(&mut self, session: SessionId) {
        let Some(target) = self.target(session) else {
            self.notice("Cannot control from here: the incoming session has ended.".into());
            return;
        };
        let source = target.source().clone();
        let token = target.display().clone();
        self.end_incoming(session, EndReason::TargetRequestedControl);
        let Some(display) = self.displays().into_iter().find(|d| d.token == token) else {
            self.notice("Cannot control from here: the input-target display is gone.".into());
            return;
        };
        let Some(id) = self.open_portal(display) else {
            return;
        };
        let Some(portal) = self.portals.get_mut(&id) else {
            return;
        };
        portal.pending_start = Some(PendingStart {
            target: source,
            device: None,
            attach: false,
        });
        self.portal_window_requests.push_back((id, token));
        self.start_pending_handoff_in(id);
    }

    pub(super) fn end_incoming(&mut self, session: SessionId, reason: EndReason) {
        if let Some(target) = self.target_mut(session) {
            let actions = target.end(reason);
            self.run_target_actions(session, actions);
        }
    }
}
