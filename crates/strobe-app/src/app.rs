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
//!   it, it redraws at 2 Hz instead (`update_covered`, TORNADO T10).
//! - **A perf line** is logged once a minute (`stats::PerfWindow`): frame
//!   rate, late frames, the worst gap, GPU time and the control window's
//!   redraw cost, so a soak's log alone says where frames went.
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
use crate::library::{Health, Library};
use crate::localtime::LocalDate;
use crate::mouse::{Button, MouseTracker};
use crate::perf::{self, Performance};
use crate::recovery::{Decision, Event, Rebuild, Recovery, Surface};
use crate::remote::{Overrides, Remote};
use crate::screenshot::{Screenshot, Target};
use crate::session::{self, Session};
use crate::show::{self, Env, Phase, Show};
use crate::stats::{
    ControlPacer, ControlTimes, FinishMode, FrameStats, PaceChange, PerfWindow, SwapPolicy,
};
use crate::ui::widgets::Tone;
use crate::ui::{
    Action, CueTransport, EditorPanel, FailurePanel, GeneratePanel, ImportPanel, ListenerPanel,
    MapUi, MediaFrom, MediaPanel, Model, NoticePanel, PerfPanel, PlaylistPanel, PlaylistUi,
    Preview, RemotePanel, ScenePreview, ShellState, TransportPanel, UiHost, UiState, shell,
};
use crate::workspace::Pane;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use strobe_diag::context;
use strobe_diag::failure::{self, Failure, Subsystem};
use strobe_gfx::{Compositor, GlChoice, GpuTimer, MediaRequest, Post, SwapWindow, WindowedContext};
use strobe_io::{Banks, Output};
use strobe_playlist::{Command, Playlist, ResumePoint};
use strobe_scene::control::def::Values;
use strobe_scene::meta::MetaValues;
use strobe_shadertoy::listener::{Listener, ListenerConfig, Token};
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

/// How often the control window redraws while the fullscreen output covers
/// it (see [`Running::update_covered`]): enough to keep its state moving,
/// too seldom for its swap to cost the show a vertical blank.
const COVERED_PERIOD: Duration = Duration::from_millis(500);
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
            Ok(running) => {
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
        if let Err(e) = gfx.set_vsync(&output, true) {
            log::warn!("output vsync: {e}");
        }
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
    /// A minute of frame timings, logged as one line (T10).
    perf_log: PerfWindow,
    /// Slows the control window when switching to it waits on the output
    /// (D216).
    control_pacer: ControlPacer,
    /// When the output waits for its own swap, so switching to the control
    /// window does not (D221).
    swap_policy: SwapPolicy,
    /// What the file hovering over a window would do if dropped (D218).
    drop_hint: Option<String>,
    gl_summary: String,
    audio: AudioInput,
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

    monitors: Vec<MonitorHandle>,
    monitor_labels: Vec<String>,
    output_monitor: Option<usize>,
    last_monitor_refresh: Instant,

    /// Whether the output window is shown. Tracked here because winit cannot
    /// report visibility on X11 or Wayland.
    output_visible: bool,
    /// When the next scene frame is due, shown or hidden.
    next_output: Instant,
    next_control: Instant,
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
        library: Library,
        scan_error: Option<String>,
    ) -> Result<Self, String> {
        let render_scale = session::clamp_scale(cli.render_scale);
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
            bindings: Bindings::default(),
            map_ui: MapUi::default(),
            pending_preset: None,
            resume_enabled: !cli.no_resume && cli.screenshot.is_none(),
            last_cue: None,
            ready_frames: None,
            ui_state,
            shell: ShellState::default(),
            playing: None,
            console: Console::default(),
            stats: FrameStats::default(),
            perf_log: PerfWindow::default(),
            control_pacer: ControlPacer::default(),
            swap_policy: SwapPolicy::new(finish_mode()),
            drop_hint: None,
            gl_summary,
            audio: AudioInput::new(cli.audio == AudioArg::Auto),
            mouse: MouseTracker::default(),
            local_date: LocalDate::from_system(),
            render_scale,
            scale_edit: render_scale,
            pending_resize: None,
            resize_now: false,
            monitors: Vec::new(),
            monitor_labels: Vec::new(),
            output_monitor: None,
            last_monitor_refresh: now,
            output_visible: cli.show_output,
            control_on_top: true,
            control_covered: false,
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
            import,
            import_state: import::State::default(),
            listener: None,
            import_token: None,
            listener_note: "Off".to_string(),
            listener_failed: false,
            listen_port: cli.listen_port,
            data_dir,
            generate_job: None,
            generate_outcome: None,
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
        if let Some(error) = scan_error {
            failure::report(
                Failure::new(Subsystem::Io, "scan")
                    .path(&running.library.dir)
                    .message(error)
                    .action("the scene list is empty"),
            );
        }
        // Reopen the scene of the last session (T480 Task 21), unless the
        // command line chose one, a playlist will, or this is a screenshot
        // (whose state is never the operator's).
        let reopen = (cli.scene.is_none() && cli.playlist.is_none() && cli.screenshot.is_none())
            .then(|| running.ui_state.last_scene.clone())
            .filter(|name| !name.is_empty());
        if let Some(name) = reopen {
            match running.library.entries.iter().position(|e| e.name == name) {
                Some(index) => {
                    log::info!("reopening {name}, the scene of the last session");
                    running.load(index);
                }
                None => log::info!("the last session's scene {name} is not in the library"),
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
        if let Some(path) = cli.playlist.as_deref() {
            running.playlist_ui.path = path.display().to_string();
            running.load_playlist(path);
        }
        Ok(running)
    }

    fn finished(&self) -> bool {
        self.quit || self.screenshot.as_ref().is_some_and(Screenshot::finished)
    }

    fn shutdown(self) {
        // Stop accepting browser requests before the windows go.
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
            next_output: now,
            next_control: now,
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
        if now.duration_since(self.last_monitor_refresh) >= MONITOR_REFRESH {
            self.refresh_monitors(event_loop);
        }
        self.drain_imports();
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
        if now >= self.next_control {
            self.stack.control.window().request_redraw();
            let period = if self.control_covered {
                COVERED_PERIOD
            } else if self.control_pacer.is_slow() {
                crate::stats::SLOW_PERIOD
            } else {
                CONTROL_PERIOD
            };
            self.next_control = now + period;
        }
        self.next_output.min(self.next_control)
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

    fn refresh_monitors(&mut self, event_loop: &ActiveEventLoop) {
        self.last_monitor_refresh = Instant::now();
        self.update_covered();
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
    }

    // --------------------------------------------------------------- events

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if matches!(event, WindowEvent::CloseRequested) {
            // The output's close button (or Alt+F4 on it) hides it: the
            // control window is the app, and closing it quits.
            if self.rebuild.is_none() && id == self.stack.output.window().id() {
                log::info!("output window closed: hiding it (close the control window to quit)");
                self.set_output_visible(false);
            } else {
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
            let _ = self.stack.ui.on_window_event(window, &event);
        }
    }

    /// A key pressed in the output window: the same commands as the control
    /// window, with no text field to respect. Esc leaves fullscreen first.
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
        let Some(chord) = chord_of(&key.logical_key) else {
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
            .and_then(|index| self.monitors.get(index))
            .cloned();
        window.set_fullscreen(Some(Fullscreen::Borderless(monitor)));
        self.set_output_visible(true);
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
                if let Some(path) = &self.media_override {
                    session
                        .player
                        .choose_media(Some(MediaRequest::for_path(path)));
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

    /// Lists the library folder again, keeping what is known of each scene.
    fn rescan_library(&mut self) {
        match Library::scan(&self.library.dir) {
            Ok(library) => {
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
                self.scan_error = None;
                self.playing = playing_dir
                    .and_then(|old| self.library.entries.iter().position(|e| e.dir == old));
                self.close_preview();
                self.ui_state.browser.selected = None;
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
        let mouse = self.mouse.uniforms([window.width, window.height], full);
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

    /// Shows a banner.
    fn say(&mut self, tone: Tone, text: impl Into<String>) {
        self.notice = Some(Notice {
            tone,
            text: text.into(),
            since: Instant::now(),
        });
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
        self.stack.perf.scene_loaded(now);
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

    /// Runs the Generate form and keeps the result as a preview.
    fn generate_preview(&mut self) {
        let result = self.playlist_ui.form.spec().and_then(|spec| {
            let snapshot = show::generate::snapshot(&self.library);
            strobe_playlist::generate(&snapshot, &spec)
                .map_err(|e| show::generate::explain(&e, &self.library))
        });
        self.playlist_ui.preview = Some(result);
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
            intake::Dropped::Unknown(why) => {
                log::warn!("dropped {}: {why}", path.display());
                self.say(Tone::Warning, why);
            }
        }
    }

    /// Plays a dropped scene folder: from where it is if the library has
    /// it, else copied into the library first, the way an import lands.
    fn dropped_scene(&mut self, dir: &std::path::Path) {
        let same = |a: &std::path::Path, b: &std::path::Path| {
            a == b
                || matches!(
                    (std::fs::canonicalize(a), std::fs::canonicalize(b)),
                    (Ok(x), Ok(y)) if x == y
                )
        };
        if let Some(index) = self.library.entries.iter().position(|e| same(&e.dir, dir)) {
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
            match self.import_state.finished(done) {
                Outcome::Imported(dir) => self.show_imported(&dir),
                Outcome::Unchanged => {
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

    fn set_scene_tags(&mut self, tags: &[String]) {
        let Some(index) = self.playing else {
            self.say(Tone::Warning, "No scene is playing to tag.");
            return;
        };
        let Some((dir, name)) = self
            .library
            .entries
            .get(index)
            .map(|e| (e.dir.clone(), e.name.clone()))
        else {
            return;
        };
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
            self.editor_doc = editor::Doc::open(&dir, &name).ok();
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
    fn editor_save(&mut self) -> bool {
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
        self.generate_outcome = Some(outcome);
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
                Action::SetSceneTags(tags) => self.set_scene_tags(&tags),
                Action::SetMedia(path) => self.set_media(path),
                Action::EditCue { at, edit } => self.edit_cue(at, &edit),
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
                    let _ = self.editor_save();
                }
                Action::EditorSaveReload => {
                    if self.editor_save() {
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
                            text: "Hold needs a playlist. Load one in the LIST pane or start with \
                                   --playlist <file>. Nothing was held."
                                .to_string(),
                            since: Instant::now(),
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
                            "Jumping to a cue needs a playlist. Load one in the LIST pane.",
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
                Action::ToggleFullscreen => self.toggle_fullscreen(),
                Action::MoveToMonitor(index) => self.move_to_monitor(index),
                Action::ClearConsole => self.console.clear(),
                Action::SelectAudioInput(choice) => self.audio.select_device(choice),
                Action::RefreshAudioDevices => self.audio.refresh_devices(),
                Action::SetReactivity(gain) => self.audio.set_reactivity(gain),
                Action::SetAudioSpeed(gain) => self.audio.set_speed(gain),
                Action::TapTempo => self.audio.tap(),
                Action::Import(text) => self.submit_import(Input::Text(text), false),
                Action::ConfirmReplace => {
                    if let Some(input) = self.import_state.confirm_replace() {
                        self.submit_import(input, true);
                    }
                }
                Action::CancelReplace => self.import_state.cancel_replace(),
                Action::SetImportListener(on) => {
                    if on {
                        self.start_listener();
                    } else {
                        self.stop_listener();
                    }
                }
                Action::SetOsc(settings) => self.remote.set_osc(settings),
                Action::SetMidiEnabled(enabled) => {
                    let mut settings = self.remote.midi_saved.clone();
                    settings.enabled = enabled;
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
                }
            }
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
        let captured = session.capture_preset(name);
        let scene = session.name.clone();
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
            }
            Err(problem) => {
                log::warn!("{problem}");
                self.say(Tone::Warning, problem);
            }
        }
    }

    /// Deletes preset `name`'s file.
    fn delete_preset(&mut self, name: &str) {
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
            self.say(Tone::Info, format!("Fav slot {} is empty.", slot + 1));
            return;
        };
        let playing = self.stack.session.as_ref().map(|s| s.name.as_str());
        if playing == Some(fav.scene.as_str()) {
            match fav.preset {
                Some(name) => self.recall_preset(&name),
                None => {
                    if let Some(session) = self.stack.session.as_mut() {
                        for index in 0..session.controls.defs().len() {
                            session.controls.set_default(index);
                        }
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
                Ok(()) => {
                    context::set(context::RENDER_SIZE, format!("{}x{}", want[0], want[1]));
                    if let Some(show) = self.stack.show.as_mut() {
                        show.resize(want, &mut self.stack.compositor);
                    }
                }
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

    /// The output window's redraw: the scene frame, then the present.
    fn redraw_output(&mut self) {
        // A redraw requested just before the window was hidden.
        if self.output_visible {
            self.scene_frame(Surface::Output);
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
        self.perf_log.frame(
            started,
            self.stack.perf.budget_ms(),
            self.stack.perf.gpu_ms(),
        );
        if let Some(line) = self.perf_log.take_due(started) {
            log::info!("{line}");
        }
        let dt = if self.screenshot.is_some() {
            SCREENSHOT_DT
        } else {
            measured
        };
        let window = [size.width as i32, size.height as i32];
        self.settle_resize([size.width, size.height], started);
        self.drain_remote(started);

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
        if show_ready {
            if let Some(report) = self.run_show(|show, env| show.frame(env, dt)) {
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

    fn redraw_control(&mut self) {
        let began = Instant::now();
        let size = self.stack.control.window().inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        let current_began = Instant::now();
        if !self.make_current(Surface::Control) {
            return;
        }
        let make_current = current_began.elapsed();
        match self.control_pacer.observe(make_current) {
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
        let playing_dir = self.stack.session.as_ref().map(|s| s.dir.clone());
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
            }),
            drop_hint: self.drop_hint.as_deref(),
            transport,
            console: &self.console,
            stats: &self.stats,
            render_scale: &mut self.scale_edit,
            monitors: &self.monitor_labels,
            output_monitor: self.output_monitor,
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
            },
            editor: EditorPanel {
                doc: self.editor_doc.as_mut(),
                playing_dir: playing_dir.as_deref(),
            },
        };
        let mut actions = std::mem::take(&mut self.actions);
        let window = self.stack.control.window();
        let shell_state = &mut self.shell;
        let ui_started = Instant::now();
        self.stack.ui.run(window, |ui| {
            shell::show(ui, shell_state, &mut model, &mut actions)
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
        self.present(Surface::Control, swap_began);
        self.perf_log.control(ControlTimes {
            total: began.elapsed(),
            make_current,
            layout: ran,
            paint,
            swap: swap_began.elapsed(),
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

/// The chord a key press in the output window stands for. Modifiers are
/// not tracked there, so only plain keys exist.
fn chord_of(key: &Key) -> Option<commands::Chord> {
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
        CommandId::FocusBrowserSearch
        | CommandId::CloseTopmostPane
        | CommandId::OpenPalette
        | CommandId::GenerateControls => None,
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

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
        let key = |text: &str| chord_of(&Key::Character(text.into()));
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
            chord_of(&Key::Named(NamedKey::F5)),
            Some(commands::Chord::plain(commands::Key::Function(5))),
            "F5"
        );
        assert_eq!(
            chord_of(&Key::Named(NamedKey::F11)),
            Some(commands::Chord::plain(commands::Key::F11)),
            "F11 keeps its variant"
        );
        assert_eq!(
            chord_of(&Key::Named(NamedKey::ArrowRight)),
            Some(commands::Chord::plain(commands::Key::Right)),
            "right"
        );
        assert_eq!(chord_of(&Key::Named(NamedKey::Tab)), None, "tab");
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
}
