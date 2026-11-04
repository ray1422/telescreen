//! Discovery with Avahi through D-Bus (NET-12). The app has no responder of its
//! own. Avahi removes the objects of a D-Bus connection when it closes, so a crash
//! cannot leave a record.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::{self, JoinHandle};

use telescreen_core::platform::{
    DISCOVERY_SERVICE, Discovery, DiscoveryEvent, DiscoverySink, FoundPeer, PlatformError,
    discovery_instance_name, fingerprint_of_txt,
};
use zbus::blocking::{Connection, MessageIterator, fdo::DBusProxy};
use zbus::message::Type;
use zbus::zvariant::OwnedObjectPath;

const AVAHI: &str = "org.freedesktop.Avahi";
const SERVER: &str = "org.freedesktop.Avahi.Server";
const ENTRY_GROUP: &str = "org.freedesktop.Avahi.EntryGroup";
const BROWSER: &str = "org.freedesktop.Avahi.ServiceBrowser";
const IF_UNSPEC: i32 = -1;
const PROTO_INET: i32 = 0;

/// The Avahi backend.
pub struct AvahiDiscovery {
    sink: DiscoverySink,
    /// The record lives as long as this connection.
    announce: Option<Connection>,
    search: Option<Search>,
}

struct Search {
    connection: Connection,
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

impl std::fmt::Debug for AvahiDiscovery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AvahiDiscovery").finish_non_exhaustive()
    }
}

impl AvahiDiscovery {
    pub fn new(sink: DiscoverySink) -> Self {
        Self {
            sink,
            announce: None,
            search: None,
        }
    }
}

fn unavailable(error: zbus::Error) -> PlatformError {
    PlatformError::Unsupported(format!("Avahi is not available: {error}"))
}

impl Discovery for AvahiDiscovery {
    fn announce(&mut self, port: u16, fingerprint: &str) -> Result<(), PlatformError> {
        self.stop_announce();
        let connection = Connection::system().map_err(unavailable)?;
        let group: OwnedObjectPath = connection
            .call_method(Some(AVAHI), "/", Some(SERVER), "EntryGroupNew", &())
            .map_err(unavailable)?
            .body()
            .deserialize()
            .map_err(unavailable)?;
        let txt = vec![format!("fp={fingerprint}").into_bytes()];
        connection
            .call_method(
                Some(AVAHI),
                &group,
                Some(ENTRY_GROUP),
                "AddService",
                &(
                    IF_UNSPEC,
                    PROTO_INET,
                    0u32,
                    discovery_instance_name(),
                    DISCOVERY_SERVICE,
                    "",
                    "",
                    port,
                    txt,
                ),
            )
            .map_err(unavailable)?;
        connection
            .call_method(Some(AVAHI), &group, Some(ENTRY_GROUP), "Commit", &())
            .map_err(unavailable)?;
        self.announce = Some(connection);
        Ok(())
    }

    fn stop_announce(&mut self) {
        self.announce = None;
    }

    fn search(&mut self) -> Result<(), PlatformError> {
        self.stop_search();
        let connection = Connection::system().map_err(unavailable)?;
        // Before the browser exists: a browser can report at once.
        DBusProxy::new(&connection)
            .map_err(unavailable)?
            .add_match_rule(
                zbus::MatchRule::builder()
                    .msg_type(Type::Signal)
                    .interface(BROWSER)
                    .map_err(unavailable)?
                    .build(),
            )
            .map_err(|e| unavailable(e.into()))?;
        let messages = MessageIterator::from(&connection);
        connection
            .call_method(
                Some(AVAHI),
                "/",
                Some(SERVER),
                "ServiceBrowserNew",
                &(IF_UNSPEC, PROTO_INET, DISCOVERY_SERVICE, "", 0u32),
            )
            .map_err(unavailable)?;
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (connection, stop, sink) = (connection.clone(), stop.clone(), self.sink.clone());
            thread::Builder::new()
                .name("telescreen-avahi".into())
                .spawn(move || browse(&connection, messages, &stop, &sink))
                .map_err(|e| PlatformError::Failed(e.to_string()))?
        };
        self.search = Some(Search {
            connection,
            stop,
            thread,
        });
        Ok(())
    }

    fn stop_search(&mut self) {
        let Some(search) = self.search.take() else {
            return;
        };
        search.stop.store(true, Ordering::SeqCst);
        // The reply wakes the iterator of the thread.
        let _ = search.connection.call_method(
            Some("org.freedesktop.DBus"),
            "/org/freedesktop/DBus",
            Some("org.freedesktop.DBus.Peer"),
            "Ping",
            &(),
        );
        // Not joined on the main thread: a running resolve can take some time.
        drop(search.thread);
    }
}

impl Drop for AvahiDiscovery {
    fn drop(&mut self) {
        self.stop_search();
    }
}

fn browse(
    connection: &Connection,
    messages: MessageIterator,
    stop: &AtomicBool,
    sink: &DiscoverySink,
) {
    for message in messages {
        if stop.load(Ordering::SeqCst) {
            return;
        }
        let Ok(message) = message else { continue };
        let header = message.header();
        if header.interface().map(|i| i.as_str()) != Some(BROWSER) {
            continue;
        }
        match header.member().map(|m| m.as_str()) {
            Some("ItemNew") => {
                let item = message
                    .body()
                    .deserialize::<(i32, i32, String, String, String, u32)>();
                if let Ok((interface, protocol, name, service, domain, _)) = item
                    && let Some(found) =
                        resolve(connection, interface, protocol, name, service, domain)
                {
                    sink(DiscoveryEvent::Found(found));
                }
            }
            Some("Failure") => {
                let reason = message
                    .body()
                    .deserialize::<String>()
                    .unwrap_or_else(|e| e.to_string());
                sink(DiscoveryEvent::Failed(format!("Avahi: {reason}")));
                return;
            }
            _ => {}
        }
    }
    if !stop.load(Ordering::SeqCst) {
        sink(DiscoveryEvent::Failed(
            "the connection to Avahi closed".into(),
        ));
    }
}

/// Resolves one record. A record that does not resolve is not a result.
fn resolve(
    connection: &Connection,
    interface: i32,
    protocol: i32,
    name: String,
    service: String,
    domain: String,
) -> Option<FoundPeer> {
    type Reply = (
        i32,
        i32,
        String,
        String,
        String,
        String,
        i32,
        String,
        u16,
        Vec<Vec<u8>>,
        u32,
    );
    let reply = connection
        .call_method(
            Some(AVAHI),
            "/",
            Some(SERVER),
            "ResolveService",
            &(interface, protocol, name, service, domain, PROTO_INET, 0u32),
        )
        .ok()?;
    let (.., address, port, txt, _): Reply = reply.body().deserialize().ok()?;
    Some(FoundPeer {
        fingerprint: txt.iter().find_map(|entry| fingerprint_of_txt(entry))?,
        addresses: vec![address.parse().ok()?],
        port,
    })
}
