//! View models and intents of Telescreen (UI-4, UI-5).
//!
//! A view model is plain data: what one screen shows. An intent is what the user
//! did. The app makes the view models; a renderer draws them and returns intents.
//! This crate has no UI library, so that a different renderer can replace
//! `telescreen-render-egui` (`docs/plan.md`, "View boundary").

use serde::{Deserialize, Serialize};
use telescreen_core::ids::{ComputerId, DeviceId, DisplayToken, SessionId};
use telescreen_core::platform::DisplayInfo;
use telescreen_core::pointer::Normalized;
use telescreen_core::session::VideoInput;

/// Identifies a verifier window, independently of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PromptId(pub u64);

/// The control window: this computer, its displays, devices, and incoming sessions.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ControlScreen {
    pub computer: String,
    pub fingerprint: String,
    pub displays: Vec<DisplayRow>,
    pub devices: Vec<DeviceRow>,
    pub incoming: Vec<IncomingRow>,
    /// The text clipboard switch, or `None` without a backend (CLIP-1, CLIP-8).
    pub clipboard: Option<bool>,
    /// `None` if this computer has no `endpoint.discovery` (NET-14).
    pub discovery: Option<DiscoveryRow>,
    /// Problems and limits that the user must know (IN-12), newest last.
    pub notices: Vec<String>,
}

/// "Allow discovery" (NET-14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryRow {
    /// The seconds until the end, or `None` if this computer is not discoverable.
    pub remaining_seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DisplayRow {
    pub token: DisplayToken,
    pub name: String,
    /// True if a portal is open on this display.
    pub has_portal: bool,
}

/// A local device and its state on this destination (VER-6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceRow {
    pub alias: String,
    pub device: DeviceId,
    /// `Unchanged` means no handoff to this computer yet.
    pub state: VideoInput,
}

/// A session where this computer is the target.
#[derive(Debug, Clone, PartialEq)]
pub struct IncomingRow {
    pub session: SessionId,
    pub source: String,
    pub state: String,
    /// `None` if Hand back is available, else the cause.
    pub hand_back: Option<String>,
}

/// The portal window on the source.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PortalScreen {
    pub display: String,
    pub matching: Matching,
    pub candidates: Vec<CandidateRow>,
    /// True if a candidate has no connection (NET-10).
    pub show_refresh: bool,
    /// True if "Refresh" can start a dial or a search (NET-10, NET-15).
    pub can_refresh: bool,
    pub session: Option<SessionRow>,
    pub notice: Option<String>,
}

/// The result of `portal-match` for the portal display (FLOW-2, AC-9).
#[derive(Debug, Clone, PartialEq, Default)]
pub enum Matching {
    #[default]
    Evaluating,
    /// No local device matches this display.
    NoMatch,
    /// More than one device matches. The user selects one.
    Ambiguous(Vec<DeviceChoice>),
    Matched(DeviceChoice),
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct DeviceChoice {
    pub alias: String,
    pub label: String,
}

/// A candidate target (FLOW-3).
#[derive(Debug, Clone, PartialEq)]
pub struct CandidateRow {
    pub target: ComputerId,
    pub label: String,
    /// `None` if the user can select it, else the reason why not.
    pub unavailable: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SessionRow {
    pub target: String,
    pub phase: SessionPhase,
    pub video_input: VideoInput,
}

/// The session state that the portal shows (UI-4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum SessionPhase {
    Requesting,
    HandingOff,
    /// The destination verifies; no acknowledgement is wanted now.
    Verifying,
    /// The destination verifier waits for `key` or a click in the portal.
    WaitingForAck {
        key: String,
    },
    Forwarding,
    Ended {
        reason: String,
    },
}

/// The verifier window on the destination (UI-5, VER-5).
#[derive(Debug, Clone, PartialEq)]
pub struct PromptScreen {
    /// `None` when the acknowledgement is on this destination (VER-4).
    pub source: Option<String>,
    pub confirm_key: String,
    /// Verifier text; for the built-in prompt, the `hint` of this computer.
    pub text: Option<String>,
    pub ack_wanted: bool,
    /// The warning of FLOW-26: the source does not show its portal.
    pub portal_hidden: bool,
    /// The prompt shows nothing. A presenter that keeps blank prompts
    /// ([`PromptPresenter::keeps_blank`]) keeps the window, so that the next
    /// content shows at once, with no open animation (FLOW-26).
    pub blank: bool,
}

impl PromptScreen {
    /// The warning of FLOW-26 on the target, for a session from `source`.
    pub fn portal_hidden(source: String) -> Self {
        Self {
            source: Some(source),
            confirm_key: String::new(),
            text: Some("Input is paused".into()),
            ack_wanted: false,
            portal_hidden: true,
            blank: false,
        }
    }

    /// The warning of FLOW-26 while the portal is on screen: nothing shows.
    pub fn portal_shown(source: String) -> Self {
        Self {
            blank: true,
            ..Self::portal_hidden(source)
        }
    }

    pub fn title(&self) -> Option<String> {
        if self.blank {
            return None;
        }
        if self.portal_hidden {
            let source = self.source.as_deref().unwrap_or("The source");
            return Some(format!("{source} does not show its portal"));
        }
        self.ack_wanted.then(|| match &self.source {
            Some(source) => format!(
                "Press {} on {} to control this computer",
                self.confirm_key, source
            ),
            None => format!(
                "Press {} or click here to confirm this display",
                self.confirm_key
            ),
        })
    }
}

/// The window at the start, before the local computer ID is known (UI-8).
/// With a `problem`, the app cannot continue, and `computers` is empty.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct SetupScreen {
    pub config_path: String,
    pub problem: Option<String>,
    /// Why the user must select again (PEER-3).
    pub notice: Option<String>,
    pub computers: Vec<ComputerChoice>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ComputerChoice {
    pub id: ComputerId,
    /// The `label` of the computer in the config, if it has one.
    pub label: Option<String>,
}

/// What the user did in the window at the start (UI-8).
#[derive(Debug, Clone, PartialEq)]
pub enum SetupIntent {
    /// This computer is `computer`. Save the ID and start.
    Choose { computer: ComputerId },
    /// Read the config file again.
    CheckAgain,
}

/// What the user did in a window. The KDE applet sends it as JSON (UI-18).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Intent {
    /// The user says an unknown device is back on this computer (VER-6).
    DeviceIsBack {
        device: DeviceId,
    },
    /// Changes the text clipboard switch for this run (CLIP-1).
    SetClipboard {
        on: bool,
    },
    /// Open the portal on this display (FLOW-2).
    OpenPortal {
        display: DisplayToken,
    },
    ClosePortal,
    /// Dial each offline candidate of the portal again (NET-10), and search (NET-15).
    Refresh,
    /// Make this computer discoverable for 10 minutes (NET-14).
    AllowDiscovery,
    /// Stop "Allow discovery" before the end (NET-14).
    StopDiscovery,
    SelectDevice {
        alias: String,
    },
    StartHandoff {
        target: ComputerId,
    },
    /// End the session of the portal (on the source).
    EndSession,
    /// End the incoming session and hand its device back to the source (HB-2).
    HandBack {
        session: SessionId,
    },
    /// End the incoming session on the target (FLOW-17).
    StopSession {
        session: SessionId,
    },
    /// End the incoming session on the target (FLOW-17).
    /// Start a session in the other direction.
    ControlFromHere {
        session: SessionId,
    },
    /// "Switch & control": open a portal on the display, then hand off (ACT-1, ACT-5).
    SwitchAndControl {
        display: DisplayToken,
        device: DeviceId,
        target: ComputerId,
    },
    /// "Switch display only": the handoff steps with no session (ACT-1, ACT-2).
    SwitchDisplayOnly {
        display: DisplayToken,
        device: DeviceId,
        target: ComputerId,
    },
    /// "Control only": open a portal on the display, then attach (ACT-3, ACT-5).
    ControlOnly {
        display: DisplayToken,
        device: DeviceId,
        target: ComputerId,
    },
    /// "Let <peer> control this display": send a control request (ACT-4).
    LetControl {
        peer: ComputerId,
        device: DeviceId,
    },
    /// "Hand back" in the session part of the menu, on the source (UI-9, HB-3).
    SourceHandBack,
}

impl VideoInputText for VideoInput {
    fn text(&self) -> &'static str {
        match self {
            VideoInput::Unchanged => "not switched",
            VideoInput::Unverified => "switched, not verified",
            VideoInput::Confirmed => "confirmed",
            VideoInput::Unknown => "unknown",
        }
    }
}

/// The user text of a device state (UI-4, VER-6).
pub trait VideoInputText {
    fn text(&self) -> &'static str;
}

/// Shows or updates the verifier window on a display of the destination (UI-5).
/// The macOS backend and the Linux layer-shell backend implement it.
pub trait PromptPresenter {
    fn show(
        &mut self,
        prompt: PromptId,
        display: &DisplayInfo,
        screen: &PromptScreen,
    ) -> Result<(), String>;
    fn hide(&mut self, prompt: PromptId);
    /// True if the presenter can show a blank prompt with no cost to the user:
    /// nothing shows, and no input goes to it (FLOW-26).
    fn keeps_blank(&self) -> bool {
        false
    }
    /// Draws the cursor of the source on a shown prompt at a position in the
    /// display, or removes it (FLOW-26). It takes no input.
    fn cursor(&mut self, _prompt: PromptId, _at: Option<Normalized>) {}
}

// ----- The menu and the main window (UI-9, UI-13, UI-18) -----

/// The menu (UI-9). One value for all platforms; the KDE applet reads it as JSON.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MenuScreen {
    /// The label of this computer.
    pub computer: String,
    pub glyph: Glyph,
    /// The sessions of this peer, as the source and as the target.
    pub sessions: Vec<MenuSession>,
    pub displays: Vec<MenuDisplay>,
    /// "Let <peer> control this display" (ACT-4).
    pub control_requests: Vec<ControlRequestRow>,
    /// The text clipboard switch, or `None` without a backend (CLIP-1).
    pub clipboard: Option<bool>,
}

/// The state of the status glyph (UI-17). "Menu open" is a state of the UI only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Glyph {
    #[default]
    Idle,
    /// This peer is the source of a session. Also when it is a target too.
    Controlling,
    /// This peer is the target of a session.
    Controlled,
}

/// The stable id of a session row (UI-18).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MenuSessionId {
    /// The session of the portal on this source.
    Portal,
    /// A session where this peer is the target.
    Incoming { session: SessionId },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MenuSession {
    pub id: MenuSessionId,
    /// The other peer of the session.
    pub peer: ComputerId,
    pub peer_label: String,
    pub device: DeviceId,
    pub role: MenuRole,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MenuRole {
    /// Actions: `SourceHandBack` and `EndSession`.
    Source {
        phase: SessionPhase,
        video_input: VideoInput,
    },
    /// Actions of FLOW-17: `HandBack`, `StopSession`, and `ControlFromHere`.
    Target {
        state: String,
        /// `None` if "Hand back" is available, else the cause (HB-2).
        hand_back: Option<String>,
    },
}

/// A local display and its `portal-match` result (UI-9, UI-18).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MenuDisplay {
    pub token: DisplayToken,
    pub name: String,
    pub matching: DisplayMatching,
    /// The local devices that match this display, each with its candidates.
    pub devices: Vec<MenuDevice>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DisplayMatching {
    /// `portal-match` did not run yet, or it runs.
    Evaluating,
    /// `portal-match` finished. `devices` can be empty.
    Done,
    /// A `portal-match` error. No device shows (AC-9).
    Failed { error: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MenuDevice {
    pub device: DeviceId,
    pub alias: String,
    pub label: String,
    /// Empty: the menu shows "No handoff routes in the config".
    pub candidates: Vec<MenuCandidate>,
}

/// A candidate with the actions of ACT-1. `None` is available; else the reason.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MenuCandidate {
    pub target: ComputerId,
    pub label: String,
    /// "online", "connecting", "searching", or "offline".
    pub status: String,
    pub switch_and_control: Option<String>,
    pub switch_display_only: Option<String>,
    pub control_only: Option<String>,
}

/// "Let <peer> control this display" for one device (ACT-4, UI-18).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlRequestRow {
    pub peer: ComputerId,
    pub peer_label: String,
    pub device: DeviceId,
    pub device_label: String,
    /// True while the request waits: the menu shows "Waiting for <peer>".
    pub waiting: bool,
}

/// The main window (UI-13): the menu, the computers, the devices, the notices.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct MainScreen {
    pub menu: MenuScreen,
    pub fingerprint: String,
    pub computers: Vec<ComputerRow>,
    /// The device states with "It is back" for an unknown device (VER-6).
    pub devices: Vec<DeviceRow>,
    /// `None` if this computer has no `endpoint.discovery` (NET-14).
    pub discovery: Option<DiscoveryRow>,
    pub notices: Vec<String>,
}

/// A computer of the config (UI-13).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ComputerRow {
    pub id: ComputerId,
    pub label: String,
    pub this_computer: bool,
    /// The local trust list has a fingerprint for it. Else the main window shows
    /// "Not trusted" and the `telescreen trust` command.
    pub trusted: bool,
    pub connected: bool,
}

/// Gives a view model to a UI only when it changed (UI-18, ADR 0012).
#[derive(Debug, Default)]
pub struct Publisher<T> {
    last: Option<T>,
}

impl<T: PartialEq + Clone> Publisher<T> {
    pub fn new() -> Self {
        Self { last: None }
    }

    /// Returns the value if it is the first one or different from the last one.
    pub fn update(&mut self, value: T) -> Option<&T> {
        if self.last.as_ref() == Some(&value) {
            return None;
        }
        self.last = Some(value);
        self.last.as_ref()
    }
}
