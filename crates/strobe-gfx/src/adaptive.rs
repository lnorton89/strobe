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
//!   After a drop the GPU time at the new size is measured over the
//!   cooldown, leaving out its first [`SETTLE_FRAMES`] (the timer's queue
//!   still holds frames drawn before the drop, and the first frames at a
//!   new size pay for its targets), and compared with the smoothed time
//!   before. A halving of the scale must cut it by at least
//!   [`MIN_CUT_PER_HALVING`] (15%; less for a smaller drop). If it did not,
//!   the scale goes back to where it was ([`AdaptiveScale::undone`] says
//!   what was tried). One such verdict could be the music, not the scene,
//!   so lowering is locked only after [`STRIKES_TO_LOCK`] in a row
//!   ([`AdaptiveScale::futile`]); a drop that pays in between clears them.
//!   The lock expires after [`LOCK_FRAMES`] frames, so a scene whose cost
//!   changes, or a verdict that was wrong, gets another try; one more futile
//!   probe then locks it again for twice as long, up to
//!   [`MAX_LOCK_FRAMES`]. A scene change lifts it
//!   ([`AdaptiveScale::reset_measurements`]); raising is always allowed.

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

/// Frames at the start of a drop's cooldown left out of judging it: the
/// GPU timer's queue (up to four frames) still holds frames drawn at the
/// old size, and the first frames at the new one pay for its new targets.
/// The rest of the cooldown, twenty frames, is what the drop is judged on.
pub const SETTLE_FRAMES: u32 = 10;

/// Futile verdicts in a row before lowering is locked. The first one only
/// goes back up: GPU time follows the music too, and a heavy passage that
/// starts just after a drop can hide what the drop saved.
pub const STRIKES_TO_LOCK: u32 = 2;

/// Frames a lock on lowering lasts the first time: 30 seconds at 60 Hz.
/// Each re-lock doubles it.
pub const LOCK_FRAMES: u32 = 1800;

/// The longest a lock lasts: four minutes at 60 Hz.
pub const MAX_LOCK_FRAMES: u32 = 8 * LOCK_FRAMES;

/// A lowering that did not cut the GPU time, as [`AdaptiveScale::undone`]
/// and [`AdaptiveScale::futile`] report it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Futile {
    /// The scale that was lowered from, and went back to.
    pub scale: f32,
    /// The scale that was tried.
    pub tried: f32,
    /// Smoothed GPU milliseconds before the drop.
    pub before_ms: f32,
    /// Mean GPU milliseconds after it, settled frames left out.
    pub after_ms: f32,
    /// Frames lowering is locked at `scale` for, or `None` when this was
    /// the first strike and the controller may try again.
    pub lock_frames: Option<u32>,
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
    /// GPU milliseconds summed over the judged frames of the last drop's
    /// cooldown, and how many there were.
    probe_ms: f32,
    probe_frames: u32,
    /// Futile verdicts in a row since the scene changed.
    strikes: u32,
    /// A drop that did not pay, twice: lowering stops at its `scale`.
    futile: Option<Futile>,
    /// Frames until [`Self::futile`] expires.
    lock_left: u32,
    /// How long the next lock lasts.
    next_lock: u32,
    /// The futile drop the last [`Self::observe`] undid.
    undone: Option<Futile>,
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
            probe_ms: 0.0,
            probe_frames: 0,
            strikes: 0,
            futile: None,
            lock_left: 0,
            next_lock: LOCK_FRAMES,
            undone: None,
        }
    }

    /// The lock on lowering, while there is one: the drop that did not cut
    /// the GPU time [`STRIKES_TO_LOCK`] times in a row. Lowering stops at
    /// its `scale` until it expires or the scene changes.
    pub fn futile(&self) -> Option<Futile> {
        self.futile
    }

    /// The futile drop the last [`Self::observe`] undid, when the scale it
    /// returned is that undoing.
    pub fn undone(&self) -> Option<Futile> {
        self.undone
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
        self.unlock();
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
        self.unlock();
        self.cooldown = 0;
    }

    fn forget(&mut self) {
        self.average_ms = None;
        self.anchor = None;
        self.probe_ms = 0.0;
        self.probe_frames = 0;
        self.over = 0;
        self.under = 0;
    }

    /// Clears every verdict, for a new scene: it may depend on resolution
    /// where the last did not.
    fn unlock(&mut self) {
        self.strikes = 0;
        self.futile = None;
        self.lock_left = 0;
        self.next_lock = LOCK_FRAMES;
        self.undone = None;
    }

    /// Feeds one frame's measured GPU time. Returns the new scale when it
    /// changed, so the caller resizes only then.
    pub fn observe(&mut self, gpu_ms: f32) -> Option<f32> {
        self.undone = None;
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
            // The judged part of a drop's cooldown: settled frames only.
            if self.anchor.is_some() && self.cooldown < COOLDOWN_FRAMES - SETTLE_FRAMES {
                self.probe_ms += gpu_ms;
                self.probe_frames += 1;
            }
            return None;
        }

        if let Some(back) = self.judge_drop() {
            return Some(back);
        }
        self.tick_lock();

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
        self.probe_ms = 0.0;
        self.probe_frames = 0;
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
    /// the anchor's scale, and on the [`STRIKES_TO_LOCK`]th verdict in a row
    /// locks lowering there. Returns that scale.
    fn judge_drop(&mut self) -> Option<f32> {
        let (from, before_ms) = self.anchor?;
        let halvings = (from / self.scale).log2();
        if halvings < MIN_JUDGED_HALVINGS || self.probe_frames == 0 {
            return None;
        }
        self.anchor = None;
        let after_ms = self.probe_ms / self.probe_frames as f32;
        self.probe_ms = 0.0;
        self.probe_frames = 0;
        let needed = 1.0 - (1.0 - MIN_CUT_PER_HALVING).powf(halvings);
        let cut = 1.0 - after_ms / before_ms.max(f32::EPSILON);
        if cut >= needed {
            self.strikes = 0;
            return None;
        }
        self.strikes += 1;
        let lock_frames = (self.strikes >= STRIKES_TO_LOCK).then_some(self.next_lock);
        let futile = Futile {
            scale: from,
            tried: self.scale,
            before_ms,
            after_ms,
            lock_frames,
        };
        if let Some(frames) = lock_frames {
            self.futile = Some(futile);
            self.lock_left = frames;
            self.next_lock = self.next_lock.saturating_mul(2).min(MAX_LOCK_FRAMES);
        }
        self.undone = Some(futile);
        self.scale = from;
        self.over = 0;
        self.under = 0;
        self.cooldown = COOLDOWN_FRAMES;
        self.average_ms = None;
        Some(from)
    }

    /// Counts a lock down by one frame. When it runs out the scene may be
    /// lowered again, and one more futile probe locks it again.
    fn tick_lock(&mut self) {
        if self.futile.is_none() {
            return;
        }
        self.lock_left = self.lock_left.saturating_sub(1);
        if self.lock_left == 0 {
            self.futile = None;
            self.strikes = STRIKES_TO_LOCK - 1;
        }
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
    /// Feeds `frames` frames of `cost(scale)` and counts the changes.
    fn changes(controller: &mut AdaptiveScale, cost: impl Fn(f32) -> f32, frames: u32) -> u32 {
        let mut changes = 0;
        for _ in 0..frames {
            if controller.observe(cost(controller.scale())).is_some() {
                changes += 1;
            }
        }
        changes
    }

    #[test]
    fn a_scene_whose_cost_ignores_the_scale_is_not_lowered_for_nothing() {
        // D280: passes pinned at 1920x1080 cost the same at every scale.
        let mut c = AdaptiveScale::new(16.0, 0.25, 1.0);
        let mut lowest = 1.0_f32;
        for _ in 0..600 {
            c.observe(40.0);
            lowest = lowest.min(c.scale());
        }
        assert!(lowest < 1.0, "the probe itself did lower it: {lowest}");
        assert_eq!(c.scale(), 1.0, "it ends where it started");
        let futile = c.futile();
        assert!(
            futile.is_some_and(|f| f.scale == 1.0
                && f.tried < 1.0
                && f.lock_frames == Some(LOCK_FRAMES)),
            "two strikes lock it, and it says what it tried: {futile:?}"
        );
        // While the lock lasts it never tries again.
        assert_eq!(
            changes(&mut c, |_| 40.0, LOCK_FRAMES - 600),
            0,
            "no further changes while the lock lasts"
        );
        // Then one more probe (a drop and its undoing), and a lock twice as
        // long.
        assert_eq!(
            changes(&mut c, |_| 40.0, 600),
            2,
            "one probe once the lock expires"
        );
        assert!(
            c.futile()
                .is_some_and(|f| f.lock_frames == Some(2 * LOCK_FRAMES)),
            "locked again for twice as long: {:?}",
            c.futile()
        );
        assert_eq!(
            changes(&mut c, |_| 40.0, 2 * LOCK_FRAMES - 600),
            0,
            "and quiet for that long"
        );
    }

    #[test]
    fn each_futile_probe_is_undone_within_a_second_or_so() {
        let mut c = AdaptiveScale::new(16.0, 0.25, 1.0);
        let mut frames_below = 0;
        for _ in 0..600 {
            c.observe(40.0);
            if c.scale() < 1.0 {
                frames_below += 1;
            }
        }
        assert!(
            frames_below <= STRIKES_TO_LOCK * (COOLDOWN_FRAMES + 2),
            "each probe lasts its cooldown, not longer: {frames_below} frames"
        );
    }

    #[test]
    fn one_heavy_passage_after_a_drop_does_not_lock_a_scene_that_scales() {
        // Area-proportional cost, but the music gets 2.5 times heavier for
        // the 45 frames after the first drop, hiding what it saved.
        let mut c = AdaptiveScale::new(16.0, 0.25, 1.0);
        let mut burst = 0_u32;
        let mut undone = 0;
        for _ in 0..1200 {
            let busy = if burst > 0 { 2.5 } else { 1.0 };
            burst = burst.saturating_sub(1);
            let changed = c.observe(40.0 * c.scale() * c.scale() * busy);
            if c.undone().is_some() {
                undone += 1;
            }
            if changed.is_some() && undone == 0 && c.scale() < 1.0 {
                burst = 45;
            }
        }
        assert_eq!(undone, 1, "the hidden saving is one strike");
        assert!(c.futile().is_none(), "but not a lock");
        assert!(c.scale() < 0.8, "and the retry lowers it: {}", c.scale());
    }

    #[test]
    fn the_frames_still_in_flight_after_a_drop_are_not_judged() {
        // 30 ms that ignore the scale and 10 that follow its area, so the
        // first drop (1.0 -> 0.6) cuts 16%, against 11% needed. After it
        // the timer still reports four frames at the old size, then four
        // that pay for the new targets at three times the cost. Judged on
        // all of them, the drop looks futile.
        let cost = |s: f32| 30.0 + 10.0 * s * s;
        let mut c = AdaptiveScale::new(16.0, 0.25, 1.0);
        let mut old_ms = cost(1.0);
        let mut since = u32::MAX;
        let mut lowered = None;
        for _ in 0..(DROP_AFTER + COOLDOWN_FRAMES + 10) {
            let new_ms = cost(c.scale());
            let ms = match since {
                0..4 => old_ms,
                4..8 => 3.0 * new_ms,
                _ => new_ms,
            };
            since = since.saturating_add(1);
            if let Some(scale) = c.observe(ms) {
                assert!(c.undone().is_none(), "the drop was undone at {scale}");
                lowered = lowered.or(Some(scale));
                old_ms = new_ms;
                since = 0;
            }
        }
        assert!(
            lowered.is_some_and(|scale| scale < 1.0) && c.scale() < 1.0,
            "the drop stands: {lowered:?}, now {}",
            c.scale()
        );
    }

    #[test]
    fn a_mostly_fixed_cost_scene_stops_lowering_when_the_gain_runs_out() {
        // 20 ms that ignore the scale plus 20 ms that follow its area.
        let cost = |s: f32| 20.0 + 20.0 * s * s;
        let mut c = AdaptiveScale::new(16.0, 0.25, 1.0);
        let scale = simulate_cost(&mut c, cost, 1000);
        assert!(scale < 1.0, "the first halvings do pay: {scale}");
        assert!(scale > 0.25, "but it does not run to the floor: {scale}");
        assert!(c.futile().is_some(), "and lowering is locked there");
        let mut frames_below = 0;
        for _ in 0..5000 {
            c.observe(cost(c.scale()));
            assert!(c.scale() <= scale + 1e-4, "it never rises: {}", c.scale());
            if c.scale() < scale - 1e-4 {
                frames_below += 1;
            }
        }
        assert!(
            frames_below <= 2 * (COOLDOWN_FRAMES + 2),
            "only the re-probes as each lock expires go lower: {frames_below} frames"
        );
        assert!(
            (c.scale() - scale).abs() < 1e-4,
            "and it ends where it settled"
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
