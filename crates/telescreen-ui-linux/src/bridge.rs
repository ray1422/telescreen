//! The `Backend` QML singleton: the view model as JSON, and the intents back.

use std::pin::Pin;
use std::sync::{Arc, Mutex};

use cxx_qt::{CxxQtThread, Threading};
use cxx_qt_lib::QString;
use telescreen_view::MainScreen;

use crate::{QtRequest, UiEvent};

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        /// The QML side of the UI. `screen` is the newest `MainScreen` as JSON.
        #[qobject]
        #[qml_element]
        #[qml_singleton]
        #[qproperty(QString, screen)]
        type Backend = super::BackendRust;

        /// Sends an `Intent` as JSON. Returns false if the JSON is not an intent.
        #[qinvokable]
        fn send(self: &Backend, intent: &QString) -> bool;

        #[qinvokable]
        #[cxx_name = "openSettings"]
        fn open_settings(self: &Backend);

        #[qinvokable]
        fn quit(self: &Backend);

        /// The main window tells Rust when it shows or hides.
        #[qinvokable]
        #[cxx_name = "setMainVisible"]
        fn set_main_visible(self: &Backend, visible: bool);

        /// Rust asks the main window to show and take the focus (UI-13, UI-15).
        #[qsignal]
        #[cxx_name = "showMainRequested"]
        fn show_main_requested(self: Pin<&mut Backend>);

        /// Rust asks the main window to close as the close button does.
        #[qsignal]
        #[cxx_name = "hideMainRequested"]
        fn hide_main_requested(self: Pin<&mut Backend>);
    }

    impl cxx_qt::Threading for Backend {}
    impl cxx_qt::Initialize for Backend {}

    unsafe extern "C++" {
        include!("app.h");
        /// Makes the `QApplication`, loads `dev.telescreen.ui/App`, and runs the
        /// Qt event loop. Returns the exit code.
        fn ts_ui_run() -> i32;

        /// Stops the Qt event loop. Qt thread only.
        fn ts_ui_quit();

        /// Makes a `QApplication` on the offscreen platform and loads
        /// `dev.telescreen.ui/App` with no event loop (DIST-10). Returns 0 if
        /// QML made the main window.
        fn ts_ui_self_check() -> i32;
    }
}

#[derive(Default)]
pub struct BackendRust {
    screen: QString,
}

type Sink = Arc<dyn Fn(UiEvent) + Send + Sync>;

struct Shared {
    /// The newest view model, also for a `Backend` that QML makes later.
    screen: String,
    thread: Option<CxxQtThread<qobject::Backend>>,
    events: Option<Sink>,
    /// The last window request before the `Backend` exists.
    pending_window: Option<QtRequest>,
    /// Rust requested the stop of the Qt loop. It is never cleared.
    quit_requested: bool,
    main_visible: bool,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared {
    screen: String::new(),
    thread: None,
    events: None,
    pending_window: None,
    quit_requested: false,
    main_visible: false,
});

fn shared() -> std::sync::MutexGuard<'static, Shared> {
    SHARED.lock().unwrap_or_else(|e| e.into_inner())
}

fn emit(event: UiEvent) {
    // Not under the lock: the sink can call `LinuxUi` or `quit`.
    let events = shared().events.clone();
    match events {
        Some(events) => events(event),
        None => eprintln!("telescreen-ui-linux: no receiver for {event:?}"),
    }
}

/// Runs `request` on the Qt thread. Qt thread only, or in a queued call.
fn apply(backend: Pin<&mut qobject::Backend>, request: QtRequest) {
    match request {
        QtRequest::ShowMain => backend.show_main_requested(),
        QtRequest::HideMain => backend.hide_main_requested(),
        QtRequest::Quit => qobject::ts_ui_quit(),
    }
}

/// Queues `request` to the Qt thread, or keeps it until the `Backend` exists.
fn request(request: QtRequest) {
    let mut shared = shared();
    if request == QtRequest::Quit {
        shared.quit_requested = true;
    }
    let Some(thread) = shared.thread.clone() else {
        if request != QtRequest::Quit {
            shared.pending_window = Some(request);
        }
        return;
    };
    if thread
        .queue(move |backend| apply(backend, request))
        .is_err()
    {
        // The Qt loop stopped, and its objects are gone.
        shared.thread = None;
    }
}

impl cxx_qt::Initialize for qobject::Backend {
    fn initialize(mut self: Pin<&mut Self>) {
        let thread = self.qt_thread();
        let (screen, requests) = {
            let mut shared = shared();
            let requests = crate::replay(shared.pending_window.take(), shared.quit_requested);
            shared.thread = Some(thread.clone());
            (shared.screen.clone(), requests)
        };
        self.as_mut().set_screen(QString::from(screen.as_str()));
        // Queued: the QML connections to the signals do not exist yet.
        for request in requests {
            _ = thread.queue(move |backend| apply(backend, request));
        }
    }
}

impl qobject::Backend {
    fn send(&self, intent: &QString) -> bool {
        match crate::parse_intent(&intent.to_string()) {
            Ok(intent) => {
                emit(UiEvent::Intent(intent));
                true
            }
            Err(e) => {
                eprintln!("telescreen-ui-linux: {e}");
                false
            }
        }
    }

    fn open_settings(&self) {
        emit(UiEvent::OpenSettings);
    }

    fn quit(&self) {
        emit(UiEvent::Quit);
    }

    fn set_main_visible(&self, visible: bool) {
        shared().main_visible = visible;
    }
}

/// The handle of the window layer to the Qt UI. Any thread can use it.
#[derive(Debug)]
pub struct LinuxUi {
    _private: (),
}

impl LinuxUi {
    /// Gives each event of the UI to `sink`. `sink` runs on the Qt thread, so it
    /// must only queue the event and wake the loop of the window layer.
    pub fn new(sink: impl Fn(UiEvent) + Send + Sync + 'static) -> Self {
        shared().events = Some(Arc::new(sink));
        Self { _private: () }
    }

    /// Shows `screen` in the main window.
    pub fn push_main(&self, screen: &MainScreen) -> Result<(), String> {
        let json = serde_json::to_string(screen).map_err(|e| e.to_string())?;
        let mut shared = shared();
        shared.screen.clone_from(&json);
        if let Some(thread) = &shared.thread {
            let queued =
                thread.queue(move |backend| backend.set_screen(QString::from(json.as_str())));
            if queued.is_err() {
                shared.thread = None;
            }
        }
        Ok(())
    }

    /// Shows the main window again after a close (UI-13, UI-15).
    pub fn show_main_window(&self) {
        request(QtRequest::ShowMain);
    }

    /// Closes the main window as the close button does: the app continues to
    /// run (UI-15).
    pub fn close_main_window(&self) {
        request(QtRequest::HideMain);
    }

    /// The last visibility that the main window reported.
    pub fn main_window_visible(&self) -> bool {
        shared().main_visible
    }
}

/// Stops the Qt event loop, also before it starts. Any thread can call it.
pub fn quit() {
    request(QtRequest::Quit);
}

/// Runs the Qt event loop on this thread until the application quits. Call it
/// on the main thread of the process. If the loop stops for a cause other than
/// [`quit`], the sink of [`LinuxUi::new`] gets [`UiEvent::Quit`].
/// Checks that the QML of the main window loads: the Qt and KDE modules that
/// the binary needs at runtime exist (DIST-10). It opens no window on a screen
/// and uses no network. Call it on the main thread, before any other Qt call,
/// and do not call [`run`] after it in the same process.
pub fn self_check() -> Result<(), String> {
    cxx_qt::init_crate!(telescreen_ui_linux);
    cxx_qt::init_qml_module!("dev.telescreen.ui");
    match qobject::ts_ui_self_check() {
        0 => Ok(()),
        _ => Err("the QML of the main window does not load; see the QML errors above".into()),
    }
}

pub fn run() -> i32 {
    cxx_qt::init_crate!(telescreen_ui_linux);
    cxx_qt::init_qml_module!("dev.telescreen.ui");
    let code = qobject::ts_ui_run();
    let event = {
        let mut shared = shared();
        // The `Backend` is gone with the QML engine.
        shared.thread = None;
        shared.main_visible = false;
        crate::exit_event(shared.quit_requested)
    };
    if let Some(event) = event {
        emit(event);
    }
    code
}
