//! The view models of the menu and the main window (UI-9, UI-13, UI-18).
//!
//! The menu shows the `portal-match` result of each local display. Each display
//! has its own evaluation in `portal_match`, apart from the portal.

use std::collections::BTreeSet;

use telescreen_core::config::Device;
use telescreen_core::ids::{ComputerId, DeviceId, DisplayToken};
use telescreen_core::platform::DisplayInfo;
use telescreen_core::routes;
use telescreen_core::session::VideoInput;
use telescreen_view::{
    CandidateAction, ComputerRow, ControlRequestRow, DeviceRow, DisplayMatching, Glyph, MainScreen,
    MenuCandidate, MenuDevice, MenuDisplay, MenuRole, MenuScreen, MenuSession, MenuSessionId,
    WakeAction,
};

use super::portal_match::{Candidate, MatchOwner};
use super::{Controller, Pending, display_name, target_phase_text};

/// `portal-match` of each local display for the menu. The evaluation of each
/// display is in [`PortalMatches`](super::portal_match::PortalMatches), with the
/// owner [`MatchOwner::Display`].
pub(super) struct DisplayEvaluation {
    /// The number in the `portal.id` of the requests. Each evaluation
    /// (`displays_changed`, "Refresh") gets a new number.
    generation: u64,
    /// The display list of this evaluation. `displays_changed` evaluates again
    /// only when the list is different.
    snapshot: Vec<DisplayInfo>,
}

impl Controller {
    /// Evaluates `portal-match` of each local device on each display (UI-18).
    /// The app calls it at the start; then the displays and "Refresh" start it
    /// again. When a display finishes, its candidates are dialed (NET-10).
    pub fn evaluate_displays(&mut self) {
        let generation = self
            .display_evaluation
            .as_ref()
            .map_or(1, |e| e.generation + 1);
        let all = self.displays();
        let devices = self.local_devices();
        self.display_evaluation = Some(DisplayEvaluation {
            generation,
            snapshot: all.clone(),
        });
        // The results of an older evaluation do not apply.
        self.matches
            .retain(|owner| !matches!(owner, MatchOwner::Display(_)));
        let mut jobs = Vec::new();
        for display in all.iter().filter(|d| d.bounds.is_some()) {
            let candidates = devices
                .iter()
                .map(|(alias, device)| Candidate {
                    alias: alias.clone(),
                    device: device.clone(),
                    display: display.clone(),
                })
                .collect();
            jobs.extend(self.matches.start(
                MatchOwner::Display(display.token.clone()),
                &format!("menu-{generation}"),
                candidates,
                &self.local,
                &all,
            ));
        }
        for (ticket, job) in jobs {
            self.spawn(Pending::PortalMatch(ticket), job);
        }
    }

    /// `displays_changed` runs every few seconds: evaluate only for a new list.
    pub(super) fn displays_differ(&self, displays: &[DisplayInfo]) -> bool {
        self.display_evaluation
            .as_ref()
            .is_some_and(|e| e.snapshot != displays)
    }

    /// A `portal-match` job of the display finished. A display with an error
    /// dials no candidate.
    pub(super) fn display_match_done(&mut self, token: &DisplayToken) {
        let Some(evaluation) = self.matches.get(&MatchOwner::Display(token.clone())) else {
            return;
        };
        if evaluation.is_pending() || evaluation.error().is_some() {
            return;
        }
        let matched: Vec<String> = evaluation.matched().map(|c| c.alias.clone()).collect();
        // NET-10: dial the candidates of each matching device.
        let devices = self.local_devices();
        let mut peers = BTreeSet::new();
        for (_, device) in devices.iter().filter(|(a, _)| matched.contains(a)) {
            for route in routes::candidates(&self.config, device) {
                peers.insert(route.target.clone());
            }
        }
        for peer in peers {
            if !self.dialing.contains(&peer) {
                self.dial_peer(peer);
            }
        }
    }

    /// The candidates of the devices on a display: the `portal-match` result
    /// of the display, and the device of a portal on it.
    pub(super) fn display_peers(&self, token: &DisplayToken) -> BTreeSet<ComputerId> {
        let mut devices: Vec<Device> = Vec::new();
        if let Some(result) = self
            .matches
            .get(&MatchOwner::Display(token.clone()))
            .filter(|e| !e.is_pending() && e.error().is_none())
        {
            devices.extend(
                self.local_devices()
                    .into_iter()
                    .filter(|(alias, _)| result.matched().any(|c| &c.alias == alias))
                    .map(|(_, device)| device),
            );
        }
        if let Some(id) = self.portal_on(token)
            && let Some(device) = self.portal_device(id)
        {
            devices.push(device.clone());
        }
        devices
            .iter()
            .flat_map(|device| routes::candidates(&self.config, device))
            .map(|route| route.target.clone())
            .collect()
    }

    /// "Refresh" of a display (NET-10): dial each candidate of the display that
    /// has no connection and no running dial, and search (NET-15). It does not
    /// run `portal-match` again (user, 2026-11-07).
    pub(super) fn refresh_display(&mut self, token: &DisplayToken) {
        for peer in self.display_peers(token) {
            if !self.connected.contains(&peer) && !self.dialing.contains(&peer) {
                self.dial_peer(peer);
            }
        }
        self.start_search();
    }

    /// Replaces the trust state of the main window (UI-13). The app reads the
    /// local trust list.
    pub fn set_trusted(&mut self, trusted: BTreeSet<ComputerId>) {
        self.trusted = trusted;
    }

    fn peer_status(&self, peer: &ComputerId) -> &'static str {
        if self.connected.contains(peer) {
            "online"
        } else if self.dialing.contains(peer) {
            "connecting"
        } else if self.searching(peer) {
            "searching"
        } else {
            "offline"
        }
    }

    fn glyph(&self) -> Glyph {
        let source = self
            .portals
            .values()
            .filter_map(|p| p.session.as_ref())
            .any(|s| !s.is_finished());
        if source {
            Glyph::Controlling
        } else if self.targets().any(|(_, t)| !t.is_finished()) {
            Glyph::Controlled
        } else {
            Glyph::Idle
        }
    }

    fn menu_sessions(&self) -> Vec<MenuSession> {
        let mut rows = Vec::new();
        // One row for each portal with a session (FLOW-25).
        for (portal, session) in self.portals.iter().filter_map(|(id, p)| {
            p.session
                .as_ref()
                .filter(|s| !s.is_finished())
                .map(|s| (*id, s))
        }) {
            rows.push(MenuSession {
                id: MenuSessionId::Portal { portal },
                peer: session.target().clone(),
                peer_label: self.label_of(session.target()),
                device: session.device().clone(),
                device_label: self.device_label(session.device()),
                role: MenuRole::Source {
                    phase: self.phase_view(session),
                    video_input: session.video_input(),
                },
            });
        }
        for (id, target) in self.targets().filter(|(_, t)| !t.is_finished()) {
            rows.push(MenuSession {
                id: MenuSessionId::Incoming { session: id },
                peer: target.source().clone(),
                peer_label: self.label_of(target.source()),
                device: target.device().clone(),
                device_label: self.device_label(target.device()),
                role: MenuRole::Target {
                    state: target_phase_text(target.phase()).into(),
                    hand_back: self.hand_back_cause(target),
                    video_input: self.device_states.get(target.device()).copied(),
                },
            });
        }
        rows
    }

    /// The label of a local device, else its alias, else its id.
    fn device_label(&self, id: &DeviceId) -> String {
        match self.local_device(id) {
            Some((alias, device)) => device.label.unwrap_or(alias),
            None => id.0.clone(),
        }
    }

    fn menu_displays(&self) -> Vec<MenuDisplay> {
        if self.display_evaluation.is_none() {
            return Vec::new();
        }
        let devices = self.local_devices();
        let mut displays: Vec<MenuDisplay> = self
            .display_cache
            .iter()
            .filter_map(|display| {
                let result = self
                    .matches
                    .get(&MatchOwner::Display(display.token.clone()))?;
                let (matching, matched) = match result.error() {
                    Some(error) => (
                        DisplayMatching::Failed {
                            error: error.to_owned(),
                        },
                        Vec::new(),
                    ),
                    None if result.is_pending() => (DisplayMatching::Evaluating, Vec::new()),
                    None => (
                        DisplayMatching::Done,
                        result.matched().map(|c| c.alias.clone()).collect(),
                    ),
                };
                let devices = devices
                    .iter()
                    .filter(|(alias, _)| matched.contains(alias))
                    .map(|(alias, device)| MenuDevice {
                        in_session: false,
                        device: device.id.clone(),
                        alias: alias.clone(),
                        label: device.label.clone().unwrap_or_else(|| alias.clone()),
                        state: self
                            .device_states
                            .get(&device.id)
                            .copied()
                            .unwrap_or(VideoInput::Unchanged),
                        candidates: routes::candidates(&self.config, device)
                            .iter()
                            .map(|route| {
                                let session = self.route_unavailable(device, route.target);
                                let mut candidate = MenuCandidate {
                                    target: route.target.clone(),
                                    label: self.label_of(route.target),
                                    status: self.peer_status(route.target).into(),
                                    switch_and_control: session.clone(),
                                    // ACT-6: only the route.
                                    switch_display_only: None,
                                    control_only: session,
                                    primary: None,
                                    wake: WakeAction::Hidden,
                                };
                                if candidate.status == "online" {
                                    candidate.primary = [
                                        (
                                            CandidateAction::SwitchAndControl,
                                            &candidate.switch_and_control,
                                        ),
                                        (
                                            CandidateAction::SwitchDisplayOnly,
                                            &candidate.switch_display_only,
                                        ),
                                        (CandidateAction::ControlOnly, &candidate.control_only),
                                    ]
                                    .into_iter()
                                    .find_map(|(action, reason)| {
                                        reason.is_none().then_some(action)
                                    });
                                }
                                if self
                                    .config
                                    .computer(route.target)
                                    .is_some_and(|c| c.wake.is_some())
                                {
                                    candidate.wake = if candidate.status == "online" {
                                        WakeAction::Unavailable {
                                            reason: "Connected".into(),
                                        }
                                    } else {
                                        WakeAction::Available
                                    };
                                }
                                candidate
                            })
                            .collect(),
                    })
                    .collect();
                Some(MenuDisplay {
                    token: display.token.clone(),
                    name: display_name(display),
                    matching,
                    devices,
                    can_refresh: self
                        .display_peers(&display.token)
                        .iter()
                        .any(|peer| self.peer_status(peer) == "offline"),
                })
            })
            .collect();
        // UI-18: the displays are in the order of their devices in the config.
        // A display with no device is last.
        let order = |display: &MenuDisplay| {
            display
                .devices
                .first()
                .and_then(|first| devices.iter().position(|(alias, _)| *alias == first.alias))
                .unwrap_or(usize::MAX)
        };
        displays.sort_by_key(order);
        displays
    }

    /// One row for each local device and each peer whose config has a route to
    /// this computer for that device (ACT-4, UI-18).
    fn control_request_rows(&self) -> Vec<ControlRequestRow> {
        let mut rows = Vec::new();
        for (alias, device) in self.local_devices() {
            for (id, computer) in &self.config.computers {
                if id == &self.local {
                    continue;
                }
                let route = computer
                    .device_by_id(&device.id)
                    .is_some_and(|(_, d)| d.handoff.contains_key(&self.local));
                if route {
                    rows.push(ControlRequestRow {
                        peer: id.clone(),
                        peer_label: self.label_of(id),
                        device: device.id.clone(),
                        device_label: device.label.clone().unwrap_or_else(|| alias.clone()),
                        waiting: self.control_request_pending(id, &device.id),
                    });
                }
            }
        }
        rows
    }

    /// The menu (UI-9). A UI draws it when it changes (UI-18).
    pub fn menu_screen(&self) -> MenuScreen {
        let sessions = self.menu_sessions();
        let mut displays = self.menu_displays();
        // UI-9, the mockups: a session as the target hides its device, and a
        // display with no other device goes away. A session as the source
        // marks its device, and its target is not a candidate.
        let mut hidden = Vec::new();
        for session in &sessions {
            let target = matches!(session.role, MenuRole::Target { .. });
            for display in &mut displays {
                if target {
                    let before = display.devices.len();
                    display.devices.retain(|d| d.device != session.device);
                    if before > 0 && display.devices.is_empty() {
                        hidden.push(display.token.clone());
                    }
                } else {
                    for device in display
                        .devices
                        .iter_mut()
                        .filter(|d| d.device == session.device)
                    {
                        device.in_session = true;
                        device.candidates.retain(|c| c.target != session.peer);
                    }
                }
            }
        }
        displays.retain(|d| !hidden.contains(&d.token));
        MenuScreen {
            computer: self.label_of(&self.local),
            glyph: self.glyph(),
            sessions,
            displays,
            control_requests: self.control_request_rows(),
            clipboard: self
                .ports
                .clipboard
                .as_ref()
                .map(|_| self.clipboard.enabled()),
        }
    }

    /// The main window (UI-13).
    pub fn main_screen(&self) -> MainScreen {
        let computers = self
            .config
            .computers
            .keys()
            .map(|id| ComputerRow {
                id: id.clone(),
                label: self.label_of(id),
                this_computer: id == &self.local,
                trusted: self.trusted.contains(id),
                connected: self.connected.contains(id),
            })
            .collect();
        let devices = self
            .local_devices()
            .into_iter()
            .map(|(alias, device)| DeviceRow {
                alias,
                state: self
                    .device_states
                    .get(&device.id)
                    .copied()
                    .unwrap_or(VideoInput::Unchanged),
                device: device.id,
            })
            .collect();
        MainScreen {
            menu: self.menu_screen(),
            fingerprint: self.fingerprint.clone(),
            computers,
            devices,
            discovery: self.control_screen().discovery,
            notices: self.notices.clone(),
        }
    }
}
