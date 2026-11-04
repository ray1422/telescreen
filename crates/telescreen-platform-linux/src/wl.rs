//! A Wayland connection with the globals that this crate uses, and the output data.

use wayland_client::globals::{GlobalList, GlobalListContents, registry_queue_init};
use wayland_client::protocol::{wl_output, wl_registry, wl_seat};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle};
use wayland_protocols::xdg::xdg_output::zv1::client::{zxdg_output_manager_v1, zxdg_output_v1};

/// The data of one output.
#[derive(Debug, Clone, Default)]
pub struct OutputData {
    pub name: Option<String>,
    pub description: Option<String>,
    pub make: Option<String>,
    pub model: Option<String>,
    pub mode: Option<(i32, i32)>,
    pub logical_position: Option<(i32, i32)>,
    pub logical_size: Option<(i32, i32)>,
}

/// A layer surface configure event: the user data of the surface and the size.
#[derive(Debug, Clone, Copy)]
pub struct Configure {
    pub surface: u64,
    pub width: u32,
    pub height: u32,
}

/// One bound output: its registry name, its proxies, and its data.
#[derive(Debug)]
pub struct Output {
    /// The name of the global in the registry. A `global_remove` event names it.
    global: u32,
    pub proxy: wl_output::WlOutput,
    xdg: Option<zxdg_output_v1::ZxdgOutputV1>,
    pub data: OutputData,
}

/// The dispatch state of a connection: the outputs, and the events that the users
/// of the connection read after a roundtrip.
///
/// The output list follows the registry: an output that the compositor adds later
/// is bound, and a removed output is released. Thus a long connection sees a
/// display that the user connects again.
#[derive(Debug, Default)]
pub struct State {
    pub outputs: Vec<Output>,
    pub configures: Vec<Configure>,
    pub closed: Vec<u64>,
    xdg_manager: Option<zxdg_output_manager_v1::ZxdgOutputManagerV1>,
}

impl State {
    fn data(&mut self, output: &wl_output::WlOutput) -> Option<&mut OutputData> {
        self.outputs
            .iter_mut()
            .find(|o| &o.proxy == output)
            .map(|o| &mut o.data)
    }

    pub fn by_name(&self, name: &str) -> Option<&wl_output::WlOutput> {
        self.outputs
            .iter()
            .find(|o| o.data.name.as_deref() == Some(name))
            .map(|o| &o.proxy)
    }

    fn bind_output(
        &mut self,
        registry: &wl_registry::WlRegistry,
        global: u32,
        version: u32,
        qh: &QueueHandle<Self>,
    ) {
        let proxy: wl_output::WlOutput = registry.bind(global, version.min(4), qh, ());
        let xdg = self
            .xdg_manager
            .as_ref()
            .map(|manager| manager.get_xdg_output(&proxy, qh, proxy.clone()));
        self.outputs.push(Output {
            global,
            proxy,
            xdg,
            data: OutputData::default(),
        });
    }

    fn remove_output(&mut self, global: u32) {
        let Some(index) = self.outputs.iter().position(|o| o.global == global) else {
            return;
        };
        let output = self.outputs.remove(index);
        if let Some(xdg) = output.xdg {
            xdg.destroy();
        }
        if output.proxy.version() >= 3 {
            output.proxy.release();
        }
    }
}

/// A connection, its queue, and the globals.
pub struct Globals {
    pub connection: Connection,
    pub queue: EventQueue<State>,
    pub state: State,
    pub list: GlobalList,
}

impl std::fmt::Debug for Globals {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Globals")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

impl Globals {
    /// Connects, binds all outputs (with xdg-output), and waits for their data.
    pub fn connect() -> Result<Self, String> {
        let connection = Connection::connect_to_env().map_err(|e| format!("Wayland: {e}"))?;
        let (list, mut queue) = registry_queue_init::<State>(&connection)
            .map_err(|e| format!("Wayland registry: {e}"))?;
        let qh = queue.handle();
        let mut state = State {
            xdg_manager: list.bind(&qh, 2..=3, ()).ok(),
            ..State::default()
        };
        for global in list.contents().clone_list() {
            if global.interface == wl_output::WlOutput::interface().name {
                state.bind_output(list.registry(), global.name, global.version, &qh);
            }
        }
        queue
            .roundtrip(&mut state)
            .map_err(|e| format!("Wayland: {e}"))?;
        queue
            .roundtrip(&mut state)
            .map_err(|e| format!("Wayland: {e}"))?;
        Ok(Self {
            connection,
            queue,
            state,
            list,
        })
    }

    pub fn has(&self, interface: &str) -> bool {
        self.list
            .contents()
            .with_list(|l| l.iter().any(|g| g.interface == interface))
    }

    pub fn seat(&self) -> Option<wl_seat::WlSeat> {
        self.list.bind(&self.queue.handle(), 1..=7, ()).ok()
    }

    pub fn handle(&self) -> QueueHandle<State> {
        self.queue.handle()
    }

    /// The output with this name, after the pending registry events. A new output
    /// gets its name one roundtrip after it is bound, so a miss gets one more.
    pub fn output(&mut self, name: &str) -> Result<wl_output::WlOutput, String> {
        for _ in 0..2 {
            self.roundtrip()?;
            if let Some(output) = self.state.by_name(name) {
                return Ok(output.clone());
            }
        }
        Err(format!("no output {name}"))
    }

    pub fn roundtrip(&mut self) -> Result<(), String> {
        self.queue
            .roundtrip(&mut self.state)
            .map(|_| ())
            .map_err(|e| format!("Wayland: {e}"))
    }
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match event {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == wl_output::WlOutput::interface().name => {
                state.bind_output(registry, name, version, qh);
            }
            wl_registry::Event::GlobalRemove { name } => state.remove_output(name),
            _ => {}
        }
    }
}

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        state: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(data) = state.data(output) else {
            return;
        };
        match event {
            wl_output::Event::Name { name } => data.name = Some(name),
            wl_output::Event::Description { description } => data.description = Some(description),
            wl_output::Event::Geometry { make, model, .. } => {
                data.make = Some(make);
                data.model = Some(model);
            }
            wl_output::Event::Mode {
                flags,
                width,
                height,
                ..
            } => {
                let current = matches!(flags, wayland_client::WEnum::Value(f) if f.contains(wl_output::Mode::Current));
                if current {
                    data.mode = Some((width, height));
                }
            }
            _ => {}
        }
    }
}

impl Dispatch<zxdg_output_v1::ZxdgOutputV1, wl_output::WlOutput> for State {
    fn event(
        state: &mut Self,
        _: &zxdg_output_v1::ZxdgOutputV1,
        event: zxdg_output_v1::Event,
        output: &wl_output::WlOutput,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let Some(data) = state.data(output) else {
            return;
        };
        match event {
            zxdg_output_v1::Event::LogicalPosition { x, y } => data.logical_position = Some((x, y)),
            zxdg_output_v1::Event::LogicalSize { width, height } => {
                data.logical_size = Some((width, height))
            }
            zxdg_output_v1::Event::Name { name } if data.name.is_none() => data.name = Some(name),
            _ => {}
        }
    }
}

/// A Dispatch implementation that ignores all events.
macro_rules! ignore_events {
    ($($t:ty),* $(,)?) => {$(
        impl wayland_client::Dispatch<$t, ()> for crate::wl::State {
            fn event(
                _: &mut Self,
                _: &$t,
                _: <$t as wayland_client::Proxy>::Event,
                _: &(),
                _: &wayland_client::Connection,
                _: &wayland_client::QueueHandle<Self>,
            ) {
            }
        }
    )*};
}
pub(crate) use ignore_events;

ignore_events!(zxdg_output_manager_v1::ZxdgOutputManagerV1, wl_seat::WlSeat);
