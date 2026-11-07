//! The source side: portals, the handoff, discovery and dialing, and the
//! captured input of a portal (FLOW-1 to FLOW-4, FLOW-8, FLOW-25).

use std::time::Instant;

use telescreen_core::config::Device;
use telescreen_core::driver::{DeviceRef, PortalInfo, Request};
use telescreen_core::ids::{ComputerId, DeviceId, DisplayToken, PortalId};
use telescreen_core::platform::{Capability, CapturedEvent, DisplayInfo};
use telescreen_core::pointer::{self, Point};
use telescreen_core::routes;
use telescreen_core::session::source::{SourceAction, SourcePhase, SourceSession};
use telescreen_core::wire::{EndReason, Message};
use telescreen_transport::{DialTarget, NetCommand};
use telescreen_view::{DeviceChoice, Matching};

use super::portal_match::{Candidate, MatchOwner, MatchResult};
use super::{AckOrigin, Controller, Job, Pending, Portal, SEARCH_TIME};

impl Controller {
    /// The ids of the portals that exist now (FLOW-25).
    pub fn portal_ids(&self) -> Vec<PortalId> {
        self.portals.keys().copied().collect()
    }

    /// The portal on the display, if any. Each display has at most one portal
    /// (FLOW-2).
    pub fn portal_on(&self, display: &DisplayToken) -> Option<PortalId> {
        self.portals
            .iter()
            .find(|(_, portal)| &portal.display.token == display)
            .map(|(id, _)| *id)
    }

    /// The portal that a report without a portal choice shows: the portal with
    /// the native focus, else the newest portal.
    pub(crate) fn main_portal(&self) -> Option<PortalId> {
        self.portals
            .iter()
            .find(|(_, portal)| portal.focused)
            .or_else(|| self.portals.iter().next_back())
            .map(|(id, _)| *id)
    }

    pub(super) fn source_session(&self, portal: PortalId) -> Option<&SourceSession> {
        self.portals.get(&portal)?.session.as_ref()
    }

    pub(super) fn source_session_mut(&mut self, portal: PortalId) -> Option<&mut SourceSession> {
        self.portals.get_mut(&portal)?.session.as_mut()
    }

    /// The portals with a session to this target.
    pub(super) fn portals_to(&self, peer: &ComputerId) -> Vec<PortalId> {
        self.portals
            .iter()
            .filter(|(_, p)| p.session.as_ref().is_some_and(|s| s.target() == peer))
            .map(|(id, _)| *id)
            .collect()
    }

    /// The user opened a portal on a display. Starts `portal-match` (FLOW-2).
    /// A portal on the same display closes with its session. The portals on
    /// other displays stay (FLOW-25).
    pub fn open_portal(&mut self, display: DisplayInfo) -> Option<PortalId> {
        if let Some(old) = self.portal_on(&display.token) {
            self.close_portal(old, EndReason::SourceClosed);
        }
        let Some(bounds) = display.bounds else {
            self.notice(
                "The portal display has no coordinates. Forwarding is not possible.".into(),
            );
            return None;
        };
        let id = PortalId(self.next_portal);
        self.next_portal += 1;
        // The new portal window takes the native focus.
        for portal in self.portals.values_mut() {
            portal.focused = false;
        }
        self.portals.insert(
            id,
            Portal {
                generation: 0,
                display,
                bounds,
                focused: true,
                focus_wanted: false,
                visible: true,
                fullscreen: true,
                guarded: false,
                matching: Matching::Evaluating,
                device: None,
                pending_start: None,
                session: None,
                pointer: Point::default(),
                notice: None,
            },
        );
        self.evaluate_portal_match(id);
        Some(id)
    }

    /// The portals whose window must get the native focus, once (FLOW-22). The
    /// capture gave a portal the keyboard for a click that the OS does not use
    /// to focus a window, for example a middle click on macOS.
    pub fn take_portal_focus_requests(&mut self) -> Vec<PortalId> {
        self.portals
            .iter_mut()
            .filter_map(|(id, portal)| std::mem::take(&mut portal.focus_wanted).then_some(*id))
            .collect()
    }

    /// Takes the next portal window request from FLOW-17 and ACT-5, with its
    /// portal: the window layer shows the portal on that display.
    pub fn take_portal_window_request(&mut self) -> Option<(PortalId, DisplayToken)> {
        while let Some((id, token)) = self.portal_window_requests.pop_front() {
            if self.portals.contains_key(&id) {
                return Some((id, token));
            }
        }
        None
    }

    /// The portal moved to a different display, or the display topology changed
    /// (FLOW-4). The old device binding does not apply.
    pub fn portal_display_changed(&mut self, id: PortalId, display: DisplayInfo) {
        self.end_portal_session(id, EndReason::PortalGone);
        let Some(bounds) = display.bounds else {
            self.close_portal(id, EndReason::PortalGone);
            return;
        };
        let Some(portal) = self.portals.get_mut(&id) else {
            return;
        };
        portal.display = display;
        portal.bounds = bounds;
        portal.device = None;
        portal.pending_start = None;
        self.evaluate_portal_match(id);
    }

    fn evaluate_portal_match(&mut self, id: PortalId) {
        let displays = self.displays();
        let devices = self.local_devices();
        let generation = self.next_match;
        self.next_match += 1;
        let Some(portal) = self.portals.get_mut(&id) else {
            return;
        };
        portal.generation = generation;
        portal.matching = Matching::Evaluating;
        let candidates = devices
            .into_iter()
            .map(|(alias, device)| Candidate {
                alias,
                device,
                display: portal.display.clone(),
            })
            .collect();
        let jobs = self.matches.start(
            MatchOwner::Portal(id),
            &format!("portal-{generation}"),
            candidates,
            &self.local,
            &displays,
        );
        if jobs.is_empty() {
            portal.matching = Matching::NoMatch;
        }
        for (ticket, job) in jobs {
            self.spawn(Pending::PortalMatch(ticket), job);
        }
    }

    pub(super) fn finish_portal_match(&mut self, id: PortalId) {
        let Some(result) = self
            .matches
            .get(&MatchOwner::Portal(id))
            .and_then(|e| e.result())
        else {
            return;
        };
        let choice = |c: &Candidate| DeviceChoice {
            alias: c.alias.clone(),
            label: c.device.label.clone().unwrap_or_else(|| c.alias.clone()),
        };
        // AC-9: an error, no match, or more than one match never selects a device.
        let (matching, device) = match result {
            MatchResult::Failed(error) => (Matching::Failed(error.to_owned()), None),
            MatchResult::NoMatch => (Matching::NoMatch, None),
            MatchResult::One(c) => (
                Matching::Matched(choice(c)),
                Some((c.alias.clone(), c.device.clone())),
            ),
            MatchResult::Many(many) => (
                Matching::Ambiguous(many.into_iter().map(choice).collect()),
                None,
            ),
        };
        let Some(portal) = self.portals.get_mut(&id) else {
            return;
        };
        portal.matching = matching;
        if device.is_some() {
            portal.device = device;
        }
        self.dial_candidates(id);
        self.start_pending_handoff_in(id);
    }

    /// Starts the pending session of the portal after its portal matching
    /// finished (FLOW-17).
    pub(super) fn start_pending_handoff_in(&mut self, id: PortalId) {
        let Some(portal) = self.portals.get_mut(&id) else {
            return;
        };
        if matches!(portal.matching, Matching::Evaluating) {
            return;
        }
        let Some(pending) = portal.pending_start.take() else {
            return;
        };
        self.start_pending(id, pending);
    }

    pub(super) fn portal_device(&self, id: PortalId) -> Option<&Device> {
        self.portals
            .get(&id)?
            .device
            .as_ref()
            .map(|(_, device)| device)
    }

    fn dial_candidates(&mut self, id: PortalId) {
        let Some(device) = self.portal_device(id) else {
            return;
        };
        let peers: Vec<_> = routes::candidates(&self.config, device)
            .iter()
            .map(|r| r.target.clone())
            .collect();
        for peer in peers {
            if !self.dialing.contains(&peer) {
                self.dial_peer(peer);
            }
        }
        self.start_search();
    }

    /// Dials the peer, or adds its targets to the race of a running dial (NET-11).
    pub(super) fn dial_peer(&mut self, peer: ComputerId) {
        if self.connected.contains(&peer) {
            return;
        }
        let Some(computer) = self.config.computer(&peer) else {
            return;
        };
        let mut targets = Vec::new();
        for target in self.found.get(&peer).into_iter().flatten().cloned().chain(
            computer.endpoint.hosts.iter().map(|host| DialTarget {
                host: host.clone(),
                port: computer.endpoint.port,
            }),
        ) {
            if !targets.contains(&target) {
                targets.push(target);
            }
        }
        if !targets.is_empty() {
            self.dialing.insert(peer.clone());
            self.ports.net.send(NetCommand::Dial { peer, targets });
        }
    }

    pub(super) fn discovery_enabled(&self, peer: &ComputerId) -> bool {
        self.config
            .computer(peer)
            .is_some_and(|c| c.endpoint.discovery.is_some())
    }

    pub(super) fn searching(&self, peer: &ComputerId) -> bool {
        self.search_deadline.is_some() && self.discovery_enabled(peer)
    }

    /// True if a candidate of a display has no connection and can be found.
    pub(super) fn candidate_to_search(&self) -> bool {
        self.display_cache
            .iter()
            .flat_map(|display| self.display_peers(&display.token))
            .any(|peer| !self.connected.contains(&peer) && self.discovery_enabled(&peer))
    }

    pub(super) fn start_search(&mut self) {
        if self.search_deadline.is_none() && self.candidate_to_search() {
            match self.ports.discovery.search() {
                Ok(()) => self.search_deadline = Some(self.now + SEARCH_TIME),
                Err(error) => self.notice(format!("Cannot search: {error}")),
            }
        }
    }

    pub(super) fn stop_search(&mut self) {
        if self.search_deadline.take().is_some() {
            self.ports.discovery.stop_search();
        }
    }

    pub(super) fn stop_discovery(&mut self) {
        if self.discovery_deadline.take().is_some() {
            self.ports.discovery.stop_announce();
        }
    }

    /// True while discovery has a timer that can change a screen.
    pub fn discovery_active(&self) -> bool {
        self.search_deadline.is_some() || self.discovery_deadline.is_some()
    }

    /// Closes all portals, for example at exit.
    pub(super) fn close_all_portals(&mut self, reason: EndReason) {
        for id in self.portal_ids() {
            self.close_portal(id, reason);
        }
    }

    /// Ends the session of the portal and closes the portal.
    pub fn close_portal(&mut self, id: PortalId, reason: EndReason) {
        self.end_portal_session(id, reason);
        self.remove_portal(id);
    }

    /// Removes the portal and stops its capture.
    pub(super) fn remove_portal(&mut self, id: PortalId) {
        self.portal_window_requests
            .retain(|(request, _)| *request != id);
        if self.portals.remove(&id).is_some() {
            self.ports.capture.disarm(id);
        }
        self.matches.remove(&MatchOwner::Portal(id));
        if self.portals.is_empty() {
            self.stop_search();
        }
    }

    pub(super) fn end_portal_session(&mut self, id: PortalId, reason: EndReason) {
        let actions = match self.source_session_mut(id) {
            Some(session) => session.end(reason),
            None => return,
        };
        self.run_source_actions(id, actions);
    }

    /// The window layer reports the native focus of a portal window (FLOW-22).
    pub fn set_portal_focus(&mut self, id: PortalId, focused: bool) {
        let Some(portal) = self.portals.get_mut(&id) else {
            return;
        };
        portal.focused = focused;
        portal.focus_wanted &= !focused;
        // FLOW-26: a hidden portal can keep the native focus, for example
        // while Mission Control is open. Then the keys stay on the source.
        let shown = portal.shown();
        self.ports.capture.set_portal_focus(id, focused && shown);
    }

    /// At the start of forwarding, gives the capture and the session the native
    /// state of the portal. The window layer reports only changes, so the portal
    /// can keep the focus, or stay off screen, from an earlier session (FLOW-22,
    /// FLOW-26).
    fn sync_portal_state(&mut self, id: PortalId) {
        let Some(portal) = self.portals.get_mut(&id) else {
            return;
        };
        let (focused, visible) = (portal.focused && portal.shown(), portal.shown());
        self.ports.capture.set_portal_focus(id, focused);
        self.ports.capture.set_portal_visible(id, visible);
        let actions = portal
            .session
            .as_mut()
            .map(|session| {
                let mut actions = session.on_keyboard(focused);
                actions.extend(session.on_portal_visible(visible));
                actions
            })
            .unwrap_or_default();
        self.run_source_actions(id, actions);
    }

    /// The window layer reports whether the portal window is occluded (FLOW-26).
    /// While the portal is not on screen, the capture forwards no input, and the
    /// target shows a warning.
    pub fn set_portal_visible(&mut self, id: PortalId, visible: bool) {
        self.set_portal_state(id, |portal| portal.visible = visible);
    }

    /// The window layer reports whether the portal window is fullscreen (UI-3).
    /// A portal that is not fullscreen is not on screen (FLOW-26). The source
    /// does not request a hand back for it.
    pub fn set_portal_fullscreen(&mut self, id: PortalId, fullscreen: bool) {
        self.set_portal_state(id, |portal| portal.fullscreen = fullscreen);
    }

    /// Changes one input of the on-screen state. Tells the capture and the
    /// session only when the result changes (FLOW-26).
    fn set_portal_state(&mut self, id: PortalId, change: impl FnOnce(&mut Portal)) {
        let Some(portal) = self.portals.get_mut(&id) else {
            return;
        };
        let was_shown = portal.shown();
        change(portal);
        let shown = portal.shown();
        if shown == was_shown {
            return;
        }
        let focused = portal.focused;
        self.ports.capture.set_portal_visible(id, shown);
        self.ports.capture.set_portal_focus(id, focused && shown);
        let actions = self
            .source_session_mut(id)
            .map(|session| session.on_portal_visible(shown))
            .unwrap_or_default();
        self.run_source_actions(id, actions);
    }

    /// The acknowledgement key or a left click from `origin`. Returns true
    /// when a verification accepted it: then the window layer must suppress
    /// the event (FLOW-9).
    pub fn acknowledge(&mut self, origin: AckOrigin) -> bool {
        // FLOW-26: clicks and keys on the warning do nothing.
        if let AckOrigin::Prompt(prompt) | AckOrigin::PromptKey(prompt) = origin
            && self.is_portal_warning(prompt)
        {
            return false;
        }
        let portal = match origin {
            AckOrigin::Prompt(prompt) => {
                return self.verify(|v, env| v.acknowledge_prompt(prompt, env));
            }
            AckOrigin::PromptKey(prompt) => {
                return self.verify(|v, env| v.acknowledge_prompt(prompt, env))
                    || self.verify(|v, env| v.acknowledge_local(env));
            }
            AckOrigin::Portal(id) => Some(id),
            AckOrigin::Computer => self.main_portal(),
        };
        portal.is_some_and(|id| self.acknowledge_in(id))
            || self.verify(|v, env| v.acknowledge_local(env))
    }

    /// The acknowledgement in one portal (FLOW-8).
    fn acknowledge_in(&mut self, id: PortalId) -> bool {
        let Some(portal) = self.portals.get_mut(&id) else {
            return false;
        };
        let focused = portal.focused;
        let Some(actions) = portal.session.as_mut().and_then(|s| s.acknowledge(focused)) else {
            return false;
        };
        self.run_source_actions(id, actions);
        true
    }

    /// True while the destination verifier of a portal wants an acknowledgement.
    pub fn awaiting_ack(&self) -> bool {
        self.portals
            .values()
            .filter_map(|p| p.session.as_ref())
            .any(SourceSession::ack_wanted)
    }

    /// "Hand back" in the session row of a portal: request the hand back of
    /// that portal only (HB-3).
    pub fn request_hand_back(&mut self, id: PortalId) {
        let actions = self
            .source_session_mut(id)
            .map(SourceSession::request_hand_back)
            .unwrap_or_default();
        self.run_source_actions(id, actions);
    }

    pub(super) fn portal_phase(&self, id: PortalId) -> Option<SourcePhase> {
        Some(self.source_session(id)?.phase())
    }

    /// Starts a handoff of the device of the portal to the target (FLOW-6).
    pub fn start_handoff(&mut self, id: PortalId, target: ComputerId) {
        if !self.portals.contains_key(&id) {
            return;
        }
        if let Some(reason) = self.candidate_unavailable(id, &target) {
            self.notice(format!(
                "Cannot switch to {}: {reason}",
                self.label_of(&target)
            ));
            return;
        }
        let Some(device) = self.portal_device(id).map(|d| d.id.clone()) else {
            return;
        };
        // FLOW-6: stop each old input stream of this device first, in both roles.
        self.end_device_sessions(&device);
        let (session, actions) = SourceSession::start(target, device);
        if let Some(portal) = self.portals.get_mut(&id) {
            portal.session = Some(session);
            portal.notice = None;
        }
        self.run_source_actions(id, actions);
    }

    pub(super) fn candidate_unavailable(
        &self,
        id: PortalId,
        target: &ComputerId,
    ) -> Option<String> {
        let device = self.portal_device(id)?;
        self.route_unavailable(device, target)
    }

    /// The FLOW-3 conditions of a session to `target` for a local device.
    pub(super) fn route_unavailable(&self, device: &Device, target: &ComputerId) -> Option<String> {
        let route = routes::candidates(&self.config, device)
            .into_iter()
            .find(|r| r.target == target);
        let Some(route) = route else {
            return Some("no handoff route".into());
        };
        if route.target_device.input_target.is_none() {
            return Some(format!("{} has no input-target for this device", target));
        }
        if let Capability::Missing(reason) = &self.ports.capabilities.capture {
            return Some(format!("input capture: {reason}"));
        }
        // A session needs capture and suppress (IN-1, FLOW-3).
        if let Capability::Missing(reason) = &self.ports.capabilities.suppress {
            return Some(format!("input suppress: {reason}"));
        }
        if self.connected.contains(target) {
            None
        } else if self.dialing.contains(target) {
            Some("connecting".into())
        } else if self.searching(target) {
            Some("searching".into())
        } else {
            Some("offline".into())
        }
    }

    /// Ends the unfinished session of each portal that uses this device.
    pub(super) fn end_portal_session_of(&mut self, device: &DeviceId) {
        let active: Vec<_> = self
            .portals
            .iter()
            .filter(|(_, p)| {
                p.session
                    .as_ref()
                    .is_some_and(|s| !s.is_finished() && s.device() == device)
            })
            .map(|(id, _)| *id)
            .collect();
        for id in active {
            self.end_portal_session(id, EndReason::NewHandoff);
        }
    }

    /// Ends each unfinished session of this device: the sessions of the portals
    /// and the sessions where this computer is the target. It releases the held
    /// state.
    pub(super) fn end_device_sessions(&mut self, device: &DeviceId) {
        self.stop_hand_back_verification(device);
        self.pending_hand_backs.retain(|(_, id)| id != device);
        self.end_portal_session_of(device);
        let incoming: Vec<_> = self
            .targets()
            .filter(|(_, t)| !t.is_finished() && t.device() == device)
            .map(|(s, _)| s)
            .collect();
        for session in incoming {
            self.end_incoming(session, EndReason::NewHandoff);
        }
    }

    pub(super) fn run_source_actions(&mut self, id: PortalId, actions: Vec<SourceAction>) {
        for action in actions {
            let Some(session) = self.source_session(id) else {
                return;
            };
            let target = session.target().clone();
            match action {
                SourceAction::Send(message) => {
                    if matches!(message, Message::HandBackRequest { .. }) {
                        let device = session.device().clone();
                        self.stop_hand_back_verification(&device);
                        self.pending_hand_backs.insert((target.clone(), device));
                    }
                    self.ports.net.send(NetCommand::Send {
                        peer: target,
                        message,
                    });
                }
                SourceAction::RunHandoff => self.run_handoff(id),
                SourceAction::StartForwarding => {
                    self.set_guard(id, false);
                    match self.ports.capture.forward(id) {
                        Ok(()) => {
                            self.send_pointer(id);
                            self.sync_portal_state(id);
                        }
                        Err(e) => {
                            self.notice(format!("Input capture failed: {e}"));
                            let actions = self.end_actions(id, EndReason::PermissionLost);
                            self.run_source_actions(id, actions);
                            return;
                        }
                    }
                }
                // A finished session disarms the capture below (FLOW-16).
                SourceAction::StopForwarding => {}
            }
        }
        let finished = self
            .source_session(id)
            .is_some_and(SourceSession::is_finished);
        if finished {
            self.set_guard(id, false);
            self.ports.capture.disarm(id);
            self.close_session_portal(id);
        }
        self.update_clipboard();
    }

    /// Changes the portal guard during the handoff and verification (FLOW-21).
    pub(super) fn set_guard(&mut self, id: PortalId, on: bool) {
        let Some(portal) = self.portals.get_mut(&id) else {
            return;
        };
        if portal.guarded == on {
            return;
        }
        portal.guarded = on;
        if let Err(e) = self.ports.capture.guard(id, on) {
            self.notice(format!("Portal guard failed: {e}"));
        }
    }

    fn end_actions(&mut self, id: PortalId, reason: EndReason) -> Vec<SourceAction> {
        self.source_session_mut(id)
            .map(|s| s.end(reason))
            .unwrap_or_default()
    }

    fn run_handoff(&mut self, id: PortalId) {
        let displays = self.displays();
        let Some(portal) = self.portals.get(&id) else {
            return;
        };
        let Some(session) = portal.session.as_ref() else {
            return;
        };
        let Some((alias, device)) = portal.device.clone() else {
            return;
        };
        let target = session.target().clone();
        let session_id = session.session();
        let Some(handoff) = device.handoff.get(&target).cloned() else {
            return;
        };
        let request = Request::on_source(
            &self.local,
            DeviceRef {
                id: device.id.clone(),
                alias,
            },
            Some(target),
            displays,
        )
        .with_session(session_id)
        .with_portal(portal_info(portal));
        let bounds = portal.bounds;
        let focused = portal.focused;
        if let Err(e) = self.ports.capture.arm(id, bounds) {
            self.notice(format!("Input capture failed: {e}"));
            self.end_portal_session(id, EndReason::PermissionLost);
            return;
        }
        self.ports.capture.set_portal_focus(id, focused);
        self.set_guard(id, true);
        self.spawn(
            Pending::Handoff {
                portal: id,
                session: session_id,
            },
            Job::Handoff {
                steps: handoff.steps,
                request,
            },
        );
    }

    fn send_pointer(&mut self, id: PortalId) {
        let Some(portal) = self.portals.get_mut(&id) else {
            return;
        };
        let position = pointer::normalize(portal.bounds, portal.pointer);
        let Some(session) = portal.session.as_mut() else {
            return;
        };
        if let Some(datagram) = session.pointer(position, Instant::now()) {
            let peer = session.target().clone();
            self.ports
                .net
                .send(NetCommand::SendPointer { peer, datagram });
        }
    }

    /// An event from the input capture (IN-2: it comes through a bounded queue).
    /// An event of a portal goes to that portal; an event of a closed portal does
    /// nothing. The events with no portal apply to all portals (FLOW-25).
    pub fn on_captured(&mut self, event: CapturedEvent) {
        let all: fn(&mut SourceSession) -> Vec<SourceAction> = match event {
            CapturedEvent::InputLost => SourceSession::on_input_lost,
            CapturedEvent::SafetyExit => |s| s.end(EndReason::SafetyExit),
            CapturedEvent::Overflow => |s| s.end(EndReason::QueueOverflow),
            other => return self.on_portal_captured(other),
        };
        for id in self.portal_ids() {
            let actions = self.source_session_mut(id).map(all).unwrap_or_default();
            self.run_source_actions(id, actions);
        }
    }

    fn on_portal_captured(&mut self, event: CapturedEvent) {
        let id = match event {
            CapturedEvent::Input { portal, .. }
            | CapturedEvent::PointerMoved { portal, .. }
            | CapturedEvent::PointerEntered { portal, .. }
            | CapturedEvent::PointerLeft { portal }
            | CapturedEvent::FocusWanted { portal }
            | CapturedEvent::KeyboardEntered { portal }
            | CapturedEvent::KeyboardLeft { portal } => portal,
            CapturedEvent::InputLost | CapturedEvent::SafetyExit | CapturedEvent::Overflow => {
                return;
            }
        };
        let Some(portal) = self.portals.get_mut(&id) else {
            return;
        };
        let shown = portal.shown();
        let Some(session) = portal.session.as_mut() else {
            return;
        };
        let actions = match event {
            CapturedEvent::Input { event, .. } => session.on_input(event),
            CapturedEvent::PointerMoved { at, .. } => {
                let forwarding = session.phase() == SourcePhase::Forwarding;
                if shown {
                    portal.pointer = portal.bounds.clamp(at);
                }
                if forwarding && shown {
                    self.send_pointer(id);
                    return;
                }
                // FLOW-26: the warning on the target shows the cursor. FLOW-7:
                // before forwarding, the prompt on the target shows it.
                let cursor = portal
                    .bounds
                    .contains(at)
                    .then(|| pointer::normalize(portal.bounds, at));
                session.portal_cursor(cursor)
            }
            CapturedEvent::PointerEntered { at, .. } => {
                let forwarding = session.phase() == SourcePhase::Forwarding;
                if forwarding && !shown {
                    // FLOW-26, UI-3: a hidden portal forwards no pointer
                    // position. The warning on the target shows the cursor.
                    session.portal_cursor(Some(pointer::normalize(portal.bounds, at)))
                } else {
                    portal.pointer = portal.bounds.clamp(at);
                    session.forget_pointer();
                    let actions = if forwarding {
                        Vec::new()
                    } else {
                        // FLOW-7: the prompt on the target shows the cursor.
                        session.portal_cursor(Some(pointer::normalize(portal.bounds, at)))
                    };
                    self.run_source_actions(id, actions);
                    self.send_pointer(id);
                    return;
                }
            }
            CapturedEvent::PointerLeft { .. } if session.phase() != SourcePhase::Forwarding => {
                // FLOW-7: the cursor image on the prompt goes away.
                session.portal_cursor(None)
            }
            CapturedEvent::FocusWanted { .. } => {
                if portal.focused {
                    // The OS reported the focus, but the capture lost it, for
                    // example at a disarm (FLOW-22).
                    self.ports.capture.set_portal_focus(id, shown);
                } else {
                    // FLOW-22: the window layer asks the OS for the native focus.
                    portal.focus_wanted = true;
                }
                return;
            }
            CapturedEvent::KeyboardEntered { .. } => session.on_keyboard(true),
            CapturedEvent::KeyboardLeft { .. } => session.on_keyboard(false),
            CapturedEvent::PointerLeft { .. }
            | CapturedEvent::InputLost
            | CapturedEvent::SafetyExit
            | CapturedEvent::Overflow => return,
        };
        self.run_source_actions(id, actions);
    }

    /// A search found the addresses of a trusted computer (NET-16).
    pub fn on_discovered(&mut self, peer: ComputerId, targets: Vec<DialTarget>) {
        self.found.insert(peer.clone(), targets);
        if self.is_offline_candidate(&peer) {
            self.dial_peer(peer);
        }
    }

    fn is_offline_candidate(&self, peer: &ComputerId) -> bool {
        !self.connected.contains(peer)
            && self.portal_ids().into_iter().any(|id| {
                self.portal_device(id).is_some_and(|device| {
                    routes::candidates(&self.config, device)
                        .iter()
                        .any(|route| route.target == peer)
                })
            })
    }

    /// The discovery backend stopped because of an error (NET-12).
    pub fn on_discovery_failed(&mut self, reason: String) {
        self.notice(format!("Discovery failed: {reason}"));
        // The responder failed, so neither a search nor a record is certain now.
        self.stop_search();
        self.stop_discovery();
    }
}

fn portal_info(portal: &Portal) -> PortalInfo {
    super::portal_match::portal_info(format!("portal-{}", portal.generation), &portal.display)
}
