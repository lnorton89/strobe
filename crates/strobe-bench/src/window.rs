// ============================================================================
// Module:       strobe_bench::window
// Description:  Runs the benchmark on a context owned by one hidden window,
//               for machines whose driver has no EGL devices (Intel on
//               Windows).
//
// Dependencies: winit (event loop, window); strobe-gfx (WindowedContext)
// ============================================================================

//! The window context for the benchmark.
//!
//! winit 0.30 only creates windows from inside the event loop, so the
//! whole benchmark runs inside [`ApplicationHandler::resumed`]: open the
//! context, turn vsync off, hand `&Gl` and `&Caps` to the job (the same
//! function the headless mode calls), drop everything, and exit the loop.
//! The loop never processes a redraw, and nothing is ever swapped. The
//! window exists to own a GL context and nothing else; scenes render into
//! the player's own offscreen targets, so the result does not depend on
//! the window's size, visibility, or whether the compositor draws it.
//!
//! ## Hidden, then tiny
//!
//! The first attempt is an invisible window. Some platforms will not give
//! an invisible window a GL surface (a Wayland surface with no commit is
//! the usual suspect), so on failure the second attempt is a 16x16
//! undecorated window parked far off-screen. `--show-window` skips
//! straight to that one.
//!
//! ## Vsync
//!
//! The surface's swap interval is set to zero. With no swaps it cannot
//! pace anything, but a driver that applies the interval to other
//! synchronisation should not be given a reason to throttle the timing.

use anyhow::{Result, anyhow};
use strobe_gfx::gl::Gl;
use strobe_gfx::{Caps, GlChoice, WindowedContext};
use winit::application::ApplicationHandler;
use winit::dpi::{PhysicalPosition, PhysicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowAttributes, WindowId};

/// Side of the stand-in window, in pixels.
const TINY_SIDE: u32 = 16;
/// Where the stand-in window is parked: far outside any desktop.
const PARKED_AT: i32 = -32_000;

/// Runs `job` on a context from a hidden window, and returns its result.
///
/// `job` receives the GL functions, what the context can do, and a line
/// naming where the context came from, for the report.
pub(crate) fn run<F>(choice: GlChoice, tiny: bool, job: F) -> Result<()>
where
    F: FnOnce(&Gl, &Caps, &str) -> Result<()>,
{
    let event_loop =
        EventLoop::new().map_err(|e| anyhow!("could not start the window system: {e}"))?;
    let mut host = Host {
        choice,
        tiny,
        job: Some(job),
        outcome: None,
    };
    event_loop
        .run_app(&mut host)
        .map_err(|e| anyhow!("the event loop failed: {e}"))?;
    host.outcome
        .unwrap_or_else(|| Err(anyhow!("the event loop ended before the benchmark ran")))
}

/// The application: it does its work once, in `resumed`.
struct Host<F> {
    choice: GlChoice,
    tiny: bool,
    job: Option<F>,
    outcome: Option<Result<()>>,
}

impl<F> ApplicationHandler for Host<F>
where
    F: FnOnce(&Gl, &Caps, &str) -> Result<()>,
{
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(job) = self.job.take() {
            self.outcome = Some(open_and_run(event_loop, self.choice, self.tiny, job));
        }
        event_loop.exit();
    }

    fn window_event(&mut self, _: &ActiveEventLoop, _: WindowId, _: WindowEvent) {}
}

fn hidden() -> WindowAttributes {
    Window::default_attributes()
        .with_title("Strobe Bench")
        .with_visible(false)
        .with_inner_size(PhysicalSize::new(TINY_SIDE, TINY_SIDE))
}

fn parked() -> WindowAttributes {
    Window::default_attributes()
        .with_title("Strobe Bench")
        .with_decorations(false)
        .with_resizable(false)
        .with_inner_size(PhysicalSize::new(TINY_SIDE, TINY_SIDE))
        .with_position(PhysicalPosition::new(PARKED_AT, PARKED_AT))
}

fn open_and_run<F>(event_loop: &ActiveEventLoop, choice: GlChoice, tiny: bool, job: F) -> Result<()>
where
    F: FnOnce(&Gl, &Caps, &str) -> Result<()>,
{
    let mut problems = Vec::new();
    let mut opened = None;
    let attempts = if tiny {
        vec![("tiny off-screen window", parked())]
    } else {
        vec![
            ("hidden window", hidden()),
            ("tiny off-screen window", parked()),
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
    let Some((what, gfx, swap)) = opened else {
        return Err(anyhow!(
            "window context: could not open one.\n{}",
            problems.join("\n")
        ));
    };

    if let Err(error) = gfx.set_vsync(&swap, false) {
        println!("warning: could not turn vsync off: {error}");
    }
    let context = format!("window ({what}), {}", gfx.backend());
    let result = job(gfx.gl(), gfx.caps(), &context);

    // The surface goes before the context and display it came from.
    drop(swap);
    drop(gfx);
    result
}
