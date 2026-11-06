//! The view models of the menu and the main window (UI-9, UI-13, UI-18).
//!
//! The menu shows the `portal-match` result of each local display. This module
//! runs that evaluation with its own jobs, apart from the portal.

use std::collections::{BTreeSet, HashMap};

use telescreen_core::driver::{DeviceRef, OperationKind, Outcome, PortalInfo, Request};
use telescreen_core::ids::{ComputerId, DisplayToken};
use telescreen_core::platform::DisplayInfo;
use telescreen_core::routes;
use telescreen_core::session::VideoInput;
use telescreen_view::{
    ComputerRow, ControlRequestRow, DeviceRow, DisplayMatching, Glyph, MainScreen, MenuCandidate,
    MenuDevice, MenuDisplay, MenuRole, MenuScreen, MenuSession, MenuSessionId,
};

use super::{Controller, Job, JobOutput, Pending, display_name, target_phase_text};

/// `portal-match` of each local display for the menu.
pub(super) struct DisplayEvaluation {
    /// Drops results of an older evaluation (`displays_changed`, "Refresh").
    generation: u64,
    /// The display list of this evaluation. `displays_changed` evaluates again
    /// only when the list is different.
    snapshot: Vec<DisplayInfo>,
    displays: HashMap<DisplayToken, DisplayResult>,
}

#[derive(Default)]
struct DisplayResult {
    pending: usize,
    /// The aliases of the local devices that match.
    matched: Vec<String>,
    error: Option<String>,
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
        let mut evaluation = DisplayEvaluation {
            generation,
            snapshot: all.clone(),
            displays: HashMap::new(),
        };
        let mut jobs = Vec::new();
        for display in all.iter().filter(|d| d.bounds.is_some()) {
            let mut result = DisplayResult::default();
            for (alias, device) in &devices {
                let Some(operation) = device.portal_match.clone() else {
                    continue;
                };
                result.pending += 1;
                let request = Request {
                    computer_id: self.local.clone(),
                    device: DeviceRef {
                        id: device.id.clone(),
                        alias: alias.clone(),
                    },
                    source_computer_id: self.local.clone(),
                    target_computer_id: None,
                    session_id: None,
                    portal: Some(PortalInfo {
                        id: format!("menu-{generation}"),
                        bounds: display.bounds,
                        coordinate_space: "local-desktop-logical",
                        display_token: Some(display.token.clone()),
                    }),
                    displays: all.clone(),
                };
                jobs.push((
                    Pending::DisplayMatch {
                        generation,
                        token: display.token.clone(),
                        alias: alias.clone(),
                    },
                    Job::Operation {
                        operation,
                        kind: OperationKind::PortalMatch,
                        request,
                    },
                ));
            }
            evaluation.displays.insert(display.token.clone(), result);
        }
        self.display_evaluation = Some(evaluation);
        for (pending, job) in jobs {
            self.spawn(pending, job);
        }
    }

    /// `displays_changed` runs every few seconds: evaluate only for a new list.
    pub(super) fn displays_differ(&self, displays: &[DisplayInfo]) -> bool {
        self.display_evaluation
            .as_ref()
            .is_some_and(|e| e.snapshot != displays)
    }

    pub(super) fn display_match_done(
        &mut self,
        generation: u64,
        token: DisplayToken,
        alias: String,
        output: JobOutput,
    ) {
        let Some(evaluation) = self
            .display_evaluation
            .as_mut()
            .filter(|e| e.generation == generation)
        else {
            return;
        };
        let Some(result) = evaluation.displays.get_mut(&token) else {
            return;
        };
        result.pending = result.pending.saturating_sub(1);
        match output {
            JobOutput::Operation(Ok(Outcome::Matched(true))) => result.matched.push(alias),
            JobOutput::Operation(Ok(Outcome::Matched(false))) => {}
            JobOutput::Operation(Err(e)) => result.error = Some(format!("{alias}: {e}")),
            other => result.error = Some(format!("{alias}: unexpected {other:?}")),
        }
        if result.pending > 0 || result.error.is_some() {
            return;
        }
        let matched = result.matched.clone();
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
            .portal
            .as_ref()
            .and_then(|p| p.session.as_ref())
            .is_some_and(|s| !s.is_finished());
        if source {
            Glyph::Controlling
        } else if self.incoming.values().any(|t| !t.is_finished()) {
            Glyph::Controlled
        } else {
            Glyph::Idle
        }
    }

    fn menu_sessions(&self) -> Vec<MenuSession> {
        let mut rows = Vec::new();
        if let Some(session) = self
            .portal
            .as_ref()
            .and_then(|p| p.session.as_ref())
            .filter(|s| !s.is_finished())
        {
            rows.push(MenuSession {
                id: MenuSessionId::Portal,
                peer: session.target().clone(),
                peer_label: self.label_of(session.target()),
                device: session.device().clone(),
                role: MenuRole::Source {
                    phase: self.phase_view(session),
                    video_input: session.video_input(),
                },
            });
        }
        for (id, target) in self.incoming.iter().filter(|(_, t)| !t.is_finished()) {
            rows.push(MenuSession {
                id: MenuSessionId::Incoming { session: *id },
                peer: target.source().clone(),
                peer_label: self.label_of(target.source()),
                device: target.device().clone(),
                role: MenuRole::Target {
                    state: target_phase_text(target.phase()).into(),
                    hand_back: self.hand_back_cause(target),
                },
            });
        }
        rows
    }

    fn menu_displays(&self) -> Vec<MenuDisplay> {
        let Some(evaluation) = &self.display_evaluation else {
            return Vec::new();
        };
        let devices = self.local_devices();
        self.display_cache
            .iter()
            .filter_map(|display| {
                let result = evaluation.displays.get(&display.token)?;
                let (matching, matched) = match &result.error {
                    Some(error) => (
                        DisplayMatching::Failed {
                            error: error.clone(),
                        },
                        Vec::new(),
                    ),
                    None if result.pending > 0 => (DisplayMatching::Evaluating, Vec::new()),
                    None => (DisplayMatching::Done, result.matched.clone()),
                };
                let devices = devices
                    .iter()
                    .filter(|(alias, _)| matched.contains(alias))
                    .map(|(alias, device)| MenuDevice {
                        device: device.id.clone(),
                        alias: alias.clone(),
                        label: device.label.clone().unwrap_or_else(|| alias.clone()),
                        candidates: routes::candidates(&self.config, device)
                            .iter()
                            .map(|route| {
                                let session = self.route_unavailable(device, route.target);
                                MenuCandidate {
                                    target: route.target.clone(),
                                    label: self.label_of(route.target),
                                    status: self.peer_status(route.target).into(),
                                    switch_and_control: session.clone(),
                                    // ACT-6: only the route.
                                    switch_display_only: None,
                                    control_only: session,
                                }
                            })
                            .collect(),
                    })
                    .collect();
                Some(MenuDisplay {
                    token: display.token.clone(),
                    name: display_name(display),
                    matching,
                    devices,
                })
            })
            .collect()
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
        MenuScreen {
            computer: self.label_of(&self.local),
            glyph: self.glyph(),
            sessions: self.menu_sessions(),
            displays: self.menu_displays(),
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
