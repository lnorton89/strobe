// ============================================================================
// Module:       strobe_gfx::gl::window
// Description:  The windowed context provider: one GL context drawing to
//               several window surfaces, on native GL or on ANGLE.
//
// Dependencies: glutin (display, config, context, surface), glutin-winit
//               (window finalisation), winit (windows), glow (loading)
// ============================================================================

//! The windowed context.
//!
//! The app has two windows, a control window (egui) and an output window
//! (the show), and they share **one** GL context. Textures and render
//! targets the scene player owns are therefore visible to the egui painter
//! without any sharing set-up, and a frame is "make the context current on
//! this window's surface, draw, swap". [`WindowedContext`] is the context
//! and the display it came from; [`SwapWindow`] is a window and its
//! surface. Neither is `Send`, because the [`Gl`] inside is not.
//!
//! ## Two backends, one type
//!
//! - [`Backend::Native`] asks glutin for the platform's own GL: WGL on
//!   Windows, GLX (then EGL) on Linux, CGL on macOS, and requests an
//!   OpenGL **3.3 core** context. This is the fast path and speaks GLSL
//!   330.
//! - [`Backend::Angle`] asks for EGL and an OpenGL **ES 3.0** context. On
//!   Windows glutin loads `libEGL.dll`, which is ANGLE (GLES on Direct3D
//!   11) when ANGLE's DLLs sit next to the executable; this is the answer
//!   for laptops whose OpenGL driver is broken or missing. The identical
//!   code compiles everywhere: elsewhere it resolves to the system's EGL
//!   and a GLES 3.0 context, which is how the ES dialect is exercised on a
//!   Linux machine.
//!
//! [`GlChoice::Auto`] tries native first and falls back to ANGLE, and the
//! error says what each attempt reported. [`GlChoice::Only`] forces one.
//!
//! ## Why the display is built here and not with `DisplayBuilder`
//!
//! glutin-winit's `DisplayBuilder` takes a config *picker* that must return
//! a config, so "no config matched" can only be expressed as a panic, and
//! its API preference cannot say "EGL only" next to WGL. Building the
//! display directly costs about twenty lines and keeps both properties:
//! failures are values, and the Native and Angle backends stay distinct.
//! The one thing `DisplayBuilder` knows that matters is that WGL needs a
//! window *before* the display, which is handled below.
//!
//! ## One context, many surfaces
//!
//! A context can be made current on any surface made from the same config.
//! Every extra window is therefore created with [`finalize_window`] against
//! the first window's config (which on X11 sets the matching visual), and
//! its surface comes from that same config.
//!
//! ## Vsync
//!
//! The swap interval belongs to the surface. The output window waits for
//! vertical blank; the control window does not, because two vsynced swaps
//! in one loop would halve the frame rate.

use super::Gl;
use super::context::glow_from_loader;
use crate::caps::Caps;
use glutin::config::{Api, Config, ConfigTemplate, ConfigTemplateBuilder, GlConfig};
use glutin::context::{
    ContextApi, ContextAttributes, ContextAttributesBuilder, GlProfile, NotCurrentContext,
    NotCurrentGlContext, PossiblyCurrentContext, PossiblyCurrentGlContext, Version,
};
use glutin::display::{Display, DisplayApiPreference, GlDisplay};
use glutin::surface::{GlSurface, Surface, SurfaceAttributesBuilder, SwapInterval, WindowSurface};
use glutin_winit::finalize_window;
use raw_window_handle::{HasDisplayHandle, HasWindowHandle, RawDisplayHandle, RawWindowHandle};
use std::ffi::CStr;
use std::fmt;
use std::num::NonZeroU32;
use winit::event_loop::ActiveEventLoop;
use winit::window::{Window, WindowAttributes};

/// Which GL stack a context runs on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// The platform's own OpenGL 3.3 core (WGL, GLX/EGL, CGL).
    Native,
    /// EGL with an OpenGL ES 3.0 context: ANGLE on Windows.
    Angle,
}

impl fmt::Display for Backend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Native => "native (OpenGL 3.3 core)",
            Self::Angle => "ANGLE/EGL (OpenGL ES 3.0)",
        })
    }
}

impl Backend {
    fn config_api(self) -> Api {
        match self {
            Self::Native => Api::OPENGL,
            Self::Angle => Api::GLES3,
        }
    }

    fn context_attributes(self, window: RawWindowHandle) -> ContextAttributes {
        match self {
            Self::Native => ContextAttributesBuilder::new()
                .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
                .with_profile(GlProfile::Core)
                .build(Some(window)),
            Self::Angle => ContextAttributesBuilder::new()
                .with_context_api(ContextApi::Gles(Some(Version::new(3, 0))))
                .build(Some(window)),
        }
    }

    fn preference(self, window: Option<RawWindowHandle>) -> DisplayApiPreference {
        match self {
            Self::Native => native_preference(window),
            Self::Angle => DisplayApiPreference::Egl,
        }
    }

    /// Whether this backend needs its first window to exist before the
    /// display does (WGL reads the pixel format from a real window).
    fn window_first(self) -> bool {
        self == Self::Native && cfg!(target_os = "windows")
    }
}

#[cfg(target_os = "windows")]
fn native_preference(window: Option<RawWindowHandle>) -> DisplayApiPreference {
    DisplayApiPreference::Wgl(window)
}

#[cfg(target_os = "macos")]
fn native_preference(window: Option<RawWindowHandle>) -> DisplayApiPreference {
    let _ = window;
    DisplayApiPreference::Cgl
}

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn native_preference(window: Option<RawWindowHandle>) -> DisplayApiPreference {
    let _ = window;
    DisplayApiPreference::GlxThenEgl(Box::new(winit::platform::x11::register_xlib_error_hook))
}

/// Which backend(s) to try.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GlChoice {
    /// Native first; ANGLE if native fails.
    #[default]
    Auto,
    /// Exactly this backend, failing if it cannot start.
    Only(Backend),
}

impl GlChoice {
    fn attempts(self) -> &'static [Backend] {
        match self {
            Self::Auto => &[Backend::Native, Backend::Angle],
            Self::Only(Backend::Native) => &[Backend::Native],
            Self::Only(Backend::Angle) => &[Backend::Angle],
        }
    }
}

/// Why a windowed context could not be created.
#[derive(Debug, thiserror::Error)]
#[error("{}", .attempts.join("; "))]
pub struct WindowError {
    /// One line per backend tried: which, and what it reported.
    pub attempts: Vec<String>,
}

/// A window and the GL surface drawing to it.
///
/// Field order is drop order: the surface goes before the window it was
/// made from.
#[derive(Debug)]
pub struct SwapWindow {
    surface: Surface<WindowSurface>,
    window: Window,
}

impl SwapWindow {
    /// The winit window.
    pub fn window(&self) -> &Window {
        &self.window
    }
}

/// One GL context, current on one window at a time.
///
/// Field order is drop order: the context goes before the config and the
/// display it was made from.
#[derive(Debug)]
pub struct WindowedContext {
    gl: Gl,
    backend: Backend,
    caps: Caps,
    context: PossiblyCurrentContext,
    config: Config,
    display: Display,
}

impl WindowedContext {
    /// Opens the context and its first window, trying backends per
    /// `choice`. The context is current on the returned window.
    pub fn open(
        event_loop: &ActiveEventLoop,
        choice: GlChoice,
        first: &WindowAttributes,
    ) -> Result<(Self, SwapWindow), WindowError> {
        let mut attempts = Vec::new();
        for &backend in choice.attempts() {
            match Self::open_backend(event_loop, backend, first) {
                Ok(opened) => {
                    log::info!(
                        "GL backend {backend}: {} | {}",
                        opened.0.caps.info.renderer,
                        opened.0.caps.info.version
                    );
                    return Ok(opened);
                }
                Err(message) => {
                    log::warn!("GL backend {backend} unavailable: {message}");
                    attempts.push(format!("{backend}: {message}"));
                }
            }
        }
        Err(WindowError { attempts })
    }

    fn open_backend(
        event_loop: &ActiveEventLoop,
        backend: Backend,
        first: &WindowAttributes,
    ) -> Result<(Self, SwapWindow), String> {
        let early = if backend.window_first() {
            Some(
                event_loop
                    .create_window(first.clone())
                    .map_err(|e| format!("window: {e}"))?,
            )
        } else {
            None
        };
        let early_handle = early
            .as_ref()
            .and_then(|window| window.window_handle().ok())
            .map(|handle| handle.as_raw());

        let display_handle = event_loop
            .display_handle()
            .map_err(|e| format!("display handle: {e}"))?
            .as_raw();
        let display = new_display(display_handle, backend.preference(early_handle))?;
        let config = pick_config(&display, template(backend, early_handle))?;

        let window = match early {
            Some(window) => window,
            None => finalize_window(event_loop, first.clone(), &config)
                .map_err(|e| format!("window: {e}"))?,
        };
        let handle = window
            .window_handle()
            .map_err(|e| format!("window handle: {e}"))?
            .as_raw();
        let not_current = create_context(&display, &config, &backend.context_attributes(handle))?;
        let surface = create_surface(&display, &config, &window)?;
        let context = not_current
            .make_current(&surface)
            .map_err(|e| format!("make current: {e}"))?;
        let gl = Gl::from_current(glow_from_loader(proc_loader(&display)));
        let caps = Caps::from_info(gl.info());
        Ok((
            Self {
                gl,
                backend,
                caps,
                context,
                config,
                display,
            },
            SwapWindow { surface, window },
        ))
    }

    /// Creates another window drawing with this context.
    pub fn add_window(
        &self,
        event_loop: &ActiveEventLoop,
        attributes: WindowAttributes,
    ) -> Result<SwapWindow, String> {
        let window = finalize_window(event_loop, attributes, &self.config)
            .map_err(|e| format!("window: {e}"))?;
        let surface = create_surface(&self.display, &self.config, &window)?;
        Ok(SwapWindow { surface, window })
    }

    /// The context's GL functions.
    pub fn gl(&self) -> &Gl {
        &self.gl
    }

    /// Which backend is running.
    pub fn backend(&self) -> Backend {
        self.backend
    }

    /// What the context can do, decided once at creation.
    pub fn caps(&self) -> &Caps {
        &self.caps
    }

    /// Makes the context current on `target`'s surface.
    pub fn make_current(&self, target: &SwapWindow) -> Result<(), String> {
        self.context
            .make_current(&target.surface)
            .map_err(|e| format!("make current: {e}"))
    }

    /// Presents `target`'s back buffer. The context must be current on it.
    pub fn swap_buffers(&self, target: &SwapWindow) -> Result<(), String> {
        target
            .surface
            .swap_buffers(&self.context)
            .map_err(|e| format!("swap buffers: {e}"))
    }

    /// Sets vsync for `target`'s surface, making the context current on it
    /// first (the swap interval applies to the current surface on EGL).
    pub fn set_vsync(&self, target: &SwapWindow, vsync: bool) -> Result<(), String> {
        self.make_current(target)?;
        let interval = if vsync {
            SwapInterval::Wait(NonZeroU32::MIN)
        } else {
            SwapInterval::DontWait
        };
        target
            .surface
            .set_swap_interval(&self.context, interval)
            .map_err(|e| format!("swap interval: {e}"))
    }

    /// Resizes `target`'s surface to the window's current size. A
    /// minimised window (zero area) is left alone.
    pub fn resize(&self, target: &SwapWindow) {
        let size = target.window.inner_size();
        if let (Some(width), Some(height)) =
            (NonZeroU32::new(size.width), NonZeroU32::new(size.height))
        {
            target.surface.resize(&self.context, width, height);
        }
    }
}

fn template(backend: Backend, window: Option<RawWindowHandle>) -> ConfigTemplate {
    let mut builder = ConfigTemplateBuilder::new()
        .with_api(backend.config_api())
        .with_alpha_size(0)
        .with_depth_size(0)
        .with_stencil_size(0)
        .with_transparency(false)
        .prefer_hardware_accelerated(None);
    if let Some(window) = window {
        builder = builder.compatible_with_native_window(window);
    }
    builder.build()
}

/// The best config: hardware accelerated, then the fewest samples (every
/// scene renders into its own targets, so multisampling the window's
/// back buffer only costs bandwidth).
fn pick_config(display: &Display, template: ConfigTemplate) -> Result<Config, String> {
    let configs = find_configs(display, template)?;
    configs
        .min_by_key(|config| (!config.hardware_accelerated(), config.num_samples()))
        .ok_or_else(|| "no matching framebuffer configuration".to_string())
}

fn proc_loader(display: &Display) -> impl FnMut(&CStr) -> *const std::ffi::c_void + '_ {
    |name| display.get_proc_address(name)
}

fn create_surface(
    display: &Display,
    config: &Config,
    window: &Window,
) -> Result<Surface<WindowSurface>, String> {
    let handle = window
        .window_handle()
        .map_err(|e| format!("window handle: {e}"))?
        .as_raw();
    let size = window.inner_size();
    let attributes = SurfaceAttributesBuilder::<WindowSurface>::new().build(
        handle,
        NonZeroU32::new(size.width).unwrap_or(NonZeroU32::MIN),
        NonZeroU32::new(size.height).unwrap_or(NonZeroU32::MIN),
    );
    new_window_surface(display, config, &attributes)
}

fn new_display(
    handle: RawDisplayHandle,
    preference: DisplayApiPreference,
) -> Result<Display, String> {
    // SAFETY: `handle` comes from the live event loop. Every
    // `WindowedContext` is created inside an event-loop callback and
    // dropped (in the app's `exiting`) before the loop returns, so the
    // display connection outlives the `Display` built on it.
    unsafe { Display::new(handle, preference) }.map_err(|e| format!("display: {e}"))
}

fn find_configs(
    display: &Display,
    template: ConfigTemplate,
) -> Result<Box<dyn Iterator<Item = Config> + '_>, String> {
    // SAFETY: the template's only native-window reference is the handle of
    // a window created just before this call and still alive during it.
    unsafe { display.find_configs(template) }.map_err(|e| format!("configs: {e}"))
}

fn create_context(
    display: &Display,
    config: &Config,
    attributes: &ContextAttributes,
) -> Result<NotCurrentContext, String> {
    // SAFETY: `config` came from `display`, and the window handle in
    // `attributes` belongs to a window that is alive for this call and is
    // kept by the returned `SwapWindow` for as long as the context lives.
    unsafe { display.create_context(config, attributes) }.map_err(|e| format!("context: {e}"))
}

fn new_window_surface(
    display: &Display,
    config: &Config,
    attributes: &glutin::surface::SurfaceAttributes<WindowSurface>,
) -> Result<Surface<WindowSurface>, String> {
    // SAFETY: the handle in `attributes` is of a live window, and the
    // surface is stored in `SwapWindow` ahead of that window, so it is
    // dropped before the window is.
    unsafe { display.create_window_surface(config, attributes) }
        .map_err(|e| format!("surface: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_tries_native_before_angle() {
        assert_eq!(
            GlChoice::Auto.attempts(),
            &[Backend::Native, Backend::Angle],
            "native first"
        );
    }

    #[test]
    fn a_forced_backend_is_the_only_attempt() {
        assert_eq!(
            GlChoice::Only(Backend::Angle).attempts(),
            &[Backend::Angle],
            "angle only"
        );
        assert_eq!(
            GlChoice::Only(Backend::Native).attempts(),
            &[Backend::Native],
            "native only"
        );
    }

    #[test]
    fn the_backends_ask_for_different_apis() {
        assert_ne!(
            Backend::Native.config_api(),
            Backend::Angle.config_api(),
            "desktop GL versus GLES"
        );
    }
}
