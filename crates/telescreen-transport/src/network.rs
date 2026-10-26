//! The QUIC network of one peer: one endpoint that listens and dials (PEER-2).
//!
//! It runs on its own thread with a tokio runtime. The app talks to it with
//! [`NetCommand`]s and gets [`NetEvent`]s through a callback.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::Duration;

use quinn::crypto::rustls::{QuicClientConfig, QuicServerConfig};
use quinn::{Connection, Endpoint, RecvStream, SendStream};
use telescreen_core::ids::ComputerId;
use telescreen_core::wire::{self, MAX_FRAME, Message, PROTOCOL_VERSION};
use tokio::sync::mpsc;

use crate::{Identity, NetCommand, NetEvent, TrustStore, tls};

/// Where the trust list comes from. `File` reads the file at each handshake, so a
/// `telescreen trust` command applies with no restart.
#[derive(Debug, Clone)]
pub enum TrustSource {
    Fixed(Arc<TrustStore>),
    File(PathBuf),
}

impl TrustSource {
    /// The current trust list. A file that cannot be read gives an empty list, so
    /// that the peer refuses all connections instead of accepting them.
    pub fn current(&self) -> TrustStore {
        match self {
            Self::Fixed(store) => (**store).clone(),
            Self::File(path) => TrustStore::load(path).unwrap_or_else(|_| TrustStore::empty()),
        }
    }
}

/// Heartbeat timing (ADR 0007).
#[derive(Debug, Clone, Copy)]
pub struct Timing {
    pub heartbeat: Duration,
    /// No message for this time closes the connection.
    pub timeout: Duration,
    /// True: the network thread sends `Heartbeat` itself. False: the app sends it
    /// from its main loop, so that a hung app stops the heartbeat and the other
    /// peer detects the hang (AC-18).
    pub auto_heartbeat: bool,
}

impl Default for Timing {
    fn default() -> Self {
        Self {
            heartbeat: Duration::from_secs(1),
            timeout: Duration::from_secs(5),
            auto_heartbeat: true,
        }
    }
}

#[derive(Debug)]
pub struct NetConfig {
    pub local: ComputerId,
    pub listen: SocketAddr,
    pub identity: Identity,
    pub trust: TrustSource,
    pub timing: Timing,
}

#[derive(Debug, thiserror::Error)]
pub enum NetError {
    #[error("TLS setup: {0}")]
    Tls(#[from] rustls::Error),
    #[error("cannot listen on {addr}: {source}")]
    Listen {
        addr: SocketAddr,
        #[source]
        source: std::io::Error,
    },
    #[error("cannot start the network thread: {0}")]
    Thread(String),
}

/// A running network. Dropping it closes all connections and stops the thread.
#[derive(Debug)]
pub struct Network {
    commands: Option<mpsc::UnboundedSender<NetCommand>>,
    local_addr: SocketAddr,
    thread: Option<thread::JoinHandle<()>>,
}

impl Network {
    pub fn start(
        config: NetConfig,
        on_event: impl FnMut(NetEvent) + Send + 'static,
    ) -> Result<Self, NetError> {
        let (commands, command_rx) = mpsc::unbounded_channel();
        let (ready_tx, ready_rx) = std::sync::mpsc::channel();
        let thread = thread::Builder::new()
            .name("telescreen-net".into())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(e) => {
                        let _ = ready_tx.send(Err(NetError::Thread(e.to_string())));
                        return;
                    }
                };
                runtime.block_on(async move {
                    match Shared::bind(config, on_event) {
                        Ok(shared) => {
                            let _ = ready_tx.send(Ok(shared.endpoint_addr()));
                            shared.run(command_rx).await;
                        }
                        Err(e) => {
                            let _ = ready_tx.send(Err(e));
                        }
                    }
                });
            })
            .map_err(|e| NetError::Thread(e.to_string()))?;
        let local_addr = ready_rx
            .recv()
            .map_err(|_| NetError::Thread("the network thread stopped".into()))??;
        Ok(Self {
            commands: Some(commands),
            local_addr,
            thread: Some(thread),
        })
    }

    /// Queues a command. It never blocks.
    pub fn send(&self, command: NetCommand) {
        if let Some(commands) = &self.commands {
            let _ = commands.send(command);
        }
    }

    pub fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }
}

impl Drop for Network {
    fn drop(&mut self) {
        self.commands = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

struct Link {
    id: u64,
    connection: Connection,
    frames: mpsc::UnboundedSender<Message>,
}

type Events = mpsc::UnboundedSender<NetEvent>;

#[derive(Clone)]
struct Shared {
    local: ComputerId,
    endpoint: Endpoint,
    identity: Arc<Identity>,
    trust: TrustSource,
    timing: Timing,
    links: Arc<Mutex<HashMap<ComputerId, Link>>>,
    next_id: Arc<Mutex<u64>>,
    events: Events,
}

impl Shared {
    fn bind(
        config: NetConfig,
        mut on_event: impl FnMut(NetEvent) + Send + 'static,
    ) -> Result<Self, NetError> {
        let server = tls::listener_config(&config.identity, config.trust.clone())?;
        let server = QuicServerConfig::try_from(server)
            .map_err(|e| NetError::Tls(rustls::Error::General(e.to_string())))?;
        let mut server = quinn::ServerConfig::with_crypto(Arc::new(server));
        server.transport_config(transport_config(config.timing));
        let endpoint =
            Endpoint::server(server, config.listen).map_err(|source| NetError::Listen {
                addr: config.listen,
                source,
            })?;
        let (events, mut event_rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                on_event(event);
            }
        });
        Ok(Self {
            local: config.local,
            endpoint,
            identity: Arc::new(config.identity),
            trust: config.trust,
            timing: config.timing,
            links: Arc::default(),
            next_id: Arc::default(),
            events,
        })
    }

    fn endpoint_addr(&self) -> SocketAddr {
        self.endpoint
            .local_addr()
            .unwrap_or(SocketAddr::from(([0, 0, 0, 0], 0)))
    }

    fn emit(&self, event: NetEvent) {
        let _ = self.events.send(event);
    }

    fn links(&self) -> std::sync::MutexGuard<'_, HashMap<ComputerId, Link>> {
        self.links.lock().unwrap_or_else(PoisonError::into_inner)
    }

    async fn run(self, mut commands: mpsc::UnboundedReceiver<NetCommand>) {
        let acceptor = self.clone();
        let accept = tokio::spawn(async move {
            while let Some(incoming) = acceptor.endpoint.accept().await {
                let shared = acceptor.clone();
                tokio::spawn(async move { shared.accept(incoming).await });
            }
        });
        while let Some(command) = commands.recv().await {
            self.command(command);
        }
        accept.abort();
        for (_, link) in self.links().drain() {
            link.connection.close(0u32.into(), b"shutdown");
        }
        self.endpoint.wait_idle().await;
    }

    fn command(&self, command: NetCommand) {
        match command {
            NetCommand::Dial { peer, endpoint } => {
                if self.links().contains_key(&peer) {
                    self.emit(NetEvent::Connected { peer });
                    return;
                }
                let shared = self.clone();
                tokio::spawn(async move {
                    if let Err(reason) = shared.dial(&peer, &endpoint.host, endpoint.port).await {
                        shared.emit(NetEvent::DialFailed { peer, reason });
                    }
                });
            }
            NetCommand::Send { peer, message } => {
                if let Some(link) = self.links().get(&peer) {
                    let _ = link.frames.send(message);
                }
            }
            NetCommand::SendPointer { peer, datagram } => {
                if let (Some(link), Ok(bytes)) =
                    (self.links().get(&peer), wire::encode_pointer(&datagram))
                {
                    let _ = link.connection.send_datagram(bytes.into());
                }
            }
            NetCommand::Close { peer } => {
                if let Some(link) = self.links().get(&peer) {
                    link.connection.close(0u32.into(), b"closed");
                }
            }
        }
    }

    async fn dial(&self, peer: &ComputerId, host: &str, port: u16) -> Result<(), String> {
        let addrs: Vec<SocketAddr> = tokio::net::lookup_host((host, port))
            .await
            .map_err(|e| format!("cannot resolve {host}: {e}"))?
            .collect();
        // The endpoint has an IPv4 socket.
        let addr = addrs
            .iter()
            .find(|a| a.is_ipv4())
            .copied()
            .ok_or_else(|| format!("{host} has no IPv4 address"))?;
        let client = tls::dialer_config(&self.identity, peer.clone(), self.trust.clone())
            .map_err(|e| e.to_string())?;
        let client = QuicClientConfig::try_from(client).map_err(|e| e.to_string())?;
        let mut client = quinn::ClientConfig::new(Arc::new(client));
        client.transport_config(transport_config(self.timing));
        let connecting = self
            .endpoint
            .connect_with(client, addr, tls::SERVER_NAME)
            .map_err(|e| e.to_string())?;
        let connection = tokio::time::timeout(self.timing.timeout, connecting)
            .await
            .map_err(|_| format!("no answer from {addr}"))?
            .map_err(|e| format!("{addr}: {e}"))?;
        let (send, mut recv) = connection.open_bi().await.map_err(|e| e.to_string())?;
        let mut send = send;
        self.send_hello(&mut send).await?;
        let hello = self.read_hello(&mut recv).await?;
        if &hello != peer {
            connection.close(1u32.into(), b"wrong computer");
            return Err(format!("{addr} says it is {hello}, not {peer}"));
        }
        self.serve(peer.clone(), connection, send, recv);
        Ok(())
    }

    async fn accept(&self, incoming: quinn::Incoming) {
        let remote = incoming.remote_address().to_string();
        let refused = |reason: String| NetEvent::Refused {
            remote: remote.clone(),
            reason,
        };
        let connection = match incoming.await {
            Ok(connection) => connection,
            Err(e) => return self.emit(refused(e.to_string())),
        };
        let accepted = tokio::time::timeout(self.timing.timeout, connection.accept_bi()).await;
        let (mut send, mut recv) = match accepted {
            Ok(Ok(streams)) => streams,
            Ok(Err(e)) => return self.emit(refused(e.to_string())),
            Err(_) => return self.emit(refused("no control stream".into())),
        };
        let peer = match self.read_hello(&mut recv).await {
            Ok(peer) => peer,
            Err(reason) => return self.emit(refused(reason)),
        };
        // The certificate passed the verifier; the Hello must name a computer that
        // this fingerprint is trusted for (NET-3).
        let trusted = tls::peer_fingerprint(&connection)
            .is_some_and(|fp| self.trust.current().is_trusted(&peer, &fp));
        if !trusted {
            connection.close(1u32.into(), b"not trusted for this computer");
            return self.emit(refused(format!(
                "the certificate is not trusted for {peer}"
            )));
        }
        if let Err(reason) = self.send_hello(&mut send).await {
            return self.emit(refused(reason));
        }
        self.serve(peer, connection, send, recv);
    }

    async fn send_hello(&self, send: &mut SendStream) -> Result<(), String> {
        let hello = Message::Hello {
            protocol: PROTOCOL_VERSION,
            computer: self.local.clone(),
        };
        write_frame(send, &hello).await
    }

    async fn read_hello(&self, recv: &mut RecvStream) -> Result<ComputerId, String> {
        match tokio::time::timeout(self.timing.timeout, read_frame(recv)).await {
            Ok(Ok(Message::Hello {
                protocol: PROTOCOL_VERSION,
                computer,
            })) => Ok(computer),
            Ok(Ok(Message::Hello { protocol, .. })) => {
                Err(format!("protocol {protocol} is not supported"))
            }
            Ok(Ok(other)) => Err(format!("expected Hello, got {other:?}")),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("no Hello".into()),
        }
    }

    /// Registers an authenticated connection and runs its tasks.
    fn serve(&self, peer: ComputerId, connection: Connection, send: SendStream, recv: RecvStream) {
        let (frames, frame_rx) = mpsc::unbounded_channel();
        let id = {
            let mut next = self.next_id.lock().unwrap_or_else(PoisonError::into_inner);
            *next += 1;
            *next
        };
        let old = self.links().insert(
            peer.clone(),
            Link {
                id,
                connection: connection.clone(),
                frames,
            },
        );
        if let Some(old) = old {
            // A new connection replaces the old one (for example after a restart of
            // the peer). Report the old one as gone before the new one.
            old.connection.close(0u32.into(), b"replaced");
            self.emit(NetEvent::Disconnected {
                peer: peer.clone(),
                reason: "replaced".into(),
            });
        }
        self.emit(NetEvent::Connected { peer: peer.clone() });

        let heartbeat = self.timing.auto_heartbeat.then_some(self.timing.heartbeat);
        tokio::spawn(write_loop(send, frame_rx, heartbeat));
        let datagrams = {
            let (shared, peer, connection) = (self.clone(), peer.clone(), connection.clone());
            tokio::spawn(async move {
                while let Ok(bytes) = connection.read_datagram().await {
                    if let Ok(datagram) = wire::decode_pointer(&bytes) {
                        shared.emit(NetEvent::Pointer {
                            peer: peer.clone(),
                            datagram,
                        });
                    }
                }
            })
        };
        let shared = self.clone();
        tokio::spawn(async move {
            let reason = shared.read_loop(&peer, recv).await;
            connection.close(0u32.into(), b"closed");
            datagrams.abort();
            let mine = {
                let mut links = shared.links();
                let mine = links.get(&peer).is_some_and(|l| l.id == id);
                if mine {
                    links.remove(&peer);
                }
                mine
            };
            if mine {
                shared.emit(NetEvent::Disconnected { peer, reason });
            }
        });
    }

    /// Reads frames until the connection fails or the heartbeat times out.
    async fn read_loop(&self, peer: &ComputerId, mut recv: RecvStream) -> String {
        loop {
            match tokio::time::timeout(self.timing.timeout, read_frame(&mut recv)).await {
                Err(_) => return "heartbeat timeout".into(),
                Ok(Err(e)) => return e,
                Ok(Ok(Message::Heartbeat)) => {}
                Ok(Ok(Message::Hello { .. })) => return "unexpected Hello".into(),
                Ok(Ok(message)) => self.emit(NetEvent::Message {
                    peer: peer.clone(),
                    message,
                }),
            }
        }
    }
}

async fn write_loop(
    mut send: SendStream,
    mut frames: mpsc::UnboundedReceiver<Message>,
    heartbeat: Option<Duration>,
) {
    // With no automatic heartbeat, the ticker is not used.
    let mut ticker = tokio::time::interval(heartbeat.unwrap_or(Duration::from_secs(3600)));
    loop {
        let message = tokio::select! {
            message = frames.recv() => match message {
                Some(message) => message,
                None => return,
            },
            _ = ticker.tick(), if heartbeat.is_some() => Message::Heartbeat,
        };
        if write_frame(&mut send, &message).await.is_err() {
            return;
        }
    }
}

fn transport_config(timing: Timing) -> Arc<quinn::TransportConfig> {
    let mut config = quinn::TransportConfig::default();
    config.keep_alive_interval(Some(timing.heartbeat));
    if let Ok(idle) = quinn::IdleTimeout::try_from(timing.timeout) {
        config.max_idle_timeout(Some(idle));
    }
    Arc::new(config)
}

async fn write_frame(send: &mut SendStream, message: &Message) -> Result<(), String> {
    let payload = wire::encode_message(message).map_err(|e| e.to_string())?;
    let length = u32::try_from(payload.len()).map_err(|e| e.to_string())?;
    send.write_all(&length.to_le_bytes())
        .await
        .map_err(|e| e.to_string())?;
    send.write_all(&payload).await.map_err(|e| e.to_string())
}

async fn read_frame(recv: &mut RecvStream) -> Result<Message, String> {
    let mut length = [0u8; 4];
    recv.read_exact(&mut length)
        .await
        .map_err(|e| e.to_string())?;
    let length = u32::from_le_bytes(length) as usize;
    if length > MAX_FRAME {
        return Err(format!("frame of {length} bytes is larger than the limit"));
    }
    let mut payload = vec![0u8; length];
    recv.read_exact(&mut payload)
        .await
        .map_err(|e| e.to_string())?;
    wire::decode_message(&payload).map_err(|e| e.to_string())
}
