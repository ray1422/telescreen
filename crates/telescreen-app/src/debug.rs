//! The debug protocol for dev builds (feature `debug-protocol`,
//! docs/debug-protocol.md). A local tool reads the state of the app and sends
//! intents through a Unix domain socket, so that a test can run without clicks.
//!
//! Each request and each response is one line of JSON. This module parses the
//! requests, writes the responses, and runs the socket server. The app answers
//! each request on its main thread.

use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::os::unix::fs::{FileTypeExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};

use serde_json::{Value, json};
use telescreen_core::ids::{ComputerId, DeviceId, DisplayToken, SessionId};
use telescreen_view::{ControlScreen, Intent, PortalScreen};

use crate::controller::PortalState;

/// One request line.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    /// `{"cmd":"state"}`: the view models and the window facts.
    State,
    /// `{"cmd":"pointer_stats"}`: current or last incoming session statistics (NET-9).
    PointerStats,
    /// `{"cmd":"intent","intent":<intent>}`: the same as a click in the UI.
    Intent(Intent),
    /// `{"cmd":"ack"}`: `AckOrigin::Computer`: the portal session, else a local verification.
    Ack,
    /// `{"cmd":"hold_keyboard","display":"<token>"}`: takes keyboard focus (UI-7).
    HoldKeyboard { display: DisplayToken },
    /// `{"cmd":"release_keyboard"}`: removes the focus holder (UI-7).
    ReleaseKeyboard,
    /// `{"cmd":"quit"}`: the same as closing the control window.
    Quit,
    /// `{"cmd":"choose_computer","computer":"<id>"}`: the same as the click in the
    /// start window (UI-8).
    ChooseComputer { computer: ComputerId },
    /// `{"cmd":"move_control","x":<x>,"y":<y>}`: moves the control window, in
    /// logical pixels. The portal opens on the display of this window.
    MoveControl { x: f64, y: f64 },
    /// `{"cmd":"menu","show":<bool>}`: opens or closes the native menu (UI-9), and
    /// returns the last view model and the panel frame. `show` is optional.
    Menu { show: Option<bool> },
    /// `{"cmd":"main_window","show":<bool>}`: opens or closes the native main
    /// window (UI-13), and returns its state and the Dock icon state (UI-14).
    MainWindow { show: Option<bool> },
}

#[derive(Debug, thiserror::Error)]
pub enum DebugError {
    #[error("invalid request: {0}")]
    Request(String),
    #[error("socket {path}: {message}")]
    Socket { path: String, message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowRole {
    Control,
    Portal,
    Prompt,
}

/// What the window system reports for one window. The view model does not show
/// where a window really is (the portal display bug of 2026-10-27).
#[derive(Debug, Clone, PartialEq)]
pub struct WindowFacts {
    pub role: WindowRole,
    /// The display token of the monitor of the window, if the window system
    /// reports it.
    pub monitor: Option<DisplayToken>,
    pub fullscreen: bool,
    pub focused: bool,
    /// The outer position in logical pixels, if the window system reports it
    /// (Wayland does not).
    pub position: Option<(f64, f64)>,
    /// The inner size in logical pixels.
    pub size: (f64, f64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct StateReport {
    pub control: ControlScreen,
    /// The main portal: the portal with the native focus, else the newest
    /// portal. Scripts read this field.
    pub portal: Option<PortalScreen>,
    /// Each portal, in the sequence of the portal ids (FLOW-25).
    pub portals: Vec<PortalState>,
    pub windows: Vec<WindowFacts>,
}

/// Parses one request line.
pub fn parse_request(line: &str) -> Result<Request, DebugError> {
    let value: Value =
        serde_json::from_str(line).map_err(|error| DebugError::Request(error.to_string()))?;
    match string_field(&value, "cmd")? {
        "state" => Ok(Request::State),
        "pointer_stats" => Ok(Request::PointerStats),
        "ack" => Ok(Request::Ack),
        "hold_keyboard" => Ok(Request::HoldKeyboard {
            display: DisplayToken::from(string_field(&value, "display")?),
        }),
        "release_keyboard" => Ok(Request::ReleaseKeyboard),
        "quit" => Ok(Request::Quit),
        "choose_computer" => Ok(Request::ChooseComputer {
            computer: ComputerId::from(string_field(&value, "computer")?),
        }),
        "menu" => Ok(Request::Menu {
            show: value.get("show").and_then(Value::as_bool),
        }),
        "main_window" => Ok(Request::MainWindow {
            show: value.get("show").and_then(Value::as_bool),
        }),
        "move_control" => Ok(Request::MoveControl {
            x: number_field(&value, "x")?,
            y: number_field(&value, "y")?,
        }),
        "intent" => {
            let intent = value
                .get("intent")
                .ok_or_else(|| DebugError::Request("missing intent".into()))?;
            parse_intent(intent).map(Request::Intent)
        }
        cmd => Err(DebugError::Request(format!("unknown command: {cmd}"))),
    }
}

fn string_field<'a>(value: &'a Value, field: &str) -> Result<&'a str, DebugError> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| DebugError::Request(format!("{field} must be a string")))
}

fn number_field(value: &Value, field: &str) -> Result<f64, DebugError> {
    value
        .get(field)
        .and_then(Value::as_f64)
        .ok_or_else(|| DebugError::Request(format!("{field} must be a number")))
}

fn parse_intent(value: &Value) -> Result<Intent, DebugError> {
    // The JSON form of the native UI (UI-18).
    if value.get("kind").is_some() {
        return serde_json::from_value(value.clone())
            .map_err(|error| DebugError::Request(error.to_string()));
    }
    if let Some(name) = value.as_str() {
        return match name {
            "AllowDiscovery" => Ok(Intent::AllowDiscovery),
            "StopDiscovery" => Ok(Intent::StopDiscovery),
            "EndSession" => Ok(Intent::EndSession { portal: None }),
            _ => Err(DebugError::Request(format!("unknown intent: {name}"))),
        };
    }
    let object = value
        .as_object()
        .filter(|object| object.len() == 1)
        .ok_or_else(|| DebugError::Request("intent must be a name or one variant object".into()))?;
    let (name, arguments) = object
        .iter()
        .next()
        .ok_or_else(|| DebugError::Request("missing intent variant".into()))?;
    match name.as_str() {
        "DeviceIsBack" => Ok(Intent::DeviceIsBack {
            device: DeviceId::from(string_field(arguments, "device")?),
        }),
        "SetClipboard" => Ok(Intent::SetClipboard {
            on: arguments
                .get("on")
                .and_then(Value::as_bool)
                .ok_or_else(|| DebugError::Request("on must be a boolean".into()))?,
        }),
        "HandBack" | "StopSession" | "ControlFromHere" => {
            let session = arguments
                .get("session")
                .and_then(Value::as_u64)
                .map(SessionId)
                .ok_or_else(|| {
                    DebugError::Request("session must be a non-negative u64 integer".into())
                })?;
            if name == "HandBack" {
                Ok(Intent::HandBack { session })
            } else if name == "StopSession" {
                Ok(Intent::StopSession { session })
            } else {
                Ok(Intent::ControlFromHere { session })
            }
        }
        _ => Err(DebugError::Request(format!("unknown intent: {name}"))),
    }
}

/// `{"ok":true,"control":"<Debug text>","portal":"<Debug text>"|null,"portals":[...],"windows":[...]}`
pub fn state_response(report: &StateReport) -> String {
    let windows: Vec<Value> = report
        .windows
        .iter()
        .map(|window| {
            let role = match window.role {
                WindowRole::Control => "control",
                WindowRole::Portal => "portal",
                WindowRole::Prompt => "prompt",
            };
            json!({
                "role": role,
                "monitor": window.monitor.as_ref().map(|monitor| monitor.0.as_str()),
                "fullscreen": window.fullscreen,
                "focused": window.focused,
                "position": window.position,
                "size": window.size,
            })
        })
        .collect();
    let portals: Vec<Value> = report
        .portals
        .iter()
        .map(|portal| {
            json!({
                "portal": portal.portal.0,
                "display": portal.display.0.as_str(),
                "session": portal.session.map(|session| session.0),
                "state": portal.state,
                "screen": format!("{:?}", portal.screen),
            })
        })
        .collect();
    json!({
        "ok": true,
        "control": format!("{:?}", report.control),
        "portal": report.portal.as_ref().map(|portal| format!("{portal:?}")),
        "portals": portals,
        "windows": windows,
    })
    .to_string()
}

/// `{"ok":true}`
pub fn ok_response() -> String {
    json!({"ok": true}).to_string()
}

/// `{"ok":false,"error":"<message>"}`
pub fn error_response(message: &str) -> String {
    json!({"ok": false, "error": message}).to_string()
}

/// The running server. Dropping it stops accepting connections and removes the
/// socket file.
#[derive(Debug)]
pub struct Server {
    path: PathBuf,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // Wake a blocking accept so it can observe the stop flag. If the socket
        // file is gone, nothing can wake it: do not wait for the thread, else the
        // main thread hangs and the status item stops responding.
        let woke = UnixStream::connect(&self.path).is_ok();
        if let Some(thread) = self.thread.take()
            && woke
        {
            let _ = thread.join();
        }
        let _ = fs::remove_file(&self.path);
    }
}

/// Starts the server on its own thread. `handler` gets each valid request and
/// returns the response line. An invalid line gets an error response, and the
/// handler does not see it.
pub fn serve<F>(path: &Path, handler: F) -> Result<Server, DebugError>
where
    F: Fn(Request) -> String + Send + 'static,
{
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_socket() => {
            fs::remove_file(path).map_err(|error| socket_error(path, error))?;
        }
        Ok(_) => return Err(socket_error(path, "path exists and is not a socket")),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(socket_error(path, error)),
    }
    let listener = UnixListener::bind(path).map_err(|error| socket_error(path, error))?;
    if let Err(error) = fs::set_permissions(path, fs::Permissions::from_mode(0o600)) {
        let _ = fs::remove_file(path);
        return Err(socket_error(path, error));
    }
    let stop = Arc::new(AtomicBool::new(false));
    let thread_stop = Arc::clone(&stop);
    let handler = Arc::new(Mutex::new(handler));
    let thread = thread::Builder::new()
        .name("debug-listener".into())
        .spawn(move || {
            while !thread_stop.load(Ordering::Acquire) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        if thread_stop.load(Ordering::Acquire) {
                            break;
                        }
                        let handler = Arc::clone(&handler);
                        let _ = thread::Builder::new()
                            .name("debug-client".into())
                            .spawn(move || serve_connection(stream, &handler));
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
        });
    match thread {
        Ok(thread) => Ok(Server {
            path: path.to_owned(),
            stop,
            thread: Some(thread),
        }),
        Err(error) => {
            let _ = fs::remove_file(path);
            Err(socket_error(path, error))
        }
    }
}

fn socket_error(path: &Path, error: impl std::fmt::Display) -> DebugError {
    DebugError::Socket {
        path: path.display().to_string(),
        message: error.to_string(),
    }
}

fn serve_connection<F>(stream: UnixStream, handler: &Mutex<F>)
where
    F: Fn(Request) -> String,
{
    let mut reader = BufReader::new(stream);
    let mut line = Vec::new();
    loop {
        line.clear();
        match reader.read_until(b'\n', &mut line) {
            Ok(0) => break,
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
        let request = std::str::from_utf8(&line)
            .map_err(|error| DebugError::Request(error.to_string()))
            .and_then(parse_request);
        let response = match request {
            Ok(request) => match handler.lock() {
                Ok(handler) => handler(request),
                Err(_) => error_response("request handler is unavailable"),
            },
            Err(error) => error_response(&error.to_string()),
        };
        if writeln!(reader.get_mut(), "{response}").is_err() {
            break;
        }
    }
}
