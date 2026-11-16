//! Desktop login detection on the system bus (STG-3, STG-9).

use std::os::unix::fs::MetadataExt;
use std::thread::{self, JoinHandle};

use telescreen_core::platform::PlatformError;
use zbus::blocking::{Connection, MessageIterator, Proxy, fdo::DBusProxy};
use zbus::message::Type;
use zbus::proxy::CacheProperties;
use zbus::zvariant::OwnedObjectPath;

const LOGIN: &str = "org.freedesktop.login1";
const MANAGER: &str = "org.freedesktop.login1.Manager";
const PATH: &str = "/org/freedesktop/login1";
const SESSION: &str = "org.freedesktop.login1.Session";

pub fn is_desktop_login(class: &str, seat: &str, uid: u32, own_uid: u32) -> bool {
    class == "user" && !seat.is_empty() && uid == own_uid
}

/// A session in the state `closing` stays after a logout while its processes
/// run. It is not a desktop session of the user any more.
pub fn is_open_session(state: &str) -> bool {
    state != "closing"
}

fn failed(error: impl std::fmt::Display) -> PlatformError {
    PlatformError::Failed(format!("logind: {error}"))
}

fn own_uid() -> Result<u32, PlatformError> {
    std::fs::metadata("/proc/self")
        .map(|metadata| metadata.uid())
        .map_err(|e| PlatformError::Failed(format!("cannot read /proc/self: {e}")))
}

fn session_proxy(connection: &Connection, path: OwnedObjectPath) -> zbus::Result<Proxy<'static>> {
    zbus::blocking::proxy::Builder::new(connection)
        .destination(LOGIN)?
        .path(path)?
        .interface(SESSION)?
        .cache_properties(CacheProperties::No)
        .build()
}

pub fn desktop_session_exists() -> Result<bool, PlatformError> {
    let own_uid = own_uid()?;
    let connection = Connection::system().map_err(failed)?;
    let reply = connection
        .call_method(Some(LOGIN), PATH, Some(MANAGER), "ListSessions", &())
        .map_err(failed)?;
    type Session = (String, u32, String, String, OwnedObjectPath);
    let sessions: Vec<Session> = reply.body().deserialize().map_err(failed)?;
    for (_, uid, _, seat, path) in sessions {
        let proxy = session_proxy(&connection, path).map_err(failed)?;
        let class: String = proxy.get_property("Class").map_err(failed)?;
        if !is_desktop_login(&class, &seat, uid, own_uid) {
            continue;
        }
        let state: String = proxy.get_property("State").map_err(failed)?;
        if is_open_session(&state) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Dropping the watch detaches its thread; the process ends it at exit.
#[derive(Debug)]
pub struct LoginWatch {
    _thread: JoinHandle<()>,
}

pub fn watch_desktop_login(
    on_login: impl Fn() + Send + 'static,
) -> Result<LoginWatch, PlatformError> {
    let own_uid = own_uid()?;
    let connection = Connection::system().map_err(failed)?;
    DBusProxy::new(&connection)
        .map_err(failed)?
        .add_match_rule(
            zbus::MatchRule::builder()
                .msg_type(Type::Signal)
                .sender(LOGIN)
                .map_err(failed)?
                .path(PATH)
                .map_err(failed)?
                .interface(MANAGER)
                .map_err(failed)?
                .member("SessionNew")
                .map_err(failed)?
                .build(),
        )
        .map_err(failed)?;
    let messages = MessageIterator::from(&connection);
    let thread = thread::Builder::new()
        .name("telescreen-logind".into())
        .spawn(move || {
            for message in messages {
                let Ok(message) = message else { continue };
                let header = message.header();
                if header.interface().map(|i| i.as_str()) != Some(MANAGER)
                    || header.member().map(|m| m.as_str()) != Some("SessionNew")
                {
                    continue;
                }
                let Ok((_, path)) = message.body().deserialize::<(String, OwnedObjectPath)>()
                else {
                    continue;
                };
                // Sessions can disappear before their properties are read.
                if let Ok(true) = new_desktop_session(&connection, path, own_uid) {
                    on_login();
                }
            }
        })
        .map_err(failed)?;
    Ok(LoginWatch { _thread: thread })
}

fn new_desktop_session(
    connection: &Connection,
    path: OwnedObjectPath,
    own_uid: u32,
) -> zbus::Result<bool> {
    let proxy = session_proxy(connection, path)?;
    let class: String = proxy.get_property("Class")?;
    let (seat, _): (String, OwnedObjectPath) = proxy.get_property("Seat")?;
    let (uid, _): (u32, OwnedObjectPath) = proxy.get_property("User")?;
    Ok(is_desktop_login(&class, &seat, uid, own_uid))
}

#[cfg(test)]
mod tests {
    use super::{is_desktop_login, is_open_session};

    #[test]
    fn only_the_own_desktop_login_matches() {
        assert!(is_desktop_login("user", "seat0", 1000, 1000));
        assert!(!is_desktop_login("user", "", 1000, 1000));
        assert!(!is_desktop_login("greeter", "seat0", 1000, 1000));
        assert!(!is_desktop_login("user", "seat0", 1001, 1000));
        assert!(!is_desktop_login("user-early", "seat0", 1000, 1000));
    }

    #[test]
    fn a_closing_session_is_not_a_desktop_session() {
        assert!(is_open_session("active"));
        assert!(is_open_session("online"));
        assert!(!is_open_session("closing"));
    }
}
