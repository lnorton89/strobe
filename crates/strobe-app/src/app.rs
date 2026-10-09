// ============================================================================
// Module:       strobe_app::app
// Description:  The winit application: creates the two windows on one GL
//               context, paces the frame loop, carries out UI actions, and
//               rebuilds the context when it is lost.
//
// Dependencies: winit (event loop); strobe-gfx (WindowedContext);
//               crate::{ui, session, library, console, stats, screenshot,
//               recovery, fault}
// ============================================================================

//! The application.
//!
//! ## Frame loop
//!
//! `about_to_wait` decides which windows are due and requests redraws;
//! the drawing happens in `RedrawRequested`, one window at a time, with
//! the shared context made current on that window's surface first.
//!
//! - **Output window**, up to [`OUTPUT_PERIOD`] apart (a 144 Hz cap that
//!   only matters where vsync is not working): audio, control tick,
//!   `ScenePlayer::render`, `present`, swap. Vsync is on, so the swap
//!   paces this loop to the display.
//! - **Control window**, [`CONTROL_PERIOD`] apart (30 Hz): the preview
//!   texture, one egui pass, paint, swap. It has vsync off so two vsynced
//!   swaps cannot halve the output rate. The throttle matters on an old
//!   laptop, where egui's tessellation and the extra swap are a real cost
//!   and 30 Hz is plenty for sliders.
//!
//! The scene is rendered in the *output* window's redraw, never the
//! control window's, so a slow or hidden control window cannot slow the
//! show down.
//!
//! ## GL state between the scene and egui
//!
//! egui changes blending, scissor, program, vertex array and texture
//! bindings. The player sets everything it needs at the start of each
//! `render`, and `present` is preceded by `plain_state`, so neither
//! depends on what egui left behind.
//!
//! ## Render resolution
//!
//! The target is the output window's size times the render scale. Window
//! resizes are applied once the size has been steady for
//! [`RESIZE_SETTLE`] (a resize reallocates every target and restarts
//! feedback scenes from black, which dragging a window edge must not do
//! sixty times a second). A render-scale change is applied immediately.
//!
//! ## Context loss (D66)
//!
//! A failed `make_current` or swap is not the end of the show. Each one is
//! an [`Event`] for the [`Recovery`] policy, which says retry, rebuild,
//! fail over to ANGLE or give up (see [`crate::recovery`] for the rules).
//! A rebuild is carried out in [`Running::recover`], from [`App::settle`],
//! where the `ActiveEventLoop` is at hand:
//!
//! 1. the old `Gl` is marked lost, so nothing calls into it again;
//! 2. the session, timers, egui and **both windows** and the context are
//!    dropped, in that order. The windows go too: WGL sets a window's pixel
//!    format once, so a window that has had a context cannot be given
//!    another safely;
//! 3. a new context and two new windows are built on the chosen backend;
//! 4. the output's monitor and fullscreen state and the control window's
//!    maximized state come back, the render scale carries over, and the
//!    scene that was playing is reloaded at its clock with its controls.
//!
//! The reset status is queried only after a failed swap (the context is
//! current then) and at most once a second as a health check, never every
//! frame, and never after a failed `make_current` (the context may not be
//! current, and a GL call without one can fault in the driver).
//! `STROBE_FAULT` ([`crate::fault`]) injects a loss to test all of this.
//!
//! ## Shutdown order
//!
//! GL objects must be released while the context is current and before
//! the windows and display they came from, so [`Running`] keeps them in a
//! [`Stack`] that declares its fields in that order and [`Stack::release`]
//! frees the scene and the egui painter explicitly first. A lost context
//! skips the explicit frees: the driver reclaims them.

use crate::audio::AudioInput;
use crate::cli::{AudioArg, Cli};
use crate::console::Console;
use crate::failures;
use crate::fault::{self, Faults};
use crate::gldebug;
use crate::library::Library;
use crate::mouse::{Button, MouseTracker};
use crate::perf::{self, Performance};
use crate::recovery::{Decision, Event, Rebuild, Recovery, Surface};
use crate::screenshot::{Screenshot, Target};
use crate::session::{self, Session};
use crate::stats::FrameStats;
use crate::ui::{Action, Model, PerfPanel, Preview, UiHost, layout};
use std::time::{Duration, Instant};
use strobe_diag::context;
use strobe_diag::failure::{self, Failure, Subsystem};
use strobe_gfx::{GlChoice, GpuTimer, SwapWindow, WindowedContext};
use strobe_scene::control::def::Values;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{Key, NamedKey};
use winit::monitor::MonitorHandle;
use winit::window::{Fullscreen, Window, WindowAttributes, WindowId};

/// Minimum time between output frames: a 144 Hz cap.
const OUTPUT_PERIOD: Duration = Duration::from_micros(6_944);
/// Time between control-window repaints: 30 Hz.
const CONTROL_PERIOD: Duration = Duration::from_millis(33);
/// How long a window size must hold still before the scene is resized.
const RESIZE_SETTLE: Duration = Duration::from_millis(150);
/// How often the monitor list is re-read.
const MONITOR_REFRESH: Duration = Duration::from_secs(2);
/// The frame step in screenshot mode, so captures are reproducible.
const SCREENSHOT_DT: f32 = 1.0 / 60.0;
/// Scene time at the first screenshot frame, so scenes are not at t = 0.
const SCREENSHOT_START: f32 = 2.0;
/// Where a moved, windowed output lands relative to the monitor's corner.
const MONITOR_INSET: i32 = 40;
/// What a `STROBE_FAULT` loss makes every `make_current` say.
const INJECTED_LOSS: &str =
    "make current: injected fault (STROBE_FAULT lose-context): the context is lost";

/// The application, before and after its windows exist.
pub struct App {
    cli: Cli,
    library: Option<Library>,
    scan_error: Option<String>,
    running: Option<Running>,
    failure: Option<String>,
}

impl App {
    /// An app that will open its windows when the event loop resumes.
    pub fn new(cli: Cli, library: Library, scan_error: Option<String>) -> Self {
        Self {
            cli,
            library: Some(library),
            scan_error,
            running: None,
            failure: None,
        }
    }

    /// Why the app stopped early, if it did.
    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    /// Stops the loop if the running app asked to or failed, and carries
    /// out a context rebuild it asked for. Runs after every event, so a
    /// rebuild decided during a redraw happens before the next one.
    fn settle(&mut self, event_loop: &ActiveEventLoop) {
        let Some(running) = self.running.as_mut() else {
            return;
        };
        if let Some(failure) = running.failure.take() {
            self.failure = Some(failure);
            event_loop.exit();
        } else if let Some(rebuild) = running.rebuild.take() {
            let Some(running) = self.running.take() else {
                return;
            };
            match running.recover(event_loop, rebuild) {
                Ok(running) => self.running = Some(running),
                Err(message) => {
                    self.failure = Some(message);
                    event_loop.exit();
                }
            }
        } else if running.finished() {
            event_loop.exit();
        }
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_some() || self.failure.is_some() {
            return;
        }
        let library = self.library.take().unwrap_or_default();
        match Running::new(event_loop, &self.cli, library, self.scan_error.take()) {
            Ok(running) => self.running = Some(running),
            Err(message) => {
                self.failure = Some(message);
                event_loop.exit();
            }
        }
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if let Some(running) = self.running.as_mut() {
            running.window_event(event_loop, id, event);
        }
        self.settle(event_loop);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(running) = self.running.as_mut() {
            let wake = running.schedule(event_loop);
            event_loop.set_control_flow(ControlFlow::WaitUntil(wake));
        }
        self.settle(event_loop);
    }

    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(running) = self.running.take() {
            running.shutdown();
        }
    }
}

/// Where the two windows go, and how they start.
#[derive(Debug, Clone)]
struct Layout {
    output: WindowAttributes,
    control: WindowAttributes,
    /// The output's fullscreen state, applied after the window exists.
    fullscreen: Option<Fullscreen>,
}

impl Layout {
    /// The layout of a fresh launch.
    fn initial() -> Self {
        Self {
            output: Window::default_attributes()
                .with_title("Strobe Output")
                .with_inner_size(LogicalSize::new(960.0, 540.0)),
            control: Window::default_attributes()
                .with_title("Strobe")
                .with_inner_size(LogicalSize::new(1360.0, 820.0))
                // Always maximized, on every launch: the control surface is
                // the operator's workspace. The size above is what an
                // un-maximized window (or a window system with no concept of
                // maximizing, such as bare Xvfb) falls back to.
                .with_maximized(true)
                .with_min_inner_size(LogicalSize::new(900.0, 560.0)),
            fullscreen: None,
        }
    }

    /// The layout the live windows are in now, so a rebuilt pair can be put
    /// back where the operator left them.
    fn capture(stack: &Stack) -> Self {
        let mut layout = Self::initial();
        let output = stack.output.window();
        layout.fullscreen = output.fullscreen();
        // A fullscreen window's size is the monitor's, not the window's own.
        if layout.fullscreen.is_none() {
            layout.output = layout.output.with_inner_size(output.inner_size());
            if let Ok(position) = output.outer_position() {
                layout.output = layout.output.with_position(position);
            }
        }
        let control = stack.control.window();
        let maximized = control.is_maximized();
        layout.control = layout.control.with_maximized(maximized);
        if !maximized {
            layout.control = layout.control.with_inner_size(control.inner_size());
            if let Ok(position) = control.outer_position() {
                layout.control = layout.control.with_position(position);
            }
        }
        layout
    }
}

/// The performance settings a context starts with.
#[derive(Debug, Clone, Copy)]
struct PerfStart {
    /// The render-scale ceiling.
    max: f32,
    /// Whether adaptive resolution is on.
    adaptive: bool,
}

/// Everything that is bound to one GL context: the scene, the timers, egui,
/// the two windows and the context itself.
///
/// Field order is drop order for the GL-owning fields: scene, timers, egui,
/// the windows, and last the context and display they share. A rebuild
/// replaces the whole stack; [`Stack::release`] takes one down.
struct Stack {
    session: Option<Session>,
    /// Owns GL timer queries, so it sits before the context in drop order.
    perf: Performance,
    ui: UiHost,
    control: SwapWindow,
    output: SwapWindow,
    gfx: WindowedContext,
}

impl Stack {
    /// Opens a context on `choice` with its two windows: the output window
    /// first (the context is made on it), then the control window, vsync,
    /// driver debug output, the session-context facts, egui and the GPU
    /// timer. The scene is loaded afterwards by the caller.
    fn open(
        event_loop: &ActiveEventLoop,
        choice: GlChoice,
        layout: &Layout,
        start: PerfStart,
    ) -> Result<Self, String> {
        let (mut gfx, output) = WindowedContext::open(event_loop, choice, &layout.output)
            .map_err(|e| format!("Could not start OpenGL.\n{e}"))?;
        // Before the Gl is shared with a second window or egui; see
        // `strobe_gfx::gl::debug`.
        gldebug::enable(gfx.gl_mut());
        record_gl_facts(&gfx);
        let control = gfx
            .add_window(event_loop, layout.control.clone())
            .map_err(|e| format!("Could not open the control window: {e}"))?;
        // Output waits for vertical blank; control must not, or two vsynced
        // swaps in one loop would halve the frame rate.
        if let Err(e) = gfx.set_vsync(&output, true) {
            log::warn!("output vsync: {e}");
        }
        if let Err(e) = gfx.set_vsync(&control, false) {
            log::warn!("control vsync: {e}");
        }
        gfx.make_current(&control)?;
        let ui = UiHost::new(event_loop, gfx.gl())?;
        if let Some(fullscreen) = layout.fullscreen.clone() {
            output.window().set_fullscreen(Some(fullscreen));
        }

        let refresh = output
            .window()
            .current_monitor()
            .and_then(|monitor| monitor.refresh_rate_millihertz());
        let perf = Performance::new(
            GpuTimer::new(gfx.gl(), gfx.caps()),
            start.max,
            start.adaptive,
            perf::budget_for(refresh),
        );
        log::info!(
            "GPU timing {}; frame budget {:.2} ms ({}); adaptive resolution {}",
            if perf.has_timer() {
                "available"
            } else {
                "unavailable"
            },
            perf.budget_ms(),
            refresh.map_or_else(
                || "monitor refresh rate unknown, assuming 60 Hz".to_string(),
                |mhz| format!("{:.1} Hz", mhz as f32 / 1000.0)
            ),
            if perf.adapting() { "on" } else { "off" },
        );
        Ok(Self {
            session: None,
            perf,
            ui,
            control,
            output,
            gfx,
        })
    }

    /// One line naming the backend, renderer and driver version.
    fn summary(&self) -> String {
        let info = &self.gfx.caps().info;
        format!(
            "{} | {} | {}",
            self.gfx.backend(),
            info.renderer,
            info.version
        )
    }

    /// Takes the stack down in dependency order. With a live context the
    /// scene, timers and egui painter free their GL objects first. With a
    /// lost one (or one that will not become current) the `Gl` is marked
    /// lost and nothing calls into it: the driver reclaims a destroyed
    /// context's objects, and a delete against a dead one can fault.
    fn release(self) {
        let Self {
            session,
            mut perf,
            mut ui,
            control,
            output,
            gfx,
        } = self;
        if !gfx.gl().is_lost() && gfx.make_current(&control).is_err() {
            gfx.gl().mark_lost();
        }
        if gfx.gl().is_lost() {
            drop(session);
            drop(perf);
            ui.abandon();
        } else {
            drop(session);
            perf.release();
            drop(perf);
            ui.destroy();
            drop(ui);
        }
        drop(control);
        drop(output);
        drop(gfx);
    }
}

/// The scene that was playing when the context was lost.
struct Resume {
    /// Its index in the library.
    index: usize,
    /// Its clock, in seconds.
    time: f32,
    /// The operator's control values.
    controls: Vec<(usize, Values)>,
}

/// What a rebuild carries over from the context it replaces.
struct Carried {
    layout: Layout,
    playing: Option<Resume>,
    scale: f32,
    start: PerfStart,
}

/// Everything that exists once the windows are open.
struct Running {
    stack: Stack,

    library: Library,
    playing: Option<usize>,
    console: Console,
    stats: FrameStats,
    gl_summary: String,
    audio: AudioInput,
    mouse: MouseTracker,

    /// The render scale in force: the adaptive controller's, or the
    /// slider's when adaptation is off.
    render_scale: f32,
    /// The slider's working value, which is the maximum scale.
    scale_edit: f32,
    pending_resize: Option<([i32; 2], Instant)>,
    resize_now: bool,

    monitors: Vec<MonitorHandle>,
    monitor_labels: Vec<String>,
    output_monitor: Option<usize>,
    last_monitor_refresh: Instant,

    next_output: Instant,
    next_control: Instant,
    actions: Vec<Action>,

    screenshot: Option<Screenshot>,
    output_frames: u64,
    start_time: f32,
    failure: Option<String>,

    /// What to do about failed frames and resets.
    recovery: Recovery,
    /// `STROBE_FAULT`, read once.
    faults: Faults,
    /// A fault-injected loss is in force: every `make_current` fails until
    /// the context is rebuilt.
    context_dead: bool,
    /// A rebuild the policy decided on, for [`App::settle`] to carry out.
    rebuild: Option<Rebuild>,
}

impl Running {
    fn new(
        event_loop: &ActiveEventLoop,
        cli: &Cli,
        library: Library,
        scan_error: Option<String>,
    ) -> Result<Self, String> {
        let render_scale = session::clamp_scale(cli.render_scale);
        let stack = Stack::open(
            event_loop,
            cli.gl.choice(),
            &Layout::initial(),
            PerfStart {
                max: render_scale,
                adaptive: !cli.no_adaptive,
            },
        )?;
        let gl_summary = stack.summary();
        let screenshot = cli
            .screenshot
            .clone()
            .map(|base| Screenshot::new(base, cli.frames));
        let now = Instant::now();
        let recovery = Recovery::new(cli.gl.choice(), stack.gfx.backend());
        let mut running = Self {
            stack,
            library,
            playing: None,
            console: Console::default(),
            stats: FrameStats::default(),
            gl_summary,
            audio: AudioInput::new(cli.audio == AudioArg::Auto),
            mouse: MouseTracker::default(),
            render_scale,
            scale_edit: render_scale,
            pending_resize: None,
            resize_now: false,
            monitors: Vec::new(),
            monitor_labels: Vec::new(),
            output_monitor: None,
            last_monitor_refresh: now,
            next_output: now,
            next_control: now,
            actions: Vec::new(),
            start_time: if screenshot.is_some() {
                SCREENSHOT_START
            } else {
                0.0
            },
            screenshot,
            output_frames: 0,
            failure: None,
            recovery,
            faults: Faults::from_env(),
            context_dead: false,
            rebuild: None,
        };
        running.refresh_monitors(event_loop);
        log::info!("GL: {}", running.gl_summary);
        if let Some(error) = scan_error {
            failure::report(
                Failure::new(Subsystem::Io, "scan")
                    .path(&running.library.dir)
                    .message(error)
                    .action("the scene list is empty"),
            );
        }
        if let Some(query) = cli.scene.as_deref() {
            match running.library.find(query) {
                Some(index) => running.load(index),
                None => failure::report(
                    Failure::new(Subsystem::Io, "find")
                        .asset(query)
                        .message(format!("no scene matching \"{query}\" in the library"))
                        .action("no scene loaded"),
                ),
            }
        }
        Ok(running)
    }

    fn finished(&self) -> bool {
        self.screenshot.as_ref().is_some_and(Screenshot::finished)
    }

    fn shutdown(self) {
        self.stack.release();
    }

    // ------------------------------------------------------- context loss

    /// Makes the context current on `surface`'s window. On failure the
    /// recovery policy hears of it and the caller skips the frame.
    fn make_current(&mut self, surface: Surface) -> bool {
        let result = if self.context_dead {
            Err(INJECTED_LOSS.to_string())
        } else {
            let target = match surface {
                Surface::Output => &self.stack.output,
                Surface::Control => &self.stack.control,
            };
            self.stack.gfx.make_current(target)
        };
        match result {
            Ok(()) => true,
            // The context is not known to be current, so the reset status
            // is not asked for: a GL call without a current context can
            // fault inside the driver.
            Err(error) => {
                self.context_failed(surface, &error, None);
                false
            }
        }
    }

    /// Presents `surface`'s back buffer; the context must be current on it.
    /// A swap failure asks the driver for the reset status (the context is
    /// current, so it is safe to) and tells the policy.
    fn present(&mut self, surface: Surface, now: Instant) {
        let result = {
            let target = match surface {
                Surface::Output => &self.stack.output,
                Surface::Control => &self.stack.control,
            };
            self.stack.gfx.swap_buffers(target)
        };
        match result {
            Ok(()) => {
                self.recovery.handle(now, Event::Presented(surface));
            }
            Err(error) => {
                let gl = self.stack.gfx.gl();
                let reset = gl.has_reset_status().then(|| gl.reset_status());
                self.context_failed(surface, &error, reset);
            }
        }
    }

    /// Reports a failed context call to the policy and acts on its answer.
    fn context_failed(
        &mut self,
        surface: Surface,
        error: &str,
        reset: Option<strobe_gfx::gl::ResetStatus>,
    ) {
        let decision = self.recovery.handle(
            Instant::now(),
            Event::Failed {
                window: surface,
                error,
                reset,
            },
        );
        match decision {
            Decision::RetryNextFrame => {
                log::warn!("{surface} window: {error}; skipping the frame and retrying");
            }
            Decision::Rebuild(rebuild) => self.begin_rebuild(rebuild),
            Decision::GiveUp(reason) => self.give_up(&reason),
            Decision::Continue => {}
        }
    }

    /// Health-checks the driver's reset status, at most once a second, on a
    /// robust context that can be asked. Returns whether a rebuild was
    /// decided (the frame should then be abandoned).
    fn check_reset(&mut self, now: Instant) -> bool {
        let gl = self.stack.gfx.gl();
        if !self.stack.gfx.robust() || !gl.has_reset_status() || !self.recovery.poll_due(now) {
            return false;
        }
        let status = gl.reset_status();
        match self.recovery.handle(
            now,
            Event::Reset {
                window: Surface::Output,
                status,
            },
        ) {
            Decision::Rebuild(rebuild) => {
                self.begin_rebuild(rebuild);
                true
            }
            Decision::GiveUp(reason) => {
                self.give_up(&reason);
                true
            }
            Decision::Continue | Decision::RetryNextFrame => false,
        }
    }

    /// Records the loss and asks [`App::settle`] to rebuild. The `Gl` is
    /// marked lost right away, so nothing touches it in the meantime.
    fn begin_rebuild(&mut self, rebuild: Rebuild) {
        if self.rebuild.is_some() {
            return;
        }
        let scene = self.stack.session.as_ref().map(|s| s.name.clone());
        failure::report(failures::context_lost(
            &rebuild,
            self.stack.gfx.backend(),
            scene.as_deref(),
        ));
        self.stack.gfx.gl().mark_lost();
        self.rebuild = Some(rebuild);
    }

    /// Ends the run through the existing failure path (a log record and
    /// the startup-failure dialog).
    fn give_up(&mut self, reason: &str) {
        failure::report(
            Failure::new(Subsystem::Context, "recover")
                .detail("backend", failures::backend_label(self.stack.gfx.backend()))
                .message(reason)
                .action("giving up; the app stops"),
        );
        self.stack.gfx.gl().mark_lost();
        self.failure = Some(format!(
            "The graphics context was lost and could not be recovered.\n{reason}"
        ));
    }

    /// `STROBE_FAULT`: the context dies as if the driver had reset it.
    fn inject_loss(&mut self) {
        log::warn!(
            "{}: injected context loss after {} output frames",
            fault::ENV,
            self.output_frames
        );
        self.stack.gfx.gl().mark_lost();
        self.context_dead = true;
    }

    /// Replaces the lost context: drops everything bound to it, builds a
    /// new context and windows on `first.backend` (or, if that fails and the
    /// policy says so, the next backend), and puts the show back. See the
    /// module docs for the steps. An `Err` is the final failure message.
    fn recover(mut self, event_loop: &ActiveEventLoop, first: Rebuild) -> Result<Self, String> {
        let started = Instant::now();
        let before = self.stack.gfx.backend();
        let scene = self.stack.session.as_ref().map(|s| s.name.clone());
        let carried = Carried {
            layout: Layout::capture(&self.stack),
            playing: self.playing.map(|index| {
                let session = self.stack.session.as_ref();
                Resume {
                    index,
                    time: session.map_or(self.start_time, Session::time),
                    controls: session.map_or_else(Vec::new, Session::control_targets),
                }
            }),
            scale: self.render_scale,
            start: PerfStart {
                max: self.stack.perf.max(),
                adaptive: self.stack.perf.enabled(),
            },
        };

        self.stack.gfx.gl().mark_lost();
        let old = self.stack;
        old.release();

        let mut attempt = first;
        let (stack, done) = loop {
            let backend = attempt.backend;
            log::info!(
                "rebuilding the GL context on {}{}",
                failures::backend_label(backend),
                if attempt.failover { " (fail-over)" } else { "" }
            );
            let opened = if self.faults.rebuild_fails(backend) {
                Err(format!(
                    "injected fault ({}): rebuilding on {} fails",
                    fault::ENV,
                    failures::backend_label(backend)
                ))
            } else {
                Stack::open(
                    event_loop,
                    GlChoice::Only(backend),
                    &carried.layout,
                    carried.start,
                )
            };
            match opened {
                Ok(stack) => {
                    self.recovery.handle(
                        Instant::now(),
                        Event::Rebuilt {
                            backend,
                            error: None,
                        },
                    );
                    break (stack, attempt);
                }
                Err(error) => {
                    let decision = self.recovery.handle(
                        Instant::now(),
                        Event::Rebuilt {
                            backend,
                            error: Some(&error),
                        },
                    );
                    failure::report(failures::context_rebuild_failed(
                        backend, &error, before, &decision,
                    ));
                    match decision {
                        Decision::Rebuild(next) => attempt = next,
                        Decision::GiveUp(reason) => {
                            return Err(format!(
                                "The graphics context was lost and could not be recovered.\n{reason}"
                            ));
                        }
                        Decision::Continue | Decision::RetryNextFrame => {
                            return Err(format!(
                                "The graphics context was lost and could not be recovered.\n{error}"
                            ));
                        }
                    }
                }
            }
        };

        let now = Instant::now();
        let mut running = Self {
            gl_summary: stack.summary(),
            stack,
            context_dead: false,
            rebuild: None,
            pending_resize: None,
            resize_now: false,
            next_output: now,
            next_control: now,
            ..self
        };
        log::info!("GL: {}", running.gl_summary);
        running.stack.perf.resume_scale(carried.scale);
        running.refresh_monitors(event_loop);
        let reloaded = match carried.playing {
            Some(resume) => {
                running.load_from(resume.index, resume.time);
                if let Some(session) = running.stack.session.as_mut() {
                    session.restore_control_targets(&resume.controls);
                }
                running.stack.session.is_some()
            }
            None => false,
        };
        let elapsed = started.elapsed();
        failure::report(failures::context_rebuilt(
            &done,
            before,
            elapsed.as_millis(),
            scene.as_deref(),
            reloaded,
        ));
        log::info!(
            "Output recovered on {} in {} ms{}",
            failures::backend_label(running.stack.gfx.backend()),
            elapsed.as_millis(),
            match (&scene, reloaded) {
                (Some(name), true) => format!("; {name} reloaded"),
                (Some(name), false) => format!("; {name} could not be reloaded"),
                (None, _) => String::new(),
            }
        );
        Ok(running)
    }

    // ----------------------------------------------------------- scheduling

    /// Requests redraws for whichever windows are due; returns when to wake.
    fn schedule(&mut self, event_loop: &ActiveEventLoop) -> Instant {
        let now = Instant::now();
        if self.rebuild.is_some() {
            // `App::settle` runs right after this and rebuilds.
            return now;
        }
        if now.duration_since(self.last_monitor_refresh) >= MONITOR_REFRESH {
            self.refresh_monitors(event_loop);
        }
        if now >= self.next_output {
            self.stack.output.window().request_redraw();
            self.next_output = now + OUTPUT_PERIOD;
        }
        if now >= self.next_control {
            self.stack.control.window().request_redraw();
            self.next_control = now + CONTROL_PERIOD;
        }
        self.next_output.min(self.next_control)
    }

    fn refresh_monitors(&mut self, event_loop: &ActiveEventLoop) {
        self.last_monitor_refresh = Instant::now();
        // The output may have moved to a display with another refresh rate.
        let refresh = self
            .stack
            .output
            .window()
            .current_monitor()
            .and_then(|monitor| monitor.refresh_rate_millihertz());
        if self.stack.perf.set_refresh(refresh) {
            log::info!(
                "frame budget now {:.2} ms for the output's monitor",
                self.stack.perf.budget_ms()
            );
        }
        self.monitors = event_loop.available_monitors().collect();
        self.monitor_labels = self
            .monitors
            .iter()
            .enumerate()
            .map(|(index, monitor)| {
                let size = monitor.size();
                format!(
                    "{}: {} ({}x{})",
                    index + 1,
                    monitor.name().unwrap_or_else(|| "Display".to_string()),
                    size.width,
                    size.height
                )
            })
            .collect();
    }

    // --------------------------------------------------------------- events

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if matches!(event, WindowEvent::CloseRequested) {
            event_loop.exit();
            return;
        }
        if self.rebuild.is_some() {
            // The windows are about to be replaced.
            return;
        }
        let is_control = id == self.stack.control.window().id();
        let is_output = id == self.stack.output.window().id();
        if matches!(event, WindowEvent::Resized(_)) {
            if is_control {
                self.stack.gfx.resize(&self.stack.control);
            } else if is_output {
                self.stack.gfx.resize(&self.stack.output);
            }
        } else if matches!(event, WindowEvent::RedrawRequested) {
            if is_output {
                self.redraw_output();
            } else if is_control {
                self.redraw_control();
            }
        } else if is_output && let WindowEvent::CursorMoved { position, .. } = &event {
            self.mouse.moved([position.x as f32, position.y as f32]);
        } else if is_output && let WindowEvent::MouseInput { state, button, .. } = &event {
            let button = match button {
                MouseButton::Left => Some(Button::Left),
                MouseButton::Right => Some(Button::Right),
                MouseButton::Middle => Some(Button::Middle),
                MouseButton::Back | MouseButton::Forward | MouseButton::Other(_) => None,
            };
            if let Some(button) = button {
                self.mouse.button(button, *state == ElementState::Pressed);
            }
        } else if let WindowEvent::KeyboardInput { event: key, .. } = &event {
            let typing = is_control && self.stack.ui.wants_keyboard();
            if !typing {
                self.key(key);
            }
        }
        if is_control && self.rebuild.is_none() {
            let window = self.stack.control.window();
            let _ = self.stack.ui.on_window_event(window, &event);
        }
    }

    fn key(&mut self, key: &KeyEvent) {
        if key.state != ElementState::Pressed || key.repeat {
            return;
        }
        if key.logical_key == Key::Named(NamedKey::F11) {
            self.toggle_fullscreen();
        } else if matches!(&key.logical_key, Key::Character(text) if text.eq_ignore_ascii_case("t"))
        {
            self.audio.tap();
        } else if key.logical_key == Key::Named(NamedKey::Escape)
            && self.stack.output.window().fullscreen().is_some()
        {
            self.stack.output.window().set_fullscreen(None);
        }
    }

    fn toggle_fullscreen(&mut self) {
        let window = self.stack.output.window();
        if window.fullscreen().is_some() {
            window.set_fullscreen(None);
        } else {
            let monitor = self
                .output_monitor
                .and_then(|index| self.monitors.get(index))
                .cloned();
            window.set_fullscreen(Some(Fullscreen::Borderless(monitor)));
        }
    }

    fn move_to_monitor(&mut self, index: usize) {
        let Some(monitor) = self.monitors.get(index).cloned() else {
            return;
        };
        self.output_monitor = Some(index);
        let window = self.stack.output.window();
        if window.fullscreen().is_some() {
            window.set_fullscreen(Some(Fullscreen::Borderless(Some(monitor))));
        } else {
            let corner = monitor.position();
            window.set_outer_position(PhysicalPosition::new(
                corner.x + MONITOR_INSET,
                corner.y + MONITOR_INSET,
            ));
        }
    }

    // ----------------------------------------------------------- scene load

    fn load(&mut self, index: usize) {
        self.load_from(index, self.start_time);
    }

    /// Loads scene `index` with its clock starting at `start` seconds.
    fn load_from(&mut self, index: usize, start: f32) {
        let Some(entry) = self.library.entries.get(index).cloned() else {
            return;
        };
        if !self.make_current(Surface::Output) {
            return;
        }
        let window = self.stack.output.window().inner_size();
        let size = session::render_size(
            [window.width, window.height],
            self.render_scale,
            self.stack.gfx.caps().info.max_texture_size,
        );
        let started = Instant::now();
        match Session::load(
            self.stack.gfx.gl(),
            self.stack.gfx.caps(),
            &entry.dir,
            &entry.name,
            size,
            start,
            &mut self.audio,
        ) {
            Ok(session) => {
                self.stack.session = Some(session);
                self.playing = Some(index);
                self.pending_resize = None;
                self.stack.perf.scene_loaded(Instant::now());
                context::set(context::SCENE, &entry.name);
                context::set(context::RENDER_SIZE, format!("{}x{}", size[0], size[1]));
                log::info!(
                    "Loaded {} ({}x{}) in {:.0} ms",
                    entry.name,
                    size[0],
                    size[1],
                    started.elapsed().as_secs_f64() * 1000.0
                );
            }
            Err(error) => failure::report(failures::scene_load(
                &entry.name,
                &entry.dir,
                &error,
                self.stack.session.is_some(),
            )),
        }
    }

    fn apply(&mut self, actions: &mut Vec<Action>) {
        for action in actions.drain(..) {
            match action {
                Action::Load(index) => self.load(index),
                Action::Reload => {
                    if let Some(index) = self.playing {
                        self.load(index);
                    }
                }
                Action::SetRenderScale(scale) => {
                    let from = self.render_scale;
                    self.scale_edit = session::clamp_scale(scale);
                    self.stack.perf.set_max(self.scale_edit);
                    self.render_scale = self.stack.perf.scale();
                    self.resize_now = true;
                    log::info!(
                        "{} set to {:.2} (was {:.2})",
                        if self.stack.perf.adapting() {
                            "maximum render scale"
                        } else {
                            "render scale"
                        },
                        self.render_scale,
                        from
                    );
                }
                Action::SetAdaptive(on) => {
                    self.stack.perf.set_enabled(on);
                    self.render_scale = self.stack.perf.scale();
                    self.resize_now = true;
                    log::info!(
                        "adaptive resolution {}{}",
                        if on { "on" } else { "off" },
                        if on && !self.stack.perf.has_timer() {
                            " (but this context has no GPU timer, so the scale stays manual)"
                        } else {
                            ""
                        }
                    );
                }
                Action::ToggleFullscreen => self.toggle_fullscreen(),
                Action::MoveToMonitor(index) => self.move_to_monitor(index),
                Action::ClearConsole => self.console.clear(),
                Action::SelectAudioInput(choice) => self.audio.select_device(choice),
                Action::RefreshAudioDevices => self.audio.refresh_devices(),
                Action::SetReactivity(gain) => self.audio.set_reactivity(gain),
                Action::SetAudioSpeed(gain) => self.audio.set_speed(gain),
                Action::TapTempo => self.audio.tap(),
            }
        }
    }

    // ------------------------------------------------------------- drawing

    /// Resizes the scene to the window times the render scale, once the
    /// size has settled (or at once after a scale change).
    fn settle_resize(&mut self, window: [u32; 2], now: Instant) {
        let want = session::render_size(
            window,
            self.render_scale,
            self.stack.gfx.caps().info.max_texture_size,
        );
        let Some(session) = self.stack.session.as_mut() else {
            return;
        };
        if session.player.render_size() == want {
            self.pending_resize = None;
            return;
        }
        let steady = matches!(
            self.pending_resize,
            Some((size, since)) if size == want && now.duration_since(since) >= RESIZE_SETTLE
        );
        if steady || self.resize_now || self.screenshot.is_some() {
            self.resize_now = false;
            self.pending_resize = None;
            match session.player.resize(want) {
                Ok(()) => context::set(context::RENDER_SIZE, format!("{}x{}", want[0], want[1])),
                Err(error) => failure::report(
                    Failure::new(Subsystem::Context, "resize")
                        .scene(&session.name)
                        .message(error)
                        .action("keeping the previous render size"),
                ),
            }
        } else if !matches!(self.pending_resize, Some((size, _)) if size == want) {
            self.pending_resize = Some((want, now));
        }
    }

    fn redraw_output(&mut self) {
        let started = Instant::now();
        let size = self.stack.output.window().inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        if self.faults.lose_context_due(self.output_frames) {
            self.inject_loss();
        }
        if !self.make_current(Surface::Output) {
            return;
        }
        let measured = self.stats.begin(started);
        let dt = if self.screenshot.is_some() {
            SCREENSHOT_DT
        } else {
            measured
        };
        let window = [size.width as i32, size.height as i32];
        self.settle_resize([size.width, size.height], started);

        let gl = self.stack.gfx.gl();
        match self.stack.session.as_mut() {
            Some(session) => {
                let render = session.player.render_size();
                let mouse = self.mouse.uniforms([size.width, size.height], render);
                // The GPU timer brackets the scene's work and the blit to
                // the output; its result arrives a frame or two later.
                self.stack.perf.begin();
                session.advance(dt, &mut self.audio, mouse);
                gl.plain_state();
                session.player.present(window);
                if let Some(change) = self.stack.perf.end(started) {
                    log::info!(
                        "adaptive resolution: render scale {:.2} -> {:.2} (GPU {:.1} ms against a {:.1} ms budget)",
                        change.from,
                        change.to,
                        change.gpu_ms,
                        change.budget_ms
                    );
                    self.render_scale = change.to;
                    self.resize_now = true;
                }
            }
            None => {
                gl.plain_state();
                gl.bind_framebuffer(None);
                gl.viewport(window[0], window[1]);
                gl.clear([0.0, 0.0, 0.0, 1.0]);
            }
        }
        self.output_frames += 1;
        context::note_frame(self.output_frames);
        if let Some(shot) = self.screenshot.as_mut()
            && shot.output_due(self.output_frames)
        {
            let pixels = gl.read_default_rgba8(window[0], window[1]);
            match shot.save(Target::Output, [size.width, size.height], pixels) {
                Ok(path) => log::info!("wrote {}", path.display()),
                Err(e) => self.failure = Some(e),
            }
        }
        self.stats.finish(started.elapsed());
        if self.check_reset(started) {
            return;
        }
        self.present(Surface::Output, started);
    }

    fn redraw_control(&mut self) {
        let size = self.stack.control.window().inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        if !self.make_current(Surface::Control) {
            return;
        }
        self.console.refresh();
        self.audio.refresh_status(Instant::now());
        let preview = self.stack.session.as_ref().map(|session| {
            let texture = session.player.output_texture();
            Preview {
                texture: self.stack.ui.preview_texture(texture.raw()),
                aspect: texture.width() as f32 / texture.height().max(1) as f32,
                size: [texture.width(), texture.height()],
            }
        });
        let (scene_title, controls, script) = match self.stack.session.as_mut() {
            Some(session) => {
                let (title, controls, script) = session.panel_parts();
                (title, Some(controls), script)
            }
            None => ("", None, None),
        };
        let mut model = Model {
            library: &self.library,
            playing: self.playing,
            scene_title,
            controls,
            preview,
            console: &self.console,
            stats: &self.stats,
            render_scale: &mut self.scale_edit,
            monitors: &self.monitor_labels,
            output_monitor: self.output_monitor,
            fullscreen: self.stack.output.window().fullscreen().is_some(),
            gl_summary: &self.gl_summary,
            audio: self.audio.panel(),
            script,
            perf: PerfPanel {
                adaptive: self.stack.perf.enabled(),
                has_timer: self.stack.perf.has_timer(),
                gpu_ms: self.stack.perf.gpu_ms(),
                budget_ms: self.stack.perf.budget_ms(),
                scale: self.render_scale,
            },
        };
        let mut actions = std::mem::take(&mut self.actions);
        let window = self.stack.control.window();
        self.stack
            .ui
            .run(window, |ui| layout::show(ui, &mut model, &mut actions));
        self.stack.ui.paint(window, crate::theme::clear_color());

        if let Some(shot) = self.screenshot.as_mut()
            && shot.control_due()
        {
            let pixels = self
                .stack
                .gfx
                .gl()
                .read_default_rgba8(size.width as i32, size.height as i32);
            match shot.save(Target::Control, [size.width, size.height], pixels) {
                Ok(path) => log::info!("wrote {}", path.display()),
                Err(e) => self.failure = Some(e),
            }
        }
        self.present(Surface::Control, Instant::now());
        if self.rebuild.is_none() {
            self.apply(&mut actions);
        }
        self.actions = actions;
    }
}

/// Writes the GL context's facts into the session context, so every crash
/// report and `--diagnose` run says which driver was in use.
fn record_gl_facts(gfx: &WindowedContext) {
    let info = &gfx.caps().info;
    context::set(context::GL_VENDOR, &info.vendor);
    context::set(context::GL_RENDERER, &info.renderer);
    context::set(context::GL_VERSION, &info.version);
    context::set(context::GL_BACKEND, failures::backend_label(gfx.backend()));
    context::set("gl.robust", gfx.robust());
}
