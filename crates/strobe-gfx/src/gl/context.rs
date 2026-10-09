// ============================================================================
// Module:       strobe_gfx::gl::context
// Description:  The headless EGL context provider (tests, benchmarks); the
//               windowed providers the app runs on are in gl::window.
//
// Dependencies: glutin (context creation), glow (function loading)
// ============================================================================

//! Context creation.
//!
//! Every provider ends the same way: a context made current on the calling
//! thread, and a [`Gl`] built from its function pointers. That ordering is
//! the invariant the rest of [`crate::gl`] relies on, which is why `Gl` has
//! no public constructor and is built only here.
//!
//! ## Headless
//!
//! [`Headless`] opens an EGL *device* display — no window system at all —
//! and makes a context current without a surface. On Linux with Mesa that
//! is llvmpipe, a real GLSL compiler and rasteriser on the CPU, so CI and
//! the benchmark tool compile and render every scene with no GPU and no
//! display. It is offered in both dialects Strobe targets:
//! [`Api::DesktopCore33`], the native path, and [`Api::Gles30`], the
//! dialect the ANGLE fallback speaks.

use super::Gl;
use glutin::api::egl::context::PossiblyCurrentContext;
use glutin::api::egl::device::Device;
use glutin::api::egl::display::Display;
use glutin::config::{ConfigSurfaceTypes, ConfigTemplate, ConfigTemplateBuilder};
use glutin::context::{
    ContextApi, ContextAttributes, ContextAttributesBuilder, GlProfile, Version,
};
use glutin::prelude::*;
use std::ffi::CStr;

/// Which GL a context speaks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Api {
    /// Desktop OpenGL 3.3 core: the native path's floor.
    DesktopCore33,
    /// OpenGL ES 3.0: the ANGLE path's dialect.
    Gles30,
}

impl Api {
    fn attributes(self) -> ContextAttributes {
        match self {
            Self::DesktopCore33 => ContextAttributesBuilder::new()
                .with_context_api(ContextApi::OpenGl(Some(Version::new(3, 3))))
                .with_profile(GlProfile::Core)
                .build(None),
            Self::Gles30 => ContextAttributesBuilder::new()
                .with_context_api(ContextApi::Gles(Some(Version::new(3, 0))))
                .build(None),
        }
    }

    fn config_api(self) -> glutin::config::Api {
        match self {
            Self::DesktopCore33 => glutin::config::Api::OPENGL,
            Self::Gles30 => glutin::config::Api::GLES3,
        }
    }
}

/// A surfaceless context on an EGL device display.
#[derive(Debug)]
pub struct Headless {
    gl: Gl,
    // Field order is drop order: the context goes before its display.
    _context: PossiblyCurrentContext,
    _display: Display,
}

impl Headless {
    /// Opens a headless context speaking `api`, current on this thread.
    pub fn new(api: Api) -> Result<Self, String> {
        let mut last_error = String::from("no EGL device found");
        let devices = Device::query_devices().map_err(|e| format!("EGL devices: {e}"))?;
        for device in devices {
            match open_on_device(&device, api) {
                Ok(headless) => return Ok(headless),
                Err(error) => last_error = error,
            }
        }
        Err(last_error)
    }

    /// The context's GL functions.
    pub fn gl(&self) -> &Gl {
        &self.gl
    }

    /// The context's GL functions, mutably. Exists for one purpose:
    /// [`super::debug::enable_debug_output`] needs the unshared `Gl`, so
    /// call it right after [`Headless::new`], before cloning.
    pub fn gl_mut(&mut self) -> &mut Gl {
        &mut self.gl
    }
}

fn open_on_device(device: &Device, api: Api) -> Result<Headless, String> {
    let display = device_display(device)?;
    let template = ConfigTemplateBuilder::new()
        .with_surface_type(ConfigSurfaceTypes::empty())
        .with_api(api.config_api())
        .build();
    let config = first_config(&display, template)?;
    let not_current = create_context(&display, &config, &api.attributes())?;
    let context = not_current
        .make_current_surfaceless()
        .map_err(|e| format!("make current: {e}"))?;
    let gl = load_gl(&display);
    Ok(Headless {
        gl,
        _context: context,
        _display: display,
    })
}

fn device_display(device: &Device) -> Result<Display, String> {
    // SAFETY: `raw_display` is None, so the only obligation in
    // `with_device`'s contract (a valid DRM handle when Some) does not apply.
    unsafe { Display::with_device(device, None) }.map_err(|e| format!("EGL display: {e}"))
}

fn first_config(
    display: &Display,
    template: ConfigTemplate,
) -> Result<glutin::api::egl::config::Config, String> {
    // SAFETY: the template holds no native window handle, so there is no
    // window whose lifetime the query could outlive.
    let mut configs =
        unsafe { display.find_configs(template) }.map_err(|e| format!("EGL configs: {e}"))?;
    configs
        .next()
        .ok_or_else(|| "no EGL config for this API".to_string())
}

fn create_context(
    display: &Display,
    config: &glutin::api::egl::config::Config,
    attributes: &ContextAttributes,
) -> Result<glutin::api::egl::context::NotCurrentContext, String> {
    // SAFETY: `config` came from `display`, and the attributes carry no
    // shared context and no raw window handle.
    unsafe { display.create_context(config, attributes) }.map_err(|e| format!("EGL context: {e}"))
}

/// Builds a [`Gl`] from `display`'s function pointers. Called only after
/// the context has been made current on this thread.
fn load_gl(display: &Display) -> Gl {
    let loader = |name: &CStr| display.get_proc_address(name);
    Gl::from_current(glow_from_loader(loader))
}

pub(super) fn glow_from_loader(
    loader: impl FnMut(&CStr) -> *const std::ffi::c_void,
) -> glow::Context {
    // SAFETY: the loader returns addresses from the display whose context
    // was made current on this thread immediately before this call; glow
    // stores them and calls none until a method is invoked.
    unsafe { glow::Context::from_loader_function_cstr(loader) }
}
