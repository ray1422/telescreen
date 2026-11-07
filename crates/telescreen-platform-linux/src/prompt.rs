//! The confirmation prompt as a layer-shell overlay on the input-target output
//! (UI-5, FLOW-7). It runs on its own thread with its own Wayland connection.
//! It takes no input while the user confirms on the source. When the
//! acknowledgement is on this destination, it takes clicks and the keyboard, and
//! a left click or Return acknowledges (VER-4). The warning of FLOW-26 takes
//! clicks and the keyboard too: Escape counts for the safety exit (IN-8), and
//! clicks and other keys do nothing.

use std::collections::HashMap;
use std::fs::File;
use std::os::fd::{AsFd, FromRawFd};
use std::os::unix::fs::FileExt;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread;
use std::time::Duration;

use telescreen_core::platform::DisplayInfo;
use telescreen_core::pointer::Normalized;
use telescreen_view::{PromptId, PromptPresenter, PromptPress, PromptScreen};
use wayland_client::protocol::{
    wl_buffer, wl_compositor, wl_keyboard, wl_pointer, wl_region, wl_seat, wl_shm, wl_shm_pool,
    wl_subcompositor, wl_subsurface, wl_surface,
};
use wayland_client::{Connection, Dispatch, QueueHandle, WEnum};
use wayland_protocols_wlr::layer_shell::v1::client::{zwlr_layer_shell_v1, zwlr_layer_surface_v1};

use crate::wl::{Configure, Globals, State, ignore_events};

ignore_events!(
    wl_compositor::WlCompositor,
    wl_region::WlRegion,
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
    wl_buffer::WlBuffer,
    wl_surface::WlSurface,
    wl_subcompositor::WlSubcompositor,
    wl_subsurface::WlSubsurface,
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

/// The user data of the seat of the prompt connection.
struct PromptSeat;

impl Dispatch<wl_seat::WlSeat, PromptSeat> for State {
    fn event(
        state: &mut Self,
        seat: &wl_seat::WlSeat,
        event: wl_seat::Event,
        _: &PromptSeat,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(capabilities),
        } = event
        else {
            return;
        };
        let input = &mut state.seat;
        if capabilities.contains(wl_seat::Capability::Pointer) && input.pointer.is_none() {
            input.pointer = Some(seat.get_pointer(qh, ()));
        }
        if capabilities.contains(wl_seat::Capability::Keyboard) && input.keyboard.is_none() {
            input.keyboard = Some(seat.get_keyboard(qh, ()));
        }
    }
}

/// The evdev codes of the acknowledgement: the left button, Return, keypad Enter.
const BTN_LEFT: u32 = 0x110;
const KEY_ENTER: u32 = 28;
const KEY_KPENTER: u32 = 96;
/// The evdev code of Escape, for the safety exit on the warning (IN-8).
/// wl_keyboard before version 10 sends no repeated keys, so each key down
/// is a press.
const KEY_ESC: u32 = 1;

impl Dispatch<wl_pointer::WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_pointer::WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let input = &mut state.seat;
        match event {
            wl_pointer::Event::Enter { surface, .. } => input.pointer_on = Some(surface),
            wl_pointer::Event::Leave { .. } => input.pointer_on = None,
            wl_pointer::Event::Button {
                button: BTN_LEFT,
                state: WEnum::Value(wl_pointer::ButtonState::Pressed),
                ..
            } => input.presses.extend(
                input
                    .pointer_on
                    .clone()
                    .map(|s| (s, PromptPress::Acknowledge)),
            ),
            _ => {}
        }
    }
}

impl Dispatch<wl_keyboard::WlKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        _: &wl_keyboard::WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let input = &mut state.seat;
        match event {
            wl_keyboard::Event::Enter { surface, .. } => input.keyboard_on = Some(surface),
            wl_keyboard::Event::Leave { .. } => input.keyboard_on = None,
            wl_keyboard::Event::Key {
                key,
                state: WEnum::Value(wl_keyboard::KeyState::Pressed),
                ..
            } => {
                let press = match key {
                    KEY_ENTER | KEY_KPENTER => PromptPress::Acknowledge,
                    KEY_ESC => PromptPress::Escape,
                    _ => return,
                };
                input
                    .presses
                    .extend(input.keyboard_on.clone().map(|s| (s, press)));
            }
            _ => {}
        }
    }
}

enum Command {
    Show {
        prompt: PromptId,
        output: String,
        screen: PromptScreen,
    },
    Hide {
        prompt: PromptId,
    },
    Cursor {
        prompt: PromptId,
        at: Option<Normalized>,
    },
}

/// The layer-shell prompt. Each call waits for the prompt thread.
#[derive(Debug)]
pub struct LayerPrompt {
    commands: Sender<Command>,
    results: Receiver<Result<(), String>>,
}

impl LayerPrompt {
    /// Starts the prompt thread. It calls `on_press` with the prompt of a left
    /// click or Return on a prompt that waits for the acknowledgement here
    /// (VER-4), and of an Escape on a prompt that takes input (IN-8).
    pub fn start(
        on_press: impl Fn(PromptId, PromptPress) + Send + 'static,
    ) -> Result<Self, String> {
        let (commands, command_rx) = channel();
        let (result_tx, results) = channel();
        let (ready_tx, ready_rx) = channel();
        thread::Builder::new()
            .name("telescreen-prompt".into())
            .spawn(move || match Overlay::connect(Box::new(on_press)) {
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
        prompt: PromptId,
        display: &DisplayInfo,
        screen: &PromptScreen,
    ) -> Result<(), String> {
        let output = display.token.0.clone();
        self.commands
            .send(Command::Show {
                prompt,
                output,
                screen: screen.clone(),
            })
            .map_err(|_| "the prompt thread stopped".to_string())?;
        self.results
            .recv_timeout(Duration::from_secs(3))
            .map_err(|_| "the prompt did not show in time".to_string())?
    }

    fn hide(&mut self, prompt: PromptId) {
        let _ = self.commands.send(Command::Hide { prompt });
    }

    /// A blank layer surface is transparent and takes no input.
    fn keeps_blank(&self) -> bool {
        true
    }

    fn cursor(&mut self, prompt: PromptId, at: Option<Normalized>) {
        let _ = self.commands.send(Command::Cursor { prompt, at });
    }
}

/// The cursor of the source on the warning (FLOW-26): a subsurface with an empty
/// input region, so that it takes no pointer input.
struct Cursor {
    surface: wl_surface::WlSurface,
    subsurface: wl_subsurface::WlSubsurface,
}

struct Surface {
    surface: wl_surface::WlSurface,
    layer: zwlr_layer_surface_v1::ZwlrLayerSurfaceV1,
    buffer: Option<wl_buffer::WlBuffer>,
    screen: PromptScreen,
    output: String,
    /// The size of the last configure.
    size: Option<(u32, u32)>,
    cursor: Option<Cursor>,
    /// The surface takes clicks and the keyboard (VER-4, FLOW-26).
    takes_input: bool,
}

struct Overlay {
    globals: Globals,
    compositor: wl_compositor::WlCompositor,
    shm: wl_shm::WlShm,
    shell: zwlr_layer_shell_v1::ZwlrLayerShellV1,
    /// `None`: the compositor has no subsurfaces, and the warning shows no cursor.
    subcompositor: Option<wl_subcompositor::WlSubcompositor>,
    /// The cursor image and its size, made at the first use.
    cursor_image: Option<(wl_buffer::WlBuffer, i32, i32)>,
    surfaces: HashMap<u64, Surface>,
    on_press: Box<dyn Fn(PromptId, PromptPress) + Send>,
}

/// The press to report for a surface that takes input. Only a prompt that
/// waits for the acknowledgement here takes it: a click or Return on the
/// warning does nothing (FLOW-26).
fn reported(screen: &PromptScreen, press: PromptPress) -> bool {
    match press {
        PromptPress::Acknowledge => screen.ack_here,
        PromptPress::Escape => true,
    }
}

impl Overlay {
    fn connect(on_press: Box<dyn Fn(PromptId, PromptPress) + Send>) -> Result<Self, String> {
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
        let subcompositor = globals.list.bind(&qh, 1..=1, ()).ok();
        // With no seat, the prompt takes no input.
        let _: Option<wl_seat::WlSeat> = globals.list.bind(&qh, 1..=7, PromptSeat).ok();
        Ok(Self {
            globals,
            compositor,
            shm,
            shell,
            subcompositor,
            cursor_image: None,
            surfaces: HashMap::new(),
            on_press,
        })
    }

    fn run(mut self, commands: Receiver<Command>, results: Sender<Result<(), String>>) {
        loop {
            match commands.recv_timeout(Duration::from_millis(250)) {
                Ok(command) => {
                    self.apply(command, &results);
                    // The cursor moves often: apply all waiting commands before
                    // the roundtrip.
                    while let Ok(command) = commands.try_recv() {
                        self.apply(command, &results);
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => return,
            }
            if self.globals.roundtrip().is_err() {
                return;
            }
            self.handle_events();
        }
    }

    fn apply(&mut self, command: Command, results: &Sender<Result<(), String>>) {
        match command {
            Command::Show {
                prompt,
                output,
                screen,
            } => {
                let result = self.show(prompt.0, &output, screen);
                let _ = results.send(result);
            }
            Command::Hide { prompt } => self.hide(prompt.0),
            Command::Cursor { prompt, at } => {
                if let Err(e) = self.cursor(prompt.0, at) {
                    eprintln!("telescreen: prompt cursor: {e}");
                }
            }
        }
    }

    fn show(&mut self, id: u64, output: &str, screen: PromptScreen) -> Result<(), String> {
        // FLOW-26: a new surface has an open animation on KWin, so change the
        // content of a shown surface in place.
        if let Some(surface) = self.surfaces.get_mut(&id)
            && surface.output == output
            && let Some((width, height)) = surface.size
        {
            let input = screen.takes_input();
            surface.screen = screen;
            self.set_input(id, input);
            return self.draw(Configure {
                surface: id,
                width,
                height,
            });
        }
        self.hide(id);
        let output_name = output.to_owned();
        let output = self.globals.output(output)?;
        let qh = self.globals.handle();
        let surface = self.compositor.create_surface(&qh, ());
        let layer = self.shell.get_layer_surface(
            &surface,
            Some(&output),
            zwlr_layer_shell_v1::Layer::Overlay,
            "telescreen-prompt".into(),
            &qh,
            id,
        );
        use zwlr_layer_surface_v1::Anchor;
        layer.set_anchor(Anchor::Top | Anchor::Bottom | Anchor::Left | Anchor::Right);
        layer.set_exclusive_zone(-1);
        let input = screen.takes_input();
        self.surfaces.insert(
            id,
            Surface {
                surface,
                layer,
                buffer: None,
                screen,
                output: output_name,
                size: None,
                cursor: None,
                // `set_input` sets the state of a new surface.
                takes_input: !input,
            },
        );
        self.set_input(id, input);
        if let Some(surface) = self.surfaces.get(&id) {
            surface.surface.commit();
        }
        self.globals.roundtrip()?;
        self.handle_events();
        match self.surfaces.get(&id) {
            Some(s) if s.buffer.is_some() => Ok(()),
            _ => Err("the compositor did not configure the prompt".into()),
        }
    }

    /// VER-4, FLOW-26: with input, the surface takes the pointer on all of its
    /// area and takes the keyboard. Else pointer events go to the windows below, and the
    /// surface never has the keyboard focus. The next commit applies it.
    fn set_input(&mut self, id: u64, input: bool) {
        use zwlr_layer_surface_v1::KeyboardInteractivity;
        let Some(surface) = self.surfaces.get_mut(&id) else {
            return;
        };
        if surface.takes_input == input {
            return;
        }
        surface.takes_input = input;
        if input {
            surface.surface.set_input_region(None);
            surface
                .layer
                .set_keyboard_interactivity(KeyboardInteractivity::Exclusive);
        } else {
            let region = self.compositor.create_region(&self.globals.handle(), ());
            surface.surface.set_input_region(Some(&region));
            region.destroy();
            surface
                .layer
                .set_keyboard_interactivity(KeyboardInteractivity::None);
        }
    }

    fn hide(&mut self, id: u64) {
        if let Some(surface) = self.surfaces.remove(&id) {
            if let Some(cursor) = surface.cursor {
                cursor.subsurface.destroy();
                cursor.surface.destroy();
            }
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
        for (pressed, press) in std::mem::take(&mut self.globals.state.seat.presses) {
            let prompt = self
                .surfaces
                .iter()
                .find(|(_, s)| s.takes_input && s.surface == pressed && reported(&s.screen, press))
                .map(|(id, _)| PromptId(*id));
            if let Some(prompt) = prompt {
                (self.on_press)(prompt, press);
            }
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
        surface.size = Some((width, height));
        let pixels = if surface.screen.blank {
            vec![0; (width * height) as usize]
        } else {
            crate::text::draw(
                width,
                height,
                &surface.screen.title().unwrap_or_default(),
                surface.screen.text.as_deref(),
                if surface.screen.portal_hidden {
                    crate::text::WARNING_BACKGROUND
                } else {
                    crate::text::BACKGROUND
                },
            )
        };
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

impl Overlay {
    /// Moves the cursor of the source on a shown prompt, or removes it (FLOW-26).
    fn cursor(&mut self, id: u64, at: Option<Normalized>) -> Result<(), String> {
        let Some(subcompositor) = self.subcompositor.clone() else {
            return Ok(());
        };
        let image = match (&self.cursor_image, at) {
            (_, None) => None,
            (Some(image), Some(_)) => Some(image.clone()),
            (None, Some(_)) => {
                let image = self.cursor_buffer()?;
                self.cursor_image = Some(image.clone());
                Some(image)
            }
        };
        let qh = self.globals.handle();
        let Some(surface) = self.surfaces.get_mut(&id) else {
            return Ok(());
        };
        let Some((width, height)) = surface.size else {
            return Ok(());
        };
        let (Some(at), Some((buffer, image_width, image_height))) = (at, image) else {
            if let Some(cursor) = &surface.cursor {
                cursor.surface.attach(None, 0, 0);
                cursor.surface.commit();
                surface.surface.commit();
            }
            return self.globals.connection.flush().map_err(|e| e.to_string());
        };
        let cursor = surface.cursor.get_or_insert_with(|| {
            let child = self.compositor.create_surface(&qh, ());
            let region = self.compositor.create_region(&qh, ());
            child.set_input_region(Some(&region));
            region.destroy();
            let subsurface = subcompositor.get_subsurface(&child, &surface.surface, &qh, ());
            subsurface.set_desync();
            Cursor {
                surface: child,
                subsurface,
            }
        });
        // The hot spot of the image is its top-left pixel.
        let x = (f64::from(at.x.clamp(0.0, 1.0)) * f64::from(width.saturating_sub(1))).round();
        let y = (f64::from(at.y.clamp(0.0, 1.0)) * f64::from(height.saturating_sub(1))).round();
        cursor.subsurface.set_position(x as i32, y as i32);
        cursor.surface.attach(Some(&buffer), 0, 0);
        cursor
            .surface
            .damage_buffer(0, 0, image_width, image_height);
        cursor.surface.commit();
        // The position applies at the next commit of the parent.
        surface.surface.commit();
        self.globals.connection.flush().map_err(|e| e.to_string())
    }

    fn cursor_buffer(&self) -> Result<(wl_buffer::WlBuffer, i32, i32), String> {
        let (width, height, pixels) = cursor_pixels();
        let bytes: Vec<u8> = pixels.iter().flat_map(|p| p.to_le_bytes()).collect();
        let file = shared_memory(bytes.len())?;
        file.write_all_at(&bytes, 0).map_err(|e| e.to_string())?;
        let size = i32::try_from(bytes.len()).map_err(|e| e.to_string())?;
        let width = i32::try_from(width).map_err(|e| e.to_string())?;
        let height = i32::try_from(height).map_err(|e| e.to_string())?;
        let qh = self.globals.handle();
        let pool = self.shm.create_pool(file.as_fd(), size, &qh, ());
        let buffer = pool.create_buffer(
            0,
            width,
            height,
            width * 4,
            wl_shm::Format::Argb8888,
            &qh,
            (),
        );
        pool.destroy();
        Ok((buffer, width, height))
    }
}

/// An arrow: `X` is the black edge, `o` is the white fill.
const ARROW: [&str; 17] = [
    "X",
    "XX",
    "XoX",
    "XooX",
    "XoooX",
    "XooooX",
    "XoooooX",
    "XooooooX",
    "XoooooooX",
    "XooooooooX",
    "XoooooXXXXX",
    "XooXooX",
    "XoX XooX",
    "XX  XooX",
    "X    XooX",
    "     XooX",
    "      XX",
];
const ARROW_SCALE: usize = 2;

/// The cursor image: its width, its height, and premultiplied ARGB pixels.
fn cursor_pixels() -> (usize, usize, Vec<u32>) {
    let columns = ARROW.iter().map(|row| row.len()).max().unwrap_or(0);
    let (width, height) = (columns * ARROW_SCALE, ARROW.len() * ARROW_SCALE);
    let mut pixels = vec![0; width * height];
    for (row, line) in ARROW.iter().enumerate() {
        for (column, cell) in line.bytes().enumerate() {
            let color = match cell {
                b'X' => 0xFF00_0000,
                b'o' => 0xFFFF_FFFF,
                _ => continue,
            };
            for dy in 0..ARROW_SCALE {
                let start = (row * ARROW_SCALE + dy) * width + column * ARROW_SCALE;
                pixels[start..start + ARROW_SCALE].fill(color);
            }
        }
    }
    (width, height, pixels)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow26_the_warning_reports_only_escape() {
        let warning = PromptScreen::portal_hidden("MacBook".into());
        assert!(warning.takes_input());
        assert!(reported(&warning, PromptPress::Escape));
        assert!(!reported(&warning, PromptPress::Acknowledge));
        let blank = PromptScreen::portal_shown("MacBook".into());
        assert!(!blank.takes_input());
    }

    #[test]
    fn flow26_cursor_image_has_its_hot_spot_at_the_top_left() {
        let (width, height, pixels) = cursor_pixels();
        assert_eq!((width, height), (22, 34));
        assert_eq!(pixels[0], 0xFF00_0000);
        assert_eq!(pixels[width - 1], 0);
        assert!(pixels.contains(&0xFFFF_FFFF));
    }
}
