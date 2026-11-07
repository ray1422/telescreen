//! The view models and the queries of the window layer (spec section 4.7).

use telescreen_core::ids::{ComputerId, DeviceId, DisplayToken, PortalId, SessionId};
use telescreen_core::platform::DisplayInfo;
use telescreen_core::pointer_stats::PointerSummary;
use telescreen_core::routes;
use telescreen_core::session::VideoInput;
use telescreen_core::session::source::{SourcePhase, SourceSession};
use telescreen_core::session::target::TargetPhase;
use telescreen_core::wire::{EndReason, RefuseReason};
use telescreen_view::{
    CandidateRow, ControlScreen, DeviceRow, DiscoveryRow, IncomingRow, PortalScreen,
    PortalSessionRow, SessionPhase, SessionRow,
};

use super::Controller;

/// The state of one portal for the debug protocol (FLOW-25,
/// docs/debug-protocol.md).
#[derive(Debug, Clone, PartialEq)]
pub struct PortalState {
    pub portal: PortalId,
    /// The display token of the portal display.
    pub display: DisplayToken,
    /// The session id, after the target granted the lease.
    pub session: Option<SessionId>,
    /// The session phase, or `None` if the portal has no session.
    pub state: Option<SessionPhase>,
    pub screen: PortalScreen,
}

impl Controller {
    /// Pointer statistics of the newest incoming session, else the last ended
    /// incoming session (NET-9).
    pub fn pointer_stats(&self) -> Option<PointerSummary> {
        self.incoming
            .last_key_value()
            .map(|(_, incoming)| incoming.stats.summary())
            .or_else(|| self.last_pointer_summary.clone())
    }

    /// The display of the current display list with this token.
    pub fn display(&self, token: &telescreen_core::ids::DisplayToken) -> Option<DisplayInfo> {
        self.display_cache
            .iter()
            .find(|d| &d.token == token)
            .cloned()
    }

    /// The bounds of all displays except the one with this token.
    /// True while a control request to `peer` for `device` waits (ACT-6).
    pub fn control_request_pending(&self, peer: &ComputerId, device: &DeviceId) -> bool {
        self.control_requests
            .contains_key(&(peer.clone(), device.clone()))
    }

    pub fn has_portal(&self) -> bool {
        !self.portals.is_empty()
    }

    /// True while the portal forwards input (FLOW-13).
    pub fn forwarding(&self, id: PortalId) -> bool {
        self.portal_phase(id) == Some(SourcePhase::Forwarding)
    }

    /// True while the portal forwards and is not on screen (FLOW-26).
    pub fn portal_hidden(&self, id: PortalId) -> bool {
        self.forwarding(id) && self.portals.get(&id).is_some_and(|portal| !portal.shown())
    }

    /// True while the portal keeps focus during handoff and verification (FLOW-21).
    pub fn guarded(&self, id: PortalId) -> bool {
        self.portals.get(&id).is_some_and(|portal| portal.guarded)
    }

    /// The screen of one portal.
    pub fn portal_screen(&self, id: PortalId) -> Option<PortalScreen> {
        let portal = self.portals.get(&id)?;
        let device = self.portal_device(id);
        let routes = device
            .as_ref()
            .map(|device| routes::candidates(&self.config, device))
            .unwrap_or_default();
        let show_refresh = routes.iter().any(|r| !self.connected.contains(r.target));
        let can_refresh = routes.iter().any(|r| {
            !self.connected.contains(r.target)
                && (!self.dialing.contains(r.target) && !self.searching(r.target)
                    || self.discovery_enabled(r.target) && self.search_deadline.is_none())
        });
        let candidates = routes
            .into_iter()
            .map(|route| CandidateRow {
                target: route.target.clone(),
                label: self.label_of(route.target),
                unavailable: self.candidate_unavailable(id, route.target),
            })
            .collect();
        let session = portal.session.as_ref().map(|s| SessionRow {
            target: self.label_of(s.target()),
            phase: self.phase_view(s),
            video_input: s.video_input(),
        });
        Some(PortalScreen {
            display: display_name(&portal.display),
            matching: portal.matching.clone(),
            candidates,
            show_refresh,
            can_refresh,
            session,
            notice: portal.notice.clone(),
        })
    }

    pub(super) fn phase_view(&self, session: &SourceSession) -> SessionPhase {
        match session.phase() {
            SourcePhase::Requesting => SessionPhase::Requesting,
            SourcePhase::HandingOff => SessionPhase::HandingOff,
            SourcePhase::Verifying if session.ack_wanted() => SessionPhase::WaitingForAck {
                key: self.labels.confirm_key.clone(),
            },
            SourcePhase::Verifying => SessionPhase::Verifying,
            SourcePhase::Forwarding => SessionPhase::Forwarding,
            SourcePhase::Refused(reason) => SessionPhase::Ended {
                reason: refuse_text(reason),
            },
            SourcePhase::Ended(reason) => SessionPhase::Ended {
                reason: end_text(reason).into(),
            },
        }
    }

    /// The state of each portal, in the sequence of the portal ids (FLOW-25).
    pub fn portal_states(&self) -> Vec<PortalState> {
        self.portals
            .iter()
            .filter_map(|(id, portal)| {
                let screen = self.portal_screen(*id)?;
                Some(PortalState {
                    portal: *id,
                    display: portal.display.token.clone(),
                    session: portal.session.as_ref().and_then(SourceSession::session),
                    state: screen.session.as_ref().map(|s| s.phase.clone()),
                    screen,
                })
            })
            .collect()
    }

    pub fn control_screen(&self) -> ControlScreen {
        // One row for the session of each portal (FLOW-25).
        let sessions = self
            .portal_ids()
            .into_iter()
            .filter_map(|portal| {
                let session = self.portal_screen(portal)?.session?;
                Some(PortalSessionRow { portal, session })
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
        let incoming = self
            .targets()
            .map(|(session, t)| IncomingRow {
                session,
                source: self.label_of(t.source()),
                state: target_phase_text(t.phase()).into(),
                hand_back: self.hand_back_cause(t),
            })
            .collect();
        ControlScreen {
            computer: self.label_of(&self.local),
            fingerprint: self.fingerprint.clone(),
            displays: self.menu_screen().displays,
            sessions,
            devices,
            incoming,
            clipboard: self
                .ports
                .clipboard
                .as_ref()
                .map(|_| self.clipboard.enabled()),
            discovery: self.discovery_enabled(&self.local).then(|| DiscoveryRow {
                remaining_seconds: self.discovery_deadline.map(|deadline| {
                    let remaining = deadline.saturating_duration_since(self.now);
                    remaining.as_secs() + u64::from(remaining.subsec_nanos() != 0)
                }),
            }),
            notices: self.notices.clone(),
        }
    }
}

pub(super) fn display_name(display: &DisplayInfo) -> String {
    display
        .metadata
        .get("name")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| display.token.0.clone())
}

pub(super) fn refuse_text(reason: RefuseReason) -> String {
    match reason {
        RefuseReason::UnknownDevice => "the target does not know this device",
        RefuseReason::NoRoute => "the target config has no route from this computer",
        RefuseReason::NoInputTarget => "the target cannot resolve its input-target display",
        RefuseReason::NoInjection => "the target cannot inject input",
        RefuseReason::NoFocusHold => "the target has no keyboard focus holder",
        RefuseReason::Busy => "the device is in use by a different session",
    }
    .into()
}

pub(super) fn end_text(reason: EndReason) -> &'static str {
    match reason {
        EndReason::HandBack => "handed back",
        EndReason::SafetyExit => "safety exit",
        EndReason::SourceClosed => "ended on this computer",
        EndReason::TargetDisconnected => "disconnected on the target",
        EndReason::TargetStopped => "the target stopped the session",
        EndReason::TargetRequestedControl => "the target took control back",
        EndReason::VerifyFailed => "the verification failed",
        EndReason::HandoffFailed => "the handoff failed",
        EndReason::HandoffPartial => "the handoff stopped after some steps",
        EndReason::PromptFailed => "the target cannot show the prompt",
        EndReason::DisplayGone => "the target display is gone",
        EndReason::PortalGone => "the portal display is gone",
        EndReason::QueueOverflow => "input queue overflow",
        EndReason::PermissionLost => "a permission is missing",
        EndReason::PeerLost => "connection lost",
        EndReason::ProtocolError => "protocol error",
        EndReason::NewHandoff => "a new handoff of this display started",
    }
}

pub(super) fn target_phase_text(phase: TargetPhase) -> &'static str {
    match phase {
        TargetPhase::AwaitingHandoff => "switching the display",
        TargetPhase::Verifying => "verifying the display",
        TargetPhase::CheckingDisplay => "starting",
        TargetPhase::Forwarding => "controls this computer",
        TargetPhase::Ended(reason) => end_text(reason),
    }
}
