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
//! `about_to_wait` decides what is due and either requests a redraw or does
//! the work itself; drawing happens one window at a time, with the shared
//! context made current on that window's surface first.
//!
//! - **The scene frame** (audio, control tick, `ScenePlayer::render`, the GPU
//!   timer, adaptive scale) runs on `next_output`'s schedule whether or not
//!   the output window can be seen (D96). The output window **starts hidden**
//!   (`--show-output` shows it), and a hidden window may never get a
//!   `RedrawRequested`, so the schedule decides who drives the frame:
//!   - *shown*: a redraw is requested up to [`OUTPUT_PERIOD`] apart (a
//!     144 Hz cap that only matters where vsync is not working). The redraw
//!     runs the scene frame on the output surface, then `present` (the
//!     letterboxed blit) and the swap. Vsync is on, so the swap paces this
//!     loop to the display.
//!   - *hidden*: `about_to_wait` runs the scene frame itself, on the
//!     control window's surface, one refresh period of the output monitor
//!     apart (60 Hz if unknown). There is no present and no swap, so no
//!     vsync pacing, and a frame is skipped while the GPU timer ring is
//!     full (`Performance::backlogged`) so the queue cannot grow.
//! - **Control window**, [`CONTROL_PERIOD`] apart (30 Hz): the preview
//!   texture, one egui pass, paint, swap. It has vsync off so two vsynced
//!   swaps cannot halve the output rate. The throttle matters on an old
//!   laptop, where egui's tessellation and the extra swap are a real cost
//!   and 30 Hz is plenty for sliders. While the fullscreen output covers
//!   it, it redraws at 2 Hz instead (`update_covered`, TORNADO T10), and a
//!   frame Windows did not paint is not swapped (`presents_control`, D508).
//!   While it shows another Strobe's window (the desktop viewer) it redraws
//!   for a new image, a change of stage or the person's input, and every
//!   [`VIEWER_PERIOD`] otherwise (D302). The frame normally runs on the control window's `RedrawRequested`,
//!   which winit delivers on Windows only from a `WM_PAINT`. Windows makes
//!   that message for the topmost dirty window first, so an output above
//!   the control window that is dirty again every pass takes every paint.
//!   When a request has gone unanswered for `stats::PAINT_PATIENCE`,
//!   `about_to_wait` runs the control frame itself, at the unseen rate,
//!   until a paint answers promptly again ([`ControlDrive`], D290). Actions,
//!   the remote passes, the desktop capture and the playlist compile all
//!   live in that frame, so a show with nobody at the console needs it.
//!   While the window is minimised the frame draws nothing and runs only
//!   those parts that do not draw (`control_chores`, D314), and so do the
//!   loop-driven frames of a window covered by the fullscreen output, but
//!   for a full draw every `COVERED_HEARTBEAT` (D315).
//! - **A perf line** is logged once a minute (`stats::PerfWindow`): frame
//!   rate, late frames, the worst gap, GPU time, the render scale and the
//!   control window's redraw cost, so a soak's log alone says where frames
//!   went.
//!
//! The scene is never rendered inside the control window's redraw, so a slow
//! control window cannot slow the show down. The control window's preview
//! paints the scene's output texture, so it works with the output hidden.
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
//! 4. the output's monitor, fullscreen and shown/hidden state and the
//!    control window's maximized state come back, the render scale carries over, and the
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
use crate::commands::{self, Bindings, CommandId, Focus};
use crate::console::Console;
use crate::editor;
use crate::failures;
use crate::fault::{self, Faults};
use crate::generate;
use crate::gldebug;
use crate::import::{self, Input, Outcome};
use crate::intake;
use crate::lan::{self, Lan, Situation};
use crate::library::{Health, Library, Synesthesia};
use crate::localtime::LocalDate;
use crate::mouse::{Button, MouseTracker};
use crate::perf::{self, Performance};
use crate::recovery::{Decision, Event, Rebuild, Recovery, Surface};
use crate::remote::{Overrides, Remote};
use crate::screenshot::{Screenshot, Target};
use crate::session::{self, Session};
use crate::show::{self, Env, Phase, Show};
use crate::snapshot;
use crate::song::SongWatch;
use crate::stats::{
    ControlDrive, ControlPacer, ControlStep, ControlTimes, FinishMode, FrameStats, PaceChange,
    PerfWindow, SwapPolicy,
};
use crate::ui::widgets::Tone;
use crate::ui::{
    Action, CueTransport, EditorPanel, FailurePanel, GeneratePanel, ImportPanel, LanPanel,
    ListenerPanel, MapUi, MediaFrom, MediaPanel, Model, NoticePanel, Offer, PerfPanel,
    PlaylistPanel, PlaylistUi, Preview, RemotePanel, ScenePreview, ShellState, TransportPanel,
    UiHost, UiState, shell,
};
use crate::workspace::Pane;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use strobe_diag::context;
use strobe_diag::failure::{self, Failure, Subsystem};
use strobe_gfx::{Compositor, GlChoice, GpuTimer, MediaRequest, Post, SwapWindow, WindowedContext};
use strobe_io::{Banks, Output};
use strobe_playlist::{Command, Playlist, ResumePoint};
use strobe_remote::bridge::Answer;
use strobe_scene::meta::MetaValues;
use strobe_shadertoy::listener::{Listener, ListenerConfig, Token};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, KeyEvent, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::monitor::MonitorHandle;
use winit::window::{Fullscreen, Window, WindowAttributes, WindowId};

/// Minimum time between output frames: a 144 Hz cap.
const OUTPUT_PERIOD: Duration = Duration::from_micros(6_944);
/// Time between control-window repaints: 30 Hz.
const CONTROL_PERIOD: Duration = Duration::from_millis(33);

/// How often the control window redraws while the fullscreen output covers
/// it (see [`Running::update_covered`]): enough to keep its state moving,
/// too seldom for its swap to cost the show a vertical blank.
const COVERED_PERIOD: Duration = Duration::from_millis(500);

/// How often a control window that is both covered by the fullscreen output
/// and starved of paints (D290) is still drawn in full (D315): its other
/// loop-driven frames only run `control_chores`. Often enough that egui's
/// state, and the picture shown the moment the output stops covering it,
/// are never more than five seconds old; a tenth of D290's 2 Hz, each of
/// which takes the shared context and on the mini PC cost the show
/// refreshes (M18: 4.2% missed covered and driven, 1.2% minimised).
const COVERED_HEARTBEAT: Duration = Duration::from_secs(5);
/// How often the control window redraws while it shows another Strobe's
/// window (the desktop viewer) and nothing has changed: enough for its
/// waiting card's clock. A new image from the host, a change of stage and
/// the person's input each bring the next redraw forward, to at most
/// [`CONTROL_PERIOD`] after the last (D302).
const VIEWER_PERIOD: Duration = Duration::from_millis(500);
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
/// Seconds the extend command adds to a cue.
const EXTEND_SECONDS: f64 = 30.0;
/// What a `STROBE_FAULT` loss makes every `make_current` say.
const INJECTED_LOSS: &str =
    "make current: injected fault (STROBE_FAULT lose-context): the context is lost";

/// Whether a control frame swaps its back buffer (D508). Not when the
/// fullscreen output covers the window and Windows sent no paint for the
/// frame (D290): nobody can see the result, and on the mini PC's Vega 3 that
/// swap blocked 44-50 ms, about three output refreshes. A desktop lease
/// drives those frames at 5 Hz, which took the show from 58 fps to 25
/// (M31). Everything else in the frame still runs, the desktop capture
/// included: it reads the back buffer before the swap. A frame Windows
/// painted swaps, covered or not, since a paint says some of the window
/// may be on screen; uncovering redraws at once (`update_covered`).
fn presents_control(covered: bool, unpainted: bool) -> bool {
    !(covered && unpainted)
}

/// The application, before and after its windows exist.
pub struct App {
    cli: Cli,
    library: Option<Library>,
    scan_error: Option<String>,
    running: Option<Running>,
    failure: Option<String>,
    /// Whether the windows ever opened, so a later failure is reported as
    /// the app stopping rather than as a failed start.
    started: bool,
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
            started: false,
        }
    }

    /// Why the app stopped early, if it did.
    pub fn failure(&self) -> Option<&str> {
        self.failure.as_deref()
    }

    /// Whether the windows opened before any failure.
    pub fn started(&self) -> bool {
        self.started
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
            Ok(mut running) => {
                running.place_output(event_loop, &self.cli);
                self.running = Some(running);
                self.started = true;
            }
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
    /// The layout of a fresh launch. The output window is created hidden
    /// unless `show_output`: it must exist either way, because the shared GL
    /// context is made on it.
    ///
    /// `custom_title_bar` opens the control window undecorated, because the
    /// UI draws its own title bar (D132). `window_size` (`--window-size`)
    /// opens it at exactly that size and *not* maximized, which is what makes
    /// a screenshot reproducible.
    fn initial(show_output: bool, custom_title_bar: bool, window_size: Option<(u32, u32)>) -> Self {
        let control = crate::brand::with_icons(Window::default_attributes())
            .with_title("Strobe")
            .with_decorations(!custom_title_bar)
            .with_min_inner_size(LogicalSize::new(720.0, 480.0));
        let control = match window_size {
            Some((width, height)) => {
                control.with_inner_size(LogicalSize::new(f64::from(width), f64::from(height)))
            }
            // Always maximized, on every launch (D65): the control surface
            // is the operator's workspace. The size is what an un-maximized
            // window (or a window system with no concept of maximizing, such
            // as bare Xvfb) falls back to.
            None => control
                .with_inner_size(LogicalSize::new(1360.0, 820.0))
                .with_maximized(true),
        };
        Self {
            output: crate::brand::with_icons(Window::default_attributes())
                .with_title("Strobe Output")
                .with_visible(show_output)
                .with_inner_size(LogicalSize::new(960.0, 540.0)),
            control,
            fullscreen: None,
        }
    }

    /// The layout the live windows are in now, so a rebuilt pair can be put
    /// back where the operator left them. `output_visible` is whether the
    /// output window is shown, which the window system may not report.
    fn capture(stack: &Stack, output_visible: bool, custom_title_bar: bool) -> Self {
        let mut layout = Self::initial(output_visible, custom_title_bar, None);
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
    /// The browser's preview: a second, small player for a scene that is
    /// not live. Owns GL objects, so it sits before the context.
    preview: Option<Session>,
    /// The playlist being played, if one is loaded. Its parked scenes own GL
    /// objects, so it sits before the context in drop order (D151).
    show: Option<Show>,
    /// The transition mixer, compiled once when the context opens (D154).
    compositor: Compositor,
    /// The output post stage (the meta controls), compiled once when the
    /// context opens and idle unless a value is off neutral (D180).
    post: Post,
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
        palette: &crate::palette::Palette,
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
        let vsync = match gfx.set_vsync(&output, true) {
            Ok(()) => true,
            Err(e) => {
                log::warn!("output vsync: {e}");
                false
            }
        };
        if let Err(e) = gfx.set_vsync(&control, false) {
            log::warn!("control vsync: {e}");
        }
        gfx.make_current(&control)?;
        // egui's painter checks the error flag before it builds anything,
        // and a fresh context can carry one it did not cause: on the T480
        // the native-to-ANGLE fail-over left GL_INVALID_OPERATION behind.
        // Drain it here, once per context, never in the frame loop.
        drain_gl_errors(gfx.gl());
        // The one place the transition program is compiled: a show never
        // compiles it again.
        let compositor = Compositor::new(gfx.gl(), gfx.caps(), [16, 16])
            .map_err(|e| format!("Could not start the transition compositor.\n{e}"))?;
        let post = Post::new(gfx.gl(), gfx.caps(), [16, 16])
            .map_err(|e| format!("Could not start the output post stage.\n{e}"))?;
        let ui = UiHost::new(event_loop, gfx.gl(), palette)?;
        if let Some(fullscreen) = layout.fullscreen.clone() {
            output.window().set_fullscreen(Some(fullscreen));
        }

        let refresh = output
            .window()
            .current_monitor()
            .and_then(|monitor| monitor.refresh_rate_millihertz());
        let mut perf = Performance::new(
            GpuTimer::new(gfx.gl(), gfx.caps()),
            start.max,
            start.adaptive,
            perf::budget_for(refresh),
        );
        // Missed refreshes lower the scale too, where there are refreshes
        // to miss (D314).
        perf.set_vsync(vsync);
        perf.set_refresh(refresh);
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
            preview: None,
            show: None,
            compositor,
            post,
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
            preview,
            show,
            compositor,
            post,
            mut perf,
            mut ui,
            control,
            output,
            gfx,
        } = self;
        if !gfx.gl().is_lost() && gfx.make_current(&control).is_err() {
            gfx.gl().mark_lost();
        }
        // The resume point is flushed and the thread joined in either case;
        // neither touches GL.
        let show = show.map(|mut show| {
            show.finish();
            show
        });
        if gfx.gl().is_lost() {
            drop(show);
            drop(session);
            drop(preview);
            drop(compositor);
            drop(post);
            drop(perf);
            ui.abandon();
        } else {
            drop(show);
            drop(session);
            drop(preview);
            drop(compositor);
            drop(post);
            perf.release();
            drop(perf);
            ui.destroy();
            drop(ui);
        }
        // A destroyed context that is still current is only destroyed
        // later, and a context on another backend cannot become current
        // while it is (glvnd refuses with EGL_BAD_ACCESS). Not a GL call.
        if let Err(error) = gfx.release_current() {
            log::warn!("{error}");
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
    /// The operator's control values, the scene's and the meta ones.
    controls: session::Targets,
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
    /// Synesthesia's install and scene folders, found at launch: listed
    /// after the library when Settings says so, and never written to
    /// (D271).
    synesthesia: Synesthesia,
    /// Whether the scene open in the Editor is one of Synesthesia's, for
    /// the folder it was worked out for, so the frame asks only when the
    /// Editor opens another.
    editor_read_only: Option<(PathBuf, bool)>,
    playing: Option<usize>,
    console: Console,
    stats: FrameStats,
    /// A minute of frame timings, logged as one line (T10).
    perf_log: PerfWindow,
    /// Slows the control window when switching to it waits on the output
    /// (D216).
    control_pacer: ControlPacer,
    /// When the output waits for its own swap, so switching to the control
    /// window does not (D221).
    swap_policy: SwapPolicy,
    /// Runs the control frames from the event loop when Windows stops
    /// painting the control window (D290).
    control_drive: ControlDrive,
    /// What the file hovering over a window would do if dropped (D218).
    drop_hint: Option<String>,
    /// A Shadertoy import that becomes a cue after this one when it lands
    /// (D229).
    pending_cue_import: Option<(crate::show::edit::At, String)>,
    /// A dropped folder of scenes still being added (D235).
    pack: Option<intake::Pack>,
    /// The watched folder, if one was given (D240).
    inbox: Option<intake::Inbox>,
    /// When the inbox was last polled.
    inbox_polled: Instant,
    gl_summary: String,
    audio: AudioInput,
    /// Hears a new song for the scene's transitions and a playlist's
    /// `audio-song` cues, or takes the operator's word (D489).
    song: SongWatch,
    mouse: MouseTracker,
    /// Local time for `strobe_Date`, with its UTC offset cached.
    local_date: LocalDate,

    /// The render scale in force: the adaptive controller's, or the
    /// slider's when adaptation is off.
    render_scale: f32,
    /// The slider's working value, which is the maximum scale.
    scale_edit: f32,
    pending_resize: Option<([i32; 2], Instant)>,
    resize_now: bool,
    /// A render size that failed to allocate, retried slowly.
    resize_backoff: ResizeBackoff,

    monitors: Vec<MonitorHandle>,
    monitor_labels: Vec<String>,
    /// What identifies each of `monitors`, in the same order (D400).
    monitor_ids: Vec<crate::output_monitor::Identity>,
    /// The output's monitor, followed across refreshes by what it is,
    /// never by its place in the list (D400).
    output_monitor: crate::output_monitor::Chosen,
    /// The `--output-monitor` that was not connected at launch, carried out
    /// when it appears (D451).
    awaited_monitor: crate::output_monitor::Awaited,
    last_monitor_refresh: Instant,
    /// The focus winit last reported for each window, for the foreground
    /// watch (D453).
    output_focused: bool,
    control_focused: bool,
    /// Whether the fullscreen output over the control window has the focus
    /// (D453), checked with the monitor list.
    foreground: crate::foreground::Watch,

    /// Whether the output window is shown. Tracked here because winit cannot
    /// report visibility on X11 or Wayland.
    output_visible: bool,
    /// The modifiers held over the output window, so its keys are the
    /// same chords as the control window's (D351).
    output_modifiers: ModifiersState,
    /// When the next scene frame is due, shown or hidden.
    next_output: Instant,
    next_control: Instant,
    /// When the control window's last redraw was requested.
    control_asked: Instant,
    /// Input reached the control window since its last redraw request; a
    /// desktop viewer redraws for it (D302).
    control_input: bool,
    /// The control window has shown it is above the output (it gained focus
    /// or the pointer reached it) since the output last went fullscreen.
    control_on_top: bool,
    /// The fullscreen output covers the control window, so it redraws at
    /// [`COVERED_PERIOD`] (TORNADO T10).
    control_covered: bool,
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

    /// The Shadertoy import worker (D113).
    import: import::Service,
    /// The import panel's state.
    import_state: import::State,
    /// The library entry the Import pane last imported, for its tag row
    /// (D264), with the library revision and folder it was found for, so
    /// the folders are compared only when one of them changes.
    imported_entry: Option<(u64, PathBuf, Option<usize>)>,
    /// The browser listener, while it is switched on (D114).
    listener: Option<Listener>,
    /// The listener's token, once one has been made.
    import_token: Option<Token>,
    /// The listener row's one-line state, and whether it is an error.
    listener_note: String,
    listener_failed: bool,
    /// The listener's port (`--listen-port`).
    listen_port: u16,
    /// Strobe's data directory: the token, the API key, the texture cache.
    data_dir: std::path::PathBuf,
    /// The generate-controls worker, while one runs (D202).
    generate_job: Option<generate::Job>,
    /// Its result, until the operator opens, discards or dismisses it.
    generate_outcome: Option<generate::Outcome>,
    /// When the running or last worker started measuring: a scene file
    /// written after it is not overwritten by a save (D488).
    generate_since: std::time::SystemTime,
    /// The last worker's scene is one of Synesthesia's, so it can only be
    /// kept as a copy (D271, D488).
    generate_read_only: bool,
    /// The scene open in the Editor (D204).
    editor_doc: Option<editor::Doc>,
    /// The scene folder the Editor last opened by itself, so a folder that
    /// cannot be read is tried once, not every frame.
    editor_auto: Option<PathBuf>,

    /// Presentation state (panes, the browser query, the theme), persisted.
    ui_state: UiState,
    /// What the shell remembers between frames.
    shell: ShellState,
    /// What is known of each scene's health, parallel to `library.entries`.
    health: Vec<Health>,
    /// Bumped whenever the library or its health changes, so the browser
    /// refilters only then.
    library_rev: u64,
    /// Why the library scan failed, if it did.
    scan_error: Option<String>,
    /// The output is blacked out (the shown output stays dark).
    blackout: bool,
    /// Whether this frame went through the post stage, so the present, the
    /// preview and a screenshot read its target (D180).
    posted: bool,
    /// `--media`: a picture or folder shown behind `syn_Media` in scenes loaded
    /// by hand.
    media_override: Option<std::path::PathBuf>,
    /// Set after the first post-stage failure so the log gets one line.
    post_failed: bool,
    /// A scene the UI asked for, to be loaded *after* the frame that shows
    /// "preparing" has been painted, because loading blocks.
    pending_load: Option<usize>,
    /// The last scene that failed to load.
    load_failure: Option<LoadFailure>,
    /// A notice banner and when it appeared.
    notice: Option<Notice>,
    /// Which library entry the browser's preview holds.
    preview_index: Option<usize>,
    /// A preview the UI asked for, prepared after the frame is painted
    /// (preparing compiles shaders, like a load).
    pending_preview: Option<usize>,
    /// When the preview was last advanced, for its own `dt`.
    preview_clock: Instant,
    /// The operator asked to quit (the title bar's close button).
    quit: bool,
    /// `--request-load`: a scene to ask for just before the control capture.
    shot_load: Option<String>,
    /// The LIST pane's state.
    playlist_ui: PlaylistUi,
    /// MIDI, OSC, mappings, macros and the fav slots (D170 to D177).
    remote: Remote,
    /// The LAN remote: off until asked for (D250 to D254).
    lan: Lan,
    /// The controls as data for a controlling Strobe (D513).
    controls_sync: crate::remote_controls::Publisher,
    /// Projector power and its web page (D502).
    projectors: crate::projectors::Projectors,
    desktop_controller: crate::desktop::Controller,
    /// Other Strobes on the LAN, looked for while Settings shows Remote
    /// (D468).
    lan_scan: LanScan,
    /// The file or folder dialog a Browse button opened (D470).
    picker: crate::picker::Picker,
    desktop_capture: crate::desktop::Capture,
    desktop_input: crate::desktop::HostInput,
    desktop_events: Vec<strobe_remote::desktop::Input>,
    /// The keyboard shortcuts in force: the defaults plus `hotkeys` (D178).
    bindings: Bindings,
    /// Outputs the mapper handed back this frame, kept to reuse its buffer.
    remote_out: Vec<Output>,
    /// The MAP pane's state.
    map_ui: MapUi,
    /// A preset to apply once the scene at this library index has loaded
    /// (a fav slot recalled a scene and a preset together).
    pending_preset: Option<(usize, String)>,
    /// Whether the resume point is read and saved (`--no-resume`, and never
    /// in screenshot mode).
    resume_enabled: bool,
    /// The playlist cue that was current last frame, to notice a change.
    last_cue: Option<usize>,
    /// The output frame count when the show became ready, so a screenshot
    /// counts frames of playback, not of loading.
    ready_frames: Option<u64>,
    /// When the control window last ran a frame. A playlist compiles one
    /// scene per control frame, and Windows can stop painting a control
    /// window that a fullscreen output covers, so the output frame takes
    /// over the compiling when the control window has gone quiet.
    last_control_frame: Instant,
    /// When the control window last drew in full, for
    /// [`COVERED_HEARTBEAT`] (D315).
    last_control_draw: Instant,
}

/// A scene that did not load, for the banner.
#[derive(Debug, Clone)]
struct LoadFailure {
    scene: String,
    summary: String,
}

/// A banner that is not a failure.
#[derive(Debug, Clone)]
struct Notice {
    tone: Tone,
    text: String,
    since: Instant,
    /// A button beside Dismiss that does what the notice suggests: its
    /// label and action (D271's Copy to my library).
    offer: Option<(&'static str, Action)>,
}

/// How long a notice stays up on its own.
const NOTICE_SECONDS: u64 = 12;

/// The size the browser's preview is rendered at: small, because it shares
/// the GPU with the show.
const PREVIEW_WIDTH: i32 = 384;

impl Running {
    fn new(
        event_loop: &ActiveEventLoop,
        cli: &Cli,
        mut library: Library,
        scan_error: Option<String>,
    ) -> Result<Self, String> {
        let render_scale = session::clamp_scale(cli.render_scale);
        if render_scale != cli.render_scale {
            log::warn!(
                "--render-scale {} is outside {:.2}..={:.2}; using {render_scale}",
                cli.render_scale,
                session::RENDER_SCALE.start(),
                session::RENDER_SCALE.end()
            );
        }
        // Screenshot mode never reads or writes the saved layout, so a
        // capture depends on neither, and a capture never changes it.
        let mut ui_state = if cli.screenshot.is_some() {
            UiState::ephemeral()
        } else {
            UiState::load()
        };
        if cli.system_title_bar {
            ui_state.custom_title_bar = false;
        }
        if let Some(theme) = &cli.theme {
            if ui_state.catalog().themes().iter().any(|t| t.id == *theme) {
                ui_state.set_theme(theme);
            } else {
                log::warn!("--theme: there is no theme called \"{theme}\"");
            }
        }
        if let Some(density) = &cli.density {
            match crate::ui::theme::Density::from_key(density) {
                Some(density) => ui_state.set_density(density),
                None => log::warn!("--density: \"{density}\" is not comfortable or compact"),
            }
        }
        if cli.no_panes {
            ui_state.panes.close_all();
        }
        for unknown in ui_state.open_named(&cli.open_pane) {
            log::warn!("--open-pane: there is no pane called \"{unknown}\"");
        }
        if let Some(query) = &cli.browser_query {
            ui_state.browser.query.clone_from(query);
        }
        if let Some(query) = &cli.palette {
            ui_state.command_palette.open_with(query);
        }
        let inbox = watch_at_launch(
            cli.watch_folder.as_deref(),
            &mut ui_state.watch,
            &library.dir,
        );
        // Synesthesia's scenes after the library's own, before anything
        // looks a scene up by name (D271).
        let synesthesia = Synesthesia::detect(cli.no_synesthesia);
        library.set_synesthesia(synesthesia.shown(ui_state.synesthesia.show));
        ui_state.synesthesia.note =
            synesthesia.note(ui_state.synesthesia.show, library.synesthesia_count());
        log::info!("Synesthesia's scenes: {}", ui_state.synesthesia.note);
        // Controlling another Strobe (D294): this laptop's id is made once
        // and kept, and `--control` decides the first thing the form does.
        if !strobe_remote::trust::valid_controller_id(&ui_state.remote.controller_id) {
            ui_state.remote.controller_id = crate::trusted::new_controller_id();
        }
        let (desktop_controller, launch_action) = crate::desktop::Controller::at_launch(
            &ui_state.remote,
            cli.control.filter(|_| cli.screenshot.is_none()),
            cli.remote_trust.as_deref(),
        );
        let stack = Stack::open(
            event_loop,
            cli.gl.choice(),
            &Layout::initial(cli.show_output, ui_state.custom_title_bar, cli.window_size),
            PerfStart {
                max: render_scale,
                adaptive: !cli.no_adaptive,
            },
            ui_state.palette(),
        )?;
        let gl_summary = stack.summary();
        let screenshot = cli
            .screenshot
            .clone()
            .map(|base| Screenshot::new(base, cli.frames));
        let now = Instant::now();
        let recovery = Recovery::new(cli.gl.choice(), stack.gfx.backend());
        let data_dir = strobe_diag::paths::base_dir();
        let import = import::Service::start(import::WorkerConfig {
            library: library.dir.clone(),
            data_dir: data_dir.clone(),
            download_textures: !cli.no_texture_download,
        })
        .map_err(|e| format!("Could not start the Shadertoy importer: {e}"))?;
        let health = vec![Health::Unknown; library.entries.len()];
        let mut running = Self {
            stack,
            library,
            synesthesia,
            editor_read_only: None,
            health,
            library_rev: 1,
            scan_error: scan_error.clone(),
            blackout: cli.blackout,
            posted: false,
            media_override: cli.media.clone(),
            post_failed: false,
            pending_load: None,
            load_failure: None,
            notice: None,
            preview_index: None,
            pending_preview: None,
            preview_clock: now,
            quit: false,
            shot_load: cli.request_load.clone(),
            playlist_ui: PlaylistUi::default(),
            remote: Remote::new(
                &data_dir,
                &Overrides {
                    workers: cli.screenshot.is_none() || cli.osc,
                    osc: cli.osc,
                    osc_port: cli.osc_port,
                    osc_all_interfaces: cli.osc_all_interfaces,
                    no_midi: cli.no_midi || cli.screenshot.is_some(),
                    file: cli.show_control.clone(),
                    read_only: cli.screenshot.is_some(),
                },
                now,
            ),
            remote_out: Vec::new(),
            controls_sync: crate::remote_controls::Publisher::default(),
            lan: Lan::new(
                data_dir.clone(),
                cli.remote_bind,
                cli.remote_port,
                cli.screenshot.is_none(),
            )
            .behind_proxy(cli.remote_proxy.clone())
            .venue(
                // Only a host reads it; with `--control` it is the laptop's.
                cli.remote_trust
                    .as_deref()
                    .filter(|_| cli.remote)
                    .and_then(crate::venue::load),
                cli.allow_desktop_control,
            ),
            projectors: crate::projectors::Projectors::new(&data_dir, cli.screenshot.is_none()),
            desktop_controller,
            lan_scan: LanScan::default(),
            picker: crate::picker::Picker::default(),
            desktop_capture: crate::desktop::Capture::default(),
            desktop_input: crate::desktop::HostInput::default(),
            desktop_events: Vec::with_capacity(129),
            bindings: Bindings::default(),
            map_ui: MapUi::default(),
            pending_preset: None,
            resume_enabled: !cli.no_resume && cli.screenshot.is_none(),
            last_cue: None,
            ready_frames: None,
            last_control_frame: Instant::now(),
            last_control_draw: Instant::now(),
            ui_state,
            shell: ShellState::default(),
            playing: None,
            console: Console::default(),
            stats: FrameStats::default(),
            perf_log: PerfWindow::default(),
            control_pacer: ControlPacer::default(),
            swap_policy: SwapPolicy::new(finish_mode()),
            control_drive: ControlDrive::default(),
            drop_hint: None,
            pending_cue_import: None,
            pack: None,
            inbox,
            inbox_polled: Instant::now(),
            gl_summary,
            audio: AudioInput::new(cli.audio == AudioArg::Auto)
                .with_ambient(cli.audio == AudioArg::Ambient)
                .with_controller(cli.control.is_some()),
            mouse: MouseTracker::default(),
            local_date: LocalDate::from_system(),
            song: SongWatch::new(),
            render_scale,
            scale_edit: render_scale,
            pending_resize: None,
            resize_now: false,
            resize_backoff: ResizeBackoff::default(),
            monitors: Vec::new(),
            monitor_labels: Vec::new(),
            monitor_ids: Vec::new(),
            output_monitor: crate::output_monitor::Chosen::default(),
            awaited_monitor: crate::output_monitor::Awaited::default(),
            last_monitor_refresh: now,
            output_focused: false,
            control_focused: false,
            foreground: crate::foreground::Watch::default(),
            output_visible: cli.show_output,
            output_modifiers: ModifiersState::empty(),
            control_on_top: true,
            control_covered: false,
            next_output: now,
            next_control: now,
            control_asked: now,
            control_input: false,
            actions: launch_action.into_iter().collect(),
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
            import,
            import_state: import::State::default(),
            imported_entry: None,
            listener: None,
            import_token: None,
            listener_note: "Off".to_string(),
            listener_failed: false,
            listen_port: cli.listen_port,
            data_dir,
            generate_job: None,
            generate_outcome: None,
            generate_since: std::time::UNIX_EPOCH,
            generate_read_only: false,
            editor_doc: None,
            editor_auto: None,
        };
        let (bindings, problems) = Bindings::from_config(&running.remote.hotkeys);
        for problem in problems {
            log::warn!("{problem}");
        }
        running.bindings = bindings;
        running.refresh_monitors(event_loop);
        log::info!("GL: {}", running.gl_summary);
        if cli.listen {
            running.start_listener();
        }
        if cli.remote {
            running.lan.start_at_launch(Instant::now());
        }
        if let Some(error) = scan_error {
            if cli.library_is_optional() {
                log::info!("no scene library, which a controller does not need: {error}");
            } else {
                failure::report(
                    Failure::new(Subsystem::Io, "scan")
                        .path(&running.library.dir)
                        .message(error)
                        .action("the scene list is empty"),
                );
            }
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
        if let Some(query) = cli.preview.as_deref() {
            match running.library.find(query) {
                Some(index) => running.preview_load(index),
                None => log::warn!("--preview: no scene matching \"{query}\" in the library"),
            }
        }
        let playlist = match cli.playlist.as_deref() {
            Some(path) => {
                running.playlist_ui.path = path.display().to_string();
                running.load_playlist(path);
                if running.stack.show.is_some() {
                    LaunchPlaylist::Loaded
                } else {
                    LaunchPlaylist::Failed
                }
            }
            None => LaunchPlaylist::None,
        };
        // Reopen the scene of the last session (T480 Task 21), unless the
        // command line chose one, a playlist plays, or this is a screenshot
        // (whose state is never the operator's). A playlist that would not
        // load leaves a scene playing rather than a black output (D452).
        let last = running.ui_state.last_scene.clone();
        let chosen = launch_scene(
            cli.scene.is_some(),
            playlist,
            cli.screenshot.is_some(),
            &last,
            running
                .library
                .entries
                .iter()
                .map(|entry| entry.name.as_str()),
        );
        let name = chosen
            .and_then(|index| running.library.entries.get(index))
            .map(|entry| entry.name.clone());
        match chosen.zip(name) {
            Some((index, name)) => {
                if playlist == LaunchPlaylist::Failed {
                    log::warn!(
                        "the playlist did not load, so {name} plays instead of a black output \
                         (D452)"
                    );
                } else {
                    log::info!("reopening {name}, the scene of the last session");
                }
                running.load(index);
            }
            None if playlist != LaunchPlaylist::Loaded && !last.is_empty() => {
                log::info!("the last session's scene {last} is not in the library");
            }
            None => {}
        }
        Ok(running)
    }

    fn finished(&self) -> bool {
        self.quit || self.screenshot.as_ref().is_some_and(Screenshot::finished)
    }

    fn shutdown(mut self) {
        // Drop the phones and stop accepting browser requests before the
        // windows go.
        self.lan.stop();
        drop(self.listener);
        // Write the layout if a change is still waiting, off the UI thread.
        self.ui_state.finish();
        // Likewise the mappings and macros, and stop the MIDI and OSC workers.
        self.remote.finish();
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
    fn check_reset(&mut self, now: Instant, surface: Surface) -> bool {
        let gl = self.stack.gfx.gl();
        if !self.stack.gfx.robust() || !gl.has_reset_status() || !self.recovery.poll_due(now) {
            return false;
        }
        let status = gl.reset_status();
        match self.recovery.handle(
            now,
            Event::Reset {
                window: surface,
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
        // A playlist is reloaded from where it was; its single-scene
        // fallback below would only load a second copy.
        let reload = self.stack.show.as_ref().map(Show::reload_spec);
        let carried = Carried {
            layout: Layout::capture(
                &self.stack,
                self.output_visible,
                self.ui_state.custom_title_bar,
            ),
            playing: self.playing.filter(|_| reload.is_none()).map(|index| {
                let session = self.stack.session.as_ref();
                Resume {
                    index,
                    time: session.map_or(self.start_time, Session::time),
                    controls: session.map(Session::control_targets).unwrap_or_default(),
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
                    self.ui_state.palette(),
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
            resize_backoff: ResizeBackoff::default(),
            next_output: now,
            next_control: now,
            // The old control window's unanswered request died with it.
            control_drive: ControlDrive::default(),
            // So did both windows' focus; the new ones report their own.
            output_focused: false,
            control_focused: false,
            // The preview and a load that was waiting belonged to the lost
            // context; the scene that was playing is reloaded below.
            preview_index: None,
            pending_preview: None,
            pending_load: None,
            last_cue: None,
            ready_frames: None,
            ..self
        };
        log::info!("GL: {}", running.gl_summary);
        running.stack.perf.resume_scale(carried.scale);
        running.refresh_monitors(event_loop);
        if let Some((source, playlist, point)) = reload {
            // The show's scenes died with the context: compile them again
            // and carry on from the cue that was playing.
            running.start_show(source, playlist, point);
        }
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
        running.notice = Some(Notice {
            tone: if reloaded || scene.is_none() {
                Tone::Info
            } else {
                Tone::Warning
            },
            text: format!(
                "The graphics context was lost and rebuilt on {} in {} ms{}.",
                failures::backend_label(running.stack.gfx.backend()),
                elapsed.as_millis(),
                match (&scene, reloaded) {
                    (Some(name), true) => format!("; {name} is playing again"),
                    (Some(name), false) => format!("; {name} could not be reloaded"),
                    (None, _) => String::new(),
                }
            ),
            since: Instant::now(),
            offer: None,
        });
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
        self.control_drive.pass();
        if let Some(shot) = self.screenshot.as_mut()
            && shot.give_up_control(now)
        {
            log::warn!(
                "screenshot: the control window drew nothing for {} s after the output \
                 capture (minimised, or not painted over Remote Desktop); ending with \
                 {} only",
                crate::screenshot::CONTROL_DEADLINE.as_secs(),
                shot.path(Target::Output).display()
            );
        }
        if now.duration_since(self.last_monitor_refresh) >= MONITOR_REFRESH {
            self.refresh_monitors(event_loop);
        }
        self.drain_imports();
        self.poll_inbox(now);
        self.step_pack();
        self.poll_generate();
        if now >= self.next_output {
            if self.output_visible {
                self.stack.output.window().request_redraw();
                self.next_output = now + OUTPUT_PERIOD;
            } else {
                // A hidden window may never be sent `RedrawRequested`, and
                // there is no swap to pace on, so the scene frame runs from
                // here, on the schedule of the output monitor's refresh.
                self.scene_frame(Surface::Control);
                self.next_output = (self.next_output + hidden_period(self.stack.perf.budget_ms()))
                    .max(Instant::now());
                if self.rebuild.is_some() {
                    return Instant::now();
                }
            }
        }
        if self.desktop_controller.active()
            && !self.control_unseen()
            && (std::mem::take(&mut self.control_input) | self.desktop_controller.needs_redraw())
        {
            // The viewer redraws for something new, never faster than the
            // ordinary window (D302).
            self.next_control = self.next_control.min(self.control_asked + CONTROL_PERIOD);
        }
        if now >= self.next_control {
            self.control_asked = now;
            self.stack.control.window().request_redraw();
            // A minimised window has nothing to draw: its frame runs only
            // the parts that do not draw (D314), and a screenshot captures
            // only what was painted.
            let minimized = self.control_minimized();
            let step = if minimized || self.screenshot.is_some() {
                ControlStep::Request
            } else {
                self.control_drive.due(now)
            };
            let period = control_period(ControlFacts {
                unseen: self.control_unseen(),
                streaming: self.lan.desktop().is_some_and(|desktop| desktop.active()),
                slow: self.control_pacer.is_slow(),
                viewing: self.desktop_controller.active(),
            });
            self.next_control = now + period;
            if minimized && self.screenshot.is_none() {
                self.control_chores(now);
                if self.rebuild.is_some() {
                    return Instant::now();
                }
            }
            let run_here = match step {
                ControlStep::Request => false,
                ControlStep::TakeOver { waited, times } => {
                    let level = if times == 1 {
                        log::Level::Info
                    } else {
                        log::Level::Debug
                    };
                    log::log!(
                        level,
                        "control window: no paint from Windows for {} ms (takeover {times}); the \
                         event loop runs its frames every {} ms until one comes (D290)",
                        waited.as_millis(),
                        period.as_millis()
                    );
                    true
                }
                ControlStep::RunHere => true,
            };
            if run_here {
                if self.control_behind_output(now) {
                    self.control_chores(now);
                } else {
                    self.redraw_control(true);
                }
                if self.rebuild.is_some() {
                    return Instant::now();
                }
            }
        }
        self.next_output.min(self.next_control)
    }

    /// Whether the control window is minimised, so a frame of it would draw
    /// nothing anyone sees. The size alone does not say: on Windows an
    /// undecorated window (the custom title bar, D132) keeps a client area
    /// when minimised, and the mini PC ran about 116 full control frames a
    /// minute in that state (M16). `is_minimized` is a window-system query
    /// (`IsIconic` on Windows), made at the control window's rate, never
    /// per output frame. A window streamed to a desktop remote (D275) still
    /// draws, since the viewer sees it.
    fn control_minimized(&self) -> bool {
        let window = self.stack.control.window();
        let size = window.inner_size();
        let streaming = self.lan.desktop().is_some_and(|desktop| desktop.active());
        size.width == 0 || size.height == 0 || (window.is_minimized() == Some(true) && !streaming)
    }

    /// Whether a loop-driven control frame should skip drawing (D315): the
    /// window is covered by the fullscreen output on its monitor and Windows
    /// withholds its paints (D290), it is not streamed to a desktop remote
    /// (D275), no input has come to it, and it drew within
    /// [`COVERED_HEARTBEAT`]. Covered as well as starved, because starved
    /// alone does not mean unseen: a windowed or second-monitor output
    /// above it in z-order starves a control window the operator is looking
    /// at, and those loop-driven frames are the only ones it gets.
    fn control_behind_output(&mut self, now: Instant) -> bool {
        if !self.control_covered
            || !self.control_drive.driving()
            || self.lan.desktop().is_some_and(|desktop| desktop.active())
        {
            return false;
        }
        // A key pressed in the focused, covered window is egui's to handle
        // now, not at the next heartbeat.
        if std::mem::take(&mut self.control_input) {
            return false;
        }
        now.saturating_duration_since(self.last_control_draw) < COVERED_HEARTBEAT
    }

    /// The parts of a control frame that do not draw, for a minimised
    /// control window (D314) or one covered and starved of paints (D315):
    /// queued actions, a scene load asked for by a
    /// remote, MIDI or a cue, the playlist's compile step, and saving
    /// settings. Without them a show run from a phone or a MIDI controller
    /// with the console minimised would never load the next scene, since a
    /// load is asked for in one frame and done in the next control frame.
    /// The browser's preview waits for the window to come back. Counts as a
    /// control frame for `redraw_output`'s compile backstop, since it
    /// compiles too.
    fn control_chores(&mut self, now: Instant) {
        self.last_control_frame = now;
        self.ui_state.tick(now);
        self.remote.persist(now);
        if self.rebuild.is_some() {
            return;
        }
        let mut actions = std::mem::take(&mut self.actions);
        self.apply(&mut actions);
        self.actions = actions;
        // Nobody sees "preparing", so there is nothing to wait for.
        if let Some(index) = self.pending_load.take() {
            self.load(index);
        }
        let compiling = self
            .stack
            .show
            .as_ref()
            .is_some_and(|show| show.phase() == Phase::Compiling);
        // The surface the scene frame keeps current: no extra switch, so no
        // wait for the output's swap (D216).
        let surface = if self.output_visible {
            Surface::Output
        } else {
            Surface::Control
        };
        if compiling && self.rebuild.is_none() && self.make_current(surface) {
            self.compile_show_step();
        }
    }

    /// Whether nobody can see the control window: the fullscreen output
    /// covers it, or Windows has stopped painting it (D290). Either way it
    /// redraws at [`COVERED_PERIOD`] (or the desktop stream's rate).
    fn control_unseen(&self) -> bool {
        self.control_covered || self.control_drive.driving()
    }

    /// Decides whether the output covers the control window: it is shown,
    /// fullscreen, on the control window's monitor, and the control window
    /// has not come above it since. Focus alone does not say: the app hands
    /// focus back to the control window when the output is shown, and F11
    /// in the control window then puts the output over it with the focus
    /// left where it was. winit reports no occlusion on Windows.
    ///
    /// TORNADO T10 ran 48 to 56 fps in exactly that state with the GPU idle;
    /// the unsynced swap of a covered window is the suspect, and a covered
    /// window has nothing to show anyway.
    fn update_covered(&mut self) {
        let output = self.stack.output.window();
        let covered = self.output_visible
            && !self.control_on_top
            && output.fullscreen().is_some()
            && output.current_monitor().is_some_and(|monitor| {
                self.stack.control.window().current_monitor().as_ref() == Some(&monitor)
            });
        if covered != self.control_covered {
            self.control_covered = covered;
            log::info!(
                "control window {}",
                if covered {
                    "covered by the fullscreen output: redrawing at 2 Hz"
                } else {
                    "in view: redrawing at 30 Hz"
                }
            );
            if !covered {
                self.next_control = Instant::now();
            }
        }
    }

    /// Says in the log when the fullscreen output over the control window
    /// has not had the focus for a while (D453): a window of another
    /// program, or the control window, holds it, and Windows may then
    /// compose the output instead of presenting it directly. Runs with the
    /// monitor list's refresh, never per frame.
    fn check_foreground(&mut self) {
        use crate::foreground::{Change, Focus};
        let covers = self.close_facts(false).output_fills_control_monitor;
        let focus = Focus::of(self.output_focused, self.control_focused);
        match self.foreground.check(covers, focus, Instant::now()) {
            Some(Change::Lost { focus, first: true }) => log::warn!(
                "output: the fullscreen output is not the foreground window ({} has the \
                 focus); Windows may compose it instead of presenting it directly, which cost \
                 5-11% of refreshes in a soak. Nothing should sit over the output (D453)",
                focus.holder()
            ),
            Some(Change::Lost {
                focus,
                first: false,
            }) => log::info!(
                "output: not the foreground window again ({} has the focus) (D453)",
                focus.holder()
            ),
            Some(Change::Back(after)) => log::info!(
                "output: in the foreground again, or no longer fullscreen over the control \
                 window, after {:.0} s (D453)",
                after.as_secs_f64()
            ),
            None => {}
        }
    }

    /// What a close request needs to know about the windows (D298).
    fn close_facts(&self, on_output: bool) -> CloseFacts {
        let output = self.stack.output.window();
        let control = self.stack.control.window();
        CloseFacts {
            on_output,
            rebuilding: self.rebuild.is_some(),
            control_minimized: control.is_minimized() == Some(true),
            output_fills_control_monitor: self.output_visible
                && output.fullscreen().is_some()
                && output
                    .current_monitor()
                    .is_some_and(|monitor| control.current_monitor().as_ref() == Some(&monitor)),
        }
    }

    fn refresh_monitors(&mut self, event_loop: &ActiveEventLoop) {
        self.last_monitor_refresh = Instant::now();
        self.update_covered();
        self.check_foreground();
        // The time zone's offset can change (daylight saving); looking it
        // up may read the zone database, so it is done here, never per frame.
        self.local_date.refresh();
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
        self.monitor_ids.clear();
        self.monitor_ids.extend(self.monitors.iter().map(|monitor| {
            crate::output_monitor::Identity {
                name: monitor.name(),
                position: [monitor.position().x, monitor.position().y],
                size: [monitor.size().width, monitor.size().height],
            }
        }));
        let followed = self.output_monitor.follow(&self.monitor_ids);
        let labels = &self.monitor_labels;
        let label = |index: usize| labels.get(index).map_or("?", String::as_str);
        match followed {
            crate::output_monitor::Followed::Same => {}
            crate::output_monitor::Followed::Moved { from, to } => log::info!(
                "output monitor: the list changed; it is {} now (was number {}) (D400)",
                label(to),
                from + 1
            ),
            crate::output_monitor::Followed::Lost => log::warn!(
                "output monitor: the chosen monitor is gone; F11 now goes fullscreen where \
                 the output is, until it comes back or another is chosen (D400)"
            ),
            crate::output_monitor::Followed::Found(at) => log::info!(
                "output monitor: the chosen monitor is back as {} (D400)",
                label(at)
            ),
        }
        if let Some((index, fullscreen)) = self.awaited_monitor.arrived(self.monitors.len()) {
            log::info!(
                "--output-monitor {}: connected now as {}; the output goes there{} (D451)",
                index + 1,
                label(index),
                if fullscreen { ", fullscreen" } else { "" }
            );
            self.move_to_monitor(index);
            if fullscreen && self.stack.output.window().fullscreen().is_none() {
                self.toggle_fullscreen();
            }
        }
    }

    // --------------------------------------------------------------- events

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if matches!(event, WindowEvent::CloseRequested) {
            // The output's close button (or Alt+F4 on it) hides it: the
            // control window is the app, and closing it quits. Unless the
            // control window cannot be reached to quit from (D298).
            let on_output = id == self.stack.output.window().id();
            let facts = self.close_facts(on_output);
            if !closing_quits(facts) {
                log::info!("output window closed: hiding it (close the control window to quit)");
                self.set_output_visible(false);
            } else {
                if on_output {
                    log::info!(
                        "output window closed with the control window out of reach ({facts:?}): quitting"
                    );
                }
                event_loop.exit();
            }
            return;
        }
        if self.rebuild.is_some() {
            // The windows are about to be replaced.
            return;
        }
        let is_control = id == self.stack.control.window().id();
        let is_output = id == self.stack.output.window().id();
        if let WindowEvent::Focused(focused) = event {
            if is_output {
                self.output_focused = focused;
            } else if is_control {
                self.control_focused = focused;
            }
        }
        if is_control && is_input(&event) {
            self.control_input = true;
        }
        if let WindowEvent::HoveredFile(path) = &event {
            self.import_state.drop_hover = true;
            self.drop_hint = Some(intake::hint(&intake::classify(path)));
        } else if matches!(event, WindowEvent::HoveredFileCancelled) {
            self.import_state.drop_hover = false;
            self.drop_hint = None;
        } else if let WindowEvent::DroppedFile(path) = &event {
            self.import_state.drop_hover = false;
            self.drop_hint = None;
            self.dropped(path);
        }
        if let WindowEvent::Resized(size) = &event {
            if is_control {
                self.stack.gfx.resize(&self.stack.control);
            } else if is_output {
                // A soak run once saw GPU time and memory step down at
                // minute 6 with nothing logged (T480 Task 17); the likeliest
                // cause is the output leaving fullscreen, so say when it
                // changes.
                log::info!(
                    "output window resized to {}x{}, fullscreen {}",
                    size.width,
                    size.height,
                    self.stack.output.window().fullscreen().is_some()
                );
                self.stack.gfx.resize(&self.stack.output);
                if self.stack.output.window().fullscreen().is_some() {
                    // Going fullscreen puts the output on top.
                    self.control_on_top = false;
                }
                self.update_covered();
            }
        } else if is_output && let WindowEvent::Occluded(occluded) = &event {
            log::info!("output window occluded: {occluded}");
        } else if matches!(event, WindowEvent::RedrawRequested) {
            if is_output {
                self.redraw_output();
            } else if is_control {
                if self.control_drive.painted(Instant::now()) {
                    log::debug!(
                        "control window: Windows paints it again; its frames follow the paints (D290)"
                    );
                    // Back from the unseen rate without waiting it out.
                    self.next_control = self.next_control.min(Instant::now() + CONTROL_PERIOD);
                }
                self.redraw_control(false);
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
        } else if is_output && let WindowEvent::ModifiersChanged(modifiers) = &event {
            self.output_modifiers = modifiers.state();
        } else if is_output && matches!(event, WindowEvent::Focused(false)) {
            // Keys let go in another window are never reported here.
            self.output_modifiers = ModifiersState::empty();
        } else if is_output && let WindowEvent::KeyboardInput { event: key, .. } = &event {
            // The control window's keys go through egui, where focus is
            // known (`ui::keys`); the output window has no UI, so its keys
            // are mapped here with the same command table.
            self.output_key(key);
        }
        if self.rebuild.is_none() {
            let raised = if matches!(event, WindowEvent::Focused(true)) {
                Some(is_control)
            } else if is_control
                && matches!(
                    event,
                    WindowEvent::CursorMoved { .. } | WindowEvent::MouseInput { .. }
                )
            {
                Some(true)
            } else {
                None
            };
            if let Some(on_top) = raised
                && on_top != self.control_on_top
            {
                self.control_on_top = on_top;
                self.update_covered();
            }
        }
        if is_control && self.rebuild.is_none() {
            let window = self.stack.control.window();
            // The response's repaint request needs no action: the control
            // window repaints on its own 30 Hz timer, and nothing else
            // reads whether egui consumed the event.
            let _response = self.stack.ui.on_window_event(window, &event);
        }
    }

    /// A key pressed in the output window: the same commands as the control
    /// window, with no text field to respect, and the same modifiers
    /// (D351). Esc leaves fullscreen first.
    fn output_key(&mut self, key: &KeyEvent) {
        if key.state != ElementState::Pressed || key.repeat {
            return;
        }
        if key.logical_key == Key::Named(NamedKey::Escape)
            && self.output_visible
            && self.stack.output.window().fullscreen().is_some()
        {
            self.stack.output.window().set_fullscreen(None);
            return;
        }
        let Some(chord) = chord_of(&key.logical_key, self.output_modifiers) else {
            return;
        };
        if let Some(command) = commands::dispatch_with(chord, Focus::default(), &self.bindings)
            && let Some(action) = command_action(command, self.blackout)
        {
            let mut actions = vec![action];
            self.apply(&mut actions);
        }
    }

    /// Shows or hides the output window. The scene keeps rendering either
    /// way, so the control window's preview does not change.
    fn set_output_visible(&mut self, visible: bool) {
        if self.output_visible == visible {
            return;
        }
        self.output_visible = visible;
        self.stack.output.window().set_visible(visible);
        if visible {
            // A freshly shown window takes focus; the operator is working in
            // the control window, so it gets it back.
            self.stack.control.window().focus_window();
        }
        log::info!("output window {}", if visible { "shown" } else { "hidden" });
    }

    /// Toggles fullscreen on the output. Asking for fullscreen means "put it
    /// on the projector", so a hidden output is shown (and made fullscreen).
    fn toggle_fullscreen(&mut self) {
        let window = self.stack.output.window();
        if self.output_visible && window.fullscreen().is_some() {
            window.set_fullscreen(None);
            return;
        }
        let monitor = self
            .output_monitor
            .index()
            .and_then(|index| self.monitors.get(index))
            .cloned();
        window.set_fullscreen(Some(Fullscreen::Borderless(monitor)));
        self.set_output_visible(true);
    }

    /// Applies `--output-monitor` and `--fullscreen` once the windows exist,
    /// so a venue autostart lands on the projector with nobody at the
    /// keyboard.
    fn place_output(&mut self, event_loop: &ActiveEventLoop, cli: &Cli) {
        if !cli.fullscreen && cli.output_monitor.is_none() {
            return;
        }
        self.refresh_monitors(event_loop);
        if let Some(number) = cli.output_monitor {
            let index = usize::try_from(number)
                .unwrap_or(usize::MAX)
                .saturating_sub(1);
            if index < self.monitors.len() {
                self.move_to_monitor(index);
            } else {
                // A projector switched on after the machine booted: wait
                // for it rather than leave the show where it opened (D451).
                self.awaited_monitor = crate::output_monitor::Awaited::new(index, cli.fullscreen);
                log::warn!(
                    "--output-monitor {number}: there are {} monitor(s); the output opens where \
                     it is, and moves to monitor {number} when it is connected (D451)",
                    self.monitors.len()
                );
            }
        }
        if cli.fullscreen {
            self.toggle_fullscreen();
        }
        log::info!(
            "output placed at launch: monitor {}, fullscreen {}",
            self.output_monitor
                .index()
                .map_or_else(|| "as opened".to_string(), |index| (index + 1).to_string()),
            cli.fullscreen
        );
    }

    fn move_to_monitor(&mut self, index: usize) {
        let Some(monitor) = self.monitors.get(index).cloned() else {
            return;
        };
        if !self.output_monitor.choose(index, &self.monitor_ids) {
            return;
        }
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
        // Sending the output to a monitor means showing it there.
        self.set_output_visible(true);
    }

    // ----------------------------------------------------------- scene load

    /// Loads scene `index` now, with its clock at the start. Used at
    /// start-up and for a context rebuild; the UI asks through
    /// [`Self::request_load`] so it can show "preparing" first.
    fn load(&mut self, index: usize) {
        self.load_from(index, self.start_time);
    }

    /// Asks for scene `index` to be loaded after the next control-window
    /// frame. Loading compiles shaders on this thread and blocks, so the
    /// frame that says "Preparing X..." must be painted first.
    fn request_load(&mut self, index: usize) {
        if index < self.library.entries.len() {
            self.pending_load = Some(index);
        }
    }

    /// Records what happened when scene `index` was tried.
    fn set_health(&mut self, index: usize, health: Health) {
        if let Some(slot) = self.health.get_mut(index)
            && *slot != health
        {
            *slot = health;
            self.library_rev += 1;
        }
    }

    /// Loads scene `index` with its clock starting at `start` seconds.
    fn load_from(&mut self, index: usize, start: f32) {
        let Some(entry) = self.library.entries.get(index).cloned() else {
            return;
        };
        if !self.make_current(Surface::Output) {
            return;
        }
        if self.stack.show.is_some() {
            self.unload_show("a scene was loaded by hand");
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
            Ok(mut session) => {
                // The operator's media, else the default preset's, else the
                // scene's own (D217, D223).
                let media = self
                    .media_override
                    .clone()
                    .or_else(|| session.startup_media());
                if media.is_some() {
                    session
                        .player
                        .choose_media(media.map(MediaRequest::for_path));
                }
                // The operator's output settings carry over (D207).
                if let Some(old) = self.stack.session.as_ref() {
                    let taken = session.meta.carry_unchanged_from(&old.meta);
                    if taken > 0 {
                        log::info!("meta: {taken} output setting(s) carried over");
                    }
                }
                self.stack.session = Some(session);
                self.playing = Some(index);
                self.ui_state.last_scene.clone_from(&entry.name);
                self.pending_resize = None;
                self.load_failure = None;
                self.set_health(index, Health::Ready);
                self.scene_loaded_for_perf(Instant::now());
                // The size drawn, which a scene that pins its final pass
                // (D280) keeps whatever size was asked for.
                let size = self
                    .stack
                    .session
                    .as_ref()
                    .map_or(size, |session| session.player.render_size());
                context::set(context::SCENE, &entry.name);
                context::set(context::RENDER_SIZE, format!("{}x{}", size[0], size[1]));
                log::info!(
                    "Loaded {} ({}x{}) in {:.0} ms",
                    entry.name,
                    size[0],
                    size[1],
                    started.elapsed().as_secs_f64() * 1000.0
                );
                if let Some((target, name)) = self.pending_preset.take()
                    && target == index
                {
                    self.recall_preset(&name);
                }
            }
            Err(error) => {
                self.pending_preset = None;
                failure::report(failures::scene_load(
                    &entry.name,
                    &entry.dir,
                    &error,
                    self.stack.session.is_some(),
                ));
                self.scene_failed(index, &entry.name, &error.to_string());
            }
        }
    }

    /// Remembers that a scene failed: its row says so, and a banner names
    /// it. The scene that was playing keeps playing.
    fn scene_failed(&mut self, index: usize, name: &str, error: &str) {
        let summary = summarize(error);
        self.set_health(index, Health::Failed(summary.clone()));
        self.load_failure = Some(LoadFailure {
            scene: name.to_string(),
            summary,
        });
    }

    /// Steps to the next (`delta` 1) or previous (`delta` -1) scene in the
    /// library, skipping scenes known to be broken.
    fn step_scene(&mut self, delta: isize) {
        // With a playlist, previous and next are cue commands (D156).
        if let Some(show) = self.stack.show.as_ref() {
            if show.phase() == Phase::Ready {
                let command = if delta > 0 {
                    Command::Next
                } else {
                    Command::Previous
                };
                self.playlist_command(command);
            }
            return;
        }
        if let Some(index) = self.step_target(delta) {
            self.request_load(index);
        }
    }

    /// The scene a step of `delta` would land on.
    fn step_target(&self, delta: isize) -> Option<usize> {
        let failed = |index: usize| matches!(self.health.get(index), Some(Health::Failed(_)));
        crate::library::step(self.playing, self.library.entries.len(), delta, failed)
    }

    /// Prepares scene `index` in the browser's preview, without touching
    /// the live scene.
    fn preview_load(&mut self, index: usize) {
        let Some(entry) = self.library.entries.get(index).cloned() else {
            return;
        };
        if !self.make_current(Surface::Control) {
            return;
        }
        let output = self.stack.output.window().inner_size();
        let aspect = output.height.max(1) as f32 / output.width.max(1) as f32;
        let height = ((PREVIEW_WIDTH as f32 * aspect).round() as i32).max(16);
        let size = [PREVIEW_WIDTH, height];
        match Session::load(
            self.stack.gfx.gl(),
            self.stack.gfx.caps(),
            &entry.dir,
            &entry.name,
            size,
            self.start_time,
            &mut self.audio,
        ) {
            Ok(session) => {
                self.stack.preview = Some(session);
                self.preview_index = Some(index);
                self.preview_clock = Instant::now();
                self.set_health(index, Health::Ready);
                log::info!("Previewing {} ({}x{})", entry.name, size[0], size[1]);
            }
            Err(error) => {
                failure::report(failures::scene_load(
                    &entry.name,
                    &entry.dir,
                    &error,
                    self.stack.session.is_some(),
                ));
                self.scene_failed(index, &entry.name, &error.to_string());
            }
        }
    }

    /// Throws the preview away.
    fn close_preview(&mut self) {
        self.stack.preview = None;
        self.preview_index = None;
    }

    /// Lists the library folder again, then Synesthesia's scenes when they
    /// are shown (D271), keeping what is known of each scene.
    fn rescan_library(&mut self) {
        let shown = self.synesthesia.shown(self.ui_state.synesthesia.show);
        match Library::scan_with(&self.library.dir, shown) {
            Ok(library) => {
                self.replace_library(library);
                self.scan_error = None;
            }
            Err(error) => {
                failure::report(
                    Failure::new(Subsystem::Io, "scan")
                        .path(&self.library.dir)
                        .message(&error)
                        .action("the library list was not refreshed"),
                );
                self.scan_error = Some(error);
            }
        }
    }

    /// Puts `library` in place of the one listed, keeping what is known of
    /// each scene and the playing mark by folder. The selection and the
    /// preview are let go: their indices may now name other scenes.
    fn replace_library(&mut self, library: Library) {
        let playing_dir = self
            .playing
            .and_then(|index| self.library.entries.get(index))
            .map(|entry| entry.dir.clone());
        let old: Vec<(std::path::PathBuf, Health)> = self
            .library
            .entries
            .iter()
            .zip(self.health.drain(..))
            .map(|(entry, health)| (entry.dir.clone(), health))
            .collect();
        self.health = library
            .entries
            .iter()
            .map(|entry| {
                old.iter()
                    .find(|(dir, _)| *dir == entry.dir)
                    .map_or(Health::Unknown, |(_, health)| health.clone())
            })
            .collect();
        self.library = library;
        self.library_rev += 1;
        self.playing =
            playing_dir.and_then(|old| self.library.entries.iter().position(|e| e.dir == old));
        self.close_preview();
        self.ui_state.browser.selected = None;
        self.ui_state.synesthesia.note = self.synesthesia.note(
            self.ui_state.synesthesia.show,
            self.library.synesthesia_count(),
        );
    }

    /// Lists Synesthesia's scenes after the library's own, or stops, as the
    /// Settings switch asks (D271), and saves the switch. Switching on looks
    /// for the install again, so one made since launch is found. The
    /// library's own scenes are listed again too; where its folder cannot
    /// be read, only Synesthesia's part changes.
    fn set_show_synesthesia(&mut self, show: bool) {
        self.ui_state.synesthesia.show = show;
        if show {
            self.synesthesia = Synesthesia::detect(self.synesthesia.off_this_run);
        }
        self.rescan_library();
        if self.scan_error.is_some() {
            let mut library = Library {
                dir: self.library.dir.clone(),
                entries: self.library.entries.clone(),
            };
            library.set_synesthesia(self.synesthesia.shown(show));
            self.replace_library(library);
        }
        log::info!(
            "Synesthesia's scenes {}: {}",
            if show { "on" } else { "off" },
            self.ui_state.synesthesia.note
        );
    }

    /// Copies the scene folder `dir` into the person's own library under a
    /// free name ([`intake::copy_as_own`], D271), lists the library again,
    /// and selects the copy. When the scene playing (by hand) is the one
    /// copied, the show carries on from the copy without a reload, since
    /// its files are the same: what Strobe writes for it from then on (tags,
    /// a picture, presets) lands in the copy. An Editor open on it moves to
    /// the copy with any unsaved edits.
    fn copy_to_library(&mut self, dir: &Path) {
        let name = strobe_scene::load::folder_title(dir);
        let landed = match intake::copy_as_own(dir, &self.library.dir) {
            Ok(landed) => landed,
            Err(why) => {
                failure::report(
                    Failure::new(Subsystem::Io, "copy to library")
                        .path(dir)
                        .message(&why)
                        .action("nothing was added to the library"),
                );
                self.say(Tone::Error, format!("Could not copy {name}: {why}."));
                return;
            }
        };
        let copy = strobe_scene::load::folder_title(&landed);
        log::info!("copied {} into the library as {copy}", dir.display());
        self.rescan_library();
        let index = self.library.entries.iter().position(|e| e.dir == landed);
        if self.stack.show.is_none()
            && let Some(session) = self.stack.session.as_mut()
            && session.dir == dir
        {
            session.dir.clone_from(&landed);
            session.name.clone_from(&copy);
            self.playing = index;
            self.ui_state.last_scene.clone_from(&copy);
            context::set(context::SCENE, &copy);
            if let Some(index) = index {
                self.set_health(index, Health::Ready);
            }
        }
        if let Some(doc) = self.editor_doc.as_mut()
            && doc.dir == dir
        {
            doc.dir.clone_from(&landed);
            doc.name.clone_from(&copy);
        }
        let row = index.and_then(|index| {
            self.ui_state
                .browser
                .visible(&self.library, &self.health, self.library_rev)
                .iter()
                .position(|visible| *visible == index)
        });
        self.ui_state.browser.selected = index;
        self.ui_state.browser.scroll_to = row;
        self.say(
            Tone::Info,
            format!("{name} is now in your library as {copy}, yours to change."),
        );
    }

    /// Whether the Editor's scene is one of Synesthesia's, worked out once
    /// per scene opened.
    fn editor_read_only(&mut self) -> bool {
        let Some(doc) = self.editor_doc.as_ref() else {
            return false;
        };
        if let Some((dir, read_only)) = &self.editor_read_only
            && *dir == doc.dir
        {
            return *read_only;
        }
        let read_only = self.synesthesia.owns(&doc.dir);
        self.editor_read_only = Some((doc.dir.clone(), read_only));
        read_only
    }

    // ------------------------------------------------------------ playlists

    /// The size the live scene renders at: the output window times the
    /// render scale.
    fn full_render_size(&self) -> [i32; 2] {
        let window = self.stack.output.window().inner_size();
        session::render_size(
            [window.width, window.height],
            self.render_scale,
            self.stack.gfx.caps().info.max_texture_size,
        )
    }

    /// Runs `f` on the show with an [`Env`] built from the context, the live
    /// scene's slot, the compositor and the audio. The show is taken out of
    /// the stack for the call, so the borrows are disjoint, and put back.
    fn run_show<R>(&mut self, f: impl FnOnce(&mut Show, &mut Env<'_>) -> R) -> Option<R> {
        let mut show = self.stack.show.take()?;
        let full = self.full_render_size();
        let window = self.stack.output.window().inner_size();
        let live = self
            .stack
            .session
            .as_ref()
            .map(|session| session.player.render_size());
        let mouse = self
            .mouse
            .uniforms([window.width, window.height], show_mouse_size(live, full));
        let date = self.local_date.now();
        let result = {
            let mut env = Env {
                gl: self.stack.gfx.gl(),
                caps: self.stack.gfx.caps(),
                active: &mut self.stack.session,
                compositor: &mut self.stack.compositor,
                audio: &mut self.audio,
                full_size: full,
                start_time: self.start_time,
                mouse,
                date,
                media: self.media_override.as_deref(),
            };
            f(&mut show, &mut env)
        };
        self.stack.show = Some(show);
        Some(result)
    }

    /// Acts on what a show frame decided.
    fn apply_show_report(&mut self, report: show::FrameReport) {
        if let Some(on) = report.blackout
            && self.blackout != on
        {
            self.blackout = on;
            log::info!("blackout {}", if on { "on" } else { "off" });
        }
        if let Some(notice) = report.notice {
            self.say(
                if notice.warning {
                    Tone::Warning
                } else {
                    Tone::Info
                },
                notice.text,
            );
        }
    }

    /// Drops the host-only actions ([`crate::desktop::host_only`]) a UI
    /// pass that carried remote desktop input produced, from `from` on, and
    /// says so on the banner both screens show (D275).
    fn refuse_remote(&mut self, actions: &mut Vec<Action>, from: usize) {
        let mut position = 0;
        let mut refused = false;
        actions.retain(|action| {
            position += 1;
            let refuse = position > from && crate::desktop::host_only(action);
            if refuse {
                log::warn!("remote desktop: refused {action:?}; it is host-only");
            }
            refused |= refuse;
            !refuse
        });
        if refused {
            self.say(
                Tone::Warning,
                "A remote desktop cannot pair devices, start listeners or change OSC. Do that \
                 at this computer.",
            );
        }
    }

    /// Shows a banner.
    fn say(&mut self, tone: Tone, text: impl Into<String>) {
        self.notice = Some(Notice {
            tone,
            text: text.into(),
            since: Instant::now(),
            offer: None,
        });
    }

    /// Whether `dir` is a scene folder of Synesthesia's, which Strobe never
    /// writes into (D271). When it is, the write is refused with a notice
    /// that says why and offers Copy to my library; `what` finishes "To
    /// ..., copy it into your library first" ("tag it", "save presets").
    fn refuse_read_only(&mut self, dir: &Path, name: &str, what: &str) -> bool {
        let Some(notice) = read_only_refusal(&self.synesthesia, dir, name, what) else {
            return false;
        };
        log::info!(
            "{name} is one of Synesthesia's scenes ({}); refused to {what}",
            dir.display()
        );
        self.notice = Some(notice);
        true
    }

    /// Notices a new current cue: the browser's "playing" mark follows it,
    /// the crash report names its scene, and the adaptive controller waits a
    /// second before judging the new load (D64).
    fn sync_playing(&mut self, now: Instant) {
        let cue = self
            .stack
            .show
            .as_ref()
            .and_then(|show| show.clock().current());
        if cue == self.last_cue {
            return;
        }
        self.last_cue = cue;
        if cue.is_none() {
            return;
        }
        if let Some(session) = self.stack.session.as_ref() {
            self.playing = self
                .library
                .entries
                .iter()
                .position(|entry| entry.dir == session.dir);
            context::set(context::SCENE, &session.name);
        }
        self.scene_loaded_for_perf(now);
    }

    /// Tells the performance controller a scene went live. A scene that
    /// pins its final pass (D280) draws the same at every scale, so the
    /// scale goes back to the ceiling with it and the next resize is a no-op.
    fn scene_loaded_for_perf(&mut self, now: Instant) {
        let pinned = self
            .stack
            .session
            .as_ref()
            .and_then(|session| session.player.pinned_size());
        self.stack.perf.scene_loaded(now, pinned);
        if pinned.is_some() {
            self.render_scale = self.stack.perf.scale();
        }
    }

    /// Frames of playback since the show became ready, for screenshot
    /// timing; `None` while it is still compiling.
    fn shot_frames(&self) -> Option<u64> {
        match self.stack.show.as_ref() {
            None => Some(self.output_frames),
            Some(show) if show.phase() == Phase::Ready => Some(
                self.output_frames
                    .saturating_sub(self.ready_frames.unwrap_or(0)),
            ),
            Some(_) => None,
        }
    }

    /// Runs a transport command on the playlist's clock.
    fn playlist_command(&mut self, command: Command) -> bool {
        self.stack
            .show
            .as_mut()
            .is_some_and(|show| show.command(command))
    }

    /// Loads a playlist file. A file that does not parse is refused with the
    /// precise error and the previous playlist (if any) keeps playing.
    fn load_playlist(&mut self, path: &Path) {
        match strobe_playlist::file::load(path) {
            Ok(playlist) => {
                self.drop_show();
                let resume = if self.resume_enabled {
                    show::resume::load(&show::resume::path(&self.data_dir))
                } else {
                    None
                };
                self.playlist_ui.path = path.display().to_string();
                self.ui_state.remember_playlist(&self.playlist_ui.path);
                self.playlist_ui.status = None;
                self.playlist_ui.dirty = false;
                self.playlist_ui.external_change = None;
                self.start_show(Some(path.to_path_buf()), playlist, resume);
            }
            Err(error) => {
                failure::report(
                    Failure::new(Subsystem::Playlist, "load")
                        .path(path)
                        .message(&error)
                        .action(if self.stack.show.is_some() {
                            "keeping the previous playlist"
                        } else {
                            "no playlist loaded"
                        }),
                );
                self.playlist_ui.status = Some((true, error.to_string()));
                self.ui_state.forget_playlist(&path.display().to_string());
                self.say(Tone::Error, format!("Playlist not loaded: {error}"));
            }
        }
    }

    /// Starts compiling and playing `playlist`.
    fn start_show(
        &mut self,
        source: Option<PathBuf>,
        playlist: Playlist,
        resume: Option<ResumePoint>,
    ) {
        let writer = self
            .resume_enabled
            .then(|| show::resume::ResumeWriter::start(show::resume::path(&self.data_dir)));
        let (name, cues) = (playlist.name.clone(), playlist.cue_count());
        let show = Show::new(source, playlist, &self.library, resume, writer);
        log::info!(
            "playlist \"{name}\": {cues} cue(s), {} step(s) to compile before it plays",
            show.progress().total
        );
        self.stack.show = Some(show);
        self.last_cue = None;
        self.ready_frames = None;
    }

    /// Closes the playlist, saving where it was. The live scene keeps
    /// playing.
    fn drop_show(&mut self) {
        if let Some(mut show) = self.stack.show.take() {
            show.finish();
        }
        self.last_cue = None;
        self.ready_frames = None;
    }

    /// [`Self::drop_show`], saying why.
    fn unload_show(&mut self, why: &str) {
        if self.stack.show.is_none() {
            return;
        }
        self.drop_show();
        log::info!("playlist unloaded: {why}");
        self.say(
            Tone::Info,
            format!("Playlist stopped: {why}. The current scene keeps playing."),
        );
    }

    /// Compiles one more of the playlist's scenes, if it still has some.
    /// Called once per control-window frame, after the frame is painted, so
    /// no frame is blocked for more than one scene (D151).
    fn compile_show_step(&mut self) {
        let compiling = self
            .stack
            .show
            .as_ref()
            .is_some_and(|show| show.phase() == Phase::Compiling);
        if !compiling {
            return;
        }
        let Some(Some(step)) = self.run_show(|show, env| show.compile_step(env)) else {
            return;
        };
        if let Some(index) = step.library_index {
            let health = match step.failure {
                Some(reason) => Health::Failed(summarize(&reason)),
                None => Health::Ready,
            };
            self.set_health(index, health);
        }
    }

    /// The playlist file was saved: reload it, keeping the old playlist if
    /// the new one does not parse.
    fn reload_playlist_file(&mut self, path: PathBuf) {
        match strobe_playlist::file::load(&path) {
            Ok(playlist) => {
                if let Some(show) = self.stack.show.as_mut() {
                    log::info!("playlist file changed: reloading {}", path.display());
                    show.replace_playlist(playlist, &self.library);
                    self.playlist_ui.status = Some((false, format!("Reloaded {}", path.display())));
                    self.playlist_ui.dirty = false;
                    self.playlist_ui.external_change = None;
                }
            }
            Err(error) => {
                failure::report(
                    Failure::new(Subsystem::Playlist, "reload")
                        .path(&path)
                        .message(&error)
                        .action("keeping the playlist that is playing"),
                );
                self.playlist_ui.status = Some((true, error.to_string()));
                self.say(
                    Tone::Warning,
                    format!("The saved playlist has an error; the old one keeps playing. {error}"),
                );
            }
        }
    }

    /// Looks at the playlist file once a second.
    fn poll_playlist_file(&mut self, dt: f32) {
        let Some(path) = self
            .stack
            .show
            .as_mut()
            .and_then(|show| show.poll_file(f64::from(dt)))
        else {
            return;
        };
        if self.playlist_ui.dirty {
            // Unsaved edits are never thrown away by a change on disk
            // (D213): the pane asks which to keep.
            log::info!(
                "playlist file changed on disk with unsaved edits: keeping the edits ({})",
                path.display()
            );
            self.playlist_ui.external_change = Some(path.display().to_string());
            return;
        }
        self.reload_playlist_file(path);
    }

    /// Makes one change to a cue of the loaded playlist (D213). The new
    /// playlist goes through the same replace a reload does, so only new
    /// scenes are compiled and the show keeps its place.
    fn edit_cue(&mut self, at: crate::show::edit::At, edit: &crate::show::edit::CueEdit) {
        let Some(show) = self.stack.show.as_mut() else {
            return;
        };
        match crate::show::edit::apply(show.playlist(), at, edit) {
            Ok((playlist, landed)) => {
                show.replace_playlist(playlist, &self.library);
                self.playlist_ui.dirty = true;
                self.playlist_ui.status = None;
                // Rows may have moved; a multi-selection would point at
                // the wrong ones.
                self.playlist_ui.also.clear();
                self.playlist_ui.selected = landed.and_then(|landed| {
                    show.rows()
                        .iter()
                        .find(|row| row.section == landed.section && row.cue == landed.cue)
                        .map(|row| row.index)
                });
                log::info!(
                    "playlist: {edit:?} at section {} cue {}",
                    at.section + 1,
                    at.cue + 1
                );
            }
            Err(why) => {
                self.playlist_ui.status = Some((true, format!("Could not change the cue: {why}")));
            }
        }
    }

    /// Makes one setting edit to several cues (D230). The selection stays.
    fn edit_cues(&mut self, ats: &[crate::show::edit::At], edit: &crate::show::edit::CueEdit) {
        let Some(show) = self.stack.show.as_mut() else {
            return;
        };
        match crate::show::edit::apply_many(show.playlist(), ats, edit) {
            Ok(playlist) => {
                show.replace_playlist(playlist, &self.library);
                self.playlist_ui.dirty = true;
                self.playlist_ui.status = None;
                if matches!(edit, crate::show::edit::CueEdit::SetEnabled(false)) {
                    // The disabled cues leave the timeline; so does the
                    // selection that named them.
                    self.playlist_ui.also.clear();
                    self.playlist_ui.selected = None;
                }
                log::info!("playlist: {edit:?} on {} cues", ats.len());
            }
            Err(why) => {
                self.playlist_ui.status = Some((true, format!("Could not change the cues: {why}")));
            }
        }
    }

    /// Runs the Generate form and keeps the result as a preview.
    fn generate_preview(&mut self) {
        let (result, excluded) = match self.playlist_ui.form.spec() {
            Ok(spec) => {
                let snapshot = show::generate::snapshot(&self.library);
                // Which scenes the exclude field left out, for the preview
                // to say (D264).
                let excluded = show::generate::excluded(&snapshot, &spec);
                let made = strobe_playlist::generate(&snapshot, &spec)
                    .map_err(|e| show::generate::explain(&e, &self.library, &excluded));
                (made, excluded)
            }
            Err(error) => (Err(error), Vec::new()),
        };
        self.playlist_ui.preview = Some(result);
        self.playlist_ui.excluded = excluded;
    }

    /// Adds the previewed section to the playlist (starting one if none is
    /// loaded).
    fn generate_commit(&mut self) {
        let Some(Ok(made)) = self.playlist_ui.preview.take() else {
            return;
        };
        let name = made.section.name.clone();
        let cues = made.section.cues.len();
        match self.stack.show.as_mut() {
            Some(show) => {
                let mut playlist = show.playlist().clone();
                playlist.sections.push(made.section);
                show.replace_playlist(playlist, &self.library);
            }
            None => {
                let mut playlist = Playlist::new("Generated");
                playlist.sections.push(made.section);
                self.start_show(None, playlist, None);
            }
        }
        self.playlist_ui.dirty = true;
        self.playlist_ui.generate_open = false;
        self.playlist_ui.status = Some((
            false,
            format!("Added section \"{name}\" with {cues} cue(s). Save to keep it."),
        ));
    }

    /// Writes the playlist to its file, or to the path in the field.
    fn save_playlist(&mut self) {
        let Some(show) = self.stack.show.as_mut() else {
            return;
        };
        let target = show.source().map(Path::to_path_buf).or_else(|| {
            let typed = self.playlist_ui.path.trim();
            (!typed.is_empty()).then(|| PathBuf::from(typed))
        });
        let Some(target) = target else {
            self.playlist_ui.status = Some((
                true,
                "Type a file path in the field above, then Save.".to_string(),
            ));
            return;
        };
        match strobe_playlist::file::save(show.playlist(), &target) {
            Ok(()) => {
                show.set_source(target.clone());
                show.accept_file();
                self.playlist_ui.dirty = false;
                self.playlist_ui.external_change = None;
                self.playlist_ui.status = Some((false, format!("Saved {}", target.display())));
                log::info!("playlist saved to {}", target.display());
            }
            Err(error) => {
                failure::report(
                    Failure::new(Subsystem::Playlist, "save")
                        .path(&target)
                        .message(&error)
                        .action("the playlist was not written"),
                );
                self.playlist_ui.status = Some((true, error.to_string()));
            }
        }
    }

    // ------------------------------------------------------ Shadertoy import

    /// A file or folder dropped on a window, sent where it belongs
    /// (`intake::classify`, D218): the import worker, the playlist loader,
    /// `syn_Media`, or the library.
    fn dropped(&mut self, path: &std::path::Path) {
        let kind = intake::classify(path);
        log::info!("dropped {} ({kind:?})", path.display());
        match kind {
            intake::Dropped::Shadertoy => {
                self.submit_import(Input::Text(path.display().to_string()), false);
            }
            intake::Dropped::Playlist => {
                self.ui_state.panes.open(Pane::Playlist);
                self.load_playlist(path);
            }
            intake::Dropped::Media => self.set_media(Some(path.to_path_buf())),
            intake::Dropped::Scene(dir) => self.dropped_scene(&dir),
            intake::Dropped::Pack(scenes) => {
                log::info!("adding {} scenes to the library", scenes.len());
                match self.pack.as_mut() {
                    Some(pack) => pack.extend(scenes),
                    None => self.pack = Some(intake::Pack::new(scenes)),
                }
            }
            intake::Dropped::Unknown(why) => {
                log::warn!("dropped {}: {why}", path.display());
                self.say(Tone::Warning, why);
            }
        }
    }

    /// Plays a dropped scene folder: from where it is if the library has
    /// it, else copied into the library first, the way an import lands.
    fn dropped_scene(&mut self, dir: &std::path::Path) {
        if let Some(index) = self
            .library
            .entries
            .iter()
            .position(|e| intake::same_dir(&e.dir, dir))
        {
            self.load(index);
            return;
        }
        match intake::copy_scene(dir, &self.library.dir) {
            Ok(landed) => {
                log::info!("copied {} into the library", landed.display());
                self.show_imported(&landed);
            }
            Err(why) => {
                failure::report(
                    Failure::new(Subsystem::Io, "drop")
                        .path(dir)
                        .message(&why)
                        .action("nothing was added to the library"),
                );
                self.say(Tone::Warning, format!("Could not add the scene: {why}."));
            }
        }
    }

    /// Takes the scenes that have settled in the watched folder into the
    /// pack queue (D240).
    fn poll_inbox(&mut self, now: Instant) {
        let Some(inbox) = self.inbox.as_mut() else {
            return;
        };
        let dt = now.duration_since(self.inbox_polled).as_secs_f32();
        self.inbox_polled = now;
        let scenes = inbox.poll(dt);
        if scenes.is_empty() {
            return;
        }
        log::info!(
            "watched folder: {} new scene(s) in {}",
            scenes.len(),
            inbox.dir().display()
        );
        match self.pack.as_mut() {
            Some(pack) => pack.extend(scenes),
            None => self.pack = Some(intake::Pack::new(scenes)),
        }
    }

    /// Adds the next scene of a dropped folder of scenes to the library,
    /// one a frame so the show keeps running; when the last is done,
    /// rescans and says how it went (D235).
    fn step_pack(&mut self) {
        let Some(pack) = self.pack.as_mut() else {
            return;
        };
        let entries = &self.library.entries;
        if pack.step(&self.library.dir, |dir| {
            entries.iter().any(|e| intake::same_dir(&e.dir, dir))
        }) {
            return;
        }
        let Some(pack) = self.pack.take() else {
            return;
        };
        for (dir, why) in pack.refused() {
            failure::report(
                Failure::new(Subsystem::Io, "drop")
                    .path(dir)
                    .message(why)
                    .action("the scene was not added to the library"),
            );
        }
        if pack.added() > 0 {
            self.rescan_library();
        }
        let tone = if pack.refused().is_empty() {
            Tone::Info
        } else {
            Tone::Warning
        };
        self.say(tone, pack.summary());
    }

    /// Queues an import on the worker. Never blocks.
    fn submit_import(&mut self, input: Input, replace: bool) {
        match self.import.submit(input.clone(), replace) {
            Ok(()) => self.import_state.queued(&input),
            Err(error) => self.import_state.refused(error),
        }
    }

    /// Takes the worker's finished jobs (non-blocking) and applies them.
    fn drain_imports(&mut self) {
        while let Some(done) = self.import.try_next() {
            let failed = done.result.as_ref().err().map(ToString::to_string);
            let label = done.input.label();
            // Only the import the cue card asked for becomes a cue.
            let for_cue = match (&self.pending_cue_import, &done.input) {
                (Some((_, wanted)), Input::Text(text)) if text == wanted => {
                    self.pending_cue_import.take().map(|(at, _)| at)
                }
                _ => None,
            };
            match (self.import_state.finished(done), for_cue) {
                (Outcome::Imported(dir), Some(at)) => self.cue_from_import(at, &dir),
                (Outcome::Exists(dir), Some(at)) => {
                    // Wanted for a cue: the copy already in the library
                    // will do, so there is nothing to ask.
                    self.import_state.prompt = None;
                    self.import_state.status = import::Status::Idle;
                    self.cue_from_import(at, &dir);
                }
                (Outcome::Imported(dir), None) => {
                    self.import_state.last = Some(dir.clone());
                    self.show_imported(&dir);
                }
                (Outcome::Exists(_), None) => {}
                (Outcome::Unchanged, _) => {
                    // "Already in the library" is a question, not a failure.
                    if let Some(message) = failed.filter(|_| self.import_state.prompt.is_none()) {
                        failure::report(
                            Failure::new(Subsystem::Import, "import")
                                .asset(label)
                                .message(message)
                                .action("nothing was added to the library"),
                        );
                    }
                }
            }
        }
    }

    /// Adds the imported (or already present) scene at `dir` as a cue after
    /// `at` (D229). The library is rescanned so the cue's name resolves;
    /// nothing is loaded by hand, so the show keeps playing.
    fn cue_from_import(&mut self, at: crate::show::edit::At, dir: &Path) {
        self.rescan_library();
        let name = self
            .library
            .entries
            .iter()
            .find(|e| e.dir == dir)
            .map(|e| e.name.clone())
            .or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()));
        let Some(name) = name else {
            return;
        };
        log::info!(
            "playlist: {name} imported and added after cue {}",
            at.cue + 1
        );
        self.edit_cue(
            at,
            &crate::show::edit::CueEdit::InsertAfter(strobe_playlist::SceneRef::Library(name)),
        );
    }

    /// Opens `name` (a library entry), or the playing scene, in the Editor
    /// (D204). An Editor holding unsaved edits to another scene keeps them
    /// and says so, rather than throwing them away.
    fn open_editor(&mut self, name: Option<&str>) {
        let target = match name {
            Some(name) => self
                .library
                .entries
                .iter()
                .find(|e| e.name == name)
                .map(|e| (e.dir.clone(), e.name.clone())),
            None => self
                .stack
                .session
                .as_ref()
                .map(|s| (s.dir.clone(), s.name.clone())),
        };
        let Some((dir, scene)) = target else {
            self.say(Tone::Warning, "There is no scene to open in the Editor.");
            return;
        };
        self.ui_state.panes.open(Pane::Editor);
        if let Some(doc) = self.editor_doc.as_ref() {
            if doc.dir == dir {
                return;
            }
            if doc.is_dirty() {
                self.say(
                    Tone::Warning,
                    format!(
                        "{} has unsaved edits in the Editor: save or revert them before opening {scene}.",
                        doc.name
                    ),
                );
                return;
            }
        }
        match editor::Doc::open(&dir, &scene) {
            Ok(doc) => {
                log::info!("editor: opened {} ({} files)", scene, doc.tabs.len());
                self.editor_doc = Some(doc);
            }
            Err(error) => self.say(Tone::Error, format!("Could not open {scene}: {error}")),
        }
    }

    /// Writes `tags` into the playing scene's `scene.json`, changing
    /// nothing else in it (D205). Refused while the Editor holds unsaved
    /// edits to the same scene, so neither write loses the other.
    /// Shows `path` behind `syn_Media` in the live scene and in every one
    /// loaded after it, by hand or by a cue that names no media of its own;
    /// `None` gives each scene its own back (D217). Not saved: the choice
    /// lasts the run, like `--media`.
    fn set_media(&mut self, path: Option<std::path::PathBuf>) {
        let path = match path.map(std::path::absolute).transpose() {
            Ok(path) => path,
            Err(error) => {
                self.say(
                    Tone::Warning,
                    format!("That media path cannot be used: {error}"),
                );
                return;
            }
        };
        if let Some(path) = &path
            && !path.exists()
        {
            self.say(Tone::Warning, format!("{} was not found.", path.display()));
            return;
        }
        match &path {
            Some(path) => log::info!("media: {} chosen for syn_Media", path.display()),
            None => log::info!("media: each scene shows its own again"),
        }
        self.media_override = path;
        if self
            .stack
            .show
            .as_ref()
            .is_some_and(Show::live_cue_has_media)
        {
            self.say(
                Tone::Info,
                "This cue names its own media; your choice shows in cues that do not.",
            );
            return;
        }
        if !self.make_current(Surface::Output) {
            return;
        }
        if let Some(session) = self.stack.session.as_mut() {
            session
                .player
                .choose_media(self.media_override.as_ref().map(MediaRequest::for_path));
        }
    }

    /// A macro's key went down or up (D228): the mapper drives the macro
    /// as a momentary button would, and anything that asks for an action
    /// (a macro on a command or a preset) is carried out.
    fn macro_key(&mut self, id: u32, down: bool) {
        let mut outputs = Vec::new();
        let outcome = {
            let (scene, meta) = match self.stack.session.as_mut() {
                Some(session) => (Some(&mut session.controls), Some(&mut session.meta)),
                None => (None, None),
            };
            let mut banks = Banks { scene, meta };
            self.remote
                .engine
                .macro_key(id, down, &mut banks, &mut outputs)
        };
        if let strobe_io::Outcome::Unavailable(why) = outcome {
            log::debug!("macro key: {why}");
        }
        let mut actions = output_actions(&mut outputs);
        if !actions.is_empty() {
            self.apply(&mut actions);
        }
    }

    /// Saves the playing scene's live frame as its picture (D222): read back
    /// once, shrunk, written as `thumbnail.jpg` and named in `scene.json`.
    /// The scene's own output is used, before blackout and the post stage,
    /// so the picture is the scene and not tonight's settings.
    fn capture_thumbnail(&mut self) {
        let Some((dir, name)) = self
            .playing
            .and_then(|index| self.library.entries.get(index))
            .map(|e| (e.dir.clone(), e.name.clone()))
        else {
            self.say(Tone::Warning, "No scene is playing to take a picture of.");
            return;
        };
        if self.refuse_read_only(&dir, &name, "give it a picture") {
            return;
        }
        if self
            .editor_doc
            .as_ref()
            .is_some_and(|doc| doc.dir == dir && doc.is_dirty())
        {
            self.say(
                Tone::Warning,
                format!("{name} has unsaved edits in the Editor: save or revert them first."),
            );
            return;
        }
        if !self.make_current(Surface::Output) {
            return;
        }
        let Some(session) = self.stack.session.as_ref() else {
            return;
        };
        let [w, h] = session.player.render_size();
        let pixels = session.player.read_output();
        match snapshot::save(&dir, [w.max(0) as u32, h.max(0) as u32], pixels) {
            Ok(path) => {
                log::info!("thumbnail: {} written for {name}", path.display());
                self.ui_state.thumbs.forget(&path);
                self.refresh_tags(&dir);
                if self.editor_doc.as_ref().is_some_and(|doc| doc.dir == dir) {
                    self.reopen_editor(&dir, &name);
                }
                self.say(Tone::Info, format!("{name} has a new picture."));
            }
            Err(why) => self.say(Tone::Error, format!("Could not save the picture: {why}")),
        }
    }

    /// The library entry the Import pane last imported. Its folder is
    /// looked for in the library only when the import or the library
    /// changed, because comparing folders asks the file system.
    fn imported_entry(&mut self) -> Option<usize> {
        let Some(last) = self.import_state.last.as_deref() else {
            self.imported_entry = None;
            return None;
        };
        if let Some((rev, dir, index)) = &self.imported_entry
            && *rev == self.library_rev
            && dir == last
        {
            return *index;
        }
        let index = self
            .library
            .entries
            .iter()
            .position(|entry| same_folder(&entry.dir, last));
        self.imported_entry = Some((self.library_rev, last.to_path_buf(), index));
        index
    }

    /// Writes `tags` into the `scene.json` of the library scene called
    /// `scene`, playing or not (D205, D264).
    fn set_scene_tags(&mut self, scene: &str, tags: &[String]) {
        let Some((dir, name)) = self
            .library
            .entries
            .iter()
            .find(|e| e.name == scene)
            .map(|e| (e.dir.clone(), e.name.clone()))
        else {
            self.say(
                Tone::Warning,
                format!("{scene} is no longer in the library, so it was not tagged."),
            );
            return;
        };
        // A name in both places is the person's own (found first); one only
        // Synesthesia has is refused (D271).
        if self.refuse_read_only(&dir, &name, "tag it") {
            return;
        }
        if self
            .editor_doc
            .as_ref()
            .is_some_and(|doc| doc.dir == dir && doc.is_dirty())
        {
            self.say(
                Tone::Warning,
                format!(
                    "{name} has unsaved edits in the Editor: save or revert them, then tag it."
                ),
            );
            return;
        }
        let path = dir.join("scene.json");
        let text = match std::fs::read_to_string(&path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => "{}\n".to_string(),
            Err(error) => {
                self.say(
                    Tone::Error,
                    format!("Could not read {name}'s scene.json: {error}"),
                );
                return;
            }
        };
        let written = match strobe_scene::tags::write(&text, tags)
            .map_err(|e| e.to_string())
            .and_then(|out| editor::write_atomically(&path, &out).map_err(|e| e.to_string()))
        {
            Ok(()) => true,
            Err(error) => {
                self.say(Tone::Error, format!("Could not tag {name}: {error}"));
                false
            }
        };
        if !written {
            return;
        }
        self.refresh_tags(&dir);
        if self.editor_doc.as_ref().is_some_and(|doc| doc.dir == dir) {
            // Not dirty (checked above): show what is now on disk.
            self.reopen_editor(&dir, &name);
        }
        log::info!("tags: {name} is now [{}]", tags.join(", "));
    }

    /// Applies a shortcut change at once (D208): the bindings are rebuilt
    /// from the table, which the remote's saver writes to
    /// `show-control.json`. A refusal (another command holds the key) is
    /// handed to the Settings pane to ask about.
    fn hotkey_changed(
        &mut self,
        command: CommandId,
        chord: Option<commands::Chord>,
        result: Result<(), CommandId>,
    ) {
        match result {
            Ok(()) => {
                let (bindings, problems) = Bindings::from_config(&self.remote.hotkeys);
                for problem in problems {
                    log::warn!("{problem}");
                }
                self.bindings = bindings;
                self.ui_state.hotkeys.clash = None;
                log::info!(
                    "hotkeys: {} is now {}",
                    command.id(),
                    self.bindings
                        .chord(command)
                        .map_or_else(|| commands::UNBOUND.to_owned(), commands::Chord::label)
                );
            }
            Err(holder) => {
                if let Some(chord) = chord {
                    self.ui_state.hotkeys.clash = Some((command, chord, holder));
                }
            }
        }
    }

    /// Reads the tags of the library entry in `dir` again, and tells the
    /// browser to refilter.
    fn refresh_tags(&mut self, dir: &Path) {
        if let Some(entry) = self.library.entries.iter_mut().find(|e| e.dir == dir) {
            entry.reread_tags();
            self.library_rev += 1;
        }
    }

    /// Writes the Editor's unsaved files. Returns whether everything was
    /// written.
    /// Re-reads the Editor's open scene from disk after Strobe itself wrote
    /// to it (a thumbnail, tags). If it cannot be read, the Editor keeps
    /// what it showed and says why, rather than going blank without a word.
    fn reopen_editor(&mut self, dir: &std::path::Path, name: &str) {
        match editor::Doc::open(dir, name) {
            Ok(doc) => self.editor_doc = Some(doc),
            Err(error) => self.say(
                Tone::Warning,
                format!("The Editor could not re-read {name}; it shows the previous text: {error}"),
            ),
        }
    }

    fn editor_save(&mut self) -> bool {
        let Some((dir, name)) = self
            .editor_doc
            .as_ref()
            .map(|doc| (doc.dir.clone(), doc.name.clone()))
        else {
            return false;
        };
        // The edits stay in the Editor; Copy to my library carries them to
        // the copy, where Save then writes (D271).
        if self.refuse_read_only(&dir, &name, "save edits to it") {
            return false;
        }
        let Some(doc) = self.editor_doc.as_mut() else {
            return false;
        };
        match doc.save(&editor::backup_root()) {
            Ok(count) => {
                if count > 0 {
                    log::info!("editor: saved {count} file(s) of {}", doc.name);
                    let dir = doc.dir.clone();
                    self.refresh_tags(&dir);
                }
                true
            }
            Err(error) => {
                let name = doc.name.clone();
                self.say(Tone::Error, format!("Could not save {name}: {error}"));
                false
            }
        }
    }

    /// Checks the Editor's `scene.json` before a Save and reload (D263). A
    /// file the loader would refuse is kept in the Editor with the reason,
    /// and nothing is written or reloaded, so the scene playing is untouched
    /// and no failure is recorded against it.
    fn editor_check(&mut self) -> bool {
        let Some(doc) = self.editor_doc.as_mut() else {
            return false;
        };
        match doc.check() {
            Ok(()) => {
                doc.invalid = None;
                true
            }
            Err(invalid) => {
                log::warn!(
                    "editor: {}'s scene.json does not parse, so it was not saved or reloaded: {}",
                    doc.name,
                    invalid.message
                );
                if let Some(index) = doc.tab_index(editor::SCENE_JSON) {
                    doc.active = index;
                }
                if let Some(at) = &invalid.at {
                    self.ui_state.editor.goto = Some((at.line, at.column));
                }
                doc.invalid = Some(invalid);
                false
            }
        }
    }

    /// Loads the Editor's scene again, the way Reload does: a scene that
    /// does not compile leaves the last good one playing.
    fn editor_reload(&mut self) {
        let Some(dir) = self.editor_doc.as_ref().map(|doc| doc.dir.clone()) else {
            return;
        };
        match self.library.entries.iter().position(|e| e.dir == dir) {
            Some(index) => self.request_load(index),
            None => self.say(
                Tone::Warning,
                "Saved. The scene is not in the library, so it was not reloaded.",
            ),
        }
    }

    /// Puts the active file's newest backup in the Editor, unsaved.
    fn editor_restore_backup(&mut self) {
        let Some(doc) = self.editor_doc.as_mut() else {
            return;
        };
        let Some(tab) = doc.tabs.get_mut(doc.active) else {
            return;
        };
        match editor::latest_backup(&editor::backup_root(), &doc.name, &tab.file) {
            Some(text) => tab.text = text,
            None => {
                let message = format!(
                    "{} has no backup yet: one is taken before its first save.",
                    tab.file
                );
                self.say(Tone::Info, message);
            }
        }
    }

    /// Starts a worker that generates controls for the playing scene (D202).
    /// The show keeps running: the worker has its own process and context.
    fn start_generate(&mut self) {
        if self.generate_job.is_some() {
            return;
        }
        let Some(session) = self.stack.session.as_ref() else {
            return;
        };
        let merge = session
            .controls
            .defs()
            .iter()
            .any(strobe_autocontrol::is_authored);
        let dir = session.dir.clone();
        let exe = match std::env::current_exe() {
            Ok(exe) => exe,
            Err(error) => {
                self.say(Tone::Error, format!("Cannot generate controls: {error}"));
                return;
            }
        };
        match generate::Job::spawn(&exe, &dir, &self.library.dir, merge) {
            Ok(job) => {
                log::info!(
                    "generate controls for {} started (merge {merge})",
                    dir.display()
                );
                self.generate_outcome = None;
                self.generate_since = std::time::SystemTime::now();
                self.generate_job = Some(job);
            }
            Err(error) => {
                self.say(Tone::Error, format!("Cannot generate controls: {error}"));
            }
        }
    }

    /// Takes the worker's result once it has one (non-blocking).
    fn poll_generate(&mut self) {
        let Some(job) = self.generate_job.as_mut() else {
            return;
        };
        let Some(outcome) = job.poll() else {
            return;
        };
        match &outcome {
            generate::Outcome::Done(summary) => log::info!(
                "generate controls for {}: {} control(s) from {} candidate(s), fidelity {:?}, {}",
                summary.scene,
                summary.kept.len(),
                summary.candidates,
                summary.fidelity,
                summary
                    .written
                    .as_ref()
                    .map_or_else(|| "nothing written".to_owned(), |p| p.display().to_string())
            ),
            generate::Outcome::Failed { message } => {
                log::warn!("generate controls for {}: {message}", job.scene);
            }
        }
        self.generate_job = None;
        self.ui_state.generate_review.clear();
        self.generate_read_only = match &outcome {
            generate::Outcome::Done(summary) => self
                .library
                .entries
                .iter()
                .any(|entry| entry.dir == summary.source && entry.read_only()),
            generate::Outcome::Failed { .. } => false,
        };
        self.generate_outcome = Some(outcome);
    }

    /// Saves the reviewed controls into the scene they were made for
    /// (D488): the review is applied to the copy, the files the generator
    /// rewrote are written over the scene's with backups, the copy is
    /// deleted, and the scene loads again with its new controls. Refused for
    /// one of Synesthesia's scenes, for one open in the Editor with unsaved
    /// edits, and for one changed since it was measured; the copy is then
    /// kept, so nothing is lost.
    fn save_generated(&mut self) {
        let Some(generate::Outcome::Done(summary)) = self.generate_outcome.take() else {
            return;
        };
        let Some(written) = summary.written.clone() else {
            return;
        };
        let source = summary.source.clone();
        let unsaved = self
            .editor_doc
            .as_ref()
            .is_some_and(|doc| doc.dir == source && doc.is_dirty());
        if self.generate_read_only || unsaved {
            self.say(
                Tone::Warning,
                if unsaved {
                    "The Editor has unsaved changes to this scene. Save or revert them, then save the controls."
                } else {
                    "This is one of Synesthesia's scenes, which Strobe never changes. Open the copy instead."
                },
            );
            self.generate_outcome = Some(generate::Outcome::Done(summary));
            return;
        }
        let reviews = std::mem::take(&mut self.ui_state.generate_review);
        match generate::review(&summary, &reviews, &self.library.dir) {
            Ok(generate::Reviewed::AsWritten | generate::Reviewed::Rewritten) => {}
            Ok(generate::Reviewed::Discarded) => {
                self.rescan_library();
                self.say(
                    Tone::Info,
                    "No control was kept, so nothing was saved and the copy was deleted.",
                );
                return;
            }
            Err(why) => {
                self.say(Tone::Error, format!("Could not apply the review: {why}"));
                self.ui_state.generate_review = reviews;
                self.generate_outcome = Some(generate::Outcome::Done(summary));
                return;
            }
        }
        let backups = editor::backup_root();
        match generate::save_into(
            &written,
            &source,
            &backups,
            &summary.scene,
            self.generate_since,
        ) {
            Ok(files) => {
                log::info!(
                    "generated controls saved into {}: {}",
                    source.display(),
                    files.join(", ")
                );
                if let Err(error) = generate::discard(&self.library.dir, &written) {
                    log::warn!("the copy {} was not deleted: {error}", written.display());
                }
                if self
                    .editor_doc
                    .as_ref()
                    .is_some_and(|doc| doc.dir == source)
                {
                    self.reopen_editor(&source, &summary.scene);
                }
                self.show_imported(&source);
                // No review edits means every proposed control was kept.
                let kept = if reviews.is_empty() {
                    summary.kept.len()
                } else {
                    reviews.iter().filter(|r| r.keep).count()
                };
                self.say(
                    Tone::Info,
                    format!(
                        "Saved {kept} control(s) into {}. What they replaced is backed up; the Editor's Restore backup undoes it.",
                        summary.scene
                    ),
                );
            }
            Err(why) => {
                log::warn!(
                    "generated controls not saved into {}: {why}",
                    source.display()
                );
                self.rescan_library();
                self.say(
                    Tone::Error,
                    format!("Nothing saved: {why}. The controls are still in the copy, which the library lists."),
                );
            }
        }
    }

    /// Loads the scene the worker wrote.
    fn open_generated(&mut self) {
        let Some(generate::Outcome::Done(summary)) = self.generate_outcome.take() else {
            return;
        };
        let Some(written) = summary.written.clone() else {
            return;
        };
        // The review's keep, drop and range edits (D219), applied before the
        // copy is opened.
        let reviews = std::mem::take(&mut self.ui_state.generate_review);
        match generate::review(&summary, &reviews, &self.library.dir) {
            Ok(generate::Reviewed::AsWritten) => self.show_imported(&written),
            Ok(generate::Reviewed::Rewritten) => {
                log::info!(
                    "generated controls: {} rewritten with {} of {} kept",
                    written.display(),
                    reviews.iter().filter(|r| r.keep).count(),
                    summary.kept.len()
                );
                self.show_imported(&written);
            }
            Ok(generate::Reviewed::Discarded) => {
                log::info!(
                    "generated controls: none kept, {} deleted",
                    written.display()
                );
                self.rescan_library();
                self.say(
                    Tone::Info,
                    "No control was kept, so the copy was deleted. The original is unchanged.",
                );
            }
            Err(why) => {
                self.say(Tone::Error, format!("Could not apply the review: {why}"));
                self.ui_state.generate_review = reviews;
                self.generate_outcome = Some(generate::Outcome::Done(summary));
            }
        }
    }

    /// Deletes the scene the worker wrote. Only a generated folder in the
    /// library can be deleted ([`generate::discard`]).
    fn discard_generated(&mut self) {
        let Some(generate::Outcome::Done(summary)) = self.generate_outcome.take() else {
            return;
        };
        let Some(written) = summary.written else {
            return;
        };
        match generate::discard(&self.library.dir, &written) {
            Ok(()) => {
                log::info!("discarded {}", written.display());
                self.rescan_library();
            }
            Err(error) => self.say(Tone::Error, format!("Could not discard it: {error}")),
        }
    }

    /// Rescans the library and loads the scene at `dir`, so a finished
    /// import is on screen without another click. Runs on the main thread:
    /// the scene's shaders are compiled here, never on the worker.
    fn show_imported(&mut self, dir: &std::path::Path) {
        self.rescan_library();
        if self.scan_error.is_some() {
            return;
        }
        match self.library.entries.iter().position(|e| e.dir == dir) {
            Some(index) => self.load(index),
            None => log::warn!(
                "imported scene {} is not in the rescanned library {}",
                dir.display(),
                self.library.dir.display()
            ),
        }
    }

    /// Switches the browser listener on: makes or reads the token, binds
    /// `127.0.0.1:<port>` and starts accepting. A failure is shown in the
    /// panel and the log, and the listener stays off.
    fn start_listener(&mut self) {
        if self.listener.is_some() {
            return;
        }
        let token = match self.import_token.clone() {
            Some(token) => token,
            None => match Token::load_or_create(&self.data_dir) {
                Ok(token) => token,
                Err(error) => {
                    self.listener_failed(
                        format!("Could not make the import token: {error}"),
                        Failure::new(Subsystem::Io, "token")
                            .path(&self.data_dir)
                            .message(&error)
                            .action("the browser listener stays off"),
                    );
                    return;
                }
            },
        };
        let started = Listener::start(
            ListenerConfig::new(self.listen_port, token.clone()),
            self.import.listener_handler(),
        );
        match started {
            Ok(listener) => {
                self.listener_note = format!("Listening on {}", listener.addr());
                self.listener_failed = false;
                log::info!(
                    "browser listener on {} (token in {})",
                    listener.addr(),
                    self.data_dir
                        .join(strobe_shadertoy::listener::TOKEN_FILE)
                        .display()
                );
                self.import_token = Some(token);
                self.listener = Some(listener);
            }
            Err(error) => self.listener_failed(
                error.to_string(),
                Failure::new(Subsystem::Import, "listen")
                    .detail("port", self.listen_port)
                    .message(&error)
                    .action("the browser listener stays off"),
            ),
        }
    }

    fn listener_failed(&mut self, note: String, failure: Failure) {
        self.listener_note = note;
        self.listener_failed = true;
        failure::report(failure);
    }

    /// Switches the listener off and frees the port.
    fn stop_listener(&mut self) {
        if let Some(listener) = self.listener.take() {
            let addr = listener.addr();
            listener.stop();
            log::info!("browser listener on {addr} stopped");
        }
        self.listener_note = "Off".to_string();
        self.listener_failed = false;
    }

    fn apply(&mut self, actions: &mut Vec<Action>) {
        for action in actions.drain(..) {
            match action {
                Action::Load(index) => self.request_load(index),
                Action::SetSceneTags { scene, tags } => self.set_scene_tags(&scene, &tags),
                Action::SetMedia(path) => self.set_media(path),
                Action::CaptureThumbnail => self.capture_thumbnail(),
                Action::CopyToLibrary(dir) => self.copy_to_library(&dir),
                Action::SetShowSynesthesia(show) => self.set_show_synesthesia(show),
                Action::ImportCueAfter { at, text } => {
                    self.pending_cue_import = Some((at, text.clone()));
                    self.submit_import(Input::Text(text), false);
                }
                Action::MacroKey { id, down } => self.macro_key(id, down),
                Action::EditCue { at, edit } => self.edit_cue(at, &edit),
                Action::EditCues { ats, edit } => self.edit_cues(&ats, &edit),
                Action::ReloadPlaylistFromDisk => {
                    if let Some(path) = self.playlist_ui.external_change.take() {
                        self.playlist_ui.dirty = false;
                        self.playlist_ui.external_change = None;
                        self.reload_playlist_file(PathBuf::from(path));
                    }
                }
                Action::SetHotkey {
                    command,
                    chord,
                    steal,
                } => {
                    let result = Bindings::assign(&mut self.remote.hotkeys, command, chord, steal);
                    self.hotkey_changed(command, chord, result);
                }
                Action::PinMacro { name, meta, pin } => {
                    let target = if meta {
                        strobe_io::Target::meta(&name)
                    } else {
                        strobe_io::Target::control(&name)
                    };
                    let macros = &mut self.remote.engine.macros;
                    if pin {
                        macros.pin(&name, target);
                        log::info!("macros: pinned {name}");
                    } else if let Some(id) = macros.for_target(&target).map(|m| m.id) {
                        macros.set_pinned(id, false);
                        log::info!("macros: unpinned {name}");
                    }
                }
                Action::ResetHotkey(command) => {
                    let result = Bindings::reset(&mut self.remote.hotkeys, command);
                    self.hotkey_changed(command, command.chord(), result);
                }
                Action::Reload => {
                    if let Some(index) = self.playing {
                        self.request_load(index);
                    }
                }
                Action::OpenEditor(name) => self.open_editor(name.as_deref()),
                Action::EditorSave => {
                    // `editor_save` reports its own failure; the result only
                    // gates a reload, and a plain Save has none.
                    self.editor_save();
                }
                Action::EditorSaveReload => {
                    if self.editor_check() && self.editor_save() {
                        self.editor_reload();
                    }
                }
                Action::EditorRevert => {
                    if let Some(doc) = self.editor_doc.as_mut() {
                        doc.revert();
                    }
                }
                Action::EditorRestoreBackup => self.editor_restore_backup(),
                Action::GenerateControls => self.start_generate(),
                Action::CancelGenerate => {
                    if let Some(mut job) = self.generate_job.take() {
                        job.stop();
                        log::info!("generate controls for {}: cancelled", job.scene);
                    }
                }
                Action::OpenGenerated => self.open_generated(),
                Action::SaveGeneratedToScene => self.save_generated(),
                Action::DiscardGenerated => self.discard_generated(),
                Action::DismissGenerated => self.generate_outcome = None,
                Action::Preview(index) => self.pending_preview = Some(index),
                Action::ClosePreview => self.close_preview(),
                Action::RescanLibrary => self.rescan_library(),
                Action::NextScene => self.step_scene(1),
                Action::PrevScene => self.step_scene(-1),
                Action::ToggleHold => {
                    if self.stack.show.is_some() {
                        self.playlist_command(Command::ToggleHold);
                    } else {
                        log::info!("hold requested: there is no playlist, so nothing was held");
                        self.notice = Some(Notice {
                            tone: Tone::Info,
                            text: "Hold needs a playlist. Open one in Playlist or start with \
                                   --playlist <file>. Nothing was held."
                                .to_string(),
                            since: Instant::now(),
                            offer: None,
                        });
                    }
                }
                Action::ExtendCue => {
                    self.playlist_command(Command::Extend(EXTEND_SECONDS));
                }
                Action::RestartCue => {
                    self.playlist_command(Command::RestartCue);
                }
                Action::SkipSection => {
                    self.playlist_command(Command::SkipSection);
                }
                Action::JumpToCue(index) => {
                    if !self.playlist_command(Command::JumpTo(index)) && self.stack.show.is_none() {
                        self.say(
                            Tone::Info,
                            "Jumping to a cue needs a playlist. Open one in Playlist.",
                        );
                    }
                }
                Action::LoadPlaylist(text) => self.load_playlist(Path::new(&text)),
                Action::UnloadPlaylist => self.unload_show("it was unloaded"),
                Action::GeneratePreview => self.generate_preview(),
                Action::GenerateCommit => self.generate_commit(),
                Action::SavePlaylist => self.save_playlist(),
                Action::SetBlackout(on) => {
                    if self.blackout != on {
                        self.blackout = on;
                        log::info!("blackout {}", if on { "on" } else { "off" });
                    }
                    // The clock keeps its own blackout flag (it is a cue
                    // command too); keep the two the same.
                    if self
                        .stack
                        .show
                        .as_ref()
                        .is_some_and(|show| show.clock().is_blackout() != on)
                    {
                        self.playlist_command(Command::ToggleBlackout);
                    }
                }
                Action::Quit => self.quit = true,
                Action::SetCustomTitleBar(on) => {
                    self.ui_state.custom_title_bar = on;
                    self.stack.control.window().set_decorations(!on);
                    log::info!(
                        "title bar: {}",
                        if on { "Strobe's own" } else { "the system's" }
                    );
                }
                Action::DismissFailure => self.load_failure = None,
                Action::DismissNotice => self.notice = None,
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
                Action::SetOutputVisible(visible) => self.set_output_visible(visible),
                Action::ToggleFullscreen => {
                    // The operator placed the output; a late projector
                    // no longer takes it (D451).
                    self.awaited_monitor.cancel();
                    self.toggle_fullscreen();
                }
                Action::MoveToMonitor(index) => {
                    self.awaited_monitor.cancel();
                    self.move_to_monitor(index);
                }
                Action::ClearConsole => self.console.clear(),
                Action::SelectAudioInput(choice) => self.audio.select_device(choice),
                Action::RefreshAudioDevices => self.audio.refresh_devices(),
                Action::SetReactivity(gain) => self.audio.set_reactivity(gain),
                Action::SetAudioSpeed(gain) => self.audio.set_speed(gain),
                Action::TapTempo => self.audio.tap(),
                Action::NewSong => self.song.ask(),
                Action::Import(text) => self.submit_import(Input::Text(text), false),
                Action::Browse(purpose) => {
                    let start = match purpose {
                        crate::picker::Purpose::Playlist => Some(PathBuf::from("playlists")),
                        crate::picker::Purpose::Shadertoy
                        | crate::picker::Purpose::WatchFolder
                        | crate::picker::Purpose::TrustFile => None,
                    };
                    self.picker.open(purpose, start);
                }
                Action::ConfirmReplace => {
                    if let Some(input) = self.import_state.confirm_replace() {
                        self.submit_import(input, true);
                    }
                }
                Action::CancelReplace => self.import_state.cancel_replace(),
                Action::SetWatchFolder { folder, enabled } => {
                    match watch_change(
                        &folder,
                        enabled,
                        &mut self.ui_state.watch,
                        &self.library.dir,
                    ) {
                        WatchChange::Start(inbox) => {
                            log::info!("watching {} for new scenes", inbox.dir().display());
                            self.inbox = Some(inbox);
                            self.inbox_polled = Instant::now();
                        }
                        WatchChange::Stop => {
                            if let Some(inbox) = self.inbox.take() {
                                log::info!("stopped watching {}", inbox.dir().display());
                            }
                        }
                        WatchChange::Keep => {
                            log::warn!("watched folder not changed: {}", self.ui_state.watch.note);
                        }
                    }
                }
                Action::SetImportListener(on) => {
                    if on {
                        self.start_listener();
                    } else {
                        self.stop_listener();
                    }
                }
                Action::SetLanRemote(on) => {
                    if on {
                        self.lan.start();
                    } else {
                        self.lan.stop();
                    }
                }
                Action::SetDesktopAccess(enabled) => self.lan.set_desktop(enabled),
                Action::SetProjectorControl(on) => self.projectors.set_enabled(on),
                Action::SetProjectorWeb(on) => self.projectors.set_web(on),
                Action::ProjectorPower { target, on } => self.projectors.power(&target, on),
                Action::RefreshProjectors => self.projectors.refresh(),
                Action::SaveProjectors => self.projectors.save_form(),
                Action::RevertProjectors => self.projectors.revert_form(),
                Action::NewProjectorToken => self.projectors.new_token(),
                Action::ConnectDesktop => {
                    self.lan.set_desktop(false);
                    self.desktop_controller.connect();
                    // The trust file's path (not its contents) and the
                    // choice survive a restart (D294).
                    self.desktop_controller
                        .remembered(&mut self.ui_state.remote);
                    // A trust file that cannot be read leaves no viewer, and
                    // after `--control` or the viewer's Reconnect the form
                    // holding the reason is not on screen: bring it up. The
                    // note names the path, never the file's contents.
                    if self.desktop_controller.use_trust && !self.desktop_controller.active() {
                        log::warn!(
                            "desktop controller: not connecting: {}",
                            self.desktop_controller.note()
                        );
                        self.desktop_controller.reveal_form();
                        self.ui_state
                            .open_settings(crate::ui::settings::SettingsPage::Remote);
                    }
                }
                Action::DisconnectDesktop => {
                    self.desktop_controller.disconnect();
                    // The local window is back: not the viewer's pace (D302).
                    self.next_control = Instant::now();
                }
                Action::ControlAnotherDesktop => {
                    self.desktop_controller.disconnect();
                    self.next_control = Instant::now();
                    self.desktop_controller.reveal_form();
                    self.ui_state
                        .open_settings(crate::ui::settings::SettingsPage::Remote);
                }
                Action::DecideRemotePairing { id, approve } => self.lan.decide(id, approve),
                Action::RevokeRemoteClient(id) => self.lan.revoke(&id),
                Action::RevokeAllRemoteClients => self.lan.revoke_all(),
                Action::SetOsc(settings) => self.remote.set_osc(settings),
                Action::SetMidiEnabled(enabled) => {
                    let mut settings = self.remote.midi_saved.clone();
                    settings.enabled = enabled;
                    self.remote.set_midi(settings);
                }
                Action::SetMidiFeedback(on) => {
                    let mut settings = self.remote.midi_saved.clone();
                    settings.feedback = on;
                    self.remote.set_midi(settings);
                }
                Action::SetMidiDevice(key, used) => self.remote.set_device_enabled(&key, used),
                Action::RescanMidi => self.remote.rescan_midi(),
                Action::SavePreset(name) => self.save_preset(&name),
                Action::RecallPreset(name) => self.recall_preset(&name),
                Action::DeletePreset(name) => self.delete_preset(&name),
                Action::RenamePreset { from, to } => self.rename_preset(&from, &to),
                Action::StoreFav(slot) => self.store_fav(slot),
                Action::ClearFav(slot) => self.remote.favs.clear(slot),
                Action::RecallFav(slot) => self.recall_fav(slot),
                Action::RunCommand(id) => self.run_command(&id),
                Action::ControlAction(action) => self.control_action(&action),
            }
        }
    }

    // --------------------------------------------------------- show control

    /// Step 1 of the frame (docs/ARCHITECTURE.md): runs the MIDI and OSC
    /// events that arrived since the last frame through the mapper. Control
    /// writes land in the live scene's banks through `ControlBank`; commands,
    /// cues, fav slots and presets come back as outputs and are carried out
    /// by the same actions the buttons send.
    fn drain_remote(&mut self, now: Instant) {
        if !self.remote.workers_running() {
            return;
        }
        let mut outputs = std::mem::take(&mut self.remote_out);
        outputs.clear();
        {
            let (scene, meta) = match self.stack.session.as_mut() {
                Some(session) => (Some(&mut session.controls), Some(&mut session.meta)),
                None => (None, None),
            };
            let mut banks = Banks { scene, meta };
            self.remote.drain(&mut banks, now, &mut outputs);
        }
        if !outputs.is_empty() {
            let mut actions = output_actions(&mut outputs);
            self.apply(&mut actions);
        }
        self.remote_out = outputs;
        let cue = self
            .stack
            .show
            .as_ref()
            .and_then(|show| show.clock().current())
            .and_then(|index| u32::try_from(index + 1).ok());
        let state = strobe_io::feedback::ShowState {
            blackout: self.blackout,
            cue,
        };
        let session = self.stack.session.as_ref();
        self.remote.feedback.frame(
            now,
            session.map(|s| &s.controls),
            session.map(|s| &s.meta),
            state,
        );
        self.remote.midi_feedback_frame(
            now,
            session.map(|s| &s.controls),
            session.map(|s| &s.meta),
        );
    }

    /// Step 1c of the frame: runs the LAN remote's commands and publishes
    /// what a phone shows (D254). Never waits: commands are taken with
    /// `try`, at most [`lan::MAX_PER_FRAME`] of them, each checked against
    /// what the app is doing now and run through the same action a key or
    /// an OSC message runs, then answered. Nothing happens while the remote
    /// is off.
    fn drain_lan(&mut self, now: Instant) {
        self.lan.maintain(now);
        if !self.lan.running() {
            return;
        }
        let ready = self
            .stack
            .show
            .as_ref()
            .is_some_and(|show| show.phase() == Phase::Ready);
        let step = self
            .step_target(1)
            .filter(|next| Some(*next) != self.playing);
        let situation = Situation {
            playlist: self.stack.show.is_some(),
            ready,
            can_step: step.is_some(),
        };
        for _ in 0..lan::MAX_PER_FRAME {
            let Some(incoming) = self.lan.next_command() else {
                break;
            };
            let command = incoming.command;
            let action = CommandId::from_id(command.id())
                .filter(|c| c.mappable())
                .and_then(|c| command_action(c, self.blackout));
            let answer = match (lan::refusal(command, situation), action) {
                (None, Some(action)) => {
                    let mut actions = vec![action];
                    self.apply(&mut actions);
                    log::info!("remote: ran {}", command.id());
                    Answer::Accepted
                }
                (Some(reason), _) => {
                    log::info!("remote: refused {}: {reason}", command.id());
                    Answer::Rejected(reason.to_string())
                }
                (None, None) => Answer::Rejected("not available here".to_string()),
            };
            incoming.answer(answer);
        }
        // The controls as data for the desktop holding the lease (D513),
        // every frame: its edits first, then what it should now show.
        if let Some(desktop) = self.lan.desktop() {
            let live = self
                .stack
                .session
                .as_mut()
                .map(|session| crate::remote_controls::Live {
                    title: &session.title,
                    scene: &mut session.controls,
                    output: &mut session.meta,
                });
            self.controls_sync.tick(&desktop, live, now);
        }
        if !self.lan.publish_due(now) {
            return;
        }
        let entries = &self.library.entries;
        let playing_name = self
            .playing
            .and_then(|i| entries.get(i))
            .map(|e| e.name.as_str());
        let (current, next, remaining, held) = match self.stack.show.as_ref() {
            Some(show) => {
                let transport = show_transport(show, playing_name, 0.0, self.blackout);
                (
                    transport.now,
                    transport.next,
                    transport.cue.and_then(|cue| cue.remaining),
                    transport.cue.is_some_and(|cue| cue.held),
                )
            }
            None => (
                playing_name,
                step.and_then(|i| entries.get(i)).map(|e| e.name.as_str()),
                None,
                false,
            ),
        };
        self.lan.publish(
            current,
            next,
            remaining,
            held,
            self.blackout,
            self.output_visible,
            lan::audio_status(self.audio.is_live(), self.audio.can_capture()),
        );
    }

    /// Runs a command from the command table by its id (a mapping, a macro
    /// or an OSC address named it).
    fn run_command(&mut self, id: &str) {
        let Some(command) = CommandId::from_id(id).filter(|c| c.mappable()) else {
            log::warn!("show control: there is no command \"{id}\"");
            return;
        };
        if let Some(action) = command_action(command, self.blackout) {
            let mut actions = vec![action];
            self.apply(&mut actions);
        }
    }

    /// Saves the playing scene's controls as preset `name` in its folder.
    fn save_preset(&mut self, name: &str) {
        let Some(session) = self.stack.session.as_mut() else {
            self.say(
                Tone::Info,
                "No scene is playing, so there is nothing to save.",
            );
            return;
        };
        let (dir, scene) = (session.dir.clone(), session.name.clone());
        if self.refuse_read_only(&dir, &scene, "save presets for it") {
            return;
        }
        let Some(session) = self.stack.session.as_mut() else {
            return;
        };
        let captured = session.capture_preset(name);
        match strobe_scene::preset::save(&session.dir, &captured) {
            Ok(path) => {
                log::info!("preset \"{name}\" of {scene} saved to {}", path.display());
                self.say(Tone::Info, format!("Preset \"{name}\" saved for {scene}."));
            }
            Err(error) => {
                failure::report(
                    Failure::new(Subsystem::Io, "save preset")
                        .asset(name)
                        .message(error.to_string())
                        .action("the preset is kept in memory until the scene is reloaded"),
                );
                self.say(Tone::Error, format!("Preset not saved: {error}"));
            }
        }
    }

    /// Carries out Synesthesia's default or random on a bank, a group or
    /// a control (D247), with the UI's randomness, as the Inspector's
    /// random button does.
    fn control_action(&mut self, action: &strobe_io::ControlAction) {
        let Some(session) = self.stack.session.as_mut() else {
            log::info!(
                "{}: no scene is playing",
                strobe_io::Target::Action(action.clone()).label()
            );
            return;
        };
        let bank = match action.bank {
            strobe_io::ActionBank::Scene => &mut session.controls,
            strobe_io::ActionBank::Meta => &mut session.meta,
        };
        let state = &mut self.ui_state;
        let covered = action.apply(bank, || state.random_unit());
        if covered == 0 {
            log::info!(
                "{}: nothing here to act on",
                strobe_io::Target::Action(action.clone()).label()
            );
        }
    }

    /// Recalls preset `name` of the playing scene.
    fn recall_preset(&mut self, name: &str) {
        let Some(session) = self.stack.session.as_mut() else {
            self.say(
                Tone::Info,
                "No scene is playing, so there is no preset to recall.",
            );
            return;
        };
        match session.apply_preset(name) {
            Ok(problems) => {
                log::info!("preset \"{name}\" recalled on {}", session.name);
                for problem in problems {
                    log::warn!("preset \"{name}\": {problem}");
                }
                // Recalled by hand, its media shows whatever else was
                // chosen (D223).
                if let Some(path) = session.preset_media(name) {
                    session
                        .player
                        .choose_media(Some(MediaRequest::for_path(path)));
                }
            }
            Err(problem) => {
                log::warn!("{problem}");
                self.say(Tone::Warning, problem);
            }
        }
    }

    /// The playing scene's folder and name, from its session.
    fn playing_folder(&self) -> Option<(PathBuf, String)> {
        self.stack
            .session
            .as_ref()
            .map(|session| (session.dir.clone(), session.name.clone()))
    }

    /// Deletes preset `name`'s file.
    fn delete_preset(&mut self, name: &str) {
        let Some((dir, scene)) = self.playing_folder() else {
            return;
        };
        if self.refuse_read_only(&dir, &scene, "delete its presets") {
            return;
        }
        let Some(session) = self.stack.session.as_mut() else {
            return;
        };
        match strobe_scene::preset::delete(&session.dir, name) {
            Ok(()) => {
                session.forget_preset(name);
                log::info!("preset \"{name}\" deleted");
            }
            Err(error) => {
                self.say(Tone::Error, format!("Preset not deleted: {error}"));
            }
        }
    }

    /// Renames a preset of the playing scene, and the fav slots that recall
    /// it (P-07). A playlist cue that names the old preset is not rewritten;
    /// the notice says so.
    fn rename_preset(&mut self, from: &str, to: &str) {
        let Some((dir, scene)) = self.playing_folder() else {
            return;
        };
        if self.refuse_read_only(&dir, &scene, "rename its presets") {
            return;
        }
        let Some(session) = self.stack.session.as_mut() else {
            return;
        };
        match strobe_scene::preset::rename(&session.dir, from, to) {
            Ok(_) => {
                session.rename_preset(from, to);
                let followed = self.remote.favs.rename_preset(&session.name, from, to);
                log::info!("preset \"{from}\" renamed to \"{to}\" ({followed} fav slot(s) follow)");
                self.say(
                    Tone::Info,
                    format!(
                        "Renamed \"{from}\" to \"{to}\". {followed} fav slot(s) follow it; a playlist cue that names the old preset does not."
                    ),
                );
            }
            Err(error) => self.say(Tone::Error, format!("Preset not renamed: {error}")),
        }
    }

    /// Stores the playing scene (and its active preset) in fav slot `slot`.
    fn store_fav(&mut self, slot: usize) {
        let Some(session) = self.stack.session.as_ref() else {
            self.say(
                Tone::Info,
                "No scene is playing, so there is nothing to store.",
            );
            return;
        };
        let fav = strobe_scene::preset::Fav {
            scene: session.name.clone(),
            preset: session.active_preset.clone(),
        };
        if self.remote.favs.set(slot, fav.clone()) {
            log::info!("fav slot {} holds {} {:?}", slot + 1, fav.scene, fav.preset);
        }
    }

    /// Recalls fav slot `slot`: loads its scene if another is playing, then
    /// applies its preset.
    fn recall_fav(&mut self, slot: usize) {
        let Some(fav) = self.remote.favs.get(slot).cloned() else {
            log::info!("fav slot {} is empty: nothing recalled", slot + 1);
            self.say(Tone::Info, format!("Fav slot {} is empty.", slot + 1));
            return;
        };
        let playing = self.stack.session.as_ref().map(|s| s.name.as_str());
        if playing == Some(fav.scene.as_str()) {
            match fav.preset {
                Some(name) => self.recall_preset(&name),
                None => {
                    if let Some(session) = self.stack.session.as_mut() {
                        session.reset_to_start();
                    }
                }
            }
            return;
        }
        let Some(index) = self
            .library
            .entries
            .iter()
            .position(|entry| entry.name == fav.scene)
        else {
            self.say(
                Tone::Warning,
                format!(
                    "Fav slot {} names \"{}\", which is not in the library.",
                    slot + 1,
                    fav.scene
                ),
            );
            return;
        };
        self.pending_preset = fav.preset.map(|name| (index, name));
        self.request_load(index);
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
        // The size asked for, not the size drawn: a scene that pins its
        // final pass (D280) draws at its own size whatever it is asked.
        if session.player.requested_size() == want {
            self.pending_resize = None;
            return;
        }
        let steady = matches!(
            self.pending_resize,
            Some((size, since)) if size == want && now.duration_since(since) >= RESIZE_SETTLE
        );
        if (steady || self.resize_now || self.screenshot.is_some())
            && self.resize_backoff.allows(want, now)
        {
            self.resize_now = false;
            self.pending_resize = None;
            match session.player.resize(want) {
                Ok(()) => {
                    self.resize_backoff.succeeded();
                    context::set(context::RENDER_SIZE, format!("{}x{}", want[0], want[1]));
                    if let Some(show) = self.stack.show.as_mut() {
                        show.resize(want, &mut self.stack.compositor);
                    }
                }
                Err(error) if self.resize_backoff.failed(want, now) => failure::report(
                    Failure::new(Subsystem::Context, "resize")
                        .scene(&session.name)
                        .message(error)
                        .action("keeping the previous render size; trying again every 5 s"),
                ),
                Err(error) => {
                    log::debug!("resize to {}x{} failed again: {error}", want[0], want[1])
                }
            }
        } else if !matches!(self.pending_resize, Some((size, _)) if size == want) {
            self.pending_resize = Some((want, now));
        }
    }

    /// The output window's redraw: the scene frame, then the present.
    fn redraw_output(&mut self) {
        // A redraw requested just before the window was hidden.
        if self.output_visible {
            self.scene_frame(Surface::Output);
            // Windows can stop painting the control window under the
            // output, and a playlist that compiles only on control frames
            // would then never start (seen live on the JaJa mini PC: stuck
            // at 1 of 14 behind a black output). The event loop now runs
            // those frames itself (D290); this stays as the backstop.
            const CONTROL_QUIET: Duration = Duration::from_millis(500);
            if self.last_control_frame.elapsed() > CONTROL_QUIET
                && self.rebuild.is_none()
                && self.make_current(Surface::Output)
            {
                self.compile_show_step();
            }
        }
    }

    /// One scene frame: advance, render, time the GPU, adapt the scale. It
    /// runs on `surface`'s context: the output's when that window is shown
    /// (the present and swap follow), else the control window's, which is
    /// always there to be made current (D96). Either way the scene lands in
    /// the player's own targets, which is what the control preview paints.
    fn scene_frame(&mut self, surface: Surface) {
        let started = Instant::now();
        let shown = surface == Surface::Output;
        let size = self.stack.output.window().inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        if self.faults.lose_context_due(self.output_frames) {
            self.inject_loss();
        }
        if self.faults.crash_due(self.output_frames) {
            log::error!(
                "{}: aborting on purpose after {} output frames",
                fault::ENV,
                self.output_frames
            );
            strobe_diag::logger::flush(Duration::from_secs(2));
            fault::crash();
        }
        if !self.make_current(surface) {
            return;
        }
        if !shown {
            // Nothing is swapped, so nothing paces the CPU against the GPU:
            // read what has finished, and sit this frame out while the whole
            // timer ring is still waiting.
            if let Some(change) = self.stack.perf.poll(started) {
                self.scale_changed(&change);
            }
            if self.stack.perf.backlogged() {
                return;
            }
        }
        let measured = self.stats.begin(started);
        if shown {
            // The interval since the last frame start, a CPU timestamp: not
            // quantised to the refresh, so the controller counts misses over
            // a window's span, where the jitter cancels (D314, D315).
            self.stack
                .perf
                .presented(self.stats.last_interval() * 1000.0, started);
        }
        self.perf_log.frame(
            started,
            self.stack.perf.budget_ms(),
            self.stack.perf.gpu_ms(),
            self.render_scale,
        );
        if let Some(line) = self.perf_log.take_due(started) {
            log::info!("{line}");
        }
        let (dt, wall_dt) = if self.screenshot.is_some() {
            (SCREENSHOT_DT, SCREENSHOT_DT)
        } else {
            (measured, self.stats.last_interval())
        };
        let window = [size.width as i32, size.height as i32];
        self.settle_resize([size.width, size.height], started);
        self.drain_remote(started);
        self.drain_lan(started);
        self.projectors.maintain(started);

        let gl = self.stack.gfx.gl().clone();
        let mut change = None;
        let show_ready = self
            .stack
            .show
            .as_ref()
            .is_some_and(|show| show.phase() == Phase::Ready);
        if show_ready && self.ready_frames.is_none() {
            self.ready_frames = Some(self.output_frames);
        }
        let begun = show_ready || self.stack.session.is_some();
        if begun {
            // The GPU timer brackets everything this frame renders: the live
            // scene, a playlist's outgoing and warming scenes, the mix, and
            // when the output is shown the blit to it. Its result arrives a
            // frame or two later. The budget is the same either way.
            self.stack.perf.begin();
        }
        // The live scene's meta reactivity and audio speed scale the
        // analyser (D184). Only the live session's: a playlist advances
        // two or three scenes a frame, and the analyser has one gain.
        if let Some(session) = self.stack.session.as_ref() {
            let meta = MetaValues::read(&session.meta);
            self.audio.set_meta_gains(meta.reactivity, meta.audio_speed);
        }
        // A stalled frame (a compile, a window drag) does not count as that
        // much listening, as the show's clock does not (D300).
        let heard_dt = if wall_dt.is_finite() && wall_dt > 0.0 {
            wall_dt.min(show::run::MAX_WALL_STEP)
        } else {
            dt
        };
        if self.song.step(heard_dt, &mut self.audio) {
            if self.stack.show.is_some() {
                self.run_show(Show::song_changed);
            } else if let Some(session) = self.stack.session.as_mut() {
                session.song_change();
            }
        }
        if show_ready {
            if let Some(report) = self.run_show(|show, env| show.frame(env, dt, wall_dt)) {
                self.apply_show_report(report);
            }
            self.poll_playlist_file(dt);
            self.sync_playing(started);
        } else if let Some(session) = self.stack.session.as_mut() {
            let render = session.player.render_size();
            let mouse = self.mouse.uniforms([size.width, size.height], render);
            session.advance(dt, &mut self.audio, mouse, self.local_date.now());
        }
        let composited = self.stack.show.as_ref().is_some_and(Show::composited);
        self.posted = self.apply_post(composited);
        if self.stack.session.is_some() || composited {
            if shown {
                gl.plain_state();
                if self.blackout {
                    // The scene keeps rendering (its clock runs, the
                    // preview shows it); only what the projector gets
                    // goes dark.
                    gl.bind_framebuffer(None);
                    gl.viewport(window[0], window[1]);
                    gl.clear([0.0, 0.0, 0.0, 1.0]);
                } else if self.posted {
                    self.stack.post.present(window);
                } else if composited {
                    self.stack.compositor.present(window);
                } else if let Some(session) = self.stack.session.as_ref() {
                    session.player.present(window);
                }
            }
        } else if shown {
            gl.plain_state();
            gl.bind_framebuffer(None);
            gl.viewport(window[0], window[1]);
            gl.clear([0.0, 0.0, 0.0, 1.0]);
        }
        if begun {
            change = self.stack.perf.end(started);
        }
        if let Some(change) = change {
            self.scale_changed(&change);
        }
        self.output_frames += 1;
        context::note_frame(self.output_frames);
        let shot_frames = self.shot_frames();
        if let Some(shot) = self.screenshot.as_mut()
            && let Some(frames) = shot_frames
            && shot.output_due(frames)
        {
            // A shown output is read from its own back buffer, so the run
            // checks what the projector gets. A hidden one has no defined
            // back buffer; the scene's output texture is read instead.
            let saved = if shown {
                let pixels = gl.read_default_rgba8(window[0], window[1]);
                shot.save(Target::Output, [size.width, size.height], pixels)
            } else if self.posted {
                let [w, h] = self.stack.post.size();
                let pixels = self.stack.post.read_output();
                shot.save_top_down(Target::Output, [w as u32, h as u32], pixels)
            } else if composited {
                let [w, h] = self.stack.compositor.size();
                let pixels = self.stack.compositor.read_output();
                shot.save_top_down(Target::Output, [w as u32, h as u32], pixels)
            } else if let Some(session) = self.stack.session.as_ref() {
                let [w, h] = session.player.render_size();
                let pixels = session.player.read_output();
                shot.save_top_down(Target::Output, [w as u32, h as u32], pixels)
            } else {
                let black = vec![0_u8; (size.width as usize) * (size.height as usize) * 4];
                shot.save_top_down(Target::Output, [size.width, size.height], black)
            };
            match saved {
                Ok(path) => log::info!("wrote {}", path.display()),
                Err(e) => self.failure = Some(e),
            }
        }
        self.stats.finish(started.elapsed());
        if self.check_reset(started, surface) {
            return;
        }
        if shown {
            self.present(Surface::Output, started);
            // Covered, not `control_unseen`: a window the loop drives because
            // Windows withholds its paints still makes the shared context
            // current twice a second, and on a machine where the wait is
            // engaged that switch would otherwise take the flip's wait back
            // and cost the show a frame each time (D221, D290).
            if self.swap_policy.finish_now(self.control_covered) {
                self.stack.gfx.gl().finish();
            }
        }
    }

    /// Runs the output post stage on the frame this tick produced (the
    /// compositor's mix while `composited`, else the live scene's output),
    /// reading the live session's meta controls. Returns whether it drew:
    /// at neutral it does nothing and the frame is used as it stands (D180).
    fn apply_post(&mut self, composited: bool) -> bool {
        let Some(session) = self.stack.session.as_ref() else {
            return false;
        };
        let values = MetaValues::read(&session.meta);
        if values.post_is_neutral() {
            return false;
        }
        let source = if composited {
            self.stack.compositor.output_texture()
        } else {
            session.player.output_texture()
        };
        match self.stack.post.apply(source, &values) {
            Ok(drew) => drew,
            Err(error) => {
                if !self.post_failed {
                    self.post_failed = true;
                    log::warn!("output post stage failed, showing the frame as drawn: {error}");
                }
                false
            }
        }
    }

    /// Adopts the adaptive controller's new scale and says why.
    fn scale_changed(&mut self, change: &perf::ScaleChange) {
        if let Some(futile) = change.futile {
            let next = match futile.lock_frames {
                Some(frames) => {
                    format!("not lowering it again for {frames} frames or until the scene changes")
                }
                None => "trying once more before holding it there".to_string(),
            };
            log::info!(
                "adaptive resolution: lowering the render scale {:.2} -> {:.2} cut GPU time only \
                 {:.1} -> {:.1} ms, so this scene's cost is not its resolution; back to {:.2}, \
                 {next} (D280)",
                futile.scale,
                futile.tried,
                futile.before_ms,
                futile.after_ms,
                change.to
            );
            self.render_scale = change.to;
            self.resize_now = true;
            return;
        }
        if let Some(late) = change.late_futile {
            log::info!(
                "adaptive resolution: lowering the render scale {:.2} -> {:.2} cut too few missed \
                 refreshes ({:.1}% of frames before, {:.1}% after), so the rest are not this \
                 scene's resolution; back to {:.2}, and missed refreshes lower it no further \
                 for {} frames (about {} s at 60 Hz) or until the scene changes (D315, D316, \
                 D318)",
                late.scale,
                late.tried,
                late.share_before * 100.0,
                late.share_after * 100.0,
                change.to,
                late.lock_frames,
                late.lock_frames / 60
            );
            self.render_scale = change.to;
            self.resize_now = true;
            return;
        }
        if let Some(missed) = change.missed {
            log::info!(
                "adaptive resolution: render scale {:.2} -> {:.2}: missed {} refreshes over the \
                 last {} frames ({:.1}% of frames, {:.1} ms cadence) with GPU {:.1} ms against a \
                 {:.1} ms budget (D314)",
                change.from,
                change.to,
                missed.late,
                missed.frames,
                missed.late as f32 * 100.0 / missed.frames.max(1) as f32,
                missed.cadence_ms,
                change.gpu_ms,
                change.budget_ms
            );
            self.render_scale = change.to;
            self.resize_now = true;
            return;
        }
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

    /// One control-window frame. `unpainted` says the event loop ran it
    /// because Windows sent no paint (D290); everything in it, the egui
    /// pass, the desktop capture, the actions, runs the same either way.
    fn redraw_control(&mut self, unpainted: bool) {
        let began = Instant::now();
        self.last_control_frame = began;
        if self.control_minimized() {
            // A paint for a minimised window: nothing to draw (D314).
            if self.screenshot.is_none() {
                self.control_chores(began);
            }
            return;
        }
        self.last_control_draw = began;
        let size = self.stack.control.window().inner_size();
        let current_began = Instant::now();
        if !self.make_current(Surface::Control) {
            return;
        }
        let make_current = current_began.elapsed();
        // A covered window redraws a few times a minute and its waits say
        // nothing about the windowed case the pacer is for (TORNADO T16).
        let change = if self.control_unseen() {
            PaceChange::None
        } else {
            self.control_pacer.observe(make_current)
        };
        match change {
            PaceChange::None => {}
            PaceChange::Slowed if self.swap_policy.on_slowed() => {
                log::info!(
                    "control window: making it current waits {:.1} ms for the output's swap; the output now waits for its own swap instead (D221)",
                    self.control_pacer.smoothed_ms()
                );
                self.control_pacer.reset();
            }
            PaceChange::Slowed => log::info!(
                "control window: making it current waits {:.1} ms for the output's vertical blank; redrawing at 10 Hz so the show keeps its frames",
                self.control_pacer.smoothed_ms()
            ),
            PaceChange::Resumed => log::info!(
                "control window: making it current is cheap again ({:.1} ms); back to 30 Hz",
                self.control_pacer.smoothed_ms()
            ),
        }
        let now = Instant::now();
        self.console.refresh();
        let mut desktop_events = std::mem::take(&mut self.desktop_events);
        self.stack.ui.set_desktop_bridge(self.lan.desktop());
        if let Some(desktop) = self.lan.desktop() {
            desktop.drain(&mut desktop_events);
        }
        if !self.lan.desktop_enabled && self.desktop_input.held() {
            desktop_events.push(strobe_remote::desktop::Input::Reset {});
        }
        if !desktop_events.is_empty() {
            let scale = self.stack.ui.pixels_per_point().max(f32::EPSILON);
            self.desktop_input.translate(
                &desktop_events,
                egui::vec2(size.width as f32 / scale, size.height as f32 / scale),
                self.stack.ui.desktop_events(),
            );
        }
        desktop_events.clear();
        self.desktop_events = desktop_events;
        self.audio.refresh_status(now);
        if self
            .notice
            .as_ref()
            .is_some_and(|n| now.duration_since(n.since).as_secs() >= NOTICE_SECONDS)
        {
            self.notice = None;
        }
        // A load asked for last frame is shown as "preparing" in this one
        // and runs after it is painted, because it blocks.
        let loading = self.pending_load.take();
        let preparing = self.pending_preview.take();

        self.advance_preview(now);
        let composited = self.stack.show.as_ref().is_some_and(Show::composited);
        let preview = if self.posted {
            // The meta controls change the picture, so the preview shows
            // what the output shows.
            let texture = self.stack.post.output_texture();
            Some(Preview {
                texture: self.stack.ui.preview_texture(texture.raw()),
                aspect: texture.width() as f32 / texture.height().max(1) as f32,
                size: [texture.width(), texture.height()],
            })
        } else if composited {
            // The mixed frame of a running transition.
            let texture = self.stack.compositor.output_texture();
            Some(Preview {
                texture: self.stack.ui.preview_texture(texture.raw()),
                aspect: texture.width() as f32 / texture.height().max(1) as f32,
                size: [texture.width(), texture.height()],
            })
        } else {
            self.stack.session.as_ref().map(|session| {
                let texture = session.player.output_texture();
                Preview {
                    texture: self.stack.ui.preview_texture(texture.raw()),
                    aspect: texture.width() as f32 / texture.height().max(1) as f32,
                    size: [texture.width(), texture.height()],
                }
            })
        };
        let scene_preview_image = match (self.stack.preview.as_ref(), self.preview_index) {
            (Some(session), Some(index)) => {
                let texture = session.player.output_texture();
                Some((
                    index,
                    Preview {
                        texture: self.stack.ui.preview_texture(texture.raw()),
                        aspect: texture.width() as f32 / texture.height().max(1) as f32,
                        size: [texture.width(), texture.height()],
                    },
                ))
            }
            _ => None,
        };
        // An Editor shown with nothing in it opens the scene on screen.
        if self.editor_doc.is_none()
            && self.ui_state.panes.is_open(Pane::Editor)
            && let Some(dir) = self.stack.session.as_ref().map(|s| s.dir.clone())
            && self.editor_auto.as_ref() != Some(&dir)
        {
            self.editor_auto = Some(dir);
            self.open_editor(None);
        }
        let next = self.step_target(1);
        let lan_view = self.lan.view();
        let projector_view = self.projectors.view();
        let scanning = !self.desktop_controller.active()
            && self.ui_state.panes.shown_in(crate::workspace::Dock::Main)
                == Some(crate::workspace::Pane::Settings)
            && self.ui_state.settings.page == crate::ui::settings::SettingsPage::Remote;
        self.lan_scan.update(scanning, Instant::now());
        if let Some((purpose, path)) = self.picker.poll() {
            self.picked(purpose, &path);
        }
        let playing_dir = self.stack.session.as_ref().map(|s| s.dir.clone());
        let editor_read_only = self.editor_read_only();
        let authored = self.stack.session.as_ref().is_some_and(|session| {
            session
                .controls
                .defs()
                .iter()
                .any(strobe_autocontrol::is_authored)
        });
        let cue_media = self
            .stack
            .show
            .as_ref()
            .is_some_and(Show::live_cue_has_media);
        let chosen_media = self.media_override.is_some();
        // Before the session is borrowed for the panels below.
        let imported_playing = self.import_state.last.as_deref().is_some_and(|last| {
            self.stack
                .session
                .as_ref()
                .is_some_and(|session| same_folder(&session.dir, last))
        });
        let imported = self.imported_entry();
        let scene_transitions = self
            .stack
            .session
            .as_ref()
            .is_some_and(Session::has_transitions);
        let (
            scene_title,
            controls,
            meta,
            script,
            scene_time,
            presets,
            active_preset,
            scene_name,
            media,
        ) = match self.stack.session.as_mut() {
            Some(session) => {
                let time = session.time();
                let parts = session.panel_parts();
                let media = media_panel(parts.player, cue_media, chosen_media);
                (
                    parts.title,
                    Some(parts.controls),
                    Some(parts.meta),
                    parts.script,
                    time,
                    parts.presets,
                    parts.active_preset,
                    Some(parts.name),
                    Some(media),
                )
            }
            None => ("", None, None, None, 0.0, &[][..], None, None, None),
        };
        let entries = &self.library.entries;
        let name_of =
            |index: Option<usize>| index.and_then(|i| entries.get(i)).map(|e| e.name.as_str());
        let loading_name = name_of(loading.or(preparing));
        let playing_name = name_of(self.playing);
        let transport = match self.stack.show.as_ref() {
            Some(show) => show_transport(show, playing_name, scene_time, self.blackout),
            None => TransportPanel {
                now: playing_name,
                next: name_of(next).filter(|_| next != self.playing),
                cue: None,
                scene_time,
                blackout: self.blackout,
                can_step: next.is_some() && next != self.playing,
            },
        };
        let kept = playing_name;
        if self.ui_state.take_theme_change() {
            self.stack
                .ui
                .apply_theme(self.ui_state.palette(), self.ui_state.density());
        }
        let mut model = Model {
            ui: &mut self.ui_state,
            library: &self.library,
            health: &self.health,
            library_rev: self.library_rev,
            scan_error: self.scan_error.as_deref(),
            playing: self.playing,
            scene_title,
            scene_transitions,
            controls,
            media,
            meta,
            preview,
            scene_preview: scene_preview_image.and_then(|(index, image)| {
                name_of(Some(index)).map(|name| ScenePreview { index, name, image })
            }),
            loading: loading_name,
            failure: self.load_failure.as_ref().map(|f| FailurePanel {
                scene: &f.scene,
                summary: &f.summary,
                kept,
            }),
            notice: self.notice.as_ref().map(|n| NoticePanel {
                tone: n.tone,
                text: &n.text,
                offer: n
                    .offer
                    .as_ref()
                    .map(|(label, action)| Offer { label, action }),
            }),
            drop_hint: self.drop_hint.as_deref(),
            transport,
            console: &self.console,
            stats: &self.stats,
            render_scale: &mut self.scale_edit,
            monitors: &self.monitor_labels,
            output_monitor: self.output_monitor.index(),
            output_visible: self.output_visible,
            fullscreen: self.stack.output.window().fullscreen().is_some(),
            maximized: self.stack.control.window().is_maximized(),
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
            picking: self.picker.busy(),
            import: ImportPanel {
                text: &mut self.import_state.text,
                status: &self.import_state.status,
                replace_title: self
                    .import_state
                    .prompt
                    .as_ref()
                    .map(|prompt| prompt.title.as_str()),
                busy: self.import_state.in_flight > 0,
                drop_hover: self.import_state.drop_hover,
                imported_playing,
                imported,
                generating: self.generate_job.is_some(),
                listener: ListenerPanel {
                    enabled: self.listener.is_some(),
                    state: &self.listener_note,
                    failed: self.listener_failed,
                    token: self.import_token.as_ref().map(Token::expose),
                },
            },
            playlist: PlaylistPanel {
                ui: &mut self.playlist_ui,
                show: self.stack.show.as_ref(),
            },
            remote: RemotePanel {
                ui: &mut self.map_ui,
                remote: &mut self.remote,
                presets,
                active_preset,
                scene_name,
                bindings: &self.bindings,
            },
            generate: GeneratePanel {
                running: self
                    .generate_job
                    .as_ref()
                    .map(|job| (job.scene.as_str(), job.elapsed().as_secs_f32())),
                outcome: self.generate_outcome.as_ref(),
                authored,
                read_only: self.generate_read_only,
            },
            editor: EditorPanel {
                doc: self.editor_doc.as_mut(),
                playing_dir: playing_dir.as_deref(),
                read_only: editor_read_only,
            },
            lan: LanPanel {
                desktop_enabled: self.lan.desktop_enabled,
                controller: &mut self.desktop_controller,
                view: &lan_view,
                address: &mut self.lan.address,
                port: &mut self.lan.port,
                found: &self.lan_scan.found,
            },
            projectors: crate::ui::model::ProjectorPanel {
                view: &projector_view,
                form: &mut self.projectors.form,
            },
        };
        let mut actions = std::mem::take(&mut self.actions);
        // Actions queued before this pass (keys, MIDI) are not the remote's.
        let queued = actions.len();
        let window = self.stack.control.window();
        let shell_state = &mut self.shell;
        let ui_started = Instant::now();
        self.stack.ui.run(window, |ui| {
            if model.lan.controller.active() {
                model.lan.controller.show(ui, &mut actions);
            } else {
                shell::show(ui, shell_state, &mut model, &mut actions);
            }
        });
        let ran = ui_started.elapsed();
        // Layout only: the paint that follows is the GL driver's (on
        // llvmpipe it is software rasterisation and dwarfs everything else,
        // so including it would make the number a measure of the CPU's
        // pixel throughput rather than of the UI).
        self.stats.record_ui(ran);
        let paint_began = Instant::now();
        self.stack.ui.paint(window, crate::ui::theme::clear_color());
        let paint = paint_began.elapsed();
        if let Some(desktop) = self.lan.desktop() {
            // Starts or collects an asynchronous readback; never waits.
            self.desktop_capture.frame(
                self.stack.gfx.gl(),
                now,
                &desktop,
                [size.width, size.height],
            );
        }
        self.ui_state.tick(now);
        self.remote.persist(now);

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
        let swap_began = Instant::now();
        if presents_control(self.control_covered, unpainted) {
            self.present(Surface::Control, swap_began);
        }
        self.perf_log.control(ControlTimes {
            total: began.elapsed(),
            make_current,
            layout: ran,
            paint,
            swap: swap_began.elapsed(),
            unpainted,
        });
        if self
            .screenshot
            .as_ref()
            .is_some_and(Screenshot::control_capture_next)
            && let Some(query) = self.shot_load.take()
        {
            match self.library.find(&query) {
                Some(index) => self.request_load(index),
                None => log::warn!("--request-load: no scene matching \"{query}\""),
            }
        }
        if self.stack.ui.remote_pass() {
            self.refuse_remote(&mut actions, queued);
        }
        if self.rebuild.is_none() {
            self.apply(&mut actions);
            // Now that "preparing" has been on screen, do the blocking work.
            if let Some(index) = loading {
                self.load(index);
            }
            if let Some(index) = preparing {
                self.preview_load(index);
            }
            self.compile_show_step();
        }
        self.actions = actions;
    }

    /// Advances the browser's preview scene one control-window frame, if it
    /// has one. It renders into its own small targets and is painted by the
    /// control window only; it never touches the live scene or the output.
    fn advance_preview(&mut self, now: Instant) {
        let Some(session) = self.stack.preview.as_mut() else {
            return;
        };
        let dt = now
            .duration_since(self.preview_clock)
            .as_secs_f32()
            .min(0.1);
        self.preview_clock = now;
        let mouse = strobe_gfx::Mouse::default();
        session.advance(dt, &mut self.audio, mouse, self.local_date.now());
    }
}

/// The actions the mapper's outputs ask for: commands, cues, fav slots and
/// presets, carried out by the same actions the buttons send. Drains
/// `outputs`.
fn output_actions(outputs: &mut Vec<Output>) -> Vec<Action> {
    let mut actions: Vec<Action> = Vec::with_capacity(outputs.len());
    for output in outputs.drain(..) {
        match output {
            Output::Command(id) => actions.push(Action::RunCommand(id)),
            Output::Cue(number) => {
                actions.push(Action::JumpToCue(number.saturating_sub(1) as usize));
            }
            Output::FavSlot(slot) => {
                actions.push(Action::RecallFav(usize::from(slot).saturating_sub(1)));
            }
            Output::Preset(name) => actions.push(Action::RecallPreset(name)),
            Output::Action(action) => actions.push(Action::ControlAction(action)),
        }
    }
    actions
}

/// What `STROBE_FINISH_AFTER_SWAP` asks for (D221). Logged unless unset.
fn finish_mode() -> FinishMode {
    const ENV: &str = "STROBE_FINISH_AFTER_SWAP";
    let value = std::env::var(ENV).ok();
    let mode = FinishMode::from_env(value.as_deref());
    match mode {
        FinishMode::On => log::info!("{ENV}=1: the output waits for each swap to complete"),
        FinishMode::Off => log::info!("{ENV}=0: the output never waits for its swap"),
        FinishMode::Auto => {}
    }
    mode
}

/// The Inspector's media row for `player` (D217). `cue_media` says the
/// playing cue names its own; `chosen` that the operator picked one.
fn media_panel(player: &strobe_gfx::ScenePlayer, cue_media: bool, chosen: bool) -> MediaPanel<'_> {
    let from = if player.media_is_scenes_own() {
        MediaFrom::Scene
    } else if cue_media {
        MediaFrom::Cue
    } else {
        MediaFrom::Chosen
    };
    MediaPanel {
        showing: player.media_request().map(|request| request.path.as_path()),
        from,
        loading: player.media_loading(),
        read: player.reads_media(),
        chosen,
    }
}

/// The transport strip's inputs when a playlist is loaded: the cue that is
/// playing, the cue a Next would go to, and the clock's times.
fn show_transport<'a>(
    show: &'a Show,
    playing_name: Option<&'a str>,
    scene_time: f32,
    blackout: bool,
) -> TransportPanel<'a> {
    let clock = show.clock();
    let ready = show.phase() == Phase::Ready;
    let current = clock.current().filter(|_| ready);
    let now = current
        .and_then(|index| show.cue_scene(index))
        .or(playing_name);
    let next = clock
        .peek_next()
        .filter(|_| ready)
        .and_then(|index| show.cue_scene(index));
    let section = |index: usize| {
        show.rows()
            .get(index)
            .map_or("", |row| row.section_name.as_str())
    };
    let waiting = show.waiting_for_anchor().map(|seconds| seconds as f32);
    let cue = if waiting.is_some() {
        Some(CueTransport {
            index: 0,
            total: show.rows().len(),
            section: section(0),
            elapsed: 0.0,
            remaining: None,
            held: false,
            transition: None,
            waiting,
        })
    } else {
        current.map(|index| CueTransport {
            index,
            total: show.rows().len(),
            section: section(index),
            elapsed: clock.elapsed() as f32,
            remaining: clock.remaining().map(|seconds| seconds as f32),
            held: clock.is_held(),
            transition: clock.transition().map(|t| t.progress as f32),
            waiting: None,
        })
    };
    TransportPanel {
        now,
        next,
        cue,
        scene_time,
        blackout,
        can_step: ready,
    }
}

/// Time between scene frames while the output window is hidden: one refresh
/// period of the output's monitor (the adaptive budget), never faster than
/// the shown output's own cap.
fn hidden_period(budget_ms: f32) -> Duration {
    if !budget_ms.is_finite() || budget_ms <= 0.0 {
        return OUTPUT_PERIOD;
    }
    Duration::from_secs_f32(budget_ms / 1000.0).max(OUTPUT_PERIOD)
}

/// Whether `event` is the person acting on a window (or the window
/// changing under them), which a desktop viewer redraws for (D302).
fn is_input(event: &WindowEvent) -> bool {
    matches!(
        event,
        WindowEvent::CursorMoved { .. }
            | WindowEvent::CursorEntered { .. }
            | WindowEvent::CursorLeft { .. }
            | WindowEvent::MouseInput { .. }
            | WindowEvent::MouseWheel { .. }
            | WindowEvent::KeyboardInput { .. }
            | WindowEvent::ModifiersChanged(_)
            | WindowEvent::Ime(_)
            | WindowEvent::Touch(_)
            | WindowEvent::Focused(_)
            | WindowEvent::Resized(_)
            | WindowEvent::ScaleFactorChanged { .. }
    )
}

/// What decides how often the control window redraws.
#[derive(Debug, Clone, Copy)]
struct ControlFacts {
    /// Nobody can see it: the fullscreen output covers it, or Windows has
    /// stopped painting it (D290).
    unseen: bool,
    /// A desktop remote holds the lease on this window (D275).
    streaming: bool,
    /// Making it current waits for the output's swap (D221).
    slow: bool,
    /// It shows another Strobe's window (the desktop viewer, D275).
    viewing: bool,
}

/// The time between control-window redraws.
fn control_period(facts: ControlFacts) -> Duration {
    if facts.unseen {
        if facts.streaming {
            crate::desktop::STREAM_PERIOD
        } else {
            COVERED_PERIOD
        }
    } else if facts.viewing {
        VIEWER_PERIOD
    } else if facts.slow {
        crate::stats::SLOW_PERIOD
    } else {
        CONTROL_PERIOD
    }
}

/// A render size that could not be allocated, so the resize to it is tried
/// again every [`ResizeBackoff::RETRY`] instead of every settle period, and
/// reported once rather than several times a second.
#[derive(Debug, Default, Clone, Copy)]
struct ResizeBackoff {
    failed: Option<([i32; 2], Instant)>,
}

impl ResizeBackoff {
    /// How long a size that failed waits before it is tried again.
    const RETRY: Duration = Duration::from_secs(5);

    /// Whether a resize to `want` may be tried at `now`.
    fn allows(&self, want: [i32; 2], now: Instant) -> bool {
        match self.failed {
            Some((size, at)) if size == want => now.duration_since(at) >= Self::RETRY,
            Some(_) | None => true,
        }
    }

    /// Records that the resize to `want` failed at `now`. Returns whether
    /// this is the first failure at that size, which is worth reporting.
    fn failed(&mut self, want: [i32; 2], now: Instant) -> bool {
        let first = !matches!(self.failed, Some((size, _)) if size == want);
        self.failed = Some((want, now));
        first
    }

    /// A resize succeeded.
    fn succeeded(&mut self) {
        self.failed = None;
    }
}

/// The render size a show's mouse uniforms are mapped to: the live scene's
/// own, which a scene that pins its final pass keeps whatever it is asked
/// (D280), as the single-scene path does; the full render size when nothing
/// is live yet.
fn show_mouse_size(live: Option<[i32; 2]>, full: [i32; 2]) -> [i32; 2] {
    live.unwrap_or(full)
}

/// Where the windows are when a close request arrives.
#[derive(Debug, Clone, Copy)]
struct CloseFacts {
    /// It is for the output window.
    on_output: bool,
    /// The windows are about to be replaced.
    rebuilding: bool,
    /// The control window is minimised.
    control_minimized: bool,
    /// The output is shown fullscreen on the control window's monitor, so
    /// it is in front of the control window whatever focus says.
    output_fills_control_monitor: bool,
}

/// What became of `--playlist` at launch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LaunchPlaylist {
    /// None was named.
    None,
    /// It loaded and plays.
    Loaded,
    /// It was named but did not load (missing, unreadable, invalid).
    Failed,
}

/// The library scene to play at launch, if any: the last session's scene,
/// unless the command line chose a scene, a playlist plays, or this is a
/// screenshot run. When a named playlist did not load, a venue would
/// otherwise show black all night, so the last session's scene plays, or
/// failing that the library's first (D452). `names` are the library's, in
/// order.
fn launch_scene<'a>(
    asked_scene: bool,
    playlist: LaunchPlaylist,
    screenshot: bool,
    last: &str,
    names: impl Iterator<Item = &'a str> + Clone,
) -> Option<usize> {
    if asked_scene || screenshot {
        return None;
    }
    let last_at = (!last.is_empty())
        .then(|| names.clone().position(|name| name == last))
        .flatten();
    match playlist {
        LaunchPlaylist::Loaded => None,
        LaunchPlaylist::None => last_at,
        LaunchPlaylist::Failed => last_at.or_else(|| names.clone().next().map(|_| 0)),
    }
}

/// Whether a close request quits the app (D298). Closing the output only
/// hides it while the operator can reach the control window to quit from
/// there. When nobody can (a fullscreen output over it, or a minimised
/// control window) the request is someone asking Strobe to stop, such as
/// `taskkill` at the end of a show, and it quits; the resume point is saved
/// on the way out. A control window Windows has stopped painting (D290) is
/// not reason enough: that happens beside a windowed output too, where the
/// operator can still see it and its close button.
fn closing_quits(facts: CloseFacts) -> bool {
    !facts.on_output
        || facts.rebuilding
        || facts.control_minimized
        || facts.output_fills_control_monitor
}

/// The chord a key press in the output window stands for, with the
/// modifiers held (D351): `command` is Ctrl, or Cmd on macOS, as egui has
/// it in the control window.
fn chord_of(key: &Key, modifiers: ModifiersState) -> Option<commands::Chord> {
    let plain = plain_chord_of(key)?;
    Some(commands::Chord {
        command: if cfg!(target_os = "macos") {
            modifiers.super_key()
        } else {
            modifiers.control_key()
        },
        shift: modifiers.shift_key(),
        alt: modifiers.alt_key(),
        ..plain
    })
}

/// [`chord_of`]'s key, with no modifiers.
fn plain_chord_of(key: &Key) -> Option<commands::Chord> {
    use commands::{Chord, Key as K};
    const FUNCTION: [NamedKey; 12] = [
        NamedKey::F1,
        NamedKey::F2,
        NamedKey::F3,
        NamedKey::F4,
        NamedKey::F5,
        NamedKey::F6,
        NamedKey::F7,
        NamedKey::F8,
        NamedKey::F9,
        NamedKey::F10,
        NamedKey::F11,
        NamedKey::F12,
    ];
    match key {
        Key::Named(NamedKey::Space) => Some(Chord::plain(K::Space)),
        Key::Named(NamedKey::ArrowLeft) => Some(Chord::plain(K::Left)),
        Key::Named(NamedKey::ArrowRight) => Some(Chord::plain(K::Right)),
        Key::Named(NamedKey::Escape) => Some(Chord::plain(K::Escape)),
        Key::Named(named) => FUNCTION
            .iter()
            .position(|f| f == named)
            .and_then(|at| u8::try_from(at + 1).ok())
            .and_then(K::function)
            .map(Chord::plain),
        Key::Character(text) => {
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) if c.is_ascii_digit() => {
                    Some(Chord::plain(K::Digit(c as u8 - b'0')))
                }
                (Some(c), None) => K::letter(c).map(Chord::plain),
                _ => None,
            }
        }
        Key::Unidentified(_) | Key::Dead(_) => None,
    }
}

/// The action a command stands for when it is invoked from the output
/// window. Presentation commands (the browser's search, closing a pane)
/// belong to the control window's UI and have no action here.
fn command_action(command: CommandId, blackout: bool) -> Option<Action> {
    match command {
        CommandId::NextScene => Some(Action::NextScene),
        CommandId::PrevScene => Some(Action::PrevScene),
        CommandId::ToggleHold => Some(Action::ToggleHold),
        CommandId::ExtendCue => Some(Action::ExtendCue),
        CommandId::RestartCue => Some(Action::RestartCue),
        CommandId::SkipSection => Some(Action::SkipSection),
        CommandId::ToggleBlackout => Some(Action::SetBlackout(!blackout)),
        CommandId::BlackoutOn => Some(Action::SetBlackout(true)),
        CommandId::BlackoutOff => Some(Action::SetBlackout(false)),
        CommandId::ToggleFullscreen => Some(Action::ToggleFullscreen),
        CommandId::TapTempo => Some(Action::TapTempo),
        CommandId::NewSong => Some(Action::NewSong),
        CommandId::FocusBrowserSearch
        | CommandId::CloseTopmostPane
        | CommandId::OpenPalette
        | CommandId::GenerateControls
        | CommandId::ToggleControlsView => None,
    }
}

/// The notice that refuses a write into `dir`, when it is one of
/// Synesthesia's scene folders (D271): whose it is, that Strobe only reads
/// it, and a Copy to my library button. `None` for a folder Strobe may
/// write.
fn read_only_refusal(
    synesthesia: &Synesthesia,
    dir: &Path,
    name: &str,
    what: &str,
) -> Option<Notice> {
    synesthesia.owns(dir).then(|| Notice {
        tone: Tone::Warning,
        text: format!(
            "{name} is Synesthesia's own and read-only. Copy it to your library to {what}."
        ),
        since: Instant::now(),
        offer: Some((
            crate::ui::tags::COPY_LABEL,
            Action::CopyToLibrary(dir.to_path_buf()),
        )),
    })
}

/// The first lines of a load error, for a banner and a row: enough to say
/// what broke (the compiler log already names `main.glsl:N`), not a page.
fn summarize(error: &str) -> String {
    /// Most characters kept.
    const LIMIT: usize = 220;
    let mut text = String::new();
    for line in error
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .take(2)
    {
        if !text.is_empty() {
            text.push_str(" / ");
        }
        text.push_str(line);
    }
    if text.chars().count() > LIMIT {
        text = text.chars().take(LIMIT).collect::<String>() + "...";
    }
    if text.is_empty() {
        text.push_str("unknown error");
    }
    text
}

/// Clears the GL error flag, logging what it held. GL keeps one flag per
/// error kind, so a handful of reads empties it; the bound only guards
/// against a driver that never reports `GL_NO_ERROR`.
fn drain_gl_errors(gl: &strobe_gfx::gl::Gl) {
    const MAX_FLAGS: usize = 8;
    for _ in 0..MAX_FLAGS {
        match gl.error() {
            Some(code) => log::debug!("cleared a stale GL error 0x{code:x} from context set-up"),
            None => return,
        }
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

/// The watched drop folder at launch (D240, D265): `--watch-folder` for
/// this run when it is given, else the folder Settings saved if it is
/// switched on. Either is checked as Settings checks it
/// ([`intake::check_watch_folder`]), and Settings shows the outcome.
fn watch_at_launch(
    cli: Option<&Path>,
    watch: &mut crate::ui::state::WatchState,
    library: &Path,
) -> Option<intake::Inbox> {
    let (folder, from_cli) = match cli {
        Some(folder) => (folder.to_path_buf(), true),
        None if watch.enabled => (PathBuf::from(watch.folder.trim()), false),
        None => return None,
    };
    match intake::check_watch_folder(&folder, library) {
        Ok(dir) => {
            log::info!("watching {} for new scenes", dir.display());
            watch.watching = true;
            watch.failed = false;
            watch.note = if from_cli {
                watch.draft = dir.display().to_string();
                format!("Watching {} (--watch-folder, this run only)", dir.display())
            } else {
                format!("Watching {}", dir.display())
            };
            Some(intake::Inbox::new(dir))
        }
        Err(why) => {
            let source = if from_cli {
                "--watch-folder"
            } else {
                "the watched folder in Settings"
            };
            log::warn!("{source}: {why} Not watching.");
            watch.watching = false;
            watch.failed = true;
            watch.note = format!("Not watching: {why}");
            None
        }
    }
}

/// What a change to the watched folder in Settings comes to (D265).
enum WatchChange {
    /// Watch this folder, replacing any watch.
    Start(intake::Inbox),
    /// Stop watching.
    Stop,
    /// The folder was refused; whatever was watched stays watched.
    Keep,
}

/// Applies Settings' watched folder to `watch` (the saved folder and
/// switch, and the line Settings shows) and says what the app must do. A
/// folder is checked as at launch, and a refused one changes nothing but
/// the line, so a typo never stops a watch that works.
fn watch_change(
    folder: &str,
    enabled: bool,
    watch: &mut crate::ui::state::WatchState,
    library: &Path,
) -> WatchChange {
    let folder = folder.trim();
    if !enabled {
        // The folder stays saved, so switching it back on needs no typing.
        watch.enabled = false;
        watch.watching = false;
        watch.failed = false;
        watch.note = "Off".to_string();
        return WatchChange::Stop;
    }
    match intake::check_watch_folder(Path::new(folder), library) {
        Ok(dir) => {
            folder.clone_into(&mut watch.folder);
            folder.clone_into(&mut watch.draft);
            watch.enabled = true;
            watch.watching = true;
            watch.failed = false;
            watch.note = format!("Watching {}", dir.display());
            WatchChange::Start(intake::Inbox::new(dir))
        }
        Err(why) => {
            watch.failed = true;
            watch.note = why;
            WatchChange::Keep
        }
    }
}

/// Whether two paths name the same folder, resolving links and relative
/// parts when both exist, and comparing as written otherwise.
fn same_folder(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

impl Running {
    /// Puts what a Browse dialog chose where it belongs (D470), and acts
    /// on it as the field's own button would: a playlist loads, an export
    /// imports, a folder is watched, a trust file fills the connect form.
    fn picked(&mut self, purpose: crate::picker::Purpose, path: &std::path::Path) {
        let text = path.display().to_string();
        log::info!("picker: {purpose:?} chose {text}");
        match purpose {
            crate::picker::Purpose::Playlist => {
                self.playlist_ui.path.clone_from(&text);
                self.actions.push(Action::LoadPlaylist(text));
            }
            crate::picker::Purpose::Shadertoy => {
                self.import_state.text.clone_from(&text);
                self.actions.push(Action::Import(text));
            }
            crate::picker::Purpose::WatchFolder => {
                self.ui_state.watch.draft.clone_from(&text);
                self.actions.push(Action::SetWatchFolder {
                    folder: text,
                    enabled: true,
                });
            }
            crate::picker::Purpose::TrustFile => {
                self.desktop_controller.trust_path = text;
                self.desktop_controller.use_trust = true;
            }
        }
    }
}

/// Other Strobes on the LAN (D468): a scanner that runs only while it is
/// wanted, and what it found, read from it at most every [`LanScan::READ`]
/// so a frame copies the list a few times a second, not every frame.
#[derive(Default)]
struct LanScan {
    scanner: Option<strobe_remote::discovery::Scanner>,
    /// The scanner would not start (no network): not tried again until
    /// the Remote page is left and opened again.
    failed: bool,
    read: Option<Instant>,
    /// What the Remote page lists.
    found: Vec<strobe_remote::discovery::Found>,
}

impl LanScan {
    /// How often the list is copied out of the scanner.
    const READ: Duration = Duration::from_millis(500);

    /// Starts or stops the scanner for `wanted`, and refreshes the list.
    fn update(&mut self, wanted: bool, now: Instant) {
        use strobe_remote::discovery;
        if !wanted {
            self.scanner = None;
            self.failed = false;
            self.read = None;
            self.found.clear();
            return;
        }
        if self.scanner.is_none() && !self.failed {
            // Asked once per opening of the page: it takes a socket call.
            let lan: Vec<std::net::Ipv4Addr> = strobe_remote::server::net::lan_address()
                .and_then(|ip| match ip {
                    std::net::IpAddr::V4(v4) => Some(v4),
                    std::net::IpAddr::V6(_) => None,
                })
                .into_iter()
                .collect();
            let targets = discovery::broadcast_targets(&lan, discovery::PORT);
            match discovery::Scanner::start(targets, Some(discovery::instance_id().to_string())) {
                Ok(scanner) => self.scanner = Some(scanner),
                Err(error) => {
                    log::warn!("remote: not looking for other Strobes: {error}");
                    self.failed = true;
                }
            }
        }
        if let Some(scanner) = &self.scanner
            && self
                .read
                .is_none_or(|at| now.saturating_duration_since(at) >= Self::READ)
        {
            self.read = Some(now);
            self.found = scanner.found();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    /// A scratch folder with a library and an inbox beside it, removed on
    /// drop.
    struct WatchScratch(PathBuf);

    impl WatchScratch {
        fn new(tag: &str) -> Result<Self> {
            let dir =
                std::env::temp_dir().join(format!("strobe-app-watch-{}-{tag}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("scenes"))?;
            std::fs::create_dir_all(dir.join("inbox"))?;
            Ok(Self(dir))
        }
    }

    impl Drop for WatchScratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_playlist_that_does_not_load_at_launch_leaves_a_scene_playing_not_black() -> Result<()> {
        let names = ["01-aurora", "05-domain-warp", "13-chromatic-rise"];
        let pick = |asked, playlist, screenshot, last| {
            launch_scene(asked, playlist, screenshot, last, names.iter().copied())
        };
        // The venue: `--playlist` names a file that is gone (or locked, or
        // has a typo). Before D452 nothing was loaded: a black output.
        assert_eq!(
            pick(false, LaunchPlaylist::Failed, false, "05-domain-warp"),
            Some(1),
            "the last session's scene plays instead"
        );
        assert_eq!(
            pick(false, LaunchPlaylist::Failed, false, ""),
            Some(0),
            "a machine that only ever ran playlists has no last scene: the first one"
        );
        assert_eq!(
            pick(false, LaunchPlaylist::Failed, false, "gone"),
            Some(0),
            "nor one still in the library"
        );
        assert_eq!(
            launch_scene(false, LaunchPlaylist::Failed, false, "", std::iter::empty()),
            None,
            "an empty library has nothing to play"
        );
        // Unchanged: a playlist that plays, a scene asked for, a screenshot.
        assert_eq!(
            pick(false, LaunchPlaylist::Loaded, false, "05-domain-warp"),
            None,
            "the playlist plays"
        );
        assert_eq!(
            pick(true, LaunchPlaylist::Failed, false, "05-domain-warp"),
            None,
            "--scene was loaded already"
        );
        assert_eq!(
            pick(false, LaunchPlaylist::Failed, true, "05-domain-warp"),
            None,
            "a screenshot never plays the operator's state"
        );
        assert_eq!(
            pick(false, LaunchPlaylist::None, false, "13-chromatic-rise"),
            Some(2),
            "no playlist: the last session's scene, as before"
        );
        assert_eq!(
            pick(false, LaunchPlaylist::None, false, ""),
            None,
            "and nothing on a first run, as before"
        );
        Ok(())
    }

    #[test]
    fn a_write_into_synesthesias_folder_is_refused_and_offers_a_copy() -> Result<()> {
        let scratch = crate::library::fake::Scratch::new("refused")?;
        let synesthesia = crate::library::fake::install(&scratch.0)?;
        let glow = scratch.0.join("install/release/glow.synScene");
        // Every write the app makes into a scene folder asks this first.
        for what in [
            "tag it",
            "give it a picture",
            "save edits to it",
            "save presets for it",
            "delete its presets",
            "rename its presets",
        ] {
            let Some(notice) = read_only_refusal(&synesthesia, &glow, "glow", what) else {
                anyhow::bail!("writing to {} was not refused ({what})", glow.display());
            };
            assert_eq!(notice.tone, Tone::Warning, "a warning, not an error");
            assert_eq!(
                notice.text,
                format!(
                    "glow is Synesthesia's own and read-only. Copy it to your library to {what}."
                ),
                "says whose it is and the way on"
            );
            assert_eq!(
                notice.offer,
                Some((
                    crate::ui::tags::COPY_LABEL,
                    Action::CopyToLibrary(glow.clone())
                )),
                "and offers the copy"
            );
        }
        let mine = scratch.0.join("mine/glow.synScene");
        assert!(
            read_only_refusal(&synesthesia, &mine, "glow", "tag it").is_none(),
            "the library's own scenes are written as before"
        );
        Ok(())
    }

    #[test]
    fn copy_to_my_library_makes_a_copy_strobe_may_write() -> Result<()> {
        let scratch = crate::library::fake::Scratch::new("copied")?;
        let synesthesia = crate::library::fake::install(&scratch.0)?;
        let mine = scratch.0.join("mine");
        crate::library::fake::scene(&mine.join("ink"), &[])?;
        let source = scratch.0.join("install/release/ink.synScene");
        let before = std::fs::read(source.join("scene.json"))?;

        let copy = intake::copy_as_own(&source, &mine).map_err(anyhow::Error::msg)?;
        assert_eq!(
            copy,
            mine.join("ink-2.synScene"),
            "the library has an ink already, so a free name"
        );
        let library =
            Library::scan_with(&mine, synesthesia.shown(true)).map_err(anyhow::Error::msg)?;
        let Some(entry) = library.entries.iter().find(|entry| entry.dir == copy) else {
            anyhow::bail!("the copy is not listed: {:?}", library.entries);
        };
        assert_eq!(
            (entry.name.as_str(), entry.source, entry.read_only()),
            ("ink-2", crate::library::Source::Own, false),
            "listed as the person's own"
        );
        assert!(
            read_only_refusal(&synesthesia, &copy, "ink-2", "tag it").is_none(),
            "and not refused"
        );
        // The write the tag field makes, on the copy.
        let json = copy.join("scene.json");
        let text = std::fs::read_to_string(&json)?;
        let tagged = strobe_scene::tags::write(&text, &["mine".to_string()])?;
        editor::write_atomically(&json, &tagged)?;
        assert_eq!(
            strobe_scene::tags::read(&std::fs::read_to_string(&json)?),
            ["mine"],
            "the copy takes a tag"
        );
        assert_eq!(
            std::fs::read(source.join("scene.json"))?,
            before,
            "Synesthesia's own file is byte for byte unchanged"
        );
        Ok(())
    }

    #[test]
    fn settings_starts_stops_and_refuses_a_watched_folder() -> Result<()> {
        let scratch = WatchScratch::new("settings")?;
        let library = scratch.0.join("scenes");
        let inbox = scratch.0.join("inbox");
        let mut watch = crate::ui::state::WatchState::default();
        let typed = format!("  {}  ", inbox.display());
        let started = watch_change(&typed, true, &mut watch, &library);
        assert!(
            matches!(&started, WatchChange::Start(found) if found.dir() == inbox),
            "a folder beside the library is watched"
        );
        assert_eq!(
            (
                watch.folder.as_str(),
                watch.enabled,
                watch.watching,
                watch.failed
            ),
            (inbox.to_str().unwrap_or_default(), true, true, false),
            "saved trimmed, and shown as watching: {watch:?}"
        );
        let refused = watch_change(&library.display().to_string(), true, &mut watch, &library);
        assert!(
            matches!(refused, WatchChange::Keep),
            "the library itself is refused, and the watch that works stays"
        );
        assert!(
            watch.failed && watch.note.contains("library itself"),
            "Settings says why: {watch:?}"
        );
        assert_eq!(
            (watch.folder.as_str(), watch.enabled),
            (inbox.to_str().unwrap_or_default(), true),
            "a refusal saves nothing"
        );
        let missing = watch_change("no/such/folder", true, &mut watch, &library);
        assert!(
            matches!(missing, WatchChange::Keep) && watch.note.contains("not a folder"),
            "a folder that is not there: {watch:?}"
        );
        let stopped = watch_change(&typed, false, &mut watch, &library);
        assert!(matches!(stopped, WatchChange::Stop), "switched off");
        assert!(
            !watch.enabled && !watch.watching && !watch.failed,
            "saved off: {watch:?}"
        );
        Ok(())
    }

    #[test]
    fn launch_watches_the_command_line_folder_first_then_the_saved_one() -> Result<()> {
        let scratch = WatchScratch::new("launch")?;
        let library = scratch.0.join("scenes");
        let inbox = scratch.0.join("inbox");
        let mut off = crate::ui::state::WatchState::default();
        assert!(
            watch_at_launch(None, &mut off, &library).is_none() && !off.watching,
            "nothing asked for, nothing watched"
        );
        let mut saved = crate::ui::state::WatchState {
            folder: inbox.display().to_string(),
            enabled: true,
            ..crate::ui::state::WatchState::default()
        };
        let found = watch_at_launch(None, &mut saved, &library);
        assert!(
            found.is_some_and(|inbox_watch| inbox_watch.dir() == inbox) && saved.watching,
            "the saved folder, switched on: {saved:?}"
        );
        let mut cli = crate::ui::state::WatchState::default();
        let found = watch_at_launch(Some(&inbox), &mut cli, &library);
        assert!(
            found.is_some() && cli.note.contains("this run only") && !cli.enabled,
            "--watch-folder is watched but not saved: {cli:?}"
        );
        let mut library_itself = crate::ui::state::WatchState::default();
        assert!(
            watch_at_launch(Some(&library), &mut library_itself, &library).is_none()
                && library_itself.failed,
            "launch refuses what Settings refuses: {library_itself:?}"
        );
        let mut gone = crate::ui::state::WatchState {
            folder: scratch.0.join("gone").display().to_string(),
            enabled: true,
            ..crate::ui::state::WatchState::default()
        };
        assert!(
            watch_at_launch(None, &mut gone, &library).is_none() && gone.failed && gone.enabled,
            "a saved folder that has gone is not watched, and stays saved: {gone:?}"
        );
        Ok(())
    }

    #[test]
    fn a_render_size_that_failed_is_retried_slowly_and_reported_once() {
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let mut backoff = ResizeBackoff::default();
        assert!(backoff.allows([1920, 1080], at(0)), "nothing failed yet");
        assert!(
            backoff.failed([1920, 1080], at(0)),
            "the first failure at a size is reported"
        );
        // The settle period is 150 ms: without a backoff it was tried, and
        // reported, about seven times a second for as long as it failed.
        assert!(
            !backoff.allows([1920, 1080], at(150)),
            "the same size is not tried again at once"
        );
        assert!(
            backoff.allows([1280, 720], at(150)),
            "another size (the window or the scale changed) is tried at once"
        );
        assert!(
            backoff.allows([1920, 1080], at(5_000)),
            "the same size is tried again after a while"
        );
        assert!(
            !backoff.failed([1920, 1080], at(5_000)),
            "and failing again there is not reported again"
        );
        assert!(
            backoff.failed([1280, 720], at(5_100)),
            "a failure at a new size is"
        );
        backoff.succeeded();
        assert!(
            backoff.allows([1280, 720], at(5_200)),
            "a success clears it"
        );
    }

    #[test]
    fn a_pinned_scene_in_a_show_gets_the_mouse_in_its_own_pixels() {
        // A square scene pinned at 1024x1024 (D280), letterboxed into a
        // 1920x1080 output while the render scale makes the full size 720p.
        let mut tracker = crate::mouse::MouseTracker::default();
        tracker.moved([960.0, 0.0]);
        let size = show_mouse_size(Some([1024, 1024]), [1280, 720]);
        let mouse = tracker.uniforms([1920, 1080], size);
        assert!(
            (mouse.mouse[0] - 512.0).abs() < 1e-3 && (mouse.mouse[1] - 1024.0).abs() < 1e-3,
            "the top centre of the window is the top centre of the scene's 1024x1024: {mouse:?}"
        );
        assert_eq!(
            show_mouse_size(None, [1280, 720]),
            [1280, 720],
            "with nothing live, the full render size"
        );
    }

    #[test]
    fn closing_the_output_quits_when_the_control_window_is_out_of_reach() {
        let reachable = CloseFacts {
            on_output: true,
            rebuilding: false,
            control_minimized: false,
            output_fills_control_monitor: false,
        };
        assert!(
            !closing_quits(reachable),
            "a windowed output beside the control window only hides"
        );
        assert!(
            closing_quits(CloseFacts {
                on_output: false,
                ..reachable
            }),
            "closing the control window quits"
        );
        assert!(
            closing_quits(CloseFacts {
                rebuilding: true,
                ..reachable
            }),
            "a close during a rebuild quits, as before"
        );
        // The JaJa mini PC: the output fullscreen on the one desktop, the
        // control window maximised behind it, focus saying it is on top
        // (so not judged covered) and `taskkill` asking the output to close.
        assert!(
            closing_quits(CloseFacts {
                output_fills_control_monitor: true,
                ..reachable
            }),
            "a fullscreen output over the control window quits"
        );
        assert!(
            closing_quits(CloseFacts {
                control_minimized: true,
                ..reachable
            }),
            "nor can a minimised one"
        );
    }

    #[test]
    fn hidden_frames_follow_the_monitor_refresh() -> Result<()> {
        let sixty = hidden_period(perf::budget_for(Some(60_000)));
        assert!(
            (sixty.as_secs_f32() - 1.0 / 60.0).abs() < 1e-4,
            "60 Hz: {sixty:?}"
        );
        let unknown = hidden_period(perf::budget_for(None));
        assert!(
            (unknown.as_secs_f32() - 1.0 / 60.0).abs() < 1e-4,
            "unknown refresh falls back to 60 Hz: {unknown:?}"
        );
        Ok(())
    }

    #[test]
    fn output_window_keys_map_to_the_same_commands() {
        let key = |text: &str| chord_of(&Key::Character(text.into()), ModifiersState::empty());
        assert_eq!(
            key("b"),
            Some(commands::Chord::plain(commands::Key::B)),
            "b"
        );
        assert_eq!(
            key("B"),
            Some(commands::Chord::plain(commands::Key::B)),
            "shifted B"
        );
        assert_eq!(
            key("x"),
            Some(commands::Chord::plain(commands::Key::Letter('X'))),
            "any letter is a key; whether it is bound is the table's business"
        );
        assert_eq!(
            commands::dispatch(
                key("x").unwrap_or(commands::Chord::plain(commands::Key::Space)),
                Focus::default()
            ),
            None,
            "and an unbound one runs nothing"
        );
        assert_eq!(
            chord_of(&Key::Named(NamedKey::F5), ModifiersState::empty()),
            Some(commands::Chord::plain(commands::Key::Function(5))),
            "F5"
        );
        assert_eq!(
            chord_of(&Key::Named(NamedKey::F11), ModifiersState::empty()),
            Some(commands::Chord::plain(commands::Key::F11)),
            "F11 keeps its variant"
        );
        assert_eq!(
            chord_of(&Key::Named(NamedKey::ArrowRight), ModifiersState::empty()),
            Some(commands::Chord::plain(commands::Key::Right)),
            "right"
        );
        assert_eq!(
            chord_of(&Key::Named(NamedKey::Tab), ModifiersState::empty()),
            None,
            "tab"
        );
    }

    #[test]
    fn output_window_keys_keep_their_modifiers() {
        let command = if cfg!(target_os = "macos") {
            ModifiersState::SUPER
        } else {
            ModifiersState::CONTROL
        };
        let run = |key: Key, modifiers: ModifiersState| {
            chord_of(&key, modifiers).and_then(|chord| commands::dispatch(chord, Focus::default()))
        };
        assert_eq!(
            run(Key::Named(NamedKey::ArrowRight), ModifiersState::SHIFT),
            Some(CommandId::SkipSection),
            "Shift+Right skips the section, as in the control window, not next cue"
        );
        assert_eq!(
            run(Key::Character("b".into()), command),
            None,
            "Ctrl+B is no command, so it does not black out the show"
        );
        assert_eq!(
            run(Key::Character("r".into()), ModifiersState::ALT),
            None,
            "Alt+R does not restart the cue"
        );
        assert_eq!(
            run(Key::Character("b".into()), ModifiersState::empty()),
            Some(CommandId::ToggleBlackout),
            "plain B still blacks out"
        );
        let (bindings, problems) = Bindings::from_config(
            &[("blackout_on".to_string(), "Shift+B".to_string())]
                .into_iter()
                .collect(),
        );
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(
            chord_of(&Key::Character("B".into()), ModifiersState::SHIFT)
                .and_then(|chord| commands::dispatch_with(chord, Focus::default(), &bindings)),
            Some(CommandId::BlackoutOn),
            "an operator's Shift+B reaches its command from the output window too"
        );
    }

    #[test]
    fn every_runtime_command_has_an_action_and_blackout_toggles() {
        for command in CommandId::ALL {
            let action = command_action(command, false);
            let presentation = matches!(
                command,
                CommandId::FocusBrowserSearch
                    | CommandId::CloseTopmostPane
                    | CommandId::OpenPalette
                    | CommandId::GenerateControls
                    | CommandId::ToggleControlsView
            );
            assert_eq!(
                action.is_none(),
                presentation,
                "{} has the wrong kind of action",
                command.name()
            );
        }
        assert_eq!(
            command_action(CommandId::ToggleBlackout, false),
            Some(Action::SetBlackout(true)),
            "off to on"
        );
        assert_eq!(
            command_action(CommandId::ToggleBlackout, true),
            Some(Action::SetBlackout(false)),
            "on to off"
        );
        for blackout in [false, true] {
            assert_eq!(
                command_action(CommandId::BlackoutOn, blackout),
                Some(Action::SetBlackout(true)),
                "on stays on, whatever the state (blackout={blackout})"
            );
            assert_eq!(
                command_action(CommandId::BlackoutOff, blackout),
                Some(Action::SetBlackout(false)),
                "off stays off, whatever the state (blackout={blackout})"
            );
        }
    }

    #[test]
    fn a_load_error_is_summarised_in_two_lines_and_bounded() {
        assert_eq!(summarize(""), "unknown error", "empty");
        assert_eq!(
            summarize("main.glsl:3: unknown identifier\n\n  foo\nmore"),
            "main.glsl:3: unknown identifier / foo",
            "two non-empty lines"
        );
        let long = "x".repeat(1000);
        assert!(summarize(&long).chars().count() <= 223, "bounded");
    }

    #[test]
    fn hidden_frames_are_never_faster_than_the_shown_cap() {
        assert_eq!(hidden_period(2.0), OUTPUT_PERIOD, "a 500 Hz monitor");
        assert_eq!(hidden_period(0.0), OUTPUT_PERIOD, "zero budget");
        assert_eq!(hidden_period(f32::NAN), OUTPUT_PERIOD, "nan budget");
    }

    #[test]
    fn a_desktop_viewer_with_nothing_new_redraws_slower_than_the_host_streams() {
        // A controller on the same two-core laptop as its host redrew its
        // viewer at 30 Hz for images that change five times a second at
        // most, and cost the host's show 210 late frames a minute. A new
        // image or the person's input brings the next redraw forward
        // (`Running::viewer_due`); with neither there is nothing to draw.
        let viewing = ControlFacts {
            unseen: false,
            streaming: false,
            slow: false,
            viewing: true,
        };
        assert!(
            control_period(viewing) > crate::desktop::STREAM_PERIOD,
            "idle viewer: {:?}",
            control_period(viewing)
        );
        assert_eq!(
            control_period(ControlFacts {
                viewing: false,
                ..viewing
            }),
            CONTROL_PERIOD,
            "the ordinary window in view keeps its 30 Hz"
        );
        assert_eq!(
            control_period(ControlFacts {
                unseen: true,
                streaming: true,
                ..viewing
            }),
            crate::desktop::STREAM_PERIOD,
            "a covered host window with a lease keeps the stream's rate (D290)"
        );
    }

    #[test]
    fn a_covered_window_nobody_painted_does_not_swap() -> Result<()> {
        assert!(
            !presents_control(true, true),
            "covered and unpainted: the swap that cost the mini PC half its frames (D508)"
        );
        assert!(
            presents_control(true, false),
            "a paint says part of the window may be on screen"
        );
        assert!(
            presents_control(false, true),
            "a window in view the loop drove still shows its frame"
        );
        assert!(presents_control(false, false), "the ordinary frame");
        Ok(())
    }
}
