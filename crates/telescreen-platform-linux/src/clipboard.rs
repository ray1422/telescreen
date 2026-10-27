//! The regular Wayland text clipboard on its own connection (CLIP-7, CLIP-8).
//! The main thread only reads a channel of changes (IN-2).

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, PipeReader, Read, Write};
use std::os::fd::{AsFd, AsRawFd};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, SyncSender, channel, sync_channel};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use telescreen_core::clipboard::{ClipboardItem, MAX_TEXT};
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

type Items = Arc<Mutex<Receiver<ClipboardItem>>>;

/// Sends clipboard work to a separate Wayland thread (CLIP-8, IN-2).
#[derive(Debug)]
pub struct LinuxClipboard {
    commands: Sender<String>,
    items: Items,
}

impl LinuxClipboard {
    /// Connects to data-control on the first seat (CLIP-8).
    /// Fails if the compositor has no data-control manager.
    pub fn start() -> Result<Self, String> {
        let (commands, command_rx) = channel();
        let (item_tx, item_rx) = sync_channel(1);
        let items = Arc::new(Mutex::new(item_rx));
        let thread_items = Arc::clone(&items);
        let (ready_tx, ready_rx) = channel();
        thread::Builder::new()
            .name("telescreen-clipboard".into())
            .spawn(move || match Backend::connect() {
                Ok(backend) => {
                    if ready_tx.send(Ok(())).is_ok() {
                        backend.run(command_rx, item_tx, thread_items);
                    }
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                }
            })
            .map_err(|error| format!("cannot start the clipboard thread: {error}"))?;
        ready_rx
            .recv_timeout(Duration::from_secs(2))
            .map_err(|error| format!("the clipboard did not start: {error}"))??;
        Ok(Self { commands, items })
    }
}

impl Clipboard for LinuxClipboard {
    fn changed(&mut self) -> Option<ClipboardItem> {
        // The producer also reads this one-slot channel to replace an unread item.
        // Never wait for its lock on the main thread (IN-2).
        self.items.try_lock().ok()?.try_recv().ok()
    }

    fn set_text(&mut self, text: &str) -> Result<(), PlatformError> {
        self.commands
            .send(text.to_owned())
            .map_err(|_| PlatformError::Failed("the clipboard thread stopped".into()))
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
        let mut state = State::default();
        queue
            .roundtrip(&mut state)
            .map_err(|error| format!("Wayland data-control: {error}"))?;
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

    fn run(mut self, commands: Receiver<String>, item_tx: SyncSender<ClipboardItem>, items: Items) {
        loop {
            match commands.recv_timeout(TICK) {
                Ok(text) => {
                    let source = self
                        .manager
                        .create_data_source(&self.queue.handle(), Arc::new(text));
                    for mime in TEXT_TYPES {
                        source.offer(mime.into());
                    }
                    self.device.set_selection(Some(&source));
                    self.state.sources.push(source);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            if let Err(error) = self.dispatch() {
                eprintln!("telescreen: clipboard: {error}");
                return;
            }
            if self.state.finished {
                return;
            }
            let Some(mut pending) = self.state.pending.take() else {
                continue;
            };
            match pending.read_ready() {
                Ok(true) => {
                    if let Some(item) = pending.into_item() {
                        let Ok(receiver) = items.lock() else {
                            return;
                        };
                        // Keep just the latest unread item (CLIP-8).
                        let _ = receiver.try_recv();
                        let _ = item_tx.try_send(item);
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
                if let Some(offer) = &id
                    && let Some(data) = state.offers.get(&offer.id())
                {
                    state.pending = PendingItem::receive(connection, offer, &data.types).ok();
                }
                // An empty selection produces no change (CLIP-8).
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

impl Dispatch<DataSource, Arc<String>> for State {
    fn event(
        state: &mut Self,
        source: &DataSource,
        event: zwlr_data_control_source_v1::Event,
        text: &Arc<String>,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_data_control_source_v1::Event::Send { mime_type, fd } => {
                if !TEXT_TYPES.contains(&mime_type.as_str()) {
                    return;
                }
                let text = Arc::clone(text);
                // The reader can stall without blocking Wayland or its own echo (CLIP-4).
                if let Err(error) = thread::Builder::new()
                    .name("telescreen-clipboard-write".into())
                    .spawn(move || {
                        let mut file = File::from(fd);
                        let _ = file.write_all(text.as_bytes());
                    })
                {
                    eprintln!("telescreen: cannot start a clipboard writer: {error}");
                }
            }
            zwlr_data_control_source_v1::Event::Cancelled => {
                state.sources.retain(|active| active != source);
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
