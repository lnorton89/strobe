// ============================================================================
// Module:       strobe_app::app
// Description:  The winit application: creates the two windows on one GL
//               context, paces the frame loop, and carries out UI actions.
//
// Dependencies: winit (event loop); strobe-gfx (WindowedContext);
//               crate::{ui, session, library, console, stats, screenshot}
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
//! ## Shutdown order
//!
//! GL objects must be released while the context is current and before
//! the windows and display they came from, so [`Running`] declares its
//! fields in that order and [`Running::shutdown`] frees the scene and the
//! egui painter explicitly first.

use crate::audio::AudioInput;
use crate::cli::{AudioArg, Cli};
use crate::console::Console;
use crate::failures;
use crate::gldebug;
use crate::library::Library;
use crate::screenshot::{Screenshot, Target};
use crate::session::{self, Session};
use crate::stats::FrameStats;
use crate::ui::{Action, Model, Preview, UiHost, layout};
use std::time::{Duration, Instant};
use strobe_diag::context;
use strobe_diag::failure::{self, Failure, Subsystem};
use strobe_gfx::{Backend, SwapWindow, WindowedContext};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalSize, PhysicalPosition};
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow};
use winit::keyboard::{Key, NamedKey};
use winit::monitor::MonitorHandle;
use winit::window::{Fullscreen, Window, WindowId};

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

    /// Stops the loop if the running app asked to or failed.
    fn settle(&mut self, event_loop: &ActiveEventLoop) {
        let Some(running) = self.running.as_mut() else {
            return;
        };
        if let Some(failure) = running.failure.take() {
            self.failure = Some(failure);
            event_loop.exit();
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

/// Everything that exists once the windows are open.
///
/// Field order is drop order for the GL-owning fields: scene, egui, the
/// windows, and last the context and display they share.
struct Running {
    session: Option<Session>,
    ui: UiHost,
    control: SwapWindow,
    output: SwapWindow,
    gfx: WindowedContext,

    library: Library,
    playing: Option<usize>,
    console: Console,
    stats: FrameStats,
    gl_summary: String,
    audio: AudioInput,

    /// The render scale in force.
    render_scale: f32,
    /// The slider's working value.
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
}

impl Running {
    fn new(
        event_loop: &ActiveEventLoop,
        cli: &Cli,
        library: Library,
        scan_error: Option<String>,
    ) -> Result<Self, String> {
        let output_attrs = Window::default_attributes()
            .with_title("Strobe Output")
            .with_inner_size(LogicalSize::new(960.0, 540.0));
        let control_attrs = Window::default_attributes()
            .with_title("Strobe")
            .with_inner_size(LogicalSize::new(1360.0, 820.0))
            .with_min_inner_size(LogicalSize::new(900.0, 560.0));

        let (mut gfx, output) = WindowedContext::open(event_loop, cli.gl.choice(), &output_attrs)
            .map_err(|e| format!("Could not start OpenGL.\n{e}"))?;
        // Before the Gl is shared with a second window or egui; see
        // `strobe_gfx::gl::debug`.
        gldebug::enable(gfx.gl_mut());
        record_gl_facts(&gfx);
        let control = gfx
            .add_window(event_loop, control_attrs)
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

        let info = &gfx.caps().info;
        let gl_summary = format!("{} | {} | {}", gfx.backend(), info.renderer, info.version);
        let screenshot = cli
            .screenshot
            .clone()
            .map(|base| Screenshot::new(base, cli.frames));
        let now = Instant::now();
        let render_scale = session::clamp_scale(cli.render_scale);
        let mut running = Self {
            session: None,
            ui,
            control,
            output,
            gfx,
            library,
            playing: None,
            console: Console::default(),
            stats: FrameStats::default(),
            gl_summary,
            audio: AudioInput::new(cli.audio == AudioArg::Auto),
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

    fn shutdown(mut self) {
        if self.gfx.make_current(&self.control).is_ok() {
            self.session = None;
            self.ui.destroy();
        }
    }

    // ----------------------------------------------------------- scheduling

    /// Requests redraws for whichever windows are due; returns when to wake.
    fn schedule(&mut self, event_loop: &ActiveEventLoop) -> Instant {
        let now = Instant::now();
        if now.duration_since(self.last_monitor_refresh) >= MONITOR_REFRESH {
            self.refresh_monitors(event_loop);
        }
        if now >= self.next_output {
            self.output.window().request_redraw();
            self.next_output = now + OUTPUT_PERIOD;
        }
        if now >= self.next_control {
            self.control.window().request_redraw();
            self.next_control = now + CONTROL_PERIOD;
        }
        self.next_output.min(self.next_control)
    }

    fn refresh_monitors(&mut self, event_loop: &ActiveEventLoop) {
        self.last_monitor_refresh = Instant::now();
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
        let is_control = id == self.control.window().id();
        let is_output = id == self.output.window().id();
        if matches!(event, WindowEvent::CloseRequested) {
            event_loop.exit();
        } else if matches!(event, WindowEvent::Resized(_)) {
            if is_control {
                self.gfx.resize(&self.control);
            } else if is_output {
                self.gfx.resize(&self.output);
            }
        } else if matches!(event, WindowEvent::RedrawRequested) {
            if is_output {
                self.redraw_output();
            } else if is_control {
                self.redraw_control();
            }
        } else if let WindowEvent::KeyboardInput { event: key, .. } = &event {
            let typing = is_control && self.ui.wants_keyboard();
            if !typing {
                self.key(key);
            }
        }
        if is_control {
            let window = self.control.window();
            let _ = self.ui.on_window_event(window, &event);
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
            && self.output.window().fullscreen().is_some()
        {
            self.output.window().set_fullscreen(None);
        }
    }

    fn toggle_fullscreen(&mut self) {
        let window = self.output.window();
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
        let window = self.output.window();
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
        let Some(entry) = self.library.entries.get(index).cloned() else {
            return;
        };
        if let Err(e) = self.gfx.make_current(&self.output) {
            self.failure = Some(e);
            return;
        }
        let window = self.output.window().inner_size();
        let size = session::render_size(
            [window.width, window.height],
            self.render_scale,
            self.gfx.caps().info.max_texture_size,
        );
        let started = Instant::now();
        match Session::load(
            self.gfx.gl(),
            self.gfx.caps(),
            &entry.dir,
            &entry.name,
            size,
            self.start_time,
        ) {
            Ok(session) => {
                self.session = Some(session);
                self.playing = Some(index);
                self.pending_resize = None;
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
                self.session.is_some(),
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
                    self.render_scale = session::clamp_scale(scale);
                    self.scale_edit = self.render_scale;
                    self.resize_now = true;
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
            self.gfx.caps().info.max_texture_size,
        );
        let Some(session) = self.session.as_mut() else {
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
        let measured = self.stats.begin(started);
        let dt = if self.screenshot.is_some() {
            SCREENSHOT_DT
        } else {
            measured
        };
        let size = self.output.window().inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        if let Err(e) = self.gfx.make_current(&self.output) {
            self.failure = Some(e);
            return;
        }
        let window = [size.width as i32, size.height as i32];
        self.settle_resize([size.width, size.height], started);

        let gl = self.gfx.gl();
        match self.session.as_mut() {
            Some(session) => {
                session.advance(dt, &mut self.audio);
                gl.plain_state();
                session.player.present(window);
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
        if let Err(e) = self.gfx.swap_buffers(&self.output) {
            log::warn!("output: {e}");
        }
    }

    fn redraw_control(&mut self) {
        let size = self.control.window().inner_size();
        if size.width == 0 || size.height == 0 {
            return;
        }
        if let Err(e) = self.gfx.make_current(&self.control) {
            self.failure = Some(e);
            return;
        }
        self.console.refresh();
        self.audio.refresh_status(Instant::now());
        let preview = self.session.as_ref().map(|session| {
            let texture = session.player.output_texture();
            Preview {
                texture: self.ui.preview_texture(texture.raw()),
                aspect: texture.width() as f32 / texture.height().max(1) as f32,
                size: [texture.width(), texture.height()],
            }
        });
        let (scene_title, controls) = match self.session.as_mut() {
            Some(session) => (session.title.as_str(), Some(&mut session.controls)),
            None => ("", None),
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
            fullscreen: self.output.window().fullscreen().is_some(),
            gl_summary: &self.gl_summary,
            audio: self.audio.panel(),
        };
        let mut actions = std::mem::take(&mut self.actions);
        let window = self.control.window();
        self.ui
            .run(window, |ui| layout::show(ui, &mut model, &mut actions));
        self.ui.paint(window, crate::theme::clear_color());

        if let Some(shot) = self.screenshot.as_mut()
            && shot.control_due()
        {
            let pixels = self
                .gfx
                .gl()
                .read_default_rgba8(size.width as i32, size.height as i32);
            match shot.save(Target::Control, [size.width, size.height], pixels) {
                Ok(path) => log::info!("wrote {}", path.display()),
                Err(e) => self.failure = Some(e),
            }
        }
        if let Err(e) = self.gfx.swap_buffers(&self.control) {
            log::warn!("control: {e}");
        }
        self.apply(&mut actions);
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
    context::set(
        context::GL_BACKEND,
        match gfx.backend() {
            Backend::Native => "native",
            Backend::Angle => "angle",
        },
    );
}
