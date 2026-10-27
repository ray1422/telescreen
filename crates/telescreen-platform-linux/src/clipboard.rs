//! The regular Wayland text clipboard on its own connection (CLIP-7, CLIP-8).
//! The connection is open only while the clipboard can sync (CLIP-11).
//! The main thread only reads a channel of changes (IN-2).

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, PipeReader, Read, Write};
use std::os::fd::{AsFd, AsRawFd};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender, channel, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use telescreen_core::clipboard::{ClipboardChange, ClipboardItem, MAX_TEXT};
use telescreen_core::platform::{Clipboard, PlatformError};
use wayland_client::backend::{ObjectId, WaylandError};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols_wlr::data_control::v1::client::{
    zwlr_data_control_device_v1::{self, ZwlrDataControlDeviceV1 as DataDevice},
    zwlr_data_control_manager_v1::ZwlrDataControlManagerV1 as DataManager,
    zwlr_data_control_offer_v1::{self, ZwlrDataControlOfferV1 as DataOffer},
    zwlr_data_control_source_v1::{self, ZwlrDataControlSourceV1 as DataSource},
};

const TEXT_TYPES: [&str; 3] = ["text/plain;charset=utf-8", "text/plain", "UTF8_STRING"];
const PASSWORD_HINT: &str = "x-kde-passwordManagerHint";
const READ_TIMEOUT: Duration = Duration::from_secs(1);
const TICK: Duration = Duration::from_millis(10);

type Changes = Arc<Mutex<Receiver<ClipboardChange>>>;

enum Command {
    Set { text: String, concealed: bool },
    ClearIfOwned,
    Close,
}

/// The data of one source that this peer offers (CLIP-7).
struct SourceData {
    text: String,
    concealed: bool,
}

/// Sends clipboard work to a separate Wayland thread while open (CLIP-8, IN-2).
#[derive(Debug, Default)]
pub struct LinuxClipboard {
    worker: Option<Worker>,
}

#[derive(Debug)]
struct Worker {
    commands: Sender<Command>,
    changes: Changes,
    thread: JoinHandle<()>,
}

impl LinuxClipboard {
    /// Checks that the compositor has data-control on the first seat (CLIP-8).
    /// Keeps no connection open and reads nothing.
    pub fn start() -> Result<Self, String> {
        drop(Backend::connect()?);
        Ok(Self::default())
    }
}

impl Clipboard for LinuxClipboard {
    fn open(&mut self) -> Result<(), PlatformError> {
        if self.worker.is_some() {
            return Ok(());
        }
        let (commands, command_rx) = channel();
        let (change_tx, change_rx) = sync_channel(1);
        let changes = Arc::new(Mutex::new(change_rx));
        let thread_changes = Arc::clone(&changes);
        let (ready_tx, ready_rx) = channel();
        let thread = thread::Builder::new()
            .name("telescreen-clipboard".into())
            .spawn(move || match Backend::connect() {
                Ok(backend) => {
                    if ready_tx.send(Ok(())).is_ok() {
                        backend.run(command_rx, change_tx, thread_changes);
                    }
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                }
            })
            .map_err(|error| {
                PlatformError::Failed(format!("cannot start the clipboard thread: {error}"))
            })?;
        ready_rx
            .recv_timeout(Duration::from_secs(2))
            .map_err(|error| {
                PlatformError::Failed(format!("the clipboard did not start: {error}"))
            })?
            .map_err(PlatformError::Failed)?;
        self.worker = Some(Worker {
            commands,
            changes,
            thread,
        });
        Ok(())
    }

    fn close(&mut self) {
        let Some(worker) = self.worker.take() else {
            return;
        };
        // The thread runs the commands in sequence, so a clear before this one is done.
        let _ = worker.commands.send(Command::Close);
        if worker.thread.join().is_err() {
            eprintln!("telescreen: the clipboard thread failed");
        }
    }

    fn changed(&mut self) -> Option<ClipboardChange> {
        // The producer also reads this one-slot channel to replace an unread change.
        // Never wait for its lock on the main thread (IN-2).
        self.worker
            .as_ref()?
            .changes
            .try_lock()
            .ok()?
            .try_recv()
            .ok()
    }

    fn set_text(&mut self, text: &str, concealed: bool) -> Result<(), PlatformError> {
        self.send(Command::Set {
            text: text.to_owned(),
            concealed,
        })
    }

    fn clear_if_owned(&mut self) {
        let _ = self.send(Command::ClearIfOwned);
    }
}

impl LinuxClipboard {
    fn send(&self, command: Command) -> Result<(), PlatformError> {
        self.worker
            .as_ref()
            .ok_or_else(|| PlatformError::Failed("the clipboard is closed".into()))?
            .commands
            .send(command)
            .map_err(|_| PlatformError::Failed("the clipboard thread stopped".into()))
    }
}

impl Drop for LinuxClipboard {
    fn drop(&mut self) {
        self.clear_if_owned();
        self.close();
    }
}

struct Backend {
    connection: Connection,
    queue: EventQueue<State>,
    state: State,
    manager: DataManager,
    device: DataDevice,
}

impl Backend {
    fn connect() -> Result<Self, String> {
        let connection =
            Connection::connect_to_env().map_err(|error| format!("Wayland: {error}"))?;
        let (list, mut queue) = registry_queue_init::<State>(&connection)
            .map_err(|error| format!("Wayland registry: {error}"))?;
        let qh = queue.handle();
        let manager: DataManager = list
            .bind(&qh, 1..=2, ())
            .map_err(|error| format!("cannot bind zwlr_data_control_manager_v1: {error}"))?;
        let seat: wl_seat::WlSeat = list
            .bind(&qh, 1..=7, ())
            .map_err(|error| format!("cannot bind the first wl_seat: {error}"))?;
        let device = manager.get_data_device(&seat, &qh, ());
        // The first selection is the baseline: do not read it (CLIP-3).
        let mut state = State {
            baseline: true,
            ..State::default()
        };
        queue
            .roundtrip(&mut state)
            .map_err(|error| format!("Wayland data-control: {error}"))?;
        state.baseline = false;
        state.cleared = false;
        if state.finished {
            return Err("the compositor stopped the data-control device".into());
        }
        Ok(Self {
            connection,
            queue,
            state,
            manager,
            device,
        })
    }

    fn run(
        mut self,
        commands: Receiver<Command>,
        change_tx: SyncSender<ClipboardChange>,
        changes: Changes,
    ) {
        let publish = |change: ClipboardChange| -> bool {
            let Ok(receiver) = changes.lock() else {
                return false;
            };
            // Keep just the latest unread change (CLIP-8).
            let _ = receiver.try_recv();
            let _ = change_tx.try_send(change);
            true
        };
        loop {
            match commands.recv_timeout(TICK) {
                Ok(Command::Set { text, concealed }) => {
                    let source = self.manager.create_data_source(
                        &self.queue.handle(),
                        Arc::new(SourceData { text, concealed }),
                    );
                    for mime in TEXT_TYPES {
                        source.offer(mime.into());
                    }
                    if concealed {
                        source.offer(PASSWORD_HINT.into());
                    }
                    self.device.set_selection(Some(&source));
                    self.state.own = Some(source.clone());
                    self.state.sources.push(source);
                }
                Ok(Command::ClearIfOwned) => {
                    // The compositor cancels the own source when a different
                    // program takes the selection (CLIP-11).
                    if self.state.own.take().is_some() {
                        self.device.set_selection(None);
                    }
                }
                Ok(Command::Close) | Err(RecvTimeoutError::Disconnected) => return,
                Err(RecvTimeoutError::Timeout) => {}
            }
            if let Err(error) = self.dispatch() {
                eprintln!("telescreen: clipboard: {error}");
                return;
            }
            if self.state.finished {
                return;
            }
            if std::mem::take(&mut self.state.cleared) && !publish(ClipboardChange::Cleared) {
                return;
            }
            let Some(mut pending) = self.state.pending.take() else {
                continue;
            };
            match pending.read_ready() {
                Ok(true) => {
                    if let Some(item) = pending.into_item()
                        && !publish(ClipboardChange::Text(item))
                    {
                        return;
                    }
                }
                Ok(false) => self.state.pending = Some(pending),
                // A failed or timed out hint must not expose a password (CLIP-7).
                Err(_) => {}
            }
        }
    }

    fn dispatch(&mut self) -> Result<(), wayland_client::DispatchError> {
        self.queue.dispatch_pending(&mut self.state)?;
        if let Some(guard) = self.connection.prepare_read() {
            match guard.read() {
                Ok(_) => {}
                Err(WaylandError::Io(error))
                    if matches!(
                        error.kind(),
                        io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                    ) => {}
                Err(error) => return Err(error.into()),
            }
        }
        self.queue.dispatch_pending(&mut self.state)?;
        match self.connection.flush() {
            Ok(()) => Ok(()),
            Err(WaylandError::Io(error)) if error.kind() == io::ErrorKind::WouldBlock => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}

impl Drop for Backend {
    fn drop(&mut self) {
        self.state.pending = None;
        self.state.own = None;
        for (_, offer) in self.state.offers.drain() {
            offer.proxy.destroy();
        }
        for source in self.state.sources.drain(..) {
            source.destroy();
        }
        if !self.state.finished {
            self.device.destroy();
        }
        self.manager.destroy();
        let _ = self.connection.flush();
    }
}

#[derive(Default)]
struct State {
    offers: HashMap<ObjectId, Offer>,
    selection: Option<DataOffer>,
    pending: Option<PendingItem>,
    sources: Vec<DataSource>,
    /// The latest source that this peer set and that the compositor did not cancel.
    own: Option<DataSource>,
    /// The next selection is the baseline (CLIP-3).
    baseline: bool,
    /// The selection holds no text now (CLIP-7).
    cleared: bool,
    finished: bool,
}

struct Offer {
    proxy: DataOffer,
    types: Vec<String>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

wayland_client::delegate_noop!(State: ignore wl_seat::WlSeat);
wayland_client::delegate_noop!(State: ignore DataManager);

impl Dispatch<DataDevice, ()> for State {
    wayland_client::event_created_child!(State, DataDevice, [
        zwlr_data_control_device_v1::EVT_DATA_OFFER_OPCODE => (DataOffer, ())
    ]);

    fn event(
        state: &mut Self,
        device: &DataDevice,
        event: zwlr_data_control_device_v1::Event,
        _: &(),
        connection: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_device_v1::Event::DataOffer { id } => {
                state.offers.insert(
                    id.id(),
                    Offer {
                        proxy: id,
                        types: Vec::new(),
                    },
                );
            }
            zwlr_data_control_device_v1::Event::Selection { id } => {
                state.pending = None;
                if let Some(previous) = state.selection.take() {
                    state.offers.remove(&previous.id());
                    previous.destroy();
                }
                state.cleared = false;
                if !state.baseline {
                    match id
                        .as_ref()
                        .and_then(|offer| Some((offer, state.offers.get(&offer.id())?)))
                    {
                        Some((offer, data)) => {
                            state.pending =
                                PendingItem::receive(connection, offer, &data.types).ok();
                            // An offer with no text type is not text (CLIP-7).
                            state.cleared = state.pending.is_none();
                        }
                        // An empty selection is a clear (CLIP-7).
                        None => state.cleared = true,
                    }
                }
                state.selection = id;
            }
            zwlr_data_control_device_v1::Event::PrimarySelection { id: Some(offer) } => {
                state.offers.remove(&offer.id());
                offer.destroy();
            }
            zwlr_data_control_device_v1::Event::Finished => {
                device.destroy();
                state.finished = true;
                state.pending = None;
            }
            _ => {}
        }
    }
}

impl Dispatch<DataOffer, ()> for State {
    fn event(
        state: &mut Self,
        offer: &DataOffer,
        event: zwlr_data_control_offer_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let zwlr_data_control_offer_v1::Event::Offer { mime_type } = event
            && let Some(data) = state.offers.get_mut(&offer.id())
        {
            data.types.push(mime_type);
        }
    }
}

impl Dispatch<DataSource, Arc<SourceData>> for State {
    fn event(
        state: &mut Self,
        source: &DataSource,
        event: zwlr_data_control_source_v1::Event,
        data: &Arc<SourceData>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_source_v1::Event::Send { mime_type, fd } => {
                // The hint gives the mark, never the text (CLIP-7).
                let hint = mime_type == PASSWORD_HINT && data.concealed;
                if !hint && !TEXT_TYPES.contains(&mime_type.as_str()) {
                    return;
                }
                let data = Arc::clone(data);
                // The reader can stall without blocking Wayland or its own echo (CLIP-4).
                if let Err(error) = thread::Builder::new()
                    .name("telescreen-clipboard-write".into())
                    .spawn(move || {
                        let mut file = File::from(fd);
                        let bytes = if hint {
                            b"secret".as_slice()
                        } else {
                            data.text.as_bytes()
                        };
                        let _ = file.write_all(bytes);
                    })
                {
                    eprintln!("telescreen: cannot start a clipboard writer: {error}");
                }
            }
            zwlr_data_control_source_v1::Event::Cancelled => {
                state.sources.retain(|active| active != source);
                if state.own.as_ref() == Some(source) {
                    state.own = None;
                }
                source.destroy();
            }
            _ => {}
        }
    }
}

struct PendingItem {
    text: ReadPipe,
    hint: Option<ReadPipe>,
    deadline: Instant,
}

impl PendingItem {
    fn receive(connection: &Connection, offer: &DataOffer, types: &[String]) -> io::Result<Self> {
        let mime = TEXT_TYPES
            .into_iter()
            .find(|mime| types.iter().any(|offered| offered == mime))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no text MIME type"))?;
        let deadline = Instant::now() + READ_TIMEOUT;
        let text = ReadPipe::receive(connection, offer, mime, MAX_TEXT + 4)?;
        let hint = if types.iter().any(|mime| mime == PASSWORD_HINT) {
            // One extra byte distinguishes "secret" from a longer value (CLIP-7).
            Some(ReadPipe::receive(connection, offer, PASSWORD_HINT, 7)?)
        } else {
            None
        };
        Ok(Self {
            text,
            hint,
            deadline,
        })
    }

    fn read_ready(&mut self) -> io::Result<bool> {
        let text_done = self.text.read_ready(self.deadline)?;
        let hint_done = match &mut self.hint {
            Some(hint) => hint.read_ready(self.deadline)?,
            None => true,
        };
        Ok(text_done && hint_done)
    }

    /// Keeps an oversized UTF-8 prefix for the core to reject (CLIP-6).
    fn into_item(self) -> Option<ClipboardItem> {
        let text = match String::from_utf8(self.text.bytes) {
            Ok(text) => text,
            Err(error) => {
                let end = error.utf8_error().valid_up_to();
                if error.as_bytes().len() <= MAX_TEXT || end <= MAX_TEXT {
                    return None;
                }
                let mut bytes = error.into_bytes();
                bytes.truncate(end);
                String::from_utf8(bytes).ok()?
            }
        };
        Some(ClipboardItem {
            text,
            concealed: self.hint.is_some_and(|hint| hint.bytes == b"secret"),
        })
    }
}

struct ReadPipe {
    reader: Option<PipeReader>,
    bytes: Vec<u8>,
    limit: usize,
}

impl ReadPipe {
    fn receive(
        connection: &Connection,
        offer: &DataOffer,
        mime: &str,
        limit: usize,
    ) -> io::Result<Self> {
        let (reader, writer) = io::pipe()?;
        // SAFETY: reader owns this live pipe fd, and F_GETFL takes no third argument.
        let flags = unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_GETFL) };
        if flags == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: reader owns this live pipe fd, and F_SETFL takes these integer flags.
        if unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1
        {
            return Err(io::Error::last_os_error());
        }
        offer.receive(mime.into(), writer.as_fd());
        let flushed = connection.flush();
        drop(writer);
        flushed.map_err(io::Error::other)?;
        Ok(Self {
            reader: Some(reader),
            bytes: Vec::new(),
            limit,
        })
    }

    /// Reads without waiting and checks one total deadline (CLIP-8).
    fn read_ready(&mut self, deadline: Instant) -> io::Result<bool> {
        let Some(reader) = &mut self.reader else {
            return Ok(true);
        };
        let mut buffer = [0; 8192];
        loop {
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "clipboard read timed out",
                ));
            }
            let available = buffer.len().min(self.limit - self.bytes.len());
            match reader.read(&mut buffer[..available]) {
                Ok(0) => {
                    self.reader = None;
                    return Ok(true);
                }
                Ok(count) => {
                    self.bytes.extend_from_slice(&buffer[..count]);
                    if self.bytes.len() == self.limit {
                        self.reader = None;
                        return Ok(true);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(false),
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
    }
}
