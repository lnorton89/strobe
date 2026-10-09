// ============================================================================
// Module:       strobe_app::ui::host
// Description:  Runs egui on the control window: input in, shapes out, and
//               the glow painter that draws them on the shared GL context.
//
// Dependencies: egui, egui-winit (input), egui_glow (Painter), strobe-gfx
// ============================================================================

//! The egui host.
//!
//! This is `egui_glow::EguiGlow` with one difference: creating the painter
//! returns an error instead of panicking. The release build aborts on
//! panic and has no console, so a painter that cannot compile its shaders
//! on some old driver must reach the startup-failure dialog, not vanish.
//!
//! ## Showing the scene inside egui
//!
//! The player's output is a GL texture on the same context, so egui can
//! draw it directly as a *native* texture ([`UiHost::preview_texture`]).
//! The player ping-pongs its final target, so the texture to show changes
//! every frame; ids are cached per GL texture name, and egui's own
//! `replace_native_texture` is avoided because it queues the replaced
//! texture for deletion, which would delete the player's.

use egui::{TextureId, ViewportId, ViewportOutput};
use egui_glow::{Painter, glow};
use egui_winit::winit::event::WindowEvent;
use egui_winit::winit::event_loop::ActiveEventLoop;
use egui_winit::winit::window::Window;
use egui_winit::{EventResponse, State};
use std::collections::HashMap;
use strobe_gfx::gl::Gl;

/// egui, its winit input state, and its GL painter.
pub struct UiHost {
    ctx: egui::Context,
    input: State,
    painter: Painter,
    shapes: Vec<egui::epaint::ClippedShape>,
    pixels_per_point: f32,
    textures_delta: egui::TexturesDelta,
    previews: HashMap<glow::Texture, TextureId>,
}

impl UiHost {
    /// Builds the host on the control window's event loop and the shared
    /// GL context (which must be current).
    pub fn new(event_loop: &ActiveEventLoop, gl: &Gl) -> Result<Self, String> {
        let painter =
            Painter::new(gl.glow(), "", None, true).map_err(|e| format!("egui painter: {e}"))?;
        let ctx = egui::Context::default();
        crate::theme::apply(&ctx);
        let input = State::new(
            ctx.clone(),
            ViewportId::ROOT,
            event_loop,
            None,
            event_loop.system_theme(),
            Some(painter.max_texture_side()),
        );
        Ok(Self {
            ctx,
            input,
            painter,
            shapes: Vec::new(),
            pixels_per_point: 1.0,
            textures_delta: egui::TexturesDelta::default(),
            previews: HashMap::new(),
        })
    }

    /// Feeds a window event to egui.
    pub fn on_window_event(&mut self, window: &Window, event: &WindowEvent) -> EventResponse {
        self.input.on_window_event(window, event)
    }

    /// Whether egui is using the keyboard (a text field has focus).
    pub fn wants_keyboard(&self) -> bool {
        self.ctx.egui_wants_keyboard_input()
    }

    /// The egui id for showing `texture`, registering it on first sight.
    pub fn preview_texture(&mut self, texture: glow::Texture) -> TextureId {
        if let Some(id) = self.previews.get(&texture) {
            return *id;
        }
        let id = self.painter.register_native_texture(texture);
        self.previews.insert(texture, id);
        id
    }

    /// Runs one UI pass. Call [`Self::paint`] afterwards.
    pub fn run(&mut self, window: &Window, run_ui: impl FnMut(&mut egui::Ui)) {
        let raw_input = self.input.take_egui_input(window);
        let egui::FullOutput {
            platform_output,
            textures_delta,
            shapes,
            pixels_per_point,
            viewport_output,
        } = self.ctx.run_ui(raw_input, run_ui);
        let mut info = egui::ViewportInfo::default();
        for (_, ViewportOutput { commands, .. }) in viewport_output {
            let mut requested = Vec::new();
            egui_winit::process_viewport_commands(
                &self.ctx,
                &mut info,
                commands,
                window,
                &mut requested,
            );
        }
        self.input.handle_platform_output(window, platform_output);
        self.shapes = shapes;
        self.pixels_per_point = pixels_per_point;
        self.textures_delta.append(textures_delta);
    }

    /// Clears the window and paints the last pass.
    pub fn paint(&mut self, window: &Window, clear: [f32; 4]) {
        let shapes = std::mem::take(&mut self.shapes);
        let mut textures_delta = std::mem::take(&mut self.textures_delta);
        for (id, deltas) in textures_delta.set.drain() {
            for delta in deltas {
                self.painter.set_texture(id, &delta);
            }
        }
        let primitives = self.ctx.tessellate(shapes, self.pixels_per_point);
        let size: [u32; 2] = window.inner_size().into();
        self.painter.clear(size, clear);
        self.painter
            .paint_primitives(size, self.pixels_per_point, &primitives);
        for id in textures_delta.free.drain() {
            self.painter.free_texture(id);
        }
    }

    /// Releases the painter's GL objects. The context must be current.
    pub fn destroy(&mut self) {
        self.painter.destroy();
    }

    /// Drops the host **without** touching GL, for a context that is lost:
    /// deleting against it could fault in the driver, and the driver frees a
    /// destroyed context's objects anyway. egui's painter would log a
    /// misleading "forgot to call destroy()" warning, so it is forgotten
    /// instead of dropped (a few kilobytes, once per lost context).
    pub fn abandon(self) {
        let Self { painter, .. } = self;
        std::mem::forget(painter);
    }
}
