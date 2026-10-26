//! The network helper (ADR 0005): a process that does all QUIC I/O and talks to
//! the app over a Unix domain socket, one JSON line for each `NetCommand` or
//! `NetEvent`. On macOS the app starts it as its own responsible process, because
//! of Local Network privacy. The protocol is local and private to this binary.

use std::io::{BufRead, BufReader, Write};
use std::net::SocketAddr;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use telescreen_core::ids::ComputerId;
use telescreen_transport::network::{Timing, TrustSource};
use telescreen_transport::{Identity, NetCommand, NetConfig, NetEvent, Network};

use crate::controller::NetPort;
use crate::paths;

/// The helper side: connect to the app, run the network, and stop when the app
/// closes the socket.
pub fn serve(socket: &Path, local: ComputerId, port: u16) -> Result<(), String> {
    let stream = UnixStream::connect(socket).map_err(|e| format!("{}: {e}", socket.display()))?;
    let writer = Arc::new(Mutex::new(stream.try_clone().map_err(|e| e.to_string())?));
    let identity = Identity::load_or_create(&paths::identity_dir()?).map_err(|e| e.to_string())?;
    let events = writer.clone();
    let network = Network::start(
        NetConfig {
            local,
            listen: SocketAddr::from(([0, 0, 0, 0], port)),
            identity,
            trust: TrustSource::File(paths::trust_file()?),
            // The app sends the heartbeat from its main loop (AC-18).
            timing: Timing {
                auto_heartbeat: false,
                ..Timing::default()
            },
        },
        move |event: NetEvent| {
            let mut stream = events.lock().unwrap_or_else(PoisonError::into_inner);
            let _ = write_line(&mut *stream, &event);
        },
    )
    .map_err(|e| e.to_string())?;
    for line in BufReader::new(stream).lines() {
        let Ok(line) = line else { break };
        match serde_json::from_str::<NetCommand>(&line) {
            Ok(command) => network.send(command),
            Err(e) => eprintln!("telescreen net-helper: bad command: {e}"),
        }
    }
    Ok(())
}

fn write_line(writer: &mut impl Write, value: &impl serde::Serialize) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(value).map_err(std::io::Error::other)?;
    line.push(b'\n');
    writer.write_all(&line)
}

/// The app side: a `NetPort` that writes commands to the helper.
#[derive(Debug)]
pub struct HelperNet {
    stream: UnixStream,
}

impl Drop for HelperNet {
    /// The reader thread has a clone of the socket. Shut down both directions, so
    /// that the helper sees the end of the stream and stops.
    fn drop(&mut self) {
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

impl NetPort for HelperNet {
    fn send(&mut self, command: NetCommand) {
        if let Err(e) = write_line(&mut self.stream, &command) {
            eprintln!("telescreen: network helper: {e}");
        }
    }
}

/// A listening socket in a new private directory, for one helper connection.
#[derive(Debug)]
pub struct HelperSocket {
    listener: UnixListener,
    pub path: PathBuf,
}

impl HelperSocket {
    pub fn bind() -> Result<Self, String> {
        static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("telescreen-{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        let path = dir.join("net.sock");
        let _ = std::fs::remove_file(&path);
        let listener = UnixListener::bind(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(Self { listener, path })
    }

    /// Waits for the helper. Then events go to `on_event` from a reader thread.
    /// `on_stop` runs once when the helper closes the socket.
    pub fn accept(
        self,
        timeout: Duration,
        on_event: impl Fn(NetEvent) + Send + 'static,
        on_stop: impl FnOnce() + Send + 'static,
    ) -> Result<HelperNet, String> {
        self.listener
            .set_nonblocking(true)
            .map_err(|e| e.to_string())?;
        let deadline = Instant::now() + timeout;
        let stream = loop {
            match self.listener.accept() {
                Ok((stream, _)) => break stream,
                Err(e)
                    if e.kind() == std::io::ErrorKind::WouldBlock && Instant::now() < deadline =>
                {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) => return Err(format!("the network helper did not connect: {e}")),
            }
        };
        let _ = std::fs::remove_file(&self.path);
        stream.set_nonblocking(false).map_err(|e| e.to_string())?;
        let reader = stream.try_clone().map_err(|e| e.to_string())?;
        std::thread::Builder::new()
            .name("telescreen-net-reader".into())
            .spawn(move || {
                for line in BufReader::new(reader).lines() {
                    let Ok(line) = line else { break };
                    if let Ok(event) = serde_json::from_str::<NetEvent>(&line) {
                        on_event(event);
                    }
                }
                on_stop();
            })
            .map_err(|e| e.to_string())?;
        Ok(HelperNet { stream })
    }
}
