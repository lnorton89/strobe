// ============================================================================
// Module:       strobe_gfx::adaptive
// Description:  Adaptive render scale: lowers resolution when the GPU misses
//               its frame budget and raises it back when there is headroom.
//
// Dependencies: None; pure arithmetic over measured frame times.
// ============================================================================

//! Adaptive resolution.
//!
//! The main lever on an old laptop is pixel count: every scene's cost is
//! roughly proportional to the pixels it shades. A show cannot drop
//! frames while someone notices and drags a slider, so the scale follows
//! the measured GPU time instead.
//!
//! ## The rules
//!
//! - **Measure GPU time, not frame time.** With vsync on, CPU frame time
//!   reads 16.7 ms whether the GPU took 4 ms or 16; only a timer query
//!   says how close to the edge a scene really is. The caller passes
//!   whatever it measured.
//! - **Cost scales with area.** Halving the scale quarters the pixels, so
//!   a frame at 1.5× budget needs the scale multiplied by `sqrt(1/1.5)`,
//!   not divided by 1.5. Going down jumps straight to the estimated right
//!   scale in one step: a projector showing judder for a second while a
//!   controller creeps toward the answer is the failure being prevented.
//! - **Going up is slow and stepwise**, and only after sustained headroom.
//!   Overshooting upward re-introduces the judder the last drop removed.
//! - **Scales are quantised** to [`STEP`], because every change
//!   reallocates the scene's render targets (and restarts feedback
//!   buffers from black), and a scale that wanders by 0.003 a frame would
//!   do that every frame.
//! - **A cooldown follows every change**, so the next decision is made on
//!   frames rendered at the new size.
//! - **A drop must pay for itself (D280).** Not every scene's cost is its
//!   pixels: a literal 1920x1080 pass stays 1920x1080 at any scale (D194).
//!   After a drop, once the cooldown has passed, the new GPU time is
//!   compared with the old. A halving of the scale must cut it by at least
//!   [`MIN_CUT_PER_HALVING`] (15%; less for a smaller drop). If it did not,
//!   the resolution is not what this scene costs: the scale goes back to
//!   where it was, lowering is locked there, and [`AdaptiveScale::futile`]
//!   says what was tried. The lock lasts until the scene changes
//!   ([`AdaptiveScale::reset_measurements`]); raising is still allowed.

/// Scale changes are multiples of this.
pub const STEP: f32 = 0.05;

/// Smoothing for the frame-time average: the weight of the newest sample.
const EMA_WEIGHT: f32 = 0.1;

/// Over budget by more than this fraction triggers a drop.
const DROP_THRESHOLD: f32 = 1.05;

/// Under this fraction of budget counts as headroom.
const RAISE_THRESHOLD: f32 = 0.70;

/// Frames to wait after a change before deciding again.
const COOLDOWN_FRAMES: u32 = 30;

/// Consecutive frames over budget before a drop: long enough to ignore a
/// single hitch (a shader warming up, a texture upload), short enough
/// that a genuinely heavy scene is fixed within a quarter second at 60 Hz.
const DROP_AFTER: u32 = 12;

/// Consecutive frames of headroom before one step up: two seconds at 60 Hz.
const RAISE_AFTER: u32 = 120;

/// The least a halving of the scale must cut the GPU time by for lowering it
/// to count as working. Area-proportional cost cuts it by 75%; a scene whose
/// passes do not follow the scale cuts less, and below this it is not worth
/// the lost sharpness.
pub const MIN_CUT_PER_HALVING: f32 = 0.15;

/// A drop is judged once it spans this many halvings of the scale (about
/// 0.84x). A smaller one cuts too little to tell from noise, so the judging
/// waits for the next drop, measured from the same starting point.
const MIN_JUDGED_HALVINGS: f32 = 0.25;

/// A lowering that did not cut the GPU time, as [`AdaptiveScale::futile`]
/// reports it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Futile {
    /// The scale that was lowered from, and is held at.
    pub scale: f32,
    /// The scale that was tried.
    pub tried: f32,
    /// Smoothed GPU milliseconds before the drop.
    pub before_ms: f32,
    /// Smoothed GPU milliseconds after it.
    pub after_ms: f32,
}

/// The controller.
#[derive(Debug, Clone, PartialEq)]
pub struct AdaptiveScale {
    budget_ms: f32,
    min: f32,
    max: f32,
    scale: f32,
    average_ms: Option<f32>,
    over: u32,
    under: u32,
    cooldown: u32,
    /// Where lowering started and what it cost there, until a drop of at
    /// least [`MIN_JUDGED_HALVINGS`] has been judged.
    anchor: Option<(f32, f32)>,
    /// A drop that did not pay: lowering stops at `scale`.
    futile: Option<Futile>,
}

impl AdaptiveScale {
    /// A controller holding GPU time under `budget_ms`, with the scale kept
    /// in `min..=max` and starting at `max`.
    pub fn new(budget_ms: f32, min: f32, max: f32) -> Self {
        let min = quantise(min.clamp(STEP, 1.0));
        let max = quantise(max.clamp(min, 1.0));
        Self {
            budget_ms: budget_ms.max(0.1),
            min,
            max,
            scale: max,
            average_ms: None,
            over: 0,
            under: 0,
            cooldown: 0,
            anchor: None,
            futile: None,
        }
    }

    /// The drop that did not cut the GPU time, if there was one since the
    /// last scene change. Lowering is locked at its `scale` meanwhile.
    pub fn futile(&self) -> Option<Futile> {
        self.futile
    }

    /// The current render scale.
    pub fn scale(&self) -> f32 {
        self.scale
    }

    /// Starts from `scale` (kept in `min..=max`) instead of the ceiling.
    /// For resuming after the GL context was rebuilt: the old scale is what
    /// the GPU could carry, and going back to the ceiling would reload the
    /// load that may have caused the loss.
    pub fn set_scale(&mut self, scale: f32) {
        self.scale = quantise(scale.clamp(self.min, self.max));
        self.forget();
        self.futile = None;
        self.cooldown = 0;
    }

    /// Changes the budget, e.g. when the output moves to a display with a
    /// different refresh rate.
    pub fn set_budget(&mut self, budget_ms: f32) {
        self.budget_ms = budget_ms.max(0.1);
    }

    /// The budget in force, in milliseconds.
    pub fn budget_ms(&self) -> f32 {
        self.budget_ms
    }

    /// Changes the ceiling (the person's manual scale). A current scale
    /// above it comes down at once; one below it stays and may creep up.
    /// Returns the new scale if it had to change.
    pub fn set_max(&mut self, max: f32) -> Option<f32> {
        self.max = quantise(max.clamp(self.min, 1.0));
        if self.scale > self.max {
            self.scale = self.max;
            self.forget();
            return Some(self.scale);
        }
        None
    }

    /// Forgets what was measured, keeping the scale. For a moment when the
    /// old measurements stop describing the load, such as a new scene. A new
    /// scene may depend on resolution where the last did not, so a lock on
    /// lowering ([`Self::futile`]) is lifted too.
    pub fn reset_measurements(&mut self) {
        self.forget();
        self.futile = None;
        self.cooldown = 0;
    }

    fn forget(&mut self) {
        self.average_ms = None;
        self.anchor = None;
        self.over = 0;
        self.under = 0;
    }

    /// Feeds one frame's measured GPU time. Returns the new scale when it
    /// changed, so the caller resizes only then.
    pub fn observe(&mut self, gpu_ms: f32) -> Option<f32> {
        if !gpu_ms.is_finite() || gpu_ms < 0.0 {
            return None;
        }
        let average = match self.average_ms {
            Some(average) => average + (gpu_ms - average) * EMA_WEIGHT,
            None => gpu_ms,
        };
        self.average_ms = Some(average);

        if self.cooldown > 0 {
            self.cooldown -= 1;
            return None;
        }

        if let Some(back) = self.judge_drop(average) {
            return Some(back);
        }

        let load = average / self.budget_ms;
        if load > DROP_THRESHOLD {
            self.over += 1;
            self.under = 0;
        } else if load < RAISE_THRESHOLD {
            self.under += 1;
            self.over = 0;
        } else {
            self.over = 0;
            self.under = 0;
        }

        let target = if self.over >= DROP_AFTER {
            // Area-proportional: aim a little under budget so one drop is
            // usually enough.
            quantise_down(self.scale * (0.9 / load).sqrt())
        } else if self.under >= RAISE_AFTER {
            quantise(self.scale + STEP)
        } else {
            return None;
        };
        // A drop that did not pay locks lowering at the scale it left.
        let floor = self
            .futile
            .map_or(self.min, |futile| futile.scale.max(self.min));
        let target = target.clamp(floor.min(self.max), self.max);
        if (target - self.scale).abs() < STEP * 0.5 {
            self.over = 0;
            self.under = 0;
            return None;
        }
        if target < self.scale {
            if self.anchor.is_none() {
                self.anchor = Some((self.scale, average));
            }
        } else {
            self.anchor = None;
        }
        self.scale = target;
        self.over = 0;
        self.under = 0;
        self.cooldown = COOLDOWN_FRAMES;
        // The average was measured at the old size; scale it to the new
        // area so the next decision does not act on stale load.
        self.average_ms = None;
        Some(target)
    }

    /// Judges the drops since [`Self::anchor`] once the frames after the
    /// last one have settled: when they cost too little less, goes back to
    /// the anchor's scale and locks lowering there. Returns that scale.
    fn judge_drop(&mut self, average: f32) -> Option<f32> {
        let (from, before_ms) = self.anchor?;
        let halvings = (from / self.scale).log2();
        if halvings < MIN_JUDGED_HALVINGS {
            return None;
        }
        self.anchor = None;
        let needed = 1.0 - (1.0 - MIN_CUT_PER_HALVING).powf(halvings);
        let cut = 1.0 - average / before_ms.max(f32::EPSILON);
        if cut >= needed {
            return None;
        }
        self.futile = Some(Futile {
            scale: from,
            tried: self.scale,
            before_ms,
            after_ms: average,
        });
        self.scale = from;
        self.over = 0;
        self.under = 0;
        self.cooldown = COOLDOWN_FRAMES;
        self.average_ms = None;
        Some(from)
    }
}

/// Rounds to the nearest [`STEP`].
fn quantise(scale: f32) -> f32 {
    (scale / STEP).round() * STEP
}

/// Rounds down to a [`STEP`], so a drop is never rounded back up to the
/// scale that was too slow.
fn quantise_down(scale: f32) -> f32 {
    (scale / STEP + 1e-4).floor() * STEP
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A GPU whose frame time is proportional to pixels shaded.
    fn simulate(controller: &mut AdaptiveScale, full_res_ms: f32, frames: u32) -> f32 {
        for _ in 0..frames {
            let ms = full_res_ms * controller.scale() * controller.scale();
            controller.observe(ms);
        }
        controller.scale()
    }

    /// A GPU whose frame time is `cost(scale)`.
    fn simulate_cost(
        controller: &mut AdaptiveScale,
        cost: impl Fn(f32) -> f32,
        frames: u32,
    ) -> f32 {
        for _ in 0..frames {
            controller.observe(cost(controller.scale()));
        }
        controller.scale()
    }

    #[test]
    fn a_resumed_controller_starts_where_it_was_told_within_its_range() {
        let mut c = AdaptiveScale::new(16.0, 0.25, 1.0);
        c.set_scale(0.5);
        assert_eq!(c.scale(), 0.5, "resumed");
        c.set_scale(0.05);
        assert_eq!(c.scale(), 0.25, "never below the minimum");
        c.set_scale(3.0);
        assert_eq!(c.scale(), 1.0, "never above the ceiling");
    }

    #[test]
    fn a_light_scene_stays_at_full_resolution() {
        let mut c = AdaptiveScale::new(16.0, 0.25, 1.0);
        assert_eq!(
            simulate(&mut c, 8.0, 600),
            1.0,
            "8 ms of a 16 ms budget needs no change"
        );
    }

    #[test]
    fn a_heavy_scene_drops_quickly_and_settles_under_budget() {
        let mut c = AdaptiveScale::new(16.0, 0.25, 1.0);
        let scale = simulate(&mut c, 40.0, 60);
        let ms = 40.0 * scale * scale;
        assert!(scale < 1.0, "a 2.5x-over scene must drop within one second");
        assert!(
            ms <= 16.0 * DROP_THRESHOLD,
            "and land under budget: {scale} -> {ms} ms"
        );
        let settled = simulate(&mut c, 40.0, 1200);
        assert!(
            (settled - scale).abs() <= STEP + 1e-4,
            "it should not oscillate: {scale} then {settled}"
        );
    }

    #[test]
    fn the_scale_recovers_after_a_heavy_section_ends() {
        let mut c = AdaptiveScale::new(16.0, 0.25, 1.0);
        simulate(&mut c, 40.0, 120);
        assert!(c.scale() < 1.0, "dropped during the heavy section");
        let recovered = simulate(&mut c, 6.0, 60 * 30);
        assert_eq!(
            recovered, 1.0,
            "back to full resolution once the scene is light again"
        );
    }

    #[test]
    fn a_single_hitch_does_not_change_the_scale() {
        let mut c = AdaptiveScale::new(16.0, 0.25, 1.0);
        simulate(&mut c, 8.0, 100);
        assert_eq!(c.observe(200.0), None, "one 200 ms hitch is ignored");
        assert_eq!(
            simulate(&mut c, 8.0, 100),
            1.0,
            "and leaves the scale alone"
        );
    }

    #[test]
    fn the_scale_respects_its_floor_and_rejects_nonsense() {
        let mut c = AdaptiveScale::new(16.0, 0.5, 1.0);
        assert_eq!(simulate(&mut c, 1000.0, 600), 0.5, "never below the floor");
        assert_eq!(c.observe(f32::NAN), None, "NaN is ignored");
        assert_eq!(c.observe(-1.0), None, "negative time is ignored");
    }

    #[test]
    fn lowering_the_ceiling_pulls_the_scale_down_at_once() {
        let mut c = AdaptiveScale::new(16.0, 0.25, 1.0);
        assert_eq!(c.set_max(0.5), Some(0.5), "1.0 comes down to the new max");
        assert_eq!(c.scale(), 0.5, "scale follows");
        assert_eq!(
            c.set_max(0.8),
            None,
            "raising the ceiling changes nothing yet"
        );
        assert_eq!(c.scale(), 0.5, "the scale waits for headroom");
        assert_eq!(
            simulate(&mut c, 4.0, 1500),
            0.8,
            "then creeps up to the new max"
        );
    }

    #[test]
    fn forgetting_measurements_stops_a_stale_average_causing_a_drop() {
        let mut c = AdaptiveScale::new(16.0, 0.25, 1.0);
        for _ in 0..11 {
            c.observe(60.0);
        }
        c.reset_measurements();
        // One more heavy frame would have been the twelfth in a row; after
        // the reset it is the first.
        assert_eq!(c.observe(60.0), None, "no drop on stale evidence");
        assert_eq!(c.scale(), 1.0, "scale untouched");
        assert!((c.budget_ms() - 16.0).abs() < f32::EPSILON, "budget kept");
    }

    #[test]
    fn scales_are_quantised_to_the_step() {
        let mut c = AdaptiveScale::new(16.0, 0.25, 1.0);
        simulate(&mut c, 23.0, 300);
        let steps = c.scale() / STEP;
        assert!(
            (steps - steps.round()).abs() < 1e-3,
            "{} is not a multiple of {STEP}",
            c.scale()
        );
    }
    #[test]
    fn a_scene_whose_cost_ignores_the_scale_is_not_lowered_for_nothing() {
        // D280: passes pinned at 1920x1080 cost the same at every scale.
        let mut c = AdaptiveScale::new(16.0, 0.25, 1.0);
        let mut lowest = 1.0_f32;
        for _ in 0..3000 {
            c.observe(40.0);
            lowest = lowest.min(c.scale());
        }
        assert_eq!(c.scale(), 1.0, "it ends where it started");
        let futile = c.futile();
        assert!(
            futile.is_some_and(|f| f.scale == 1.0 && f.tried < 1.0),
            "and says what it tried: {futile:?}"
        );
        // After the first probe it must never try again: the lowest scale
        // reached is the probe's, once.
        let mut probes = 0;
        for _ in 0..3000 {
            if c.observe(40.0).is_some() {
                probes += 1;
            }
        }
        assert_eq!(probes, 0, "no further changes while the scene stays put");
        assert!(lowest < 1.0, "the probe itself did lower it: {lowest}");
    }

    #[test]
    fn the_futile_drop_is_undone_within_a_second_or_so() {
        let mut c = AdaptiveScale::new(16.0, 0.25, 1.0);
        let mut frames_below = 0;
        for _ in 0..600 {
            c.observe(40.0);
            if c.scale() < 1.0 {
                frames_below += 1;
            }
        }
        assert!(
            frames_below <= COOLDOWN_FRAMES + 2,
            "the probe lasts the cooldown, not longer: {frames_below} frames"
        );
    }

    #[test]
    fn a_mostly_fixed_cost_scene_stops_lowering_when_the_gain_runs_out() {
        // 20 ms that ignore the scale plus 20 ms that follow its area.
        let mut c = AdaptiveScale::new(16.0, 0.25, 1.0);
        let scale = simulate_cost(&mut c, |s| 20.0 + 20.0 * s * s, 3000);
        assert!(scale < 1.0, "the first halvings do pay: {scale}");
        assert!(scale > 0.25, "but it does not run to the floor: {scale}");
        let settled = simulate_cost(&mut c, |s| 20.0 + 20.0 * s * s, 3000);
        assert!(
            (settled - scale).abs() < 1e-4,
            "and it stays put: {scale} then {settled}"
        );
    }

    #[test]
    fn a_drop_that_cuts_by_the_required_share_is_kept() {
        // Exactly 20% off for a halving of the scale: above the 15% line.
        let mut c = AdaptiveScale::new(16.0, 0.25, 1.0);
        let cost = |s: f32| 40.0 * (1.0 - 0.2 * (1.0 - s) / 0.5);
        let scale = simulate_cost(&mut c, cost, 120);
        assert!(scale < 1.0, "a drop that pays stays dropped: {scale}");
        assert!(c.futile().is_none(), "and is not called futile");
    }

    #[test]
    fn a_new_scene_may_lower_again() {
        let mut c = AdaptiveScale::new(16.0, 0.25, 1.0);
        simulate_cost(&mut c, |_| 40.0, 300);
        assert!(c.futile().is_some(), "locked on the first scene");
        c.reset_measurements();
        assert!(c.futile().is_none(), "a scene change lifts the lock");
        let scale = simulate(&mut c, 40.0, 120);
        assert!(scale < 1.0, "and a scene that scales drops: {scale}");
    }

    #[test]
    fn a_locked_scene_still_rises_when_it_gets_light() {
        let mut c = AdaptiveScale::new(16.0, 0.25, 1.0);
        c.set_scale(0.8);
        simulate_cost(&mut c, |_| 40.0, 300);
        assert!(c.futile().is_some(), "locked at 0.8");
        assert_eq!(
            simulate_cost(&mut c, |_| 4.0, 1500),
            1.0,
            "headroom still raises the scale"
        );
    }
}
