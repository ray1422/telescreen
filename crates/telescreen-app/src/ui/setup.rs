//! The window shown before the local computer is known (UI-8).

use telescreen_render_egui::SharedGpu;
use telescreen_view::{SetupIntent, SetupScreen};
use winit::application::ApplicationHandler;
use winit::dpi::LogicalSize;
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::platform::run_on_demand::EventLoopExtRunOnDemand;
use winit::window::{Window, WindowId};

use super::{Surface, UserEvent, platform};

pub(super) fn run_setup<T>(
    event_loop: &mut EventLoop<UserEvent>,
    gpu: &mut SharedGpu,
    step: impl FnMut(SetupIntent) -> Result<Option<T>, SetupScreen>,
    screen: SetupScreen,
) -> Result<Option<T>, String> {
    let mut setup = Setup {
        screen,
        surface: None,
        result: Ok(None),
        gpu,
        step,
    };
    let run = event_loop.run_app_on_demand(&mut setup);
    // Release the window and its renderer before another handler uses the loop.
    // Hide it first: after the run ended, macOS kept the dropped window on the screen.
    if let Some(surface) = &setup.surface {
        surface.window.set_visible(false);
    }
    setup.surface = None;
    run.map_err(|e| e.to_string())?;
    setup.result
}

struct Setup<'a, T, F> {
    screen: SetupScreen,
    surface: Option<Surface>,
    result: Result<Option<T>, String>,
    gpu: &'a mut SharedGpu,
    step: F,
}

impl<T, F> Setup<'_, T, F>
where
    F: FnMut(SetupIntent) -> Result<Option<T>, SetupScreen>,
{
    fn apply(&mut self, event_loop: &ActiveEventLoop, intent: SetupIntent) {
        match (self.step)(intent) {
            Ok(Some(value)) => {
                self.result = Ok(Some(value));
                event_loop.exit();
            }
            Ok(None) => {}
            Err(screen) => {
                self.screen = screen;
                if let Some(surface) = &self.surface {
                    surface.window.request_redraw();
                }
            }
        }
    }
}

impl<T, F> ApplicationHandler<UserEvent> for Setup<'_, T, F>
where
    F: FnMut(SetupIntent) -> Result<Option<T>, SetupScreen>,
{
    #[cfg(feature = "debug-protocol")]
    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: UserEvent) {
        use crate::debug::{self, Request};
        if let UserEvent::Debug(request, reply) = event {
            let response = match request {
                Request::ChooseComputer { computer } => {
                    self.apply(event_loop, SetupIntent::Choose { computer });
                    debug::ok_response()
                }
                Request::Quit => {
                    event_loop.exit();
                    debug::ok_response()
                }
                _ => debug::error_response("the start window is open"),
            };
            let _ = reply.send(response);
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.surface.is_some() {
            return;
        }
        let attributes = platform::window_attributes(Window::default_attributes())
            .with_title("Telescreen")
            .with_inner_size(LogicalSize::new(560.0, 520.0));
        match event_loop.create_window(attributes) {
            Ok(window) => {
                let (surface, error) = Surface::new(window, self.gpu);
                self.surface = Some(surface);
                if let Some(error) = error {
                    self.result = Err(error);
                    event_loop.exit();
                } else if let Some(surface) = &self.surface {
                    surface.window.request_redraw();
                }
            }
            Err(e) => {
                self.result = Err(format!("cannot open the window: {e}"));
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        let Some(surface) = self.surface.as_mut().filter(|s| s.window.id() == id) else {
            return;
        };
        if let WindowEvent::RedrawRequested = event {
            let intents = match &mut surface.renderer {
                Some(renderer) => renderer.draw_setup(&self.screen),
                None => Vec::new(),
            };
            for intent in intents {
                self.apply(event_loop, intent);
            }
            return;
        }
        if let WindowEvent::CloseRequested = event {
            event_loop.exit();
            return;
        }
        surface.on_event(&event);
    }
}
