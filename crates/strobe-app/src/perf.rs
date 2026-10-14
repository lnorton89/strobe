// ============================================================================
// Module:       strobe_app::perf
// Description:  GPU timing and adaptive resolution wired together: the output
//               frame's GPU time feeds the adaptive controller, which moves
//               the render scale under the person's manual maximum.
//
// Dependencies: strobe-gfx (GpuTimer, AdaptiveScale); crate::session (scale
//               range)
// ============================================================================

//! Performance control.
//!
//! ```text
//! output frame:   timer.begin ... advance + present ... timer.end
//!                 timer.poll -> GPU ms of an earlier frame (never waits)
//!                 -> adaptive.observe -> new scale? -> app resizes
//! ```
//!
//! ## Rules
//!
//! - **The slider is the ceiling.** With *Adaptive* on, the render-scale
//!   slider is the maximum the controller may use; with it off, the slider
//!   is the scale. Moving the slider rebuilds the controller at the new
//!   ceiling.
//! - **Budget** is one refresh period of the output's monitor
//!   (`1000 / Hz`), or 16.67 ms when the monitor does not say.
//! - **No change in the first second after a scene load**, and the
//!   measurements are forgotten at the load: a new scene's cost says nothing
//!   about the old one's, and its first frames pay for compiling and
//!   allocating.
//! - **Every change is logged**, with the GPU time that caused it.
//! - **A scene that pins its final pass is left at the ceiling (D280).**
//!   Its passes are absolute pixels, so the render scale changes nothing it
//!   draws; lowering it would cut nothing and cost sharpness elsewhere.
//!   The load says so once, and the controller does not run for it.
//! - **A drop that cuts nothing is undone (D280).** When a scene's cost is
//!   not its resolution for some other reason, the controller notices that
//!   the GPU time did not fall, goes back up, and stops lowering until the
//!   scene changes. The change says so and the log gives the numbers.
//! - **No timer, no adaptation.** On a context without timer queries the
//!   toggle has nothing to measure, so the scale stays where the slider
//!   puts it and the status bar says so.

use crate::session;
use std::time::{Duration, Instant};
use strobe_gfx::GpuTimer;
use strobe_gfx::adaptive::{AdaptiveScale, Futile};

/// The budget when the monitor reports no refresh rate: 60 Hz.
pub const FALLBACK_BUDGET_MS: f32 = 1000.0 / 60.0;

/// How long after a scene load the scale is left alone.
pub const HOLD: Duration = Duration::from_secs(1);

/// Weight of the newest sample in the displayed GPU time.
const DISPLAY_SMOOTHING: f32 = 0.1;

/// A change the controller made.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScaleChange {
    /// The scale before.
    pub from: f32,
    /// The scale now.
    pub to: f32,
    /// The GPU time sample that tipped it, in milliseconds.
    pub gpu_ms: f32,
    /// The budget it was judged against.
    pub budget_ms: f32,
    /// Set when this change is the controller undoing a drop that did not
    /// cut the GPU time (`to` is then the scale it went back to).
    pub futile: Option<Futile>,
}

/// The budget for a monitor refresh rate in millihertz, as winit reports it.
pub fn budget_for(refresh_millihertz: Option<u32>) -> f32 {
    match refresh_millihertz {
        Some(mhz) if mhz > 0 => 1.0e6 / mhz as f32,
        Some(_) | None => FALLBACK_BUDGET_MS,
    }
}

/// GPU timing and the adaptive controller.
#[derive(Debug)]
pub struct Performance {
    timer: Option<GpuTimer>,
    /// Whether there is a GPU time to adapt to. A field rather than
    /// `timer.is_some()` so the controller's rules can be tested without a
    /// GL context, and so freeing the queries at shutdown changes nothing.
    measurable: bool,
    adaptive: AdaptiveScale,
    enabled: bool,
    /// The manual maximum scale.
    max: f32,
    gpu_ms: Option<f32>,
    hold_until: Option<Instant>,
    /// The size the live scene pins its final pass to (D280), if it does.
    pinned: Option<[i32; 2]>,
}

impl Performance {
    /// Starts with `max` as the ceiling, adaptation `enabled` or not, and
    /// `budget_ms` per frame. `timer` is `None` where the context cannot
    /// time the GPU.
    pub fn new(timer: Option<GpuTimer>, max: f32, enabled: bool, budget_ms: f32) -> Self {
        let max = session::clamp_scale(max);
        Self {
            measurable: timer.is_some(),
            timer,
            adaptive: controller(budget_ms, max),
            enabled,
            max,
            gpu_ms: None,
            hold_until: None,
            pinned: None,
        }
    }

    /// Whether the scale is being adapted: asked for, and measurable.
    pub fn adapting(&self) -> bool {
        self.enabled && self.measurable
    }

    /// Whether the toggle is on (it may have nothing to measure with).
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Whether the GPU can be timed on this context.
    pub fn has_timer(&self) -> bool {
        self.measurable
    }

    /// The scale to render at: the controller's when adapting, else the
    /// manual one.
    pub fn scale(&self) -> f32 {
        if self.adapting() {
            self.adaptive.scale()
        } else {
            self.max
        }
    }

    /// The manual maximum.
    pub fn max(&self) -> f32 {
        self.max
    }

    /// Smoothed GPU milliseconds per frame, once any have been measured.
    pub fn gpu_ms(&self) -> Option<f32> {
        self.gpu_ms
    }

    /// The per-frame budget in milliseconds.
    pub fn budget_ms(&self) -> f32 {
        self.adaptive.budget_ms()
    }

    /// Turns adaptation on or off. Turning it on starts a fresh controller
    /// at the ceiling.
    pub fn set_enabled(&mut self, enabled: bool) {
        if enabled && !self.enabled {
            self.adaptive = controller(self.adaptive.budget_ms(), self.max);
        }
        self.enabled = enabled;
    }

    /// Sets the manual maximum scale (the slider).
    pub fn set_max(&mut self, max: f32) {
        self.max = session::clamp_scale(max);
        self.adaptive = controller(self.adaptive.budget_ms(), self.max);
    }

    /// Resumes the controller at `scale`, for a rebuilt GL context: the
    /// scale the old one had settled on is what this GPU could carry, and
    /// starting again from the ceiling would reload the very load that may
    /// have caused the loss.
    pub fn resume_scale(&mut self, scale: f32) {
        self.adaptive.set_scale(scale);
    }

    /// Adopts the output monitor's refresh rate. Returns whether the budget
    /// changed enough to matter.
    pub fn set_refresh(&mut self, refresh_millihertz: Option<u32>) -> bool {
        let budget = budget_for(refresh_millihertz);
        if (budget - self.adaptive.budget_ms()).abs() < 0.05 {
            return false;
        }
        self.adaptive.set_budget(budget);
        true
    }

    /// A scene was loaded at `now`: forget the measurements and leave the
    /// scale alone for [`HOLD`]. `pinned` is the size the scene pins its
    /// final pass to, if it does (D280): the scale then returns to the
    /// ceiling and stays there, since it changes nothing the scene draws.
    pub fn scene_loaded(&mut self, now: Instant, pinned: Option<[i32; 2]>) {
        self.adaptive.reset_measurements();
        self.hold_until = Some(now + HOLD);
        self.pinned = pinned;
        if let Some([width, height]) = pinned {
            self.adaptive.set_scale(self.max);
            if self.adapting() {
                log::info!(
                    "adaptive resolution: this scene renders at {width}x{height} whatever the \
                     render scale (D280), so the scale stays at {:.2}",
                    self.max
                );
            }
        }
    }

    /// Whether the live scene pins its final pass, so the scale is moot.
    pub fn pinned(&self) -> Option<[i32; 2]> {
        self.pinned
    }

    /// Starts timing the output frame's GPU work.
    pub fn begin(&mut self) {
        if let Some(timer) = self.timer.as_mut() {
            timer.begin();
        }
    }

    /// Stops timing, reads whatever results are ready (never waiting), and
    /// feeds them to the controller. Returns the last scale change, if any.
    pub fn end(&mut self, now: Instant) -> Option<ScaleChange> {
        self.timer.as_mut()?.end();
        self.poll(now)
    }

    /// Reads whatever results are ready (never waiting) and feeds them to
    /// the controller. Returns the last scale change, if any.
    pub fn poll(&mut self, now: Instant) -> Option<ScaleChange> {
        let timer = self.timer.as_mut()?;
        let mut samples = [0.0_f32; strobe_gfx::timing::SLOTS];
        let mut count = 0;
        timer.poll(|ms| {
            if let Some(slot) = samples.get_mut(count) {
                *slot = ms;
                count += 1;
            }
        });
        let mut change = None;
        for ms in samples.iter().take(count).copied() {
            change = self.feed(ms, now).or(change);
        }
        change
    }

    /// Whether every timer slot is still waiting for its result, meaning the
    /// GPU is [`strobe_gfx::timing::SLOTS`] frames behind. A frame with
    /// nothing to pace it (a hidden output has no swap) is skipped while this
    /// holds. Always false without a timer.
    pub fn backlogged(&self) -> bool {
        self.timer
            .as_ref()
            .is_some_and(|timer| timer.in_flight() >= strobe_gfx::timing::SLOTS)
    }

    /// Takes one GPU time sample at `now`.
    fn feed(&mut self, ms: f32, now: Instant) -> Option<ScaleChange> {
        self.gpu_ms = Some(match self.gpu_ms {
            Some(average) => average + (ms - average) * DISPLAY_SMOOTHING,
            None => ms,
        });
        if !self.adapting()
            || self.pinned.is_some()
            || self.hold_until.is_some_and(|until| now < until)
        {
            return None;
        }
        let from = self.adaptive.scale();
        let before = self.adaptive.futile();
        let to = self.adaptive.observe(ms)?;
        let after = self.adaptive.futile();
        Some(ScaleChange {
            from,
            to,
            gpu_ms: ms,
            budget_ms: self.adaptive.budget_ms(),
            futile: after.filter(|_| after != before),
        })
    }

    /// Frees the GL queries. The context must be current.
    pub fn release(&mut self) {
        self.timer = None;
    }
}

/// A controller at `max`, never below the UI's lowest scale.
fn controller(budget_ms: f32, max: f32) -> AdaptiveScale {
    AdaptiveScale::new(budget_ms, *session::RENDER_SCALE.start(), max)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A controller that behaves as if the GPU could be timed: tests have
    /// no GL context, so they drive `feed` directly.
    fn measuring(max: f32) -> Performance {
        let mut perf = Performance::new(None, max, true, FALLBACK_BUDGET_MS);
        perf.measurable = true;
        perf
    }

    /// Feeds `count` frames of `ms` GPU time at `now`; the last change.
    fn frames(perf: &mut Performance, ms: f32, now: Instant, count: u32) -> Option<ScaleChange> {
        let mut change = None;
        for _ in 0..count {
            change = perf.feed(ms, now).or(change);
        }
        change
    }

    #[test]
    fn the_budget_is_one_refresh_period() {
        assert!(
            (budget_for(Some(60_000)) - 16.666_666).abs() < 1e-3,
            "60 Hz"
        );
        assert!(
            (budget_for(Some(144_000)) - 6.944_444).abs() < 1e-3,
            "144 Hz"
        );
        assert!(
            (budget_for(None) - FALLBACK_BUDGET_MS).abs() < 1e-6,
            "unknown"
        );
        assert!(
            (budget_for(Some(0)) - FALLBACK_BUDGET_MS).abs() < 1e-6,
            "zero"
        );
    }

    #[test]
    fn without_a_timer_the_scale_is_the_sliders() {
        let mut perf = Performance::new(None, 0.8, true, FALLBACK_BUDGET_MS);
        assert!(!perf.adapting(), "nothing to measure with");
        assert_eq!(
            frames(&mut perf, 40.0, Instant::now(), 100),
            None,
            "no change"
        );
        assert!(
            (perf.scale() - 0.8).abs() < 1e-6,
            "slider decides: {}",
            perf.scale()
        );
    }

    #[test]
    fn the_display_average_tracks_samples() {
        let mut perf = Performance::new(None, 1.0, false, FALLBACK_BUDGET_MS);
        assert!(perf.gpu_ms().is_none(), "nothing yet");
        perf.feed(10.0, Instant::now());
        perf.feed(20.0, Instant::now());
        let shown = perf.gpu_ms().unwrap_or_default();
        assert!(shown > 10.0 && shown < 20.0, "smoothed: {shown}");
    }

    #[test]
    fn set_max_rebuilds_the_controller_at_the_new_ceiling() {
        let mut perf = measuring(1.0);
        perf.set_max(0.5);
        assert!((perf.max() - 0.5).abs() < 1e-6, "ceiling");
        assert!(
            (perf.adaptive.scale() - 0.5).abs() < 1e-6,
            "controller starts at it"
        );
    }

    #[test]
    fn a_heavy_scene_drops_the_scale_and_the_change_says_why() {
        let mut perf = measuring(1.0);
        let change = frames(&mut perf, 40.0, Instant::now(), 30).unwrap_or(ScaleChange {
            from: 1.0,
            to: 1.0,
            gpu_ms: 0.0,
            budget_ms: 0.0,
            futile: None,
        });
        assert!(change.to < change.from, "scale must drop: {change:?}");
        assert!(
            (change.gpu_ms - 40.0).abs() < 1e-6,
            "the sample that tipped it"
        );
        assert!(
            (change.budget_ms - FALLBACK_BUDGET_MS).abs() < 1e-3,
            "budget"
        );
        assert!((perf.scale() - change.to).abs() < 1e-6, "scale in force");
    }

    #[test]
    fn nothing_changes_in_the_first_second_after_a_scene_load() {
        let mut perf = measuring(1.0);
        let loaded = Instant::now();
        perf.scene_loaded(loaded, None);
        assert_eq!(
            frames(&mut perf, 40.0, loaded + Duration::from_millis(900), 60),
            None,
            "held"
        );
        assert!((perf.scale() - 1.0).abs() < 1e-6, "scale untouched");
        let after = loaded + HOLD + Duration::from_millis(10);
        assert!(
            frames(&mut perf, 40.0, after, 30).is_some(),
            "adapts once the hold is over"
        );
    }

    #[test]
    fn turning_adaptation_off_returns_the_scale_to_the_slider() {
        let mut perf = measuring(0.9);
        frames(&mut perf, 40.0, Instant::now(), 30);
        assert!(perf.scale() < 0.9, "dropped: {}", perf.scale());
        perf.set_enabled(false);
        assert!((perf.scale() - 0.9).abs() < 1e-6, "manual again");
        perf.set_enabled(true);
        assert!(
            (perf.scale() - 0.9).abs() < 1e-6,
            "a fresh start at the ceiling"
        );
    }

    #[test]
    fn a_refresh_rate_change_moves_the_budget() {
        let mut perf = Performance::new(None, 1.0, true, FALLBACK_BUDGET_MS);
        assert!(!perf.set_refresh(Some(60_000)), "same budget, no change");
        assert!(perf.set_refresh(Some(144_000)), "a faster display");
        assert!(
            (perf.budget_ms() - 6.944_444).abs() < 1e-3,
            "budget {}",
            perf.budget_ms()
        );
    }

    #[test]
    fn a_scene_that_pins_its_final_pass_stays_at_the_ceiling() {
        let mut perf = measuring(1.0);
        frames(&mut perf, 40.0, Instant::now(), 30);
        assert!(
            perf.scale() < 1.0,
            "the last scene dropped: {}",
            perf.scale()
        );
        let loaded = Instant::now();
        perf.scene_loaded(loaded, Some([1920, 1080]));
        assert!(
            (perf.scale() - 1.0).abs() < 1e-6,
            "back to the ceiling: {}",
            perf.scale()
        );
        let after = loaded + HOLD + Duration::from_millis(10);
        assert_eq!(
            frames(&mut perf, 40.0, after, 600),
            None,
            "and no drop however long it runs over"
        );
        assert!((perf.scale() - 1.0).abs() < 1e-6, "still 1.0");
        assert_eq!(perf.pinned(), Some([1920, 1080]), "the size is remembered");
        perf.scene_loaded(after, None);
        assert!(
            frames(&mut perf, 40.0, after + HOLD, 30).is_some(),
            "the next scene adapts again"
        );
    }

    #[test]
    fn a_drop_that_cuts_nothing_is_undone_and_says_so() {
        let mut perf = measuring(1.0);
        let now = Instant::now();
        let mut changes = Vec::new();
        for _ in 0..600 {
            changes.extend(perf.feed(40.0, now));
        }
        assert_eq!(changes.len(), 2, "one drop, one undo: {changes:?}");
        let undo = changes.last().copied();
        assert!(
            undo.is_some_and(|c| c.futile.is_some() && c.to > c.from),
            "the second change goes back up and says why: {undo:?}"
        );
        assert!((perf.scale() - 1.0).abs() < 1e-6, "ends at 1.0");
    }
}
