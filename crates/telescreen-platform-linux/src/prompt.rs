//! The confirmation prompt as a layer-shell overlay on the input-target output
//! (UI-5, FLOW-7). It runs on its own thread with its own Wayland connection.
//! It does not take keyboard or pointer input: the user confirms on the source.

use std::collections::HashMap;
use std::fs::File;
use std::os::fd::{AsFd, FromRawFd};
use std::os::unix::fs::FileExt;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::Duration;

use telescreen_core::ids::SessionId;
use telescreen_core::platform::DisplayInfo;
use telescreen_view::{PromptPresenter, PromptScreen};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_region, wl_shm, wl_shm_pool, wl_surface,
};
use wayland_client::{Connection, Dispatch, QueueHandle};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::wl::{Configure, Globals, State, ignore_events};

ignore_events!(
    wl_compositor::WlCompositor,
    wl_region::WlRegion,
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
    wl_buffer::WlBuffer,
    wl_surface::WlSurface,
    zwlr_layer_shell_v1::ZwlrLayerShellV1
);

impl Dispatch<zwlr_layer_surface_v1::ZwlrLayerSurfaceV1, u64> for State {
    fn event(
        state: &mut Self,
        layer: &zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
        event: zwlr_layer_surface_v1::Event,
        surface: &u64,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_layer_surface_v1::Event::Configure {
                serial,
                width,
                height,
            } => {
                layer.ack_configure(serial);
                state.configures.push(Configure {
                    surface: *surface,
                    width,
                    height,
                });
            }
            zwlr_layer_surface_v1::Event::Closed => state.closed.push(*surface),
            _ => {}
        }
    }
}

enum Command {
    Show {
        session: SessionId,
        output: String,
        screen: PromptScreen,
    },
    Hide {
        session: SessionId,
    },
}

/// The layer-shell prompt. Each call waits for the prompt thread.
#[derive(Debug)]
pub struct LayerPrompt {
    commands: Sender<Command>,
    results: Receiver<Result<(), String>>,
}

impl LayerPrompt {
    pub fn start() -> Result<Self, String> {
        let (commands, command_rx) = channel();
        let (result_tx, results) = channel();
        let (ready_tx, ready_rx) = channel();
        thread::Builder::new()
            .name("telescreen-prompt".into())
            .spawn(move || match Overlay::connect() {
                Ok(overlay) => {
                    let _ = ready_tx.send(Ok(()));
                    overlay.run(command_rx, result_tx);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            })
            .map_err(|e| e.to_string())?;
        ready_rx.recv().map_err(|e| e.to_string())??;
        Ok(Self { commands, results })
    }
}

impl PromptPresenter for LayerPrompt {
    fn show(
        &mut self,
        session: SessionId,
        display: &DisplayInfo,
        screen: &PromptScreen,
    ) -> Result<(), String> {
        let output = display.token.0.clone();
        self.commands
            .send(Command::Show {
                session,
                output,
                screen: screen.clone(),
            })
            .map_err(|_| "the prompt thread stopped".to_string())?;
        self.results
            .recv_timeout(Duration::from_secs(3))
            .map_err(|_| "the prompt did not show in time".to_string())?
    }

    fn hide(&mut self, session: SessionId) {
        let _ = self.commands.send(Command::Hide { session });
    }
}

struct Surface {
    surface: wl_surface::WlSurface,
    layer: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    buffer: Option<wl_buffer::WlBuffer>,
    screen: PromptScreen,
}

struct Overlay {
    globals: Globals,
    compositor: wl_compositor::WlCompositor,
    shm: wl_shm::WlShm,
    shell: zwlr_layer_shell_v1::ZwlrLayerShellV1,
    surfaces: HashMap<u64, Surface>,
}

impl Overlay {
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
            surfaces: HashMap::new(),
        })
    }

    fn run(mut self, commands: Receiver<Command>, results: Sender<Result<(), String>>) {
        loop {
            match commands.recv_timeout(Duration::from_millis(250)) {
                Ok(Command::Show {
                    session,
                    output,
                    screen,
                }) => {
                    let result = self.show(session.0, &output, screen);
                    let _ = results.send(result);
                }
                Ok(Command::Hide { session }) => self.hide(session.0),
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            }
            if self.globals.roundtrip().is_err() {
                return;
            }
            self.handle_events();
        }
    }

    fn show(&mut self, id: u64, output: &str, screen: PromptScreen) -> Result<(), String> {
        self.hide(id);
        self.globals.roundtrip()?;
        let output = self
            .globals
            .state
            .by_name(output)
            .cloned()
            .ok_or(format!("no output {output}"))?;
        let qh = self.globals.handle();
        let surface = self.compositor.create_surface(&qh, ());
        // An empty input region: pointer events go to the windows below.
        let region = self.compositor.create_region(&qh, ());
        surface.set_input_region(Some(&region));
        region.destroy();
        let layer = self.shell.get_layer_surface(
            &surface,
            Some(&output),
            zwlr_layer_shell_v1::Layer::Overlay,
            "telescreen-prompt".into(),
            &qh,
            id,
        );
        use zwlr_layer_surface_v1::{Anchor, KeyboardInteractivity};
        layer.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
        layer.set_exclusive_zone(-1);
        layer.set_keyboard_interactivity(KeyboardInteractivity::None);
        surface.commit();
        self.surfaces.insert(
            id,
            Surface {
                surface,
                layer,
                buffer: None,
                screen,
            },
        );
        self.globals.roundtrip()?;
        self.handle_events();
        match self.surfaces.get(&id) {
            Some(s) if s.buffer.is_some() => Ok(()),
            _ => Err("the compositor did not configure the prompt".into()),
        }
    }

    fn hide(&mut self, id: u64) {
        if let Some(surface) = self.surfaces.remove(&id) {
            surface.layer.destroy();
            surface.surface.destroy();
            if let Some(buffer) = surface.buffer {
                buffer.destroy();
            }
            let _ = self.globals.connection.flush();
        }
    }

    fn handle_events(&mut self) {
        for id in std::mem::take(&mut self.globals.state.closed) {
            self.hide(id);
        }
        for configure in std::mem::take(&mut self.globals.state.configures) {
            if let Err(e) = self.draw(configure) {
                eprintln!("telescreen: prompt: {e}");
            }
        }
    }

    fn draw(&mut self, configure: Configure) -> Result<(), String> {
        let qh = self.globals.handle();
        let Some(surface) = self.surfaces.get_mut(&configure.surface) else {
            return Ok(());
        };
        let (width, height) = (configure.width.max(1), configure.height.max(1));
        let pixels = crate::text::prompt(
            width,
            height,
            &surface.screen.title(),
            surface.screen.hint.as_deref(),
        );
        let bytes: Vec<u8> = pixels.iter().flat_map(|p| p.to_le_bytes()).collect();
        let file = shared_memory(bytes.len())?;
        file.write_all_at(&bytes, 0).map_err(|e| e.to_string())?;
        let size = i32::try_from(bytes.len()).map_err(|e| e.to_string())?;
        let pool = self.shm.create_pool(file.as_fd(), size, &qh, ());
        let stride = i32::try_from(width * 4).map_err(|e| e.to_string())?;
        let buffer = pool.create_buffer(
            0,
            i32::try_from(width).map_err(|e| e.to_string())?,
            i32::try_from(height).map_err(|e| e.to_string())?,
            stride,
            wl_shm::Format::Argb8888,
            &qh,
            (),
        );
        pool.destroy();
        surface.surface.attach(Some(&buffer), 0, 0);
        surface.surface.damage_buffer(0, 0, i32::MAX, i32::MAX);
        surface.surface.commit();
        if let Some(old) = surface.buffer.replace(buffer) {
            old.destroy();
        }
        self.globals.connection.flush().map_err(|e| e.to_string())
    }
}

/// An anonymous shared memory file for a wl_shm pool.
fn shared_memory(size: usize) -> Result<File, String> {
    // SAFETY: memfd_create with a valid C string; the new fd is owned by the File.
    let fd = unsafe { libc::memfd_create(c"telescreen-prompt".as_ptr(), libc::MFD_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    // SAFETY: fd is a new, valid file descriptor that nothing else owns.
    let file = unsafe { File::from_raw_fd(fd) };
    file.set_len(size as u64).map_err(|e| e.to_string())?;
    Ok(file)
}
