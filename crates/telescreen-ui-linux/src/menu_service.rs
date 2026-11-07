//! The D-Bus service of the Plasma applet (UI-11) and the single instance
//! (UI-15). `tools/fake_menu_service.py` serves the same interface with a
//! fixture.
//!
//! The service has its own blocking zbus connection. zbus runs the method calls
//! on the executor thread of that connection, so a method only gives a
//! [`UiEvent`] to the sink and does not block.

use std::collections::HashMap;
use std::sync::Arc;

use telescreen_view::MenuScreen;
use zbus::blocking::Connection;
use zbus::fdo::{self, RequestNameFlags, RequestNameReply};
use zbus::zvariant::Value;

use crate::{BUS_NAME, MENU_INTERFACE, MENU_PATH, UiEvent};

type Sink = Arc<dyn Fn(UiEvent) + Send + Sync>;

/// The object `/dev/telescreen/Menu` with the interface `dev.telescreen.Menu1`.
///
/// Before the app starts (the start window of UI-8), it has no sink. Then
/// `Menu` is empty, `ShowMainWindow` does nothing, because the main window opens
/// when the app starts (UI-13), and the other methods reply with an error.
#[derive(Default)]
pub struct MenuService {
    /// The newest `MenuScreen` as JSON.
    menu: String,
    sink: Option<Sink>,
}

impl std::fmt::Debug for MenuService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MenuService")
            .field("menu", &self.menu)
            .field("sink", &self.sink.is_some())
            .finish()
    }
}

impl MenuService {
    /// A service that gives each method call to `sink`.
    pub fn with_sink(sink: impl Fn(UiEvent) + Send + Sync + 'static) -> Self {
        Self {
            menu: String::new(),
            sink: Some(Arc::new(sink)),
        }
    }

    fn emit(&self, event: UiEvent) -> fdo::Result<()> {
        match &self.sink {
            Some(sink) => {
                sink(event);
                Ok(())
            }
            None => Err(fdo::Error::Failed("Telescreen has not started".into())),
        }
    }
}

#[zbus::interface(name = "dev.telescreen.Menu1")]
impl MenuService {
    /// The `MenuScreen` as JSON (UI-18).
    #[zbus(property, name = "Menu")]
    fn menu(&self) -> String {
        self.menu.clone()
    }

    /// Runs one `Intent` as JSON. JSON that is not an intent gives
    /// `org.freedesktop.DBus.Error.InvalidArgs`.
    fn send(&self, intent: &str) -> fdo::Result<()> {
        let intent = crate::parse_intent(intent).map_err(fdo::Error::InvalidArgs)?;
        self.emit(UiEvent::Intent(intent))
    }

    /// "Open Telescreen", and a second start (UI-13, UI-15).
    fn show_main_window(&self) -> fdo::Result<()> {
        if self.sink.is_none() {
            return Ok(());
        }
        self.emit(UiEvent::ShowMainWindow)
    }

    /// "Configure Telescreen…" (UI-14).
    fn open_settings(&self) -> fdo::Result<()> {
        self.emit(UiEvent::OpenSettings)
    }

    /// "Quit Telescreen" (UI-15).
    fn quit(&self) -> fdo::Result<()> {
        self.emit(UiEvent::Quit)
    }
}

/// The result of [`claim`].
#[derive(Debug)]
pub enum Claim {
    /// This process owns the bus name and serves the menu.
    Owner(MenuServer),
    /// A different process owns the bus name. The result of its
    /// `ShowMainWindow()` call.
    Running(Result<(), String>),
}

/// Whether a reply of `RequestName` makes this process the single instance.
fn owns_name(reply: &RequestNameReply) -> bool {
    matches!(
        reply,
        RequestNameReply::PrimaryOwner | RequestNameReply::AlreadyOwner
    )
}

/// Serves the menu, then requests the bus name with no queue (UI-11, UI-15).
/// If a different process owns the name, it calls `ShowMainWindow()` on that
/// process. An error means that the session bus is not available.
pub fn claim() -> zbus::Result<Claim> {
    let connection = Connection::session()?;
    // Serve first, so that no method call comes before the object exists.
    connection
        .object_server()
        .at(MENU_PATH, MenuService::default())?;
    let reply =
        connection.request_name_with_flags(BUS_NAME, RequestNameFlags::DoNotQueue.into())?;
    if owns_name(&reply) {
        return Ok(Claim::Owner(MenuServer { connection }));
    }
    let shown = connection
        .call_method(
            Some(BUS_NAME),
            MENU_PATH,
            Some(MENU_INTERFACE),
            "ShowMainWindow",
            &(),
        )
        .map(drop)
        .map_err(|e| e.to_string());
    Ok(Claim::Running(shown))
}

/// The handle of the window layer to the D-Bus service. A drop releases the
/// bus name.
#[derive(Debug)]
pub struct MenuServer {
    connection: Connection,
}

impl MenuServer {
    fn service(&self) -> Result<zbus::blocking::object_server::InterfaceRef<MenuService>, String> {
        self.connection
            .object_server()
            .interface::<_, MenuService>(MENU_PATH)
            .map_err(|e| e.to_string())
    }

    /// Gives each method call to `sink`. `sink` runs on the zbus thread, so it
    /// must only queue the event and wake the loop of the window layer.
    pub fn set_sink(&self, sink: impl Fn(UiEvent) + Send + Sync + 'static) -> Result<(), String> {
        self.service()?.get_mut().sink = Some(Arc::new(sink));
        Ok(())
    }

    /// Sets the property `Menu` and sends `PropertiesChanged` (UI-11).
    pub fn push_menu(&self, menu: &MenuScreen) -> Result<(), String> {
        let json = crate::menu_json(menu)?;
        self.service()?.get_mut().menu.clone_from(&json);
        let changed = HashMap::from([("Menu", Value::from(json.as_str()))]);
        self.connection
            .emit_signal(
                None::<&str>,
                MENU_PATH,
                "org.freedesktop.DBus.Properties",
                "PropertiesChanged",
                &(MENU_INTERFACE, changed, Vec::<&str>::new()),
            )
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)] // Tests may panic on a failed setup.

    use std::sync::Mutex;

    use telescreen_view::Intent;
    use zbus::object_server::Interface;

    use super::*;

    fn recording() -> (MenuService, Arc<Mutex<Vec<UiEvent>>>) {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink = events.clone();
        let service = MenuService::with_sink(move |event| sink.lock().unwrap().push(event));
        (service, events)
    }

    #[test]
    fn ui11_each_method_gives_its_ui_event() {
        let (service, events) = recording();
        service.send(r#"{"kind":"allow_discovery"}"#).unwrap();
        service.show_main_window().unwrap();
        service.open_settings().unwrap();
        service.quit().unwrap();
        assert_eq!(
            *events.lock().unwrap(),
            vec![
                UiEvent::Intent(Intent::AllowDiscovery),
                UiEvent::ShowMainWindow,
                UiEvent::OpenSettings,
                UiEvent::Quit,
            ]
        );
    }

    #[test]
    fn ui11_send_with_json_that_is_not_an_intent_replies_invalid_args() {
        let (service, events) = recording();
        assert!(matches!(
            service.send("{\"kind\":"),
            Err(fdo::Error::InvalidArgs(_))
        ));
        assert!(matches!(
            service.send(r#"{"kind":"no_such_intent"}"#),
            Err(fdo::Error::InvalidArgs(_))
        ));
        assert!(events.lock().unwrap().is_empty());
    }

    #[test]
    fn ui13_before_the_app_starts_only_show_main_window_succeeds() {
        let service = MenuService::default();
        assert_eq!(service.menu(), "");
        assert!(service.show_main_window().is_ok());
        assert!(service.send(r#"{"kind":"allow_discovery"}"#).is_err());
        assert!(service.open_settings().is_err());
        assert!(service.quit().is_err());
    }

    #[test]
    fn ui15_only_the_primary_owner_is_the_single_instance() {
        assert!(owns_name(&RequestNameReply::PrimaryOwner));
        assert!(owns_name(&RequestNameReply::AlreadyOwner));
        assert!(!owns_name(&RequestNameReply::Exists));
        assert!(!owns_name(&RequestNameReply::InQueue));
    }

    /// The members that the applet and `tools/fake_menu_service.py` use.
    #[test]
    fn ui11_the_interface_has_the_members_of_the_contract() {
        assert_eq!(MenuService::name().as_str(), MENU_INTERFACE);
        let mut xml = String::new();
        MenuService::default().introspect_to_writer(&mut xml, 0);
        for member in [
            r#"<method name="Send">"#,
            r#"<arg name="intent" type="s" direction="in"/>"#,
            r#"<method name="ShowMainWindow">"#,
            r#"<method name="OpenSettings">"#,
            r#"<method name="Quit">"#,
            r#"<property name="Menu" type="s" access="read"/>"#,
        ] {
            assert!(xml.contains(member), "{member} is not in {xml}");
        }
    }
}
