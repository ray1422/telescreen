//! The `Backend` QML singleton: the view model as JSON, and the intents back.

use std::pin::Pin;
use std::sync::Mutex;
use std::sync::mpsc::Sender;

use cxx_qt::{CxxQtThread, Threading};
use cxx_qt_lib::QString;
use telescreen_view::{Intent, MainScreen};

use crate::UiEvent;

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
    }

    impl cxx_qt::Threading for Backend {}
    impl cxx_qt::Initialize for Backend {}

    unsafe extern "C++" {
        include!("app.h");
        /// Makes the `QApplication`, loads `dev.telescreen.ui/App`, and runs the
        /// Qt event loop. Returns the exit code.
        fn ts_ui_run() -> i32;
    }
}

#[derive(Default)]
pub struct BackendRust {
    screen: QString,
}

struct Shared {
    /// The newest view model, also for a `Backend` that QML makes later.
    screen: String,
    thread: Option<CxxQtThread<qobject::Backend>>,
    events: Option<Sender<UiEvent>>,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared {
    screen: String::new(),
    thread: None,
    events: None,
});

fn shared() -> std::sync::MutexGuard<'static, Shared> {
    SHARED.lock().unwrap_or_else(|e| e.into_inner())
}

fn emit(event: UiEvent) {
    match &shared().events {
        Some(events) => _ = events.send(event),
        None => eprintln!("telescreen-ui-linux: no receiver for {event:?}"),
    }
}

impl cxx_qt::Initialize for qobject::Backend {
    fn initialize(mut self: Pin<&mut Self>) {
        let thread = self.qt_thread();
        let screen = {
            let mut shared = shared();
            shared.thread = Some(thread);
            shared.screen.clone()
        };
        self.as_mut().set_screen(QString::from(screen.as_str()));
    }
}

impl qobject::Backend {
    fn send(&self, intent: &QString) -> bool {
        match serde_json::from_str::<Intent>(&intent.to_string()) {
            Ok(intent) => {
                emit(UiEvent::Intent(intent));
                true
            }
            Err(e) => {
                eprintln!("telescreen-ui-linux: not an intent: {e}: {intent}");
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
}

/// Gives the UI events to `events`.
pub fn set_events(events: Sender<UiEvent>) {
    shared().events = Some(events);
}

/// Shows `screen` in the main window. Any thread can call it.
pub fn push_main(screen: &MainScreen) -> Result<(), String> {
    let json = serde_json::to_string(screen).map_err(|e| e.to_string())?;
    let mut shared = shared();
    shared.screen.clone_from(&json);
    if let Some(thread) = &shared.thread {
        let queued = thread.queue(move |backend| backend.set_screen(QString::from(json.as_str())));
        if queued.is_err() {
            shared.thread = None;
        }
    }
    Ok(())
}

/// Runs the Qt event loop on this thread until the application quits.
pub fn run() -> i32 {
    cxx_qt::init_crate!(telescreen_ui_linux);
    cxx_qt::init_qml_module!("dev.telescreen.ui");
    qobject::ts_ui_run()
}
