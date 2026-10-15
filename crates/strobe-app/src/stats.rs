// ============================================================================
// Module:       strobe_app::stats
// Description:  Frame-time readout: smoothed CPU time per frame and the
//               frame rate the output window is actually achieving; the
//               per-minute perf line; and the control window's pacing
//               (make-current pacer, swap policy, paint-starvation drive).
//
// Dependencies: std only
// ============================================================================

//! Frame statistics.
//!
//! "CPU ms" is the time the app spends producing a frame, up to the point
//! it hands the finished frame to the driver; it excludes the wait for
//! vertical blank inside the swap. It is a cost figure, not a latency
//! one. The frame rate comes from the interval between frame starts, so a
//! vsync wait does count there, which is what makes it the rate the
//! audience sees. Both are exponential moving averages.

use std::time::{Duration, Instant};

/// Weight of the newest sample in the averages.
const SMOOTHING: f32 = 0.1;

/// Longest frame interval reported as `dt`; a stall (window drag, a long
/// compile) must not fling every smoothed control across its range.
const MAX_DT: f32 = 0.1;

/// Smoothed frame timings.
#[derive(Debug, Default)]
pub struct FrameStats {
    last_start: Option<Instant>,
    interval_ms: f32,
    cpu_ms: f32,
    ui_ms: f32,
}

impl FrameStats {
    /// Marks the start of a frame at `now` and returns the seconds since
    /// the previous one, clamped.
    pub fn begin(&mut self, now: Instant) -> f32 {
        let dt = match self.last_start {
            Some(previous) => now.duration_since(previous).as_secs_f32(),
            None => 1.0 / 60.0,
        };
        self.last_start = Some(now);
        if self.interval_ms == 0.0 {
            self.interval_ms = dt * 1000.0;
        } else {
            self.interval_ms += (dt * 1000.0 - self.interval_ms) * SMOOTHING;
        }
        dt.min(MAX_DT)
    }

    /// Records the CPU time one frame took.
    pub fn finish(&mut self, cpu: Duration) {
        let ms = cpu.as_secs_f32() * 1000.0;
        if self.cpu_ms == 0.0 {
            self.cpu_ms = ms;
        } else {
            self.cpu_ms += (ms - self.cpu_ms) * SMOOTHING;
        }
    }

    /// Records the time one control-window egui pass took (building and
    /// laying out every widget, not the GL paint), so a UI that has become expensive shows up in the
    /// diagnostics instead of as an unexplained dropped frame.
    pub fn record_ui(&mut self, took: Duration) {
        let ms = took.as_secs_f32() * 1000.0;
        if self.ui_ms == 0.0 {
            self.ui_ms = ms;
        } else {
            self.ui_ms += (ms - self.ui_ms) * SMOOTHING;
        }
    }

    /// Smoothed milliseconds an egui layout pass takes.
    pub fn ui_ms(&self) -> f32 {
        self.ui_ms
    }

    /// Smoothed CPU milliseconds per frame.
    pub fn cpu_ms(&self) -> f32 {
        self.cpu_ms
    }

    /// Smoothed frames per second.
    pub fn fps(&self) -> f32 {
        if self.interval_ms > 0.0 {
            1000.0 / self.interval_ms
        } else {
            0.0
        }
    }
}

/// How long one [`PerfWindow`] summarises.
pub const PERF_WINDOW: Duration = Duration::from_secs(60);

/// A frame counts as late when its interval is this many frame budgets.
const LATE_FACTOR: f32 = 1.5;

/// A minute of frame timings, summarised for the log.
///
/// TORNADO T10 ran a fullscreen show at 48 to 56 fps with the GPU busy for
/// under 3 ms a frame, and nothing in the log said where the time went.
/// One line a minute (frame rate, late frames, the worst gap, GPU time and
/// the control window's whole redraw, swap included) settles that kind of
/// question from a soak's log alone. Recording is a few additions; the
/// line is formatted once a minute, never per frame.
#[derive(Debug, Default)]
pub struct PerfWindow {
    started: Option<Instant>,
    last_frame: Option<Instant>,
    frames: u32,
    late: u32,
    worst_ms: f32,
    gpu_sum: f32,
    gpu_frames: u32,
    control_frames: u32,
    control_sum: f32,
    control_worst: f32,
    /// Control redraws the event loop ran without a paint (D290).
    control_unpainted: u32,
    /// Per part of a control redraw, in [`ControlTimes`]' order: summed
    /// milliseconds.
    part_sums: [f32; 4],
}

/// Where one control-window redraw spent its time (TORNADO T12: the swap
/// itself returned in 0.03 ms while the redraw took 17.5 ms, so the wait is
/// elsewhere; this says where).
#[derive(Clone, Copy, Debug, Default)]
pub struct ControlTimes {
    /// The whole redraw.
    pub total: Duration,
    /// Making the context current on the control window's surface.
    pub make_current: Duration,
    /// The egui pass: building and laying out the widgets.
    pub layout: Duration,
    /// egui's GL paint, up to the swap.
    pub paint: Duration,
    /// The buffer swap.
    pub swap: Duration,
    /// The event loop ran this redraw because no paint came (D290).
    pub unpainted: bool,
}

impl PerfWindow {
    /// Records a scene frame starting at `now`, against a frame budget of
    /// `budget_ms`, with the GPU time last measured.
    pub fn frame(&mut self, now: Instant, budget_ms: f32, gpu_ms: Option<f32>) {
        self.started.get_or_insert(now);
        if let Some(previous) = self.last_frame {
            let ms = now.duration_since(previous).as_secs_f32() * 1000.0;
            self.worst_ms = self.worst_ms.max(ms);
            if ms > budget_ms * LATE_FACTOR {
                self.late += 1;
            }
        }
        self.last_frame = Some(now);
        self.frames += 1;
        if let Some(gpu) = gpu_ms.filter(|g| g.is_finite()) {
            self.gpu_sum += gpu;
            self.gpu_frames += 1;
        }
    }

    /// Records one control-window redraw and where its time went.
    pub fn control(&mut self, times: ControlTimes) {
        let ms = |d: Duration| d.as_secs_f32() * 1000.0;
        self.control_frames += 1;
        self.control_sum += ms(times.total);
        self.control_worst = self.control_worst.max(ms(times.total));
        if times.unpainted {
            self.control_unpainted += 1;
        }
        for (sum, part) in self.part_sums.iter_mut().zip([
            times.make_current,
            times.layout,
            times.paint,
            times.swap,
        ]) {
            *sum += ms(part);
        }
    }

    /// The summary line when [`PERF_WINDOW`] has passed since the first
    /// frame recorded, and the window starts over (keeping the last frame's
    /// time so the next interval is measured across the boundary).
    pub fn take_due(&mut self, now: Instant) -> Option<String> {
        let started = self.started?;
        let span = now.duration_since(started).as_secs_f32();
        if span < PERF_WINDOW.as_secs_f32() {
            return None;
        }
        let mean = |sum: f32, n: u32| if n > 0 { sum / n as f32 } else { 0.0 };
        let line = format!(
            "perf: {:.1} fps over {:.0} s, {} late of {} frames, worst gap {:.1} ms, \
             gpu {:.2} ms, control {} redraws {:.2} ms (worst {:.1} ms, {} unpainted): \
             make-current {:.2}, layout {:.2}, paint {:.2}, swap {:.2} ms",
            self.frames as f32 / span,
            span,
            self.late,
            self.frames,
            self.worst_ms,
            mean(self.gpu_sum, self.gpu_frames),
            self.control_frames,
            mean(self.control_sum, self.control_frames),
            self.control_worst,
            self.control_unpainted,
            mean(self.part_sums[0], self.control_frames),
            mean(self.part_sums[1], self.control_frames),
            mean(self.part_sums[2], self.control_frames),
            mean(self.part_sums[3], self.control_frames),
        );
        *self = Self {
            started: Some(now),
            last_frame: self.last_frame,
            ..Self::default()
        };
        Some(line)
    }
}

/// Paces the control window by what making its context current costs
/// (D216).
///
/// TORNADO T13 (GTX 1080 Ti, windowed) found each control redraw spending
/// about 16 ms in `make_current`: switching the shared context to the
/// control window waits for the output's vsynced swap to finish, and every
/// such wait cost the show a vertical blank. When that wait settles above
/// [`SLOW_ABOVE_MS`], the control window drops to [`SLOW_PERIOD`] until it
/// settles below [`FAST_BELOW_MS`]; the gap between the two keeps it from
/// flapping. The T480 (Intel) never waits there, so this never engages
/// on it.
#[derive(Debug, Default)]
pub struct ControlPacer {
    smoothed_ms: f32,
    slow: bool,
}

/// Above this smoothed make-current cost the control window slows down.
pub const SLOW_ABOVE_MS: f32 = 8.0;

/// Below this it returns to its normal rate.
pub const FAST_BELOW_MS: f32 = 3.0;

/// The control window's period while slowed: 10 Hz.
pub const SLOW_PERIOD: Duration = Duration::from_millis(100);

/// Weight of the newest make-current sample.
const PACER_SMOOTHING: f32 = 0.2;

/// What a [`ControlPacer::observe`] changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaceChange {
    /// Nothing.
    None,
    /// It just slowed down.
    Slowed,
    /// It just went back to normal.
    Resumed,
}

impl ControlPacer {
    /// Records one redraw's make-current time.
    pub fn observe(&mut self, make_current: Duration) -> PaceChange {
        let ms = make_current.as_secs_f32() * 1000.0;
        self.smoothed_ms += (ms - self.smoothed_ms) * PACER_SMOOTHING;
        if !self.slow && self.smoothed_ms > SLOW_ABOVE_MS {
            self.slow = true;
            return PaceChange::Slowed;
        }
        if self.slow && self.smoothed_ms < FAST_BELOW_MS {
            self.slow = false;
            return PaceChange::Resumed;
        }
        PaceChange::None
    }

    /// Whether the control window is slowed.
    pub fn is_slow(&self) -> bool {
        self.slow
    }

    /// The smoothed make-current cost, in milliseconds.
    pub fn smoothed_ms(&self) -> f32 {
        self.smoothed_ms
    }

    /// Starts measuring again from nothing, at the normal rate.
    pub fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Runs the control window's frames from the event loop when Windows stops
/// painting it (D290).
///
/// winit delivers `RedrawRequested` on Windows only from a `WM_PAINT`, and
/// Windows makes a `WM_PAINT` for the first dirty window of the thread in
/// z-order. winit stops dispatching after each `RedrawRequested`, and the
/// output asks for its next frame in every `about_to_wait` once its
/// vsync-blocked frame outlasts `OUTPUT_PERIOD`. So while the output is
/// above the control window, the output is always dirty first and the
/// control window's paint never comes: the JaJa mini PC logged
/// "control 0 redraws" a minute with the output at 60 fps. Every request is
/// still made as before; when one has gone unanswered for
/// [`PAINT_PATIENCE`] and [`PAINT_PASSES`] passes of the event loop, the
/// control frame runs from `about_to_wait` itself, at the control window's
/// period, until a paint answers promptly again. Counting passes keeps a
/// loop that was simply blocked (a scene compiling for a second) from
/// reading as a window Windows will not paint.
#[derive(Debug, Default)]
pub struct ControlDrive {
    /// When the oldest unanswered `request_redraw` was made.
    asked: Option<Instant>,
    /// Passes of the event loop since then, each a chance for the paint.
    passes: u32,
    /// Whether the event loop is running the control frames.
    driving: bool,
    /// How many times the event loop has taken over, for the log.
    takeovers: u32,
}

/// How long a requested control redraw may go unanswered before the event
/// loop runs the control frame itself. A painted window is answered within
/// one output frame; this is fifteen of them at 60 Hz.
pub const PAINT_PATIENCE: Duration = Duration::from_millis(250);

/// How many passes of the event loop a request must also go unanswered
/// through. A starved window sees one pass per output frame, fifteen in
/// [`PAINT_PATIENCE`]; a loop blocked for that long sees one or two.
pub const PAINT_PASSES: u32 = 4;

/// What to do with a control frame that is due ([`ControlDrive::due`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlStep {
    /// Request a redraw and let its `RedrawRequested` run the frame.
    Request,
    /// Request a redraw, and run the frame now: no paint has come for
    /// [`PAINT_PATIENCE`]. This is the first such frame since paints last
    /// came.
    TakeOver {
        /// How long the oldest request has gone unanswered.
        waited: Duration,
        /// The takeovers so far, this one included.
        times: u32,
    },
    /// Request a redraw, and run the frame now: the event loop is already
    /// running the control frames.
    RunHere,
}

impl ControlDrive {
    /// Counts one pass of the event loop (`about_to_wait`).
    pub fn pass(&mut self) {
        self.passes = self.passes.saturating_add(1);
    }

    /// The control window is due at `now`: says whether the event loop
    /// runs its frame or a paint is waited for. The caller requests a
    /// redraw either way, so a paint that does come is never lost.
    pub fn due(&mut self, now: Instant) -> ControlStep {
        let Some(asked) = self.asked else {
            self.asked = Some(now);
            self.passes = 0;
            return ControlStep::Request;
        };
        let waited = now.saturating_duration_since(asked);
        if waited < PAINT_PATIENCE || self.passes < PAINT_PASSES {
            return ControlStep::Request;
        }
        if self.driving {
            return ControlStep::RunHere;
        }
        self.driving = true;
        self.takeovers = self.takeovers.saturating_add(1);
        ControlStep::TakeOver {
            waited,
            times: self.takeovers,
        }
    }

    /// A `RedrawRequested` reached the control window at `now`. Returns
    /// true when that hands the frames back to the paints: the event loop
    /// was running them, and this paint answered its request within
    /// [`PAINT_PATIENCE`]. A late paint (one an output frame happened to
    /// let through) leaves the event loop in charge, so the two do not take
    /// turns.
    pub fn painted(&mut self, now: Instant) -> bool {
        let prompt = self
            .asked
            .take()
            .is_none_or(|asked| now.saturating_duration_since(asked) < PAINT_PATIENCE);
        if self.driving && prompt {
            self.driving = false;
            return true;
        }
        false
    }

    /// Whether the event loop is running the control frames, which means
    /// something above the control window is taking all of its paints.
    pub fn driving(&self) -> bool {
        self.driving
    }
}

/// Whether the output waits for its own swap (`glFinish` after it), from
/// `STROBE_FINISH_AFTER_SWAP` (D221).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FinishMode {
    /// Unset: only once the pacer has seen the control window wait.
    #[default]
    Auto,
    /// `1`: always.
    On,
    /// `0`: never; the pacer's 10 Hz is the only remedy.
    Off,
}

impl FinishMode {
    /// The mode `STROBE_FINISH_AFTER_SWAP`'s value asks for.
    pub fn from_env(value: Option<&str>) -> Self {
        match value.map(str::trim) {
            Some("1") => Self::On,
            Some("0") => Self::Off,
            Some(_) | None => Self::Auto,
        }
    }
}

/// When the output waits for its swap (D221). TORNADO T15 found that on
/// NVIDIA a windowed output's vsynced swap makes the next switch of the
/// shared context to the control window wait about 16 ms, which costs the
/// show a frame per control redraw; waiting for the swap on the output's
/// side instead (`glFinish` right after it) took make-current to 1.5 ms and
/// late frames from about 570 a minute to under 10, at 30 Hz. The wait
/// costs the CPU/GPU overlap of one frame, so in `Auto` it engages only
/// once the pacer has seen the problem, and never while a fullscreen output
/// covers the control window (which then hardly redraws).
#[derive(Debug, Default)]
pub struct SwapPolicy {
    mode: FinishMode,
    engaged: bool,
}

impl SwapPolicy {
    /// A policy in `mode`.
    pub fn new(mode: FinishMode) -> Self {
        Self {
            mode,
            engaged: false,
        }
    }

    /// The pacer just slowed the control window. Returns whether this
    /// engaged the wait instead, in which case the pacer should start over
    /// at the normal rate; the second time, or with the mode not `Auto`, the
    /// pacer's slowing stands.
    pub fn on_slowed(&mut self) -> bool {
        if self.mode == FinishMode::Auto && !self.engaged {
            self.engaged = true;
            return true;
        }
        false
    }

    /// Whether the output should wait for the swap it just made.
    pub fn finish_now(&self, control_covered: bool) -> bool {
        match self.mode {
            FinishMode::On => true,
            FinishMode::Off => false,
            FinishMode::Auto => self.engaged && !control_covered,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_pacer_slows_on_a_sustained_wait_and_resumes_with_hysteresis() {
        let mut pacer = ControlPacer::default();
        let ms = Duration::from_millis;
        assert_eq!(
            pacer.observe(ms(16)),
            PaceChange::None,
            "one slow redraw is not a trend"
        );
        let mut slowed = false;
        for _ in 0..10 {
            slowed |= pacer.observe(ms(16)) == PaceChange::Slowed;
        }
        assert!(slowed && pacer.is_slow(), "a sustained wait slows it");
        for _ in 0..3 {
            assert_eq!(
                pacer.observe(ms(5)),
                PaceChange::None,
                "in the gap: no flapping"
            );
        }
        assert!(pacer.is_slow(), "still slow between the thresholds");
        let mut resumed = false;
        for _ in 0..20 {
            resumed |= pacer.observe(ms(0)) == PaceChange::Resumed;
        }
        assert!(
            resumed && !pacer.is_slow(),
            "a cheap make-current brings it back"
        );
    }

    /// Runs `passes` passes of the event loop.
    fn passes(drive: &mut ControlDrive, passes: u32) {
        for _ in 0..passes {
            drive.pass();
        }
    }

    #[test]
    fn a_painted_control_window_is_never_driven_by_the_loop() {
        let mut drive = ControlDrive::default();
        let start = Instant::now();
        for frame in 0..300_u32 {
            // 30 Hz, each request answered by a paint 20 ms later, with
            // two output frames in between.
            let at = start + Duration::from_millis(33) * frame;
            assert_eq!(
                drive.due(at),
                ControlStep::Request,
                "frame {frame} waits for its paint"
            );
            passes(&mut drive, 2);
            assert!(
                !drive.painted(at + Duration::from_millis(20)),
                "nothing to hand back at frame {frame}"
            );
        }
        assert!(!drive.driving(), "paints keep the frames");
    }

    #[test]
    fn a_control_window_with_no_paints_is_run_by_the_loop_at_its_own_period() {
        // The JaJa mini PC: requests at 30 Hz, an output frame every 16 ms,
        // and never a paint.
        let mut drive = ControlDrive::default();
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        let patience = PAINT_PATIENCE.as_millis() as u64;
        assert_eq!(drive.due(at(0)), ControlStep::Request, "first request");
        passes(&mut drive, 2);
        assert_eq!(
            drive.due(at(33)),
            ControlStep::Request,
            "one missed paint is not a starved window"
        );
        passes(&mut drive, 13);
        assert_eq!(
            drive.due(at(patience - 1)),
            ControlStep::Request,
            "still inside the patience"
        );
        assert_eq!(
            drive.due(at(patience)),
            ControlStep::TakeOver {
                waited: PAINT_PATIENCE,
                times: 1
            },
            "the loop takes over once the patience is spent"
        );
        assert!(drive.driving(), "and is now running the frames");
        for frame in 1..1000_u64 {
            passes(&mut drive, 2);
            assert_eq!(
                drive.due(at(patience + 33 * frame)),
                ControlStep::RunHere,
                "every due frame runs from the loop ({frame})"
            );
        }
    }

    #[test]
    fn a_blocked_loop_is_not_a_window_without_paints() {
        // A scene compiles for 1.4 s inside an output frame: the request
        // is old, but the loop has had one pass in which to deliver it.
        let mut drive = ControlDrive::default();
        let start = Instant::now();
        drive.due(start);
        drive.pass();
        assert_eq!(
            drive.due(start + Duration::from_millis(1400)),
            ControlStep::Request,
            "too few passes to say the paint is being withheld"
        );
        assert!(
            !drive.painted(start + Duration::from_millis(1410)),
            "the paint comes on the next pass"
        );
        assert!(!drive.driving(), "the loop never took the frames");
    }

    #[test]
    fn the_loop_hands_back_only_to_a_prompt_paint() {
        let mut drive = ControlDrive::default();
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        drive.due(at(0));
        passes(&mut drive, 30);
        assert!(
            matches!(drive.due(at(500)), ControlStep::TakeOver { .. }),
            "a covered window's next frame, half a second on, takes over"
        );
        assert!(
            !drive.painted(at(900)),
            "a paint 900 ms after its request is a lucky one: the loop keeps the frames"
        );
        assert!(drive.driving(), "still driving after a late paint");
        assert_eq!(drive.due(at(1000)), ControlStep::Request, "asked afresh");
        passes(&mut drive, 30);
        assert_eq!(
            drive.due(at(1500)),
            ControlStep::RunHere,
            "unanswered again: run here, without a second takeover"
        );
        assert!(
            !drive.painted(at(2010)),
            "a request standing since 1000 ms is not answered promptly"
        );
        assert_eq!(drive.due(at(2033)), ControlStep::Request, "asked afresh");
        assert!(
            drive.painted(at(2040)),
            "a paint 7 ms after its request hands the frames back"
        );
        assert!(!drive.driving(), "paints have the frames again");
        drive.due(at(3000));
        passes(&mut drive, 20);
        assert_eq!(
            drive.due(at(3300)),
            ControlStep::TakeOver {
                waited: Duration::from_millis(300),
                times: 2
            },
            "a second takeover is counted"
        );
    }

    #[test]
    fn the_swap_wait_engages_once_on_the_first_slowdown_and_only_in_auto() {
        assert_eq!(FinishMode::from_env(None), FinishMode::Auto, "unset");
        assert_eq!(FinishMode::from_env(Some("1")), FinishMode::On, "1");
        assert_eq!(FinishMode::from_env(Some(" 0 ")), FinishMode::Off, "0");
        assert_eq!(FinishMode::from_env(Some("yes")), FinishMode::Auto, "other");

        let mut auto = SwapPolicy::new(FinishMode::Auto);
        assert!(!auto.finish_now(false), "off until the pacer sees a wait");
        assert!(auto.on_slowed(), "the first slowdown engages it");
        assert!(auto.finish_now(false), "then the output waits for its swap");
        assert!(
            !auto.finish_now(true),
            "but not while the output covers the control window"
        );
        assert!(
            !auto.on_slowed(),
            "a second slowdown is the pacer's: the wait did not help"
        );

        let mut off = SwapPolicy::new(FinishMode::Off);
        assert!(!off.on_slowed() && !off.finish_now(false), "off is off");
        let on = SwapPolicy::new(FinishMode::On);
        assert!(on.finish_now(true), "on is on, covered or not");
    }

    #[test]
    fn a_minute_is_summarised_once_with_late_frames_counted() {
        let mut window = PerfWindow::default();
        let start = Instant::now();
        let mut at = start;
        for i in 0..3600_u32 {
            // Every tenth frame misses a vertical blank.
            let gap = if i % 10 == 9 { 33_333 } else { 16_667 };
            at += Duration::from_micros(gap);
            window.frame(at, 16.67, Some(2.0));
            if i % 2 == 0 {
                window.control(ControlTimes {
                    total: Duration::from_millis(3),
                    make_current: Duration::from_micros(500),
                    layout: Duration::from_micros(1000),
                    paint: Duration::from_micros(500),
                    swap: Duration::from_millis(1),
                    unpainted: i % 4 == 0,
                });
            }
            if i < 3000 {
                assert!(window.take_due(at).is_none(), "not a minute yet at {i}");
            }
        }
        let line = window
            .take_due(at + Duration::from_secs(1))
            .unwrap_or_default();
        assert!(line.contains("360 late of 3600"), "{line}");
        assert!(line.contains("worst gap 33.3 ms"), "{line}");
        assert!(line.contains("gpu 2.00 ms"), "{line}");
        assert!(line.contains("control 1800 redraws 3.00 ms"), "{line}");
        assert!(line.contains("900 unpainted"), "{line}");
        assert!(
            line.contains("make-current 0.50, layout 1.00, paint 0.50, swap 1.00 ms"),
            "{line}"
        );
        assert!(
            window.take_due(at + Duration::from_secs(2)).is_none(),
            "the window started over"
        );
    }

    #[test]
    fn steady_sixty_hertz_reads_sixty() {
        let mut stats = FrameStats::default();
        let start = Instant::now();
        for i in 0..120_u32 {
            stats.begin(start + Duration::from_micros(16_667) * i);
        }
        assert!((stats.fps() - 60.0).abs() < 0.5, "fps was {}", stats.fps());
    }

    #[test]
    fn the_ui_pass_time_is_smoothed_like_the_others() {
        let mut stats = FrameStats::default();
        assert_eq!(stats.ui_ms(), 0.0, "nothing measured yet");
        stats.record_ui(Duration::from_millis(4));
        assert!((stats.ui_ms() - 4.0).abs() < 1e-3, "first sample");
        stats.record_ui(Duration::from_millis(14));
        assert!(
            stats.ui_ms() > 4.0 && stats.ui_ms() < 14.0,
            "{}",
            stats.ui_ms()
        );
    }

    #[test]
    fn a_stall_is_clamped_for_the_scene_clock() {
        let mut stats = FrameStats::default();
        let start = Instant::now();
        stats.begin(start);
        let dt = stats.begin(start + Duration::from_secs(5));
        assert!((dt - MAX_DT).abs() < f32::EPSILON, "dt was {dt}");
    }
}
