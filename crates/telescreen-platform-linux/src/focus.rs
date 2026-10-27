//! The invisible keyboard focus holder on the target (FLOW-23, UI-7).
//! It runs on its own thread with its own Wayland connection.
//! It never takes a wl_keyboard, so keys sent to it are lost.

use std::fs::File;
use std::os::fd::{AsFd, FromRawFd};
use std::os::unix::fs::FileExt;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::{Duration, Instant};

use telescreen_core::platform::{DisplayInfo, FocusHolder, PlatformError};
use wayland_client::protocol::{wl_buffer, wl_compositor, wl_shm, wl_surface};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::wl::Globals;

const TIMEOUT: Duration = Duration::from_secs(2);

enum Command {
    Hold {
        output: String,
        result: Sender<Result<(), String>>,
    },
    Release {
        result: Sender<()>,
    },
}

/// Holds keyboard focus without showing anything or taking pointer input (UI-7).
#[derive(Debug)]
pub struct LinuxFocusHolder {
    commands: Sender<Command>,
    held: bool,
}

impl LinuxFocusHolder {
    /// Connects and starts the focus holder thread (UI-7).
    /// Fails if the compositor has no layer-shell protocol.
    pub fn start() -> Result<Self, String> {
        let (commands, command_rx) = channel();
        let (ready_tx, ready_rx) = channel();
        thread::Builder::new()
            .name("telescreen-focus".into())
            .spawn(move || match Holder::connect() {
                Ok(holder) => {
                    if ready_tx.send(Ok(())).is_ok() {
                        holder.run(command_rx);
                    }
                }
                Err(error) => {
                    let _ = ready_tx.send(Err(error));
                }
            })
            .map_err(|error| error.to_string())?;
        ready_rx
            .recv_timeout(TIMEOUT)
            .map_err(|error| format!("the focus holder did not start: {error}"))??;
        Ok(Self {
            commands,
            held: false,
        })
    }
}

impl FocusHolder for LinuxFocusHolder {
    fn hold(&mut self, display: &DisplayInfo) -> Result<(), PlatformError> {
        if self.held {
            return Ok(());
        }
        let (result, reply) = channel();
        self.commands
            .send(Command::Hold {
                output: display.token.0.clone(),
                result,
            })
            .map_err(|_| PlatformError::Failed("the focus holder thread stopped".into()))?;
        match reply.recv_timeout(TIMEOUT) {
            Ok(result) => {
                result.map_err(PlatformError::Failed)?;
                self.held = true;
                Ok(())
            }
            Err(error) => {
                let (result, _) = channel();
                let _ = self.commands.send(Command::Release { result });
                Err(PlatformError::Failed(format!(
                    "the focus holder did not respond: {error}"
                )))
            }
        }
    }

    fn release(&mut self) {
        if !self.held {
            return;
        }
        self.held = false;
        let (result, reply) = channel();
        if self.commands.send(Command::Release { result }).is_ok() {
            let _ = reply.recv_timeout(TIMEOUT);
        }
    }
}

struct Surface {
    surface: wl_surface::WlSurface,
    layer: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    buffer: Option<wl_buffer::WlBuffer>,
}

struct Holder {
    globals: Globals,
    compositor: wl_compositor::WlCompositor,
    shm: wl_shm::WlShm,
    shell: zwlr_layer_shell_v1::ZwlrLayerShellV1,
    surface: Option<Surface>,
}

impl Holder {
    fn connect() -> Result<Self, String> {
        let globals = Globals::connect()?;
        let qh = globals.handle();
        let compositor = globals
            .list
            .bind(&qh, 4..=6, ())
            .map_err(|_| "no wl_compositor".to_string())?;
        let shm = globals
            .list
            .bind(&qh, 1..=1, ())
            .map_err(|_| "no wl_shm".to_string())?;
        let shell = globals
            .list
            .bind(&qh, 1..=4, ())
            .map_err(|_| "the compositor has no zwlr_layer_shell_v1".to_string())?;
        Ok(Self {
            globals,
            compositor,
            shm,
            shell,
            surface: None,
        })
    }

    fn run(mut self, commands: Receiver<Command>) {
        loop {
            match commands.recv_timeout(Duration::from_millis(250)) {
                Ok(Command::Hold { output, result }) => {
                    let held = self.hold(&output);
                    let failed = held.is_err();
                    let abandoned = result.send(held).is_err();
                    if failed || abandoned {
                        self.release();
                    }
                }
                Ok(Command::Release { result }) => {
                    self.release();
                    if self.globals.roundtrip().is_err() {
                        return;
                    }
                    let _ = result.send(());
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    self.release();
                    return;
                }
            }
            if self.globals.roundtrip().is_err() {
                return;
            }
            if self.handle_events().is_err() {
                self.release();
            }
        }
    }

    fn hold(&mut self, output: &str) -> Result<(), String> {
        if self.surface.is_some() {
            return Ok(());
        }
        self.globals.roundtrip()?;
        let output = self
            .globals
            .state
            .by_name(output)
            .cloned()
            .ok_or_else(|| format!("no output {output}"))?;
        let qh = self.globals.handle();
        let surface = self.compositor.create_surface(&qh, ());
        let region = self.compositor.create_region(&qh, ());
        surface.set_input_region(Some(&region));
        region.destroy();
        let layer = self.shell.get_layer_surface(
            &surface,
            Some(&output),
            zwlr_layer_shell_v1::Layer::Overlay,
            "telescreen-focus".into(),
            &qh,
            0_u64,
        );
        use zwlr_layer_surface_v1::{Anchor, KeyboardInteractivity};
        layer.set_size(1, 1);
        layer.set_anchor(Anchor::Top | Anchor::Left);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        surface.commit();
        self.surface = Some(Surface {
            surface,
            layer,
            buffer: None,
        });
        let deadline = Instant::now() + TIMEOUT;
        loop {
            self.globals.roundtrip()?;
            self.handle_events()?;
            match &self.surface {
                Some(surface) if surface.buffer.is_some() => {
                    // Wait for the compositor to process the buffer commit (UI-7).
                    self.globals.roundtrip()?;
                    self.handle_events()?;
                    return self
                        .surface
                        .as_ref()
                        .map(|_| ())
                        .ok_or_else(|| "the compositor closed the focus holder".into());
                }
                None => return Err("the compositor closed the focus holder".into()),
                _ => {}
            }
            if Instant::now() >= deadline {
                return Err("the compositor did not configure the focus holder in time".into());
            }
            thread::sleep(Duration::from_millis(10));
        }
    }

    fn release(&mut self) {
        if let Some(surface) = self.surface.take() {
            surface.layer.destroy();
            surface.surface.destroy();
            if let Some(buffer) = surface.buffer {
                buffer.destroy();
            }
            let _ = self.globals.connection.flush();
        }
    }

    /// Attaches the transparent buffer after the first configure (UI-7).
    fn handle_events(&mut self) -> Result<(), String> {
        if !std::mem::take(&mut self.globals.state.closed).is_empty() {
            self.release();
        }
        let configured = !std::mem::take(&mut self.globals.state.configures).is_empty();
        let Some(surface) = self.surface.as_mut() else {
            return Ok(());
        };
        if !configured || surface.buffer.is_some() {
            return Ok(());
        }
        let file = shared_memory()?;
        file.write_all_at(&[0; 4], 0)
            .map_err(|error| error.to_string())?;
        let qh = self.globals.handle();
        let pool = self.shm.create_pool(file.as_fd(), 4, &qh, ());
        let buffer = pool.create_buffer(0, 1, 1, 4, wl_shm::Format::Argb8888, &qh, ());
        pool.destroy();
        surface.surface.attach(Some(&buffer), 0, 0);
        surface.surface.damage_buffer(0, 0, 1, 1);
        surface.surface.commit();
        surface.buffer = Some(buffer);
        self.globals
            .connection
            .flush()
            .map_err(|error| error.to_string())
    }
}

/// Makes an anonymous shared memory file for one transparent pixel (UI-7).
fn shared_memory() -> Result<File, String> {
    // SAFETY: memfd_create gets a valid C string and returns a new owned fd.
    let fd = unsafe { libc::memfd_create(c"telescreen-focus".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // SAFETY: fd is a new, valid file descriptor that nothing else owns.
    let file = unsafe { File::from_raw_fd(fd) };
    file.set_len(4).map_err(|error| error.to_string())?;
    Ok(file)
}
