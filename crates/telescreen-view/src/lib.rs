//! View models and intents of Telescreen (UI-4, UI-5).
//!
//! A view model is plain data: what one screen shows. An intent is what the user
//! did. The app makes the view models; a renderer draws them and returns intents.
//! This crate has no UI library, so that a different renderer can replace
//! `telescreen-render-egui` (`docs/plan.md`, "View boundary").

use telescreen_core::ids::{ComputerId, DisplayToken, SessionId};
use telescreen_core::platform::DisplayInfo;
use telescreen_core::session::VideoInput;

/// The control window: this computer, its displays, and incoming sessions.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ControlScreen {
    pub computer: String,
    pub fingerprint: String,
    pub displays: Vec<DisplayRow>,
    pub incoming: Vec<IncomingRow>,
    /// Problems and limits that the user must know (IN-12), newest last.
    pub notices: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DisplayRow {
    pub token: DisplayToken,
    pub name: String,
    /// True if a portal is open on this display.
    pub has_portal: bool,
}

/// A session where this computer is the target.
#[derive(Debug, Clone, PartialEq)]
pub struct IncomingRow {
    pub session: SessionId,
    pub source: String,
    pub state: String,
}

/// The portal window on the source.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PortalScreen {
    pub display: String,
    pub matching: Matching,
    pub candidates: Vec<CandidateRow>,
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
#[derive(Debug, Clone, PartialEq)]
pub enum SessionPhase {
    Requesting,
    HandingOff,
    WaitingForPrompt,
    /// The prompt shows on the target. The user presses `key` or clicks the portal.
    WaitingForConfirmation {
        key: String,
    },
    Starting,
    Forwarding,
    /// The pointer is out of the portal. `release` is the release shortcut.
    Paused {
        release: String,
    },
    Ended {
        reason: String,
    },
}

/// The confirmation prompt on the target (FLOW-7, ADR 0002).
#[derive(Debug, Clone, PartialEq)]
pub struct PromptScreen {
    pub source: String,
    pub confirm_key: String,
    /// The `hint` of this computer, as plain text.
    pub hint: Option<String>,
}

impl PromptScreen {
    pub fn title(&self) -> String {
        format!(
            "Press {} on {} to control this computer",
            self.confirm_key, self.source
        )
    }
}

/// What the user did in a window.
#[derive(Debug, Clone, PartialEq)]
pub enum Intent {
    /// Open the portal on the display of the control window (user, 2026-10-27).
    OpenPortal,
    ClosePortal,
    SelectDevice {
        alias: String,
    },
    StartHandoff {
        target: ComputerId,
    },
    /// End the session of the portal (on the source).
    EndSession,
    /// "disconnect" on the target.
    Disconnect {
        session: SessionId,
    },
    /// Request control back on the target (FLOW-17).
    TakeControl {
        session: SessionId,
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

/// The user text of a video input state (UI-4).
pub trait VideoInputText {
    fn text(&self) -> &'static str;
}

/// Shows the confirmation prompt on a display of the target (UI-5). The macOS
/// backend and the Linux layer-shell backend implement it.
pub trait PromptPresenter {
    fn show(
        &mut self,
        session: SessionId,
        display: &DisplayInfo,
        screen: &PromptScreen,
    ) -> Result<(), String>;
    fn hide(&mut self, session: SessionId);
}
