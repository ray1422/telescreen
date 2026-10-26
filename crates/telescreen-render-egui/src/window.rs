//! One egui context and one wgpu surface for one winit window.

use std::{num::NonZeroU32, sync::Arc};

use telescreen_view::{ControlScreen, Intent, PortalScreen, PromptScreen};
use winit::event::WindowEvent;
use winit::window::Window;

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("graphics setup: {0}")]
    Setup(String),
}

/// Draws view models on one window. Make one for each window.
pub struct WindowRenderer {
    window: Arc<Window>,
    context: egui::Context,
    state: egui_winit::State,
    painter: egui_wgpu::winit::Painter,
    textures_delta: egui::TexturesDelta,
}

impl std::fmt::Debug for WindowRenderer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WindowRenderer")
            .field("window", &self.window.id())
            .finish_non_exhaustive()
    }
}

/// The wgpu instance, adapter, device, and queue that all windows share. The
/// first window makes them. With one device for each window, NVIDIA on Wayland
/// crashed when one window closed (hardware test 2026-10-27).
#[derive(Default)]
pub struct SharedGpu(Option<egui_wgpu::WgpuSetupExisting>);

impl std::fmt::Debug for SharedGpu {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SharedGpu")
            .field("ready", &self.0.is_some())
            .finish()
    }
}

impl WindowRenderer {
    /// Makes the egui context, the egui-winit state, and the egui-wgpu painter for
    /// the window. Blocks until the GPU device is ready. The first window makes
    /// the shared GPU objects in `gpu`; the other windows use them.
    pub fn new(window: Arc<Window>, gpu: &mut SharedGpu) -> Result<Self, RenderError> {
        let context = egui::Context::default();
        let wgpu_setup = match &gpu.0 {
            Some(existing) => egui_wgpu::WgpuSetup::Existing(existing.clone()),
            None => egui_wgpu::WgpuSetup::from_display_handle(Arc::clone(&window)),
        };
        let config = egui_wgpu::WgpuConfiguration {
            wgpu_setup,
            ..Default::default()
        };
        let painter_context = context.clone();
        let painter_window = Arc::clone(&window);
        let painter = pollster::block_on(async move {
            let mut painter = egui_wgpu::winit::Painter::new(
                painter_context,
                config,
                false,
                egui_wgpu::RendererOptions::default(),
            )
            .await;
            painter
                .set_window(egui::ViewportId::ROOT, Some(painter_window))
                .await?;
            Ok::<_, egui_wgpu::WgpuError>(painter)
        })
        .map_err(|error| RenderError::Setup(error.to_string()))?;
        if gpu.0.is_none()
            && let Some(render_state) = painter.render_state()
        {
            gpu.0 = Some(egui_wgpu::WgpuSetupExisting {
                instance: render_state.instance,
                adapter: render_state.adapter,
                device: render_state.device,
                queue: render_state.queue,
            });
        }
        let state = egui_winit::State::new(
            context.clone(),
            egui::ViewportId::ROOT,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            window.theme(),
            painter.max_texture_side(),
        );
        Ok(Self {
            window,
            context,
            state,
            painter,
            textures_delta: egui::TexturesDelta::default(),
        })
    }

    pub fn window(&self) -> &Arc<Window> {
        &self.window
    }

    /// Gives a window event to egui. Handles `Resized` and `ScaleFactorChanged` for
    /// the surface. Returns true if the window must be drawn again.
    pub fn on_window_event(&mut self, event: &WindowEvent) -> bool {
        let response = self.state.on_window_event(&self.window, event);
        let size = match event {
            WindowEvent::Resized(size) => Some(*size),
            WindowEvent::ScaleFactorChanged { .. } => Some(self.window.inner_size()),
            _ => None,
        };
        if let Some(size) = size {
            if let (Some(width), Some(height)) =
                (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
            {
                self.painter
                    .on_window_resized(egui::ViewportId::ROOT, width, height);
            }
            return true;
        }
        response.repaint
    }

    /// Draws the control window and returns the intents of this frame.
    pub fn draw_control(&mut self, screen: &ControlScreen) -> Vec<Intent> {
        self.draw(|ui| super::screens::control(ui, screen))
    }

    /// Draws the portal window and returns the intents of this frame.
    pub fn draw_portal(&mut self, screen: &PortalScreen) -> Vec<Intent> {
        self.draw(|ui| super::screens::portal(ui, screen))
    }

    /// Draws the confirmation prompt (macOS prompt window).
    pub fn draw_prompt(&mut self, screen: &PromptScreen) {
        self.draw(|ui| {
            super::screens::prompt(ui, screen);
            Vec::new()
        });
    }

    fn draw(&mut self, mut contents: impl FnMut(&mut egui::Ui) -> Vec<Intent>) -> Vec<Intent> {
        let input = self.state.take_egui_input(&self.window);
        let mut intents = Vec::new();
        let output = self.context.run_ui(input, |ui| {
            intents = egui::CentralPanel::default().show(ui, &mut contents).inner;
        });
        self.state
            .handle_platform_output(&self.window, output.platform_output);
        let primitives = self
            .context
            .tessellate(output.shapes, output.pixels_per_point);
        // The painter retains unapplied changes after a skipped frame. Keep them
        // for the next paint, including when a window has no drawable area.
        self.textures_delta.append(output.textures_delta);
        let size = self.window.inner_size();
        if size.width > 0 && size.height > 0 {
            self.painter.paint_and_update_textures(
                egui::ViewportId::ROOT,
                output.pixels_per_point,
                [0.0, 0.0, 0.0, 1.0],
                &primitives,
                &mut self.textures_delta,
                Vec::new(),
                &self.window,
            );
        }
        if output
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .is_some_and(|viewport| viewport.repaint_delay.is_zero())
        {
            self.window.request_redraw();
        }
        intents
    }
}

impl Drop for WindowRenderer {
    fn drop(&mut self) {
        // Pending texture changes no longer need applying when the painter goes away.
        self.textures_delta.clear();
    }
}
