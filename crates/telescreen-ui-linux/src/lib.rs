//! The native Linux UI of Telescreen (ADR 0012, M20 step g): the main window in
//! QML through cxx-qt. The Plasma applet is a separate QML package in `applet/`;
//! it reads the menu view model over D-Bus (UI-11, UI-18).
//!
//! The UI only draws view models and returns intents. [`run`] runs the Qt event
//! loop on the main thread of the process. The window layer of `telescreen-app`
//! runs on a different thread: it makes a [`LinuxUi`], gives the newest
//! [`MainScreen`](telescreen_view::MainScreen) with [`LinuxUi::push_main`], and
//! gets each user action as a [`UiEvent`] in the sink of [`LinuxUi::new`]. The
//! QML gets the view model as JSON. The calls of `LinuxUi` and [`quit`] go to
//! the Qt thread as queued calls.
//!
//! The D-Bus service of the applet (UI-11) is in [`MenuServer`]. Its bus name
//! also makes the single instance (UI-15): [`claim`] serves the menu and
//! requests the name, or tells the running instance to show its main window.
//!
//! `examples/preview.rs` shows the window with a fixture.

use telescreen_view::{Intent, MenuScreen};

#[cfg(target_os = "linux")]
mod bridge;
#[cfg(target_os = "linux")]
mod menu_service;

#[cfg(target_os = "linux")]
pub use bridge::{LinuxUi, quit, run};
#[cfg(target_os = "linux")]
pub use menu_service::{Claim, MenuServer, MenuService, claim};

/// The session bus name of Telescreen (UI-11, UI-15).
pub const BUS_NAME: &str = "dev.telescreen.Telescreen";
/// The object of the menu (UI-11).
pub const MENU_PATH: &str = "/dev/telescreen/Menu";
/// The interface of the menu (UI-11).
pub const MENU_INTERFACE: &str = "dev.telescreen.Menu1";

/// What the user did in the Linux UI.
#[derive(Debug, Clone, PartialEq)]
pub enum UiEvent {
    Intent(Intent),
    /// "Open Telescreen" in the applet, or a second start of Telescreen
    /// (UI-13, UI-15).
    ShowMainWindow,
    /// "Configure Telescreen…" (UI-14).
    OpenSettings,
    /// "Quit Telescreen" (UI-15). The Qt loop also sends it when it stops for a
    /// different cause, for example the end of the desktop session.
    Quit,
}

/// Reads one `Intent` from its JSON (UI-18). The QML `Backend` and the D-Bus
/// method `Send` use it.
pub fn parse_intent(json: &str) -> Result<Intent, String> {
    serde_json::from_str(json).map_err(|e| format!("not an intent: {e}: {json}"))
}

/// The value of the D-Bus property `Menu` (UI-11, UI-18).
pub fn menu_json(menu: &MenuScreen) -> Result<String, String> {
    serde_json::to_string(menu).map_err(|e| e.to_string())
}

/// A request of Rust to the Qt thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
enum QtRequest {
    ShowMain,
    HideMain,
    Quit,
}

/// The requests that wait until QML makes the `Backend`: before that, Rust
/// cannot queue a call to the Qt thread. Only the last window request counts.
/// The window requests come before the quit.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn replay(window: Option<QtRequest>, quit: bool) -> Vec<QtRequest> {
    let mut requests: Vec<_> = window.into_iter().collect();
    if quit {
        requests.push(QtRequest::Quit);
    }
    requests
}

/// The event that the window layer gets when the Qt loop stops. If Rust did
/// not request the stop, the app must stop too (UI-15: one process).
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn exit_event(quit_requested: bool) -> Option<UiEvent> {
    (!quit_requested).then_some(UiEvent::Quit)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // Tests may panic on a failed setup.

    use super::*;
    use telescreen_view::MainScreen;

    #[test]
    fn ui18_an_intent_json_round_trips() {
        let intent: Intent = parse_intent(r#"{"kind":"set_clipboard","on":false}"#).unwrap();
        assert_eq!(intent, Intent::SetClipboard { on: false });
        let json = serde_json::to_string(&intent).unwrap();
        assert_eq!(parse_intent(&json).unwrap(), intent);
    }

    #[test]
    fn ui18_json_that_is_not_an_intent_is_an_error() {
        assert!(parse_intent("{").is_err());
        assert!(parse_intent(r#"{"kind":"no_such_intent"}"#).is_err());
        assert!(parse_intent(r#"{"kind":"set_clipboard"}"#).is_err());
    }

    #[test]
    fn ui11_the_menu_property_round_trips_as_a_menu_screen() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures/main-idle.json");
        let screen: MainScreen =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        let json = menu_json(&screen.menu).unwrap();
        let back: MenuScreen = serde_json::from_str(&json).unwrap();
        assert_eq!(back, screen.menu);
    }

    /// The applet and the fake service use the same names as the app.
    #[test]
    fn ui11_the_applet_and_the_fake_service_use_the_same_names() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        for file in [
            "applet/dev.telescreen.menu/contents/ui/main.qml",
            "tools/fake_menu_service.py",
        ] {
            let text = std::fs::read_to_string(root.join(file)).unwrap();
            for name in [BUS_NAME, MENU_PATH, MENU_INTERFACE] {
                assert!(text.contains(&format!("\"{name}\"")), "{file}: {name}");
            }
        }
        let metadata =
            std::fs::read_to_string(root.join("applet/dev.telescreen.menu/metadata.json")).unwrap();
        assert!(metadata.contains(&format!("\"{BUS_NAME}\"")));
    }

    #[test]
    fn ui15_a_qt_stop_that_rust_did_not_request_stops_the_app() {
        assert_eq!(exit_event(false), Some(UiEvent::Quit));
        assert_eq!(exit_event(true), None);
    }

    #[test]
    fn ui15_requests_before_the_backend_replay_in_order() {
        assert_eq!(replay(None, false), vec![]);
        assert_eq!(
            replay(Some(QtRequest::HideMain), true),
            vec![QtRequest::HideMain, QtRequest::Quit]
        );
        assert_eq!(replay(None, true), vec![QtRequest::Quit]);
        assert_eq!(
            replay(Some(QtRequest::ShowMain), false),
            vec![QtRequest::ShowMain]
        );
    }
}
