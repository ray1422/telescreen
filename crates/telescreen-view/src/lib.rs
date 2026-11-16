//! View models and intents of Telescreen (UI-4, UI-5).
//!
//! A view model is plain data: what one screen shows. An intent is what the user
//! did. The app makes the view models; a renderer draws them and returns intents.
//! This crate has no UI library, so that a different renderer can replace
//! `telescreen-render-egui` (`docs/plan.md`, "View boundary").

use serde::{Deserialize, Serialize};
use telescreen_core::ids::{ComputerId, DeviceId, DisplayToken, PortalId, SessionId};
use telescreen_core::platform::DisplayInfo;
use telescreen_core::pointer::Normalized;
use telescreen_core::session::VideoInput;
use telescreen_core::wire::Stage;

/// Identifies a verifier window, independently of a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PromptId(pub u64);

/// The control window: this computer, its displays, devices, and sessions.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ControlScreen {
    pub computer: String,
    pub fingerprint: String,
    /// The displays with the actions of each candidate (ACT-1, ACT-5).
    pub displays: Vec<MenuDisplay>,
    /// The session of each portal on this source, in the sequence of the
    /// portal ids (FLOW-25).
    pub sessions: Vec<PortalSessionRow>,
    pub devices: Vec<DeviceRow>,
    pub incoming: Vec<IncomingRow>,
    /// The text clipboard switch, or `None` without a backend (CLIP-1, CLIP-8).
    pub clipboard: Option<bool>,
    /// `None` if this computer has no `endpoint.discovery` (NET-14).
    pub discovery: Option<DiscoveryRow>,
    /// Problems and limits that the user must know (IN-12), newest last.
    pub notices: Vec<String>,
}

/// The session of one portal on this source (FLOW-25). The actions of the row
/// name this portal: `SourceHandBack` and `EndSession` (HB-3).
#[derive(Debug, Clone, PartialEq)]
pub struct PortalSessionRow {
    pub portal: PortalId,
    pub session: SessionRow,
}

/// "Allow discovery" (NET-14).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryRow {
    /// The seconds until the end, or `None` if this computer is not discoverable.
    pub remaining_seconds: Option<u64>,
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
    /// The login stage of the target ended with a login. The portal stays
    /// open, and the source waits for the desktop session of the target
    /// (STG-9).
    WaitingForDesktop,
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
    /// The user acknowledges in this prompt window, on this computer (VER-4).
    /// Then the window takes clicks and the keyboard. The controller sets it;
    /// a shell does not derive it from the other fields.
    pub ack_here: bool,
    /// The warning of FLOW-26: the source does not show its portal. A warning
    /// that is not blank takes clicks and the keyboard (see
    /// [`PromptScreen::takes_input`]), but it takes no acknowledgement.
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
            text: Some("Input is paused\nPress Escape 10 times to end the session".into()),
            ack_wanted: false,
            ack_here: false,
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

    /// The window takes clicks and the keyboard: for the acknowledgement on
    /// this computer (VER-4), or for the safety exit on the warning (FLOW-26,
    /// IN-8). A blank prompt takes no input.
    pub fn takes_input(&self) -> bool {
        !self.blank && (self.ack_here || self.portal_hidden)
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
    DeviceIsBack { device: DeviceId },
    /// Changes the text clipboard switch for this run (CLIP-1).
    SetClipboard { on: bool },
    /// Dial each offline candidate of the display one time (NET-10), and
    /// search (NET-15). It does not run `portal-match` again.
    Refresh { display: DisplayToken },
    /// Make this computer discoverable for 10 minutes (NET-14).
    AllowDiscovery,
    /// Stop "Allow discovery" before the end (NET-14).
    StopDiscovery,
    /// End the session of the portal (on the source). `None`: the portal with
    /// the native focus, else the newest portal (FLOW-25).
    EndSession {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        portal: Option<PortalId>,
    },
    /// End the incoming session and hand its device back to the source (HB-2).
    HandBack { session: SessionId },
    /// End the incoming session on the target (FLOW-17).
    StopSession { session: SessionId },
    /// End the incoming session on the target (FLOW-17).
    /// Start a session in the other direction.
    ControlFromHere { session: SessionId },
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
    LetControl { peer: ComputerId, device: DeviceId },
    /// "Wake <peer>": send a Wake-on-LAN magic packet (ACT-7).
    Wake { target: ComputerId },
    /// "Switch back": run the `reclaim` steps of the device on this computer,
    /// then verify here (ACT-8).
    SwitchBack {
        display: DisplayToken,
        device: DeviceId,
    },
    /// "Switch & log in…": open the hint window for the login stage of the
    /// target (STG-6, STG-7).
    OpenLoginHint {
        display: DisplayToken,
        device: DeviceId,
        target: ComputerId,
    },
    /// "Cancel" or the close button of the hint window (STG-7).
    CloseLoginHint,
    /// "Switch & log in": run the handoff steps, open a portal on the display,
    /// and start a stage session with the target (STG-8).
    SwitchAndLogIn {
        display: DisplayToken,
        device: DeviceId,
        target: ComputerId,
    },
    /// "Hand back" in the session part of the menu, on the source (UI-9, HB-3).
    /// `None`: as for `EndSession`.
    SourceHandBack {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        portal: Option<PortalId>,
    },
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

/// A press in a prompt window that takes input ([`PromptScreen::takes_input`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PromptPress {
    /// A left click, Return, or keypad Enter (VER-4).
    Acknowledge,
    /// An Escape key down that is not an auto-repeat (IN-8, FLOW-26).
    Escape,
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
    /// display, or removes it (FLOW-7, FLOW-26).
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
    /// The hint window of the login stage, while it is open (STG-7).
    #[serde(default)]
    pub login_hint: Option<LoginHint>,
}

/// The command that starts the login stage on the target (STG-2). The hint
/// window shows it.
pub const LOGIN_STAGE_COMMAND: &str = "telescreen run --stage login";

/// The hint window of the login stage (STG-7).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LoginHint {
    pub display: DisplayToken,
    pub device: DeviceId,
    pub target: ComputerId,
    pub target_label: String,
    /// [`LOGIN_STAGE_COMMAND`]. The user runs it on the target.
    pub command: String,
    pub state: LoginHintState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LoginHintState {
    /// The target is not connected. The source searches and dials.
    Waiting,
    /// The target is connected in the login stage. The window shows the
    /// button "Switch & log in".
    Ready,
    /// The target is connected with no stage: the user is logged in.
    LoggedIn,
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
    /// The session of a portal on this source (FLOW-25).
    Portal { portal: PortalId },
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
    /// The name of the device on this computer, for the session card: the
    /// display of a session as the target is not in the list (UI-9).
    #[serde(default)]
    pub device_label: String,
    pub role: MenuRole,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MenuRole {
    /// Actions: `SourceHandBack` and `EndSession`.
    Source {
        phase: SessionPhase,
        video_input: VideoInput,
        /// A stage session: it sends keys only, and it has no "Hand back"
        /// (STG-8).
        #[serde(default)]
        keys_only: bool,
        /// The device has `reclaim`: the session part shows "Switch back"
        /// (ACT-8, STG-8).
        #[serde(default)]
        switch_back: bool,
    },
    /// Actions of FLOW-17: `HandBack`, `StopSession`, and `ControlFromHere`.
    Target {
        state: String,
        /// `None` if "Hand back" is available, else the cause (HB-2).
        hand_back: Option<String>,
        /// The device state on this computer (VER-6), if it has one.
        #[serde(default)]
        video_input: Option<VideoInput>,
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
    /// True if "Refresh" can start a dial or a search: a candidate of the
    /// display is offline (NET-10).
    pub can_refresh: bool,
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
    /// The state of the device on this computer (VER-6). `Confirmed`: the
    /// display shows this computer.
    pub state: VideoInput,
    /// Empty: the menu shows "No handoff routes in the config".
    pub candidates: Vec<MenuCandidate>,
    /// A session of this computer as the source uses the device (UI-9). Its
    /// target is not in `candidates`. A session as the target hides the
    /// device: the session card stands for it.
    #[serde(default)]
    pub in_session: bool,
    /// The device has `reclaim`: the menu shows the "Switch back" row (ACT-8,
    /// UI-18).
    #[serde(default)]
    pub switch_back: bool,
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
    /// The action that a click on the row runs (UI-18). `None` while the
    /// candidate is not connected, or if no action is available. The platform
    /// UIs use this value; they do not select the action themselves.
    #[serde(default)]
    pub primary: Option<CandidateAction>,
    /// "Wake <peer>" in the "⋯" menu (ACT-7).
    #[serde(default)]
    pub wake: WakeAction,
    /// The stage that the connected candidate told (STG-6). `None`: not
    /// connected, or a desktop session.
    #[serde(default)]
    pub stage: Option<Stage>,
    /// "Switch & log in" (STG-6).
    #[serde(default)]
    pub log_in: LogInAction,
}

/// The state of "Switch & log in" for a candidate (STG-6).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LogInAction {
    /// The config of the candidate has no login stage for this computer. The
    /// menu does not show the action.
    #[default]
    Hidden,
    /// The candidate is not connected: "Switch & log in…" opens the hint
    /// window (`Intent::OpenLoginHint`).
    Hint,
    /// The candidate is connected in the login stage: "Switch & log in" starts
    /// the stage session (`Intent::SwitchAndLogIn`).
    Start,
    /// The menu shows the action disabled, with the reason.
    Unavailable { reason: String },
}

/// An action of ACT-1 that a click on the row can run (UI-18). An action that
/// is not in ACT-1 is never the primary action: it is only in the "⋯" menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateAction {
    SwitchAndControl,
    SwitchDisplayOnly,
    ControlOnly,
}

/// The state of "Wake <peer>" (ACT-7).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WakeAction {
    /// The config of the peer has no `wake`. The menu does not show the action.
    #[default]
    Hidden,
    Available,
    /// The menu shows the action disabled, with the reason.
    Unavailable {
        reason: String,
    },
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
