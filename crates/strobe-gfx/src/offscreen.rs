// ============================================================================
// Module:       strobe_gfx::offscreen
// Description:  Runs a job on a GL context that needs no visible output:
//               an EGL device when the machine has one, else one hidden
//               window. Shared by the bench and auto-controls.
//
// Dependencies: winit (event loop, window); strobe_gfx::gl (Headless,
//               WindowedContext), strobe_gfx::caps
// ============================================================================

//! A GL context for work that draws nothing to the screen.
//!
//! Tools that render scenes to measure or compare them (`strobe-bench`,
//! `strobe-autocontrol`) need a context and nothing else. There are two
//! ways to get one, and which works depends on the machine:
//!
//! - **Headless**: an EGL *device* display, no window system. This is what
//!   CI and a cloud container have (Mesa llvmpipe), and it needs no
//!   display server.
//! - **Window**: one hidden winit window owning a [`WindowedContext`], the
//!   provider the app itself uses. Intel's Windows driver exposes no EGL
//!   devices (`Enumerating devices is not supported by the EGL instance`),
//!   so on the owner's laptop this is the only way (D70). It also measures
//!   the native (WGL) and ANGLE contexts the show really runs on.
//!
//! [`run`] picks per a [`Request`] and calls the job with the same three
//! things either way: the GL functions, what the context can do, and a line
//! saying where the context came from (for a report). The job cannot tell
//! the modes apart except through that line, and the frames it renders are
//! identical (D70, D140).
//!
//! ```no_run
//! use strobe_gfx::offscreen::{self, OffscreenError, Request};
//!
//! fn renderer() -> Result<String, OffscreenError> {
//!     offscreen::run(&Request::default(), |why| eprintln!("{why}"), |_gl, caps, context| {
//!         format!("{} via {context}", caps.info.renderer)
//!     })
//! }
//! ```
//!
//! ## Window mode, and its two limits
//!
//! winit 0.30 only creates windows from inside the event loop, so the whole
//! job runs inside [`ApplicationHandler::resumed`]: open the context, turn
//! vsync off, call the job, drop everything, exit the loop, and return the
//! job's value. The loop never processes a redraw and nothing is swapped.
//! The window exists to own a context; scenes render into the player's own
//! offscreen targets, so the result does not depend on the window's size,
//! visibility, or whether a compositor draws it.
//!
//! - **Main thread.** winit refuses to build an event loop elsewhere on
//!   most platforms (it panics on X11 and Wayland, and macOS requires it).
//!   Call [`run`] and [`run_window`] from `main`, never from a test thread.
//! - **Once per process.** winit allows one event loop per process and
//!   never lets a second be built, so window mode can run once. A caller
//!   with several jobs puts them all in one closure. That is why the app,
//!   which already owns an event loop and a context, must not call this: it
//!   hands its own `&Gl` and `&Caps` to the library instead.
//!
//! ## Hidden, then tiny (D71)
//!
//! The first attempt is an invisible window. Some platforms will not give
//! an invisible window a GL surface (a Wayland surface with no commit is
//! the usual suspect), so on failure the second attempt is a 16x16
//! undecorated window parked far off-screen. [`Request::tiny`] skips
//! straight to that one.
//!
//! ## Vsync
//!
//! The surface's swap interval is set to zero. With no swaps it cannot
//! pace anything, but a driver that applies the interval to other
//! synchronisation should not be given a reason to throttle timings. If
//! the driver refuses, the context line says so.
//!
//! ## `api` and `gl` must agree (D72)
//!
//! A headless context chooses its dialect with [`Request::api`]; a window
//! context gets it from the backend (native is GLSL 330 core, ANGLE is GLES
//! 3.0). [`window_choice`] folds the two: an unset `api` means desktop for
//! headless and whatever `gl` implies for a window, `Gles30` means ANGLE,
//! and a contradiction is an error, not a silent override.

use crate::caps::Caps;
use crate::gl::Gl;
use crate::gl::context::{Api, Headless};
use crate::gl::debug::{DebugLevel, enable_debug_output};
use crate::gl::window::{Backend, GlChoice, WindowedContext};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

/// Side of the stand-in window, in pixels.
const TINY_SIDE: u32 = 16;
/// Where the stand-in window is parked: far outside any desktop.
const PARKED_AT: i32 = -32_000;

/// Where the context comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ContextMode {
    /// Headless if the machine has an EGL device, else a window.
    #[default]
    Auto,
    /// An EGL device with no window system. Fails if there is none.
    Headless,
    /// One hidden window owning a [`WindowedContext`].
    Window,
}

/// What kind of context to get.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Request {
    /// Headless, window, or whichever works.
    pub mode: ContextMode,
    /// The dialect asked for. `None` is desktop for headless and whatever
    /// [`gl`](Self::gl) implies for a window.
    pub api: Option<Api>,
    /// The window backend: native, ANGLE, or native then ANGLE.
    pub gl: GlChoice,
    /// Window mode only: skip the hidden window and start with the tiny
    /// off-screen one (D71).
    pub tiny: bool,
    /// Window mode only: the stand-in window's client size, in pixels.
    /// `None` is [`TINY_SIDE`] square. Nothing the job renders depends on it
    /// (D296); it exists so a machine that draws black above some size can
    /// be bisected by making the window bigger than the render.
    pub window_size: Option<[u32; 2]>,
}

/// Why no context could be had.
#[derive(Debug, thiserror::Error)]
pub enum OffscreenError {
    /// `api` and `gl` ask for different things.
    #[error("{0}")]
    Conflict(String),
    /// No headless context could be opened.
    #[error("headless context: {0}")]
    Headless(String),
    /// The window system could not be started, or no window gave a context.
    #[error("{0}")]
    Window(String),
}

/// The window backend `api` and `gl` ask for together (D72).
///
/// # Errors
///
/// [`OffscreenError::Conflict`] when they contradict: `Gles30` on the
/// native backend, or `DesktopCore33` on ANGLE.
pub fn window_choice(api: Option<Api>, gl: GlChoice) -> Result<GlChoice, OffscreenError> {
    match (gl, api) {
        (GlChoice::Auto, None | Some(Api::DesktopCore33)) => Ok(GlChoice::Auto),
        (GlChoice::Auto | GlChoice::Only(Backend::Angle), Some(Api::Gles30))
        | (GlChoice::Only(Backend::Angle), None) => Ok(GlChoice::Only(Backend::Angle)),
        (GlChoice::Only(Backend::Native), None | Some(Api::DesktopCore33)) => {
            Ok(GlChoice::Only(Backend::Native))
        }
        (GlChoice::Only(Backend::Native), Some(Api::Gles30)) => Err(OffscreenError::Conflict(
            "--api gles is the ANGLE dialect; it cannot run on --gl native".to_owned(),
        )),
        (GlChoice::Only(Backend::Angle), Some(Api::DesktopCore33)) => {
            Err(OffscreenError::Conflict(
                "--api desktop is the native dialect; it cannot run on --gl angle".to_owned(),
            ))
        }
    }
}

/// Runs `job` on the context `request` asks for, and returns its value.
///
/// `job` receives the GL functions, what the context can do, and a line
/// naming where the context came from. With [`ContextMode::Auto`], when
/// the headless attempt fails `notice` is called once, before the job, with
/// a line saying why the window is used instead.
///
/// Window mode must run on the main thread, once per process (see the
/// module docs).
///
/// # Errors
///
/// [`OffscreenError::Conflict`] when `api` and `gl` contradict (checked in
/// every mode, so a mistake is not hidden by the machine you tried it on);
/// [`OffscreenError::Headless`] when `Headless` finds no device;
/// [`OffscreenError::Window`] when the window system or its context fails.
pub fn run<T, N, F>(request: &Request, notice: N, job: F) -> Result<T, OffscreenError>
where
    N: FnOnce(&str),
    F: FnOnce(&Gl, &Caps, &str) -> T,
{
    let choice = window_choice(request.api, request.gl)?;
    match request.mode {
        ContextMode::Headless => run_headless(request.api, job),
        ContextMode::Window => run_window_sized(choice, request.tiny, request.window_size, job),
        ContextMode::Auto => match open_headless(request.api) {
            Ok(headless) => Ok(call_headless(&headless, request.api, job)),
            Err(error) => {
                notice(&format!("{error}; using a window context instead"));
                run_window_sized(choice, request.tiny, request.window_size, job)
            }
        },
    }
}

/// Runs `job` on a headless EGL device context.
///
/// # Errors
///
/// [`OffscreenError::Headless`] when the machine has no EGL device that
/// gives a context.
pub fn run_headless<T, F>(api: Option<Api>, job: F) -> Result<T, OffscreenError>
where
    F: FnOnce(&Gl, &Caps, &str) -> T,
{
    let headless = open_headless(api)?;
    Ok(call_headless(&headless, api, job))
}

fn open_headless(api: Option<Api>) -> Result<Headless, OffscreenError> {
    let mut headless =
        Headless::new(api.unwrap_or(Api::DesktopCore33)).map_err(OffscreenError::Headless)?;
    debug_output_from_env(headless.gl_mut());
    Ok(headless)
}

/// The variable that asks an offscreen tool for the driver's own messages.
const DEBUG_ENV: &str = "STROBE_GL_DEBUG";

/// Turns on `KHR_debug` output when `STROBE_GL_DEBUG` asks, the way the app
/// does, so a bench run on a machine that renders black can show what the
/// driver said about it. Unset means off here (unlike the app, which
/// defaults to errors): an offscreen tool's output is a report.
fn debug_output_from_env(gl: &mut Gl) {
    let setting = std::env::var(DEBUG_ENV).ok();
    let level = match setting.as_deref().map(str::trim) {
        None | Some("" | "0" | "off" | "false") => return,
        Some("info") => DebugLevel::Info,
        Some("warn" | "warning" | "warnings") => DebugLevel::Warnings,
        Some("errors" | "error") => DebugLevel::Errors,
        Some(_) => DebugLevel::All,
    };
    enable_debug_output(gl, level);
}

fn call_headless<T, F>(headless: &Headless, api: Option<Api>, job: F) -> T
where
    F: FnOnce(&Gl, &Caps, &str) -> T,
{
    let gl = headless.gl();
    let context = format!(
        "headless EGL device, {:?}",
        api.unwrap_or(Api::DesktopCore33)
    );
    job(gl, &Caps::from_info(gl.info()), &context)
}

/// Runs `job` on a context from a hidden window, and returns its value.
///
/// Main thread, once per process; see the module docs.
///
/// # Errors
///
/// [`OffscreenError::Window`] when the event loop cannot start, runs no
/// job, or no window gives a context.
pub fn run_window<T, F>(choice: GlChoice, tiny: bool, job: F) -> Result<T, OffscreenError>
where
    F: FnOnce(&Gl, &Caps, &str) -> T,
{
    run_window_sized(choice, tiny, None, job)
}

/// [`run_window`] with the stand-in window's client size chosen
/// ([`Request::window_size`]).
///
/// # Errors
///
/// As [`run_window`].
pub fn run_window_sized<T, F>(
    choice: GlChoice,
    tiny: bool,
    window_size: Option<[u32; 2]>,
    job: F,
) -> Result<T, OffscreenError>
where
    F: FnOnce(&Gl, &Caps, &str) -> T,
{
    let event_loop = EventLoop::new()
        .map_err(|e| OffscreenError::Window(format!("could not start the window system: {e}")))?;
    let mut host = Host {
        choice,
        tiny,
        side: window_size.unwrap_or([TINY_SIDE, TINY_SIDE]),
        job: Some(job),
        outcome: None,
    };
    event_loop
        .run_app(&mut host)
        .map_err(|e| OffscreenError::Window(format!("the event loop failed: {e}")))?;
    host.outcome.unwrap_or_else(|| {
        Err(OffscreenError::Window(
            "the event loop ended before the job ran".to_owned(),
        ))
    })
}

/// The application: it does its work once, in `resumed`.
struct Host<F, T> {
    choice: GlChoice,
    tiny: bool,
    side: [u32; 2],
    job: Option<F>,
    outcome: Option<Result<T, OffscreenError>>,
}

impl<F, T> ApplicationHandler for Host<F, T>
where
    F: FnOnce(&Gl, &Caps, &str) -> T,
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(job) = self.job.take() {
            self.outcome = Some(open_and_run(
                event_loop,
                self.choice,
                self.tiny,
                self.side,
                job,
            ));
        }
        event_loop.exit();
    }

    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
}

fn hidden([width, height]: [u32; 2]) -> WindowAttributes {
    Window::default_attributes()
        .with_title("Strobe offscreen")
        .with_visible(false)
        .with_inner_size(PhysicalSize::new(width, height))
}

fn parked([width, height]: [u32; 2]) -> WindowAttributes {
    Window::default_attributes()
        .with_title("Strobe offscreen")
        .with_decorations(false)
        .with_resizable(false)
        .with_inner_size(PhysicalSize::new(width, height))
        .with_position(PhysicalPosition::new(PARKED_AT, PARKED_AT))
}

fn open_and_run<T, F>(
    event_loop: &ActiveEventLoop,
    choice: GlChoice,
    tiny: bool,
    side: [u32; 2],
    job: F,
) -> Result<T, OffscreenError>
where
    F: FnOnce(&Gl, &Caps, &str) -> T,
{
    let mut problems = Vec::new();
    let mut opened = None;
    let attempts = if tiny {
        vec![("tiny off-screen window", parked(side))]
    } else {
        vec![
            ("hidden window", hidden(side)),
            ("tiny off-screen window", parked(side)),
        ]
    };
    for (what, attributes) in attempts {
        match WindowedContext::open(event_loop, choice, &attributes) {
            Ok((gfx, swap)) => {
                opened = Some((what, gfx, swap));
                break;
            }
            Err(error) => problems.push(format!("{what}: {error}")),
        }
    }
    let Some((what, mut gfx, swap)) = opened else {
        return Err(OffscreenError::Window(format!(
            "window context: could not open one ({})",
            problems.join("; ")
        )));
    };

    debug_output_from_env(gfx.gl_mut());
    let mut context = format!("window ({what}), {}", gfx.backend());
    if side != [TINY_SIDE, TINY_SIDE] {
        // The size the platform gave, which can be less than asked: Windows
        // caps a new window at about the screen's size.
        let given = swap.window().inner_size();
        context.push_str(&format!(", {}x{} window", given.width, given.height));
        if [given.width, given.height] != side {
            context.push_str(&format!(" (asked for {}x{})", side[0], side[1]));
        }
    }
    if let Err(error) = gfx.set_vsync(&swap, false) {
        log::warn!("offscreen window: could not turn vsync off: {error}");
        context.push_str(&format!(" (vsync left on: {error})"));
    }
    let value = job(gfx.gl(), gfx.caps(), &context);

    // The surface goes before the context and display it came from.
    drop(swap);
    drop(gfx);
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_and_backend_fold_to_one_choice() -> anyhow::Result<()> {
        let auto = GlChoice::Auto;
        let native = GlChoice::Only(Backend::Native);
        let angle = GlChoice::Only(Backend::Angle);
        let cases = [
            (auto, None, Some(auto)),
            (auto, Some(Api::DesktopCore33), Some(auto)),
            (auto, Some(Api::Gles30), Some(angle)),
            (native, None, Some(native)),
            (native, Some(Api::DesktopCore33), Some(native)),
            (native, Some(Api::Gles30), None),
            (angle, None, Some(angle)),
            (angle, Some(Api::Gles30), Some(angle)),
            (angle, Some(Api::DesktopCore33), None),
        ];
        for (gl, api, want) in cases {
            let got = window_choice(api, gl).ok();
            anyhow::ensure!(
                got == want,
                "gl {gl:?} api {api:?}: got {got:?}, want {want:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn a_contradiction_fails_in_every_mode() -> anyhow::Result<()> {
        for mode in [
            ContextMode::Auto,
            ContextMode::Headless,
            ContextMode::Window,
        ] {
            let request = Request {
                mode,
                api: Some(Api::Gles30),
                gl: GlChoice::Only(Backend::Native),
                tiny: false,
                window_size: None,
            };
            let outcome = run(&request, |_| {}, |_, _, _| ());
            anyhow::ensure!(
                matches!(outcome, Err(OffscreenError::Conflict(_))),
                "{mode:?} accepted --api gles with --gl native"
            );
        }
        Ok(())
    }
}
