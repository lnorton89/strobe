// ============================================================================
// Module:       strobe_gfx::adaptive
// Description:  Adaptive render scale: lowers resolution when the GPU misses
//               its frame budget or the display's refreshes are being missed,
//               and raises it back when there is headroom.
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
//! - **Missed refreshes count, not only the mean (D314).** A scene whose
//!   mean GPU time sits under the budget can still miss vertical blank on
//!   enough frames (GPU spikes, present and compositor overhead) that the
//!   audience sees 42-45 fps, a mix of one- and two-period intervals; the
//!   mini PC did this for minutes with the mean rule satisfied (M16). So
//!   the caller also reports each presented frame's interval
//!   ([`AdaptiveScale::presented`]). Over a window of [`LATE_WINDOW`]
//!   frames, the refreshes that passed (the window's span over the display
//!   cadence) minus the frames presented are the refreshes missed (D315:
//!   the measured intervals are not quantised to the refresh, so counting
//!   long intervals undercounts; the span is exact). [`LATE_WINDOWS_TO_DROP`]
//!   windows in a row with at least [`LATE_PER_WINDOW`] missed each lower
//!   the scale by one step of about 15% of the area, as long as the GPU is
//!   busy enough to be the cause ([`LATE_MIN_LOAD`] of the cadence), or a
//!   late step on this scene has already paid (D317: then the misses are
//!   known to follow the scale, and the next step is judged like the last).
//! - **A late drop must pay for itself too (D315, D316).** The misses may
//!   not be the GPU's. After the drop's cooldown, [`JUDGE_WINDOWS`] windows
//!   are counted. The drop stands when the misses fell under the drop line,
//!   or fell by at least [`LATE_MIN_CUT`] of what they were and by at least
//!   [`LATE_MIN_CUT_POINTS`] of the frames; otherwise it is undone and the
//!   late rule may not lower below that scale again for [`LATE_LOCK_FRAMES`]
//!   (60 s), doubling on each failed retry up to [`MAX_LATE_LOCK_FRAMES`]
//!   (4 min), or until the scene changes ([`AdaptiveScale::late_floor`],
//!   D318). A verdict whose windows were disturbed (a frame over
//!   [`HITCH_CADENCES`] cadences, or the GPU time up by [`GPU_RISE`] after a
//!   step down) is put off for fresh windows, up to [`MAX_DEFERRALS`] times,
//!   rather than locked on (D318: M21's first launch locked for the night on
//!   a window disturbed by the previous process's teardown). The mini PC's M18 hunted 0.65-0.80
//!   without a verdict; M19 showed D315's halving test was too strict: a
//!   0.10 step that pays cuts the misses by 35-40%, and it takes two.
//!   [`MissCount`] is the one count of missed refreshes, shared with the
//!   per-minute perf line so the two always agree.
//! - **Settled, the next step needs stronger evidence (D319).** Once a late
//!   step has paid, or after [`STEADY_WINDOWS`] windows (30 s) at a scale,
//!   two windows at the drop line are not enough: at a steady 2.7% missed,
//!   two 120-frame windows reach 5 misses each by chance about one time in
//!   twenty, so a held scene crept down a step (M23). The misses over the
//!   last [`RECENT_WINDOWS`] windows (up to 10 s) must then average
//!   [`SETTLED_FACTOR`] times the drop line (6.25%). A real change (14-40%
//!   missed) clears that within two windows.
//! - **Climbing back after a late drop is slow (D314, D315).** The GPU time
//!   the frames were late at is remembered, and the scale climbs back only
//!   to a size predicted to cost clearly less than that
//!   ([`RECLIMB_MARGIN`]), and only once [`CLEAR_WINDOWS_TO_RAISE`] windows
//!   in a row have missed fewer than [`LATE_CLEAR_PER_WINDOW`] refreshes
//!   (30 s at 60 Hz), so neither a steady scene nor one whose misses are
//!   not its pixels climbs back into the judder, while a scene that gets
//!   cheaper still does.
//!   The cadence is the display's period but never faster than 60 Hz
//!   ([`FASTEST_CADENCE_MS`]), so a 144 Hz monitor's two-period frames are
//!   not late. With no known display period (an unknown refresh rate, a
//!   hidden output with nothing presented) the rule is off and only the
//!   mean decides, as before.

/// Scale changes are multiples of this.
pub const STEP: f32 = 0.05;

/// Smoothing for the frame-time average: the weight of the newest sample.
const EMA_WEIGHT: f32 = 0.1;

/// The most one sample may enter the average as, in multiples of the
/// larger of the average and the budget.
const HITCH_CAP: f32 = 2.0;

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

/// The longest one presented interval counts as, in display cadences: a
/// stall (a shader compile, a window drag) is one event, so it counts as two
/// missed refreshes at most, not as a window's worth.
pub const STALL_CADENCES: f32 = 3.0;

/// The fastest cadence the late rule holds the output to: 60 Hz. A faster
/// display's period is not the target, so a 144 Hz monitor showing a 10 ms
/// scene every second refresh (13.9 ms) is not missing anything.
pub const FASTEST_CADENCE_MS: f32 = 1000.0 / 60.0;

/// Presented frames the late share is counted over: two seconds at 60 Hz.
pub const LATE_WINDOW: u32 = 120;

/// Refreshes missed in one window that count it as missing: 5 of 120,
/// about 4%, or two or three visible judders a second (57.5 fps). M16
/// measured 7 to 10% late frames; the shipped scenes that held 60 fps on
/// the mini PC ran 0 to 1.3%.
pub const LATE_PER_WINDOW: u32 = 5;

/// A window missing fewer refreshes than this counts as clear: 1 of 120 at
/// most, 59.5 fps or better. M18's clean scene (16) ran at 0.2%.
pub const LATE_CLEAR_PER_WINDOW: u32 = 2;

/// Clear windows in a row before a late drop may be climbed back from:
/// 30 seconds at 60 Hz. The misses come and go with the music, and a climb
/// that brings them back costs another four seconds of judder and a drop;
/// half a minute of clean output is evidence, two seconds is not.
pub const CLEAR_WINDOWS_TO_RAISE: u32 = 15;

/// Windows counted after a late drop's cooldown to judge it: four seconds
/// at 60 Hz, as many as triggered it.
pub const JUDGE_WINDOWS: u32 = 2;

/// The least relative cut in the share of refreshes missed for a late drop
/// to pay (D316): a fifth of what triggered it. A 0.10 step removes about a
/// fifth of the pixels, and where the GPU is the cause it cut the misses by
/// 35-40% on the mini PC (M19: 40 -> 26% and 23 -> 14% of frames); a step
/// that does under half of that is mostly not the GPU. At 40% over 240
/// frames the count's own noise is about 4 points, a tenth of it, so a fifth
/// is two of those. D315 asked for a halving, which undid steps that paid.
pub const LATE_MIN_CUT: f32 = 0.2;

/// The least absolute cut, in share of frames, for a late drop to pay
/// (D316): 5 points, about 3 fps at 60 Hz. Each step costs a fifth of the
/// pixels, and less than that is not worth the sharpness; it is also about
/// twice the count's noise at the shares where the returns run out (15% of
/// 240 frames is 36 misses, give or take 6). On the old 13 it stops the
/// late rule at 0.80 (15% -> 10.5% at 0.70 is 4.5 points) where M18 had
/// hunted 0.65-0.80 for 2 fps.
pub const LATE_MIN_CUT_POINTS: f32 = 0.05;

/// How long a late step that did not pay stops the late rule at its scale,
/// the first time: 60 s at 60 Hz (D318). A retry that fails again costs
/// about 8 s (four of judging at the lower size, and two resizes, which
/// restart feedback scenes from black); once a minute that is about an
/// eighth of the time at the worst, and it falls fast with the backoff. A
/// false lock (M21 run A) heals within a minute instead of lasting as long as
/// the scene is held, which in a show can be the whole night.
pub const LATE_LOCK_FRAMES: u32 = 3600;

/// The longest a late lock lasts after repeated failures: four minutes at
/// 60 Hz (60, 120, 240 s). A scene whose misses truly ignore its scale then
/// pays about 8 s in 240, 3% of the time, for the chance to heal.
pub const MAX_LATE_LOCK_FRAMES: u32 = 4 * LATE_LOCK_FRAMES;

/// A window with any interval over this many cadences (100 ms at 60 Hz) is
/// disturbed: a stall that is not the scene's (a process exiting, a driver
/// tearing down a context, a compile), and no evidence for or against a
/// step (D318). M21 run A's first minute had a 142 ms gap.
pub const HITCH_CADENCES: f32 = 6.0;

/// A step down whose judged windows show the GPU time above this many times
/// what it was before the step was disturbed (D318): a smaller picture
/// cannot cost more, so something else is loading the GPU. M21 run A: 14.9
/// ms where 13 takes 13.6.
pub const GPU_RISE: f32 = 1.05;

/// Times a verdict on disturbed windows is put off for fresh ones before it
/// is taken anyway (D318): sixteen more seconds at most, enough to outlast a
/// teardown like M21 run A's, after which a machine that is always disturbed
/// still gets a verdict (and the lock's retries) rather than a step held
/// open. Waiting costs no judder, only the lower scale's sharpness for the
/// while.
pub const MAX_DEFERRALS: u32 = 4;

/// Missing windows in a row before a drop: four seconds of judder at 60 Hz.
/// One burst (a heavy beat, a transition, a hitch) fills one window, not two.
pub const LATE_WINDOWS_TO_DROP: u32 = 2;

/// Windows at one scale after which it counts as settled (D319): 30 s at
/// 60 Hz, the same half minute the climb back waits for.
pub const STEADY_WINDOWS: u32 = 15;

/// Windows a settled scale's misses are averaged over (D319): 600 frames,
/// 10 s at 60 Hz.
pub const RECENT_WINDOWS: usize = 5;

/// How far over the drop line a settled scale's misses must average before
/// it steps (D319): 1.5 times, 6.25% of the frames, 37.5 misses in 600.
/// At M23's steady 2.7% the 600 frames hold 16 misses, give or take 4: the
/// line is five of those away, about 2 in a million a check by chance (1 in
/// 500 over a 30-minute hold), against D314's one in twenty. The shares a
/// real change brought (14-40%, M18-M23) clear it within two windows:
/// 14% is 17 a window, so two windows on top of three quiet ones are 40.
/// A share between the line and this (4.2-6.25%, 56-57.5 fps) is left
/// alone once settled.
pub const SETTLED_FACTOR: f32 = 1.5;

/// The least GPU load, as a fraction of the cadence, at which late frames
/// are blamed on the GPU. Below it the misses come from somewhere the
/// render scale cannot reach (the CPU, the compositor, another window), and
/// lowering would cost sharpness for nothing; it also bounds how far the
/// late rule alone can take a scene. Once a late step on the scene has paid
/// it no longer applies (D317): the verdict is better evidence than the
/// load. M20 sat at 0.80 for three to four minutes, 52 fps and 14% missed,
/// because 13's 9.1 ms there was under this line until a loud passage
/// lifted the average past it; the step it then took paid (14% -> 2.7%).
pub const LATE_MIN_LOAD: f32 = 0.6;

/// The area one late drop keeps. Each drop is one small step (1.0 -> 0.90,
/// 0.90 -> 0.80 after quantising down), because the late share does not say
/// how far over the edge the frames are, only that they are.
const LATE_AREA: f32 = 0.85;

/// After a late drop, raising is allowed only to a scale whose predicted
/// GPU time (area-proportional, which over-predicts for a scene with fixed
/// costs) is under this fraction of the GPU time the frames were late at.
/// Without it a steady scene climbs back into the same judder two seconds
/// after leaving it.
pub const RECLIMB_MARGIN: f32 = 0.85;

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

/// A late drop that did not cut the misses, as [`AdaptiveScale::late_undone`]
/// reports it (D315).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LateFutile {
    /// The scale that was lowered from, and went back to: the late rule
    /// lowers no further than this for `lock_frames` or until the scene
    /// changes.
    pub scale: f32,
    /// The scale that was tried.
    pub tried: f32,
    /// Share of refreshes missed over the windows that triggered the drop.
    pub share_before: f32,
    /// Share missed over the judged windows after it.
    pub share_after: f32,
    /// Frames the late rule stays stopped at `scale` for (D318).
    pub lock_frames: u32,
}

/// The windows after a late drop, collected to judge it.
#[derive(Debug, Clone, Copy, PartialEq)]
struct LateProbe {
    /// The scale the drop left.
    from: f32,
    /// Share of refreshes missed that triggered it.
    share_before: f32,
    /// Refreshes missed and frames presented since its cooldown.
    missed: u32,
    frames: u32,
    windows: u32,
    /// The smoothed GPU time before the step, and the GPU time summed over
    /// the judged windows (D318).
    before_ms: f32,
    gpu_sum: f32,
    gpu_frames: u32,
    /// A judged window held a frame over [`HITCH_CADENCES`] (D318).
    disturbed: bool,
    /// Verdicts put off so far (D318).
    deferrals: u32,
}

/// The cadence presented frames are judged against for a display `period_ms`:
/// that period, never faster than [`FASTEST_CADENCE_MS`].
pub fn cadence_for(period_ms: f32) -> f32 {
    period_ms.max(FASTEST_CADENCE_MS)
}

/// Refreshes missed over a run of presented frames (D315, D316): the
/// refreshes that passed (each interval over the cadence, capped at
/// [`STALL_CADENCES`]) minus the frames shown. A timestamp that lands late
/// on one frame lands early on the next, so jitter cancels over the run,
/// where counting intervals over a threshold would not. The controller's
/// windows and the per-minute perf line both count with this, so the log
/// and the scale changes say the same thing.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct MissCount {
    frames: u32,
    refreshes: f32,
    worst_ms: f32,
}

impl MissCount {
    /// Adds one presented frame that came `interval_ms` after the last, on
    /// a display judged at `cadence_ms`.
    pub fn add(&mut self, interval_ms: f32, cadence_ms: f32) {
        if !interval_ms.is_finite() || interval_ms < 0.0 || cadence_ms <= 0.0 {
            return;
        }
        self.frames += 1;
        self.refreshes += interval_ms.min(cadence_ms * STALL_CADENCES) / cadence_ms;
        self.worst_ms = self.worst_ms.max(interval_ms);
    }

    /// The longest interval counted, uncapped.
    pub fn worst_ms(&self) -> f32 {
        self.worst_ms
    }

    /// Frames counted.
    pub fn frames(&self) -> u32 {
        self.frames
    }

    /// Refreshes missed: refreshes that passed minus frames shown, never
    /// below zero.
    pub fn missed(&self) -> u32 {
        (self.refreshes - self.frames as f32).round().max(0.0) as u32
    }

    /// Refreshes missed per frame shown, 0 with no frames.
    pub fn share(&self) -> f32 {
        if self.frames == 0 {
            0.0
        } else {
            self.missed() as f32 / self.frames as f32
        }
    }
}

/// A drop made because presented frames kept missing the display's
/// refresh, as [`AdaptiveScale::missed`] reports it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Missed {
    /// Refreshes missed over the windows that caused the drop.
    pub late: u32,
    /// Frames presented over those windows.
    pub frames: u32,
    /// The display cadence the intervals were judged against.
    pub cadence_ms: f32,
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
    /// The display cadence the late rule judges intervals against, or
    /// `None` when the rule is off.
    cadence_ms: Option<f32>,
    /// The current window's count.
    window: MissCount,
    /// Missing windows in a row, and the refreshes missed and frames over
    /// the last [`LATE_WINDOWS_TO_DROP`] of them (D317: the run as a whole
    /// grew to 99 windows in M20 and was reported as the trigger).
    late_windows: u32,
    late_run: u32,
    late_run_frames: u32,
    /// The last missing window's count, for the run above.
    last_missed: u32,
    last_frames: u32,
    /// A late step on this scene paid: its misses follow the scale, so the
    /// load line no longer gates the next (D317).
    late_proven: bool,
    /// The last [`RECENT_WINDOWS`] windows' misses, a ring, how many are in
    /// it and where the next goes (D319).
    recent: [u32; RECENT_WINDOWS],
    recent_len: usize,
    recent_next: usize,
    /// Windows since the scale last changed (D319).
    steady_windows: u32,
    /// The last late verdict at this scale paid: it is settled (D319).
    late_settled: bool,
    /// Clear windows in a row.
    clear_windows: u32,
    /// The late drop being judged.
    late_probe: Option<LateProbe>,
    /// The late rule lowers no further than this until the lock runs out or
    /// the scene changes.
    late_floor: Option<f32>,
    /// Frames until [`Self::late_floor`] expires, and how long the next
    /// late lock lasts (D318).
    late_lock_left: u32,
    late_next_lock: u32,
    /// The futile late drop the last [`Self::observe`] undid.
    late_undone: Option<LateFutile>,
    /// The smoothed GPU time frames were missing refreshes at, since the
    /// measurements were last forgotten: the ceiling for climbing back.
    late_ms: Option<f32>,
    /// The late drop the last [`Self::observe`] made.
    missed: Option<Missed>,
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
            cadence_ms: None,
            window: MissCount::default(),
            late_windows: 0,
            late_run: 0,
            late_run_frames: 0,
            last_missed: 0,
            last_frames: 0,
            late_proven: false,
            recent: [0; RECENT_WINDOWS],
            recent_len: 0,
            recent_next: 0,
            steady_windows: 0,
            late_settled: false,
            clear_windows: 0,
            late_probe: None,
            late_floor: None,
            late_lock_left: 0,
            late_next_lock: LATE_LOCK_FRAMES,
            late_undone: None,
            late_ms: None,
            missed: None,
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

    /// The late drop the last [`Self::observe`] made, when the scale it
    /// returned is one: presented frames kept missing the refresh.
    pub fn missed(&self) -> Option<Missed> {
        self.missed
    }

    /// The late drop the last [`Self::observe`] undid because the misses did
    /// not fall, when the scale it returned is that undoing (D315).
    pub fn late_undone(&self) -> Option<LateFutile> {
        self.late_undone
    }

    /// The scale the late rule lowers no further than, after a late drop
    /// that did not cut the misses, until its lock runs out or the scene
    /// changes (D315, D318).
    pub fn late_floor(&self) -> Option<f32> {
        self.late_floor
    }

    /// The display's refresh period in milliseconds, or `None` when it is
    /// unknown: the late rule is then off. The cadence judged against is
    /// this period, never faster than [`FASTEST_CADENCE_MS`].
    pub fn set_display_period(&mut self, period_ms: Option<f32>) {
        let cadence = period_ms
            .filter(|ms| ms.is_finite() && *ms > 0.0)
            .map(cadence_for);
        if cadence != self.cadence_ms {
            self.cadence_ms = cadence;
            self.restart_window();
        }
    }

    /// The cadence the late rule judges against, while it is on.
    pub fn cadence_ms(&self) -> Option<f32> {
        self.cadence_ms
    }

    /// Records one presented frame's interval since the one before, in
    /// milliseconds. It only counts; [`Self::observe`] decides. Intervals
    /// inside a cooldown are left out, since the first frames at a new size
    /// pay for its targets.
    pub fn presented(&mut self, interval_ms: f32) {
        let Some(cadence) = self.cadence_ms else {
            return;
        };
        if !interval_ms.is_finite() || interval_ms < 0.0 || self.cooldown > 0 {
            return;
        }
        self.window.add(interval_ms, cadence);
        if self.window.frames() < LATE_WINDOW {
            return;
        }
        let missed = self.window.missed();
        let frames = self.window.frames();
        let hitch = self.window.worst_ms() > cadence * HITCH_CADENCES;
        self.window = MissCount::default();
        if missed >= LATE_PER_WINDOW {
            // The trigger is the last two windows, not the whole run.
            let (before, before_frames) = if self.late_windows > 0 {
                (self.last_missed, self.last_frames)
            } else {
                (0, 0)
            };
            self.late_windows += 1;
            self.late_run = before + missed;
            self.late_run_frames = before_frames + frames;
            self.last_missed = missed;
            self.last_frames = frames;
        } else {
            self.late_windows = 0;
            self.late_run = 0;
            self.late_run_frames = 0;
        }
        if let Some(slot) = self.recent.get_mut(self.recent_next) {
            *slot = missed;
        }
        self.recent_next = (self.recent_next + 1) % RECENT_WINDOWS;
        self.recent_len = (self.recent_len + 1).min(RECENT_WINDOWS);
        self.steady_windows = self.steady_windows.saturating_add(1);
        if missed < LATE_CLEAR_PER_WINDOW {
            self.clear_windows = self.clear_windows.saturating_add(1);
        } else {
            self.clear_windows = 0;
        }
        if let Some(probe) = self.late_probe.as_mut()
            && probe.windows < JUDGE_WINDOWS
        {
            probe.missed += missed;
            probe.frames += frames;
            probe.windows += 1;
            probe.disturbed |= hitch;
        }
    }

    /// Starts the late count over, for a change of scale or cadence.
    fn restart_window(&mut self) {
        self.window = MissCount::default();
        self.late_windows = 0;
        self.late_run = 0;
        self.late_run_frames = 0;
        self.last_missed = 0;
        self.last_frames = 0;
        self.clear_windows = 0;
        self.recent = [0; RECENT_WINDOWS];
        self.recent_len = 0;
        self.recent_next = 0;
        self.steady_windows = 0;
        self.late_settled = false;
    }

    /// The evidence for a late step, as (refreshes missed, frames) when it
    /// is enough: two windows in a row at the drop line, or, once the scale
    /// is settled (D319), the recent windows averaging [`SETTLED_FACTOR`]
    /// times it.
    fn late_evidence(&self) -> Option<(u32, u32)> {
        if self.late_settled || self.steady_windows >= STEADY_WINDOWS {
            if self.recent_len < LATE_WINDOWS_TO_DROP as usize {
                return None;
            }
            let missed: u32 = self.recent.iter().take(self.recent_len).sum();
            let windows = self.recent_len as f32;
            let needed = SETTLED_FACTOR * LATE_PER_WINDOW as f32 * windows;
            return (missed as f32 >= needed)
                .then_some((missed, self.recent_len as u32 * LATE_WINDOW));
        }
        (self.late_windows >= LATE_WINDOWS_TO_DROP).then_some((self.late_run, self.late_run_frames))
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
        self.late_ms = None;
        self.late_probe = None;
        self.late_proven = false;
        self.restart_window();
    }

    /// Clears every verdict, for a new scene: it may depend on resolution
    /// where the last did not.
    fn unlock(&mut self) {
        self.strikes = 0;
        self.futile = None;
        self.lock_left = 0;
        self.next_lock = LOCK_FRAMES;
        self.undone = None;
        self.late_floor = None;
        self.late_lock_left = 0;
        self.late_next_lock = LATE_LOCK_FRAMES;
        self.late_undone = None;
    }

    /// Feeds one frame's measured GPU time. Returns the new scale when it
    /// changed, so the caller resizes only then.
    pub fn observe(&mut self, gpu_ms: f32) -> Option<f32> {
        self.undone = None;
        self.missed = None;
        self.late_undone = None;
        if !gpu_ms.is_finite() || gpu_ms < 0.0 {
            return None;
        }
        // One hitch enters the average as at most twice the larger of the
        // average and the budget. Uncapped, a 200 ms frame on a 13.6 ms
        // scene held the average over the drop line for 14 frames, two more
        // than a drop needs (D314). A scene that really got heavier still
        // gets there a few frames later: the cap rises with the average.
        // The first sample after the measurements were forgotten starts
        // the average as it is (a scene load's own hitches are held off by
        // the caller).
        let average = match self.average_ms {
            Some(average) => {
                let sample = gpu_ms.min(HITCH_CAP * average.max(self.budget_ms));
                average + (sample - average) * EMA_WEIGHT
            }
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
        // The GPU time over a late step's judged windows (D318).
        if let Some(probe) = self.late_probe.as_mut()
            && probe.windows < JUDGE_WINDOWS
        {
            probe.gpu_sum += gpu_ms;
            probe.gpu_frames += 1;
        }

        if let Some(back) = self.judge_drop() {
            return Some(back);
        }
        if let Some(back) = self.judge_late_drop() {
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

        // One late drop at a time: the next waits for this one's verdict.
        // A step that paid shows the misses follow the scale (D317).
        // Settled, the evidence must be stronger (D319).
        let evidence = self.late_evidence().filter(|_| {
            self.late_probe.is_none()
                && self
                    .cadence_ms
                    .is_some_and(|cadence| self.late_proven || average >= cadence * LATE_MIN_LOAD)
        });
        let late = evidence.is_some();
        let target = if self.over >= DROP_AFTER {
            // Area-proportional: aim a little under budget so one drop is
            // usually enough.
            quantise_down(self.scale * (0.9 / load).sqrt())
        } else if late {
            // Refreshes are being missed with the mean under budget: one
            // small step, and remember what the GPU took while they were.
            self.late_ms = Some(self.late_ms.map_or(average, |ms| ms.min(average)));
            let (late, frames) = evidence.unwrap_or((self.late_run, self.late_run_frames));
            self.missed = self.cadence_ms.map(|cadence_ms| Missed {
                late,
                frames,
                cadence_ms,
            });
            // A late drop that did not pay stops the late rule at its scale.
            quantise_down(self.scale * LATE_AREA.sqrt()).max(self.late_floor.unwrap_or(0.0))
        } else if self.under >= RAISE_AFTER {
            let up = quantise(self.scale + STEP);
            // Not while refreshes are being missed, and after a late drop
            // only after half a minute without misses, and only to a size
            // predicted to cost clearly less than the GPU time the frames
            // were late at.
            let ratio = up / self.scale.max(STEP);
            let predicted = average * ratio * ratio;
            if self.late_windows > 0
                || self.late_ms.is_some_and(|late_ms| {
                    self.clear_windows < CLEAR_WINDOWS_TO_RAISE
                        || predicted > late_ms * RECLIMB_MARGIN
                })
            {
                self.under = 0;
                return None;
            }
            up
        } else {
            return None;
        };
        // A drop that did not pay locks lowering at the scale it left.
        let floor = self
            .futile
            .map_or(self.min, |futile| futile.scale.max(self.min));
        let target = target.clamp(floor.min(self.max), self.max);
        if (target - self.scale).abs() < STEP * 0.5 {
            // At the floor, the lock or the ceiling: nothing to change. A
            // late step that could not be taken counts its windows afresh;
            // anything else leaves the late count running.
            self.over = 0;
            self.under = 0;
            if self.missed.take().is_some() {
                self.restart_window();
            }
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
        // A late drop is judged on the misses after it; any other change
        // makes a pending verdict meaningless.
        self.late_probe = match self.missed {
            Some(missed) if missed.frames > 0 => Some(LateProbe {
                from: self.scale,
                share_before: missed.late as f32 / missed.frames as f32,
                missed: 0,
                frames: 0,
                windows: 0,
                before_ms: average,
                gpu_sum: 0.0,
                gpu_frames: 0,
                disturbed: false,
                deferrals: 0,
            }),
            Some(_) | None => None,
        };
        self.scale = target;
        self.over = 0;
        self.under = 0;
        self.cooldown = COOLDOWN_FRAMES;
        self.restart_window();
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
        self.late_probe = None;
        self.restart_window();
        self.average_ms = None;
        Some(from)
    }

    /// Judges the last late drop once [`JUDGE_WINDOWS`] windows after its
    /// cooldown are in: unless the misses fell under the drop line, or by
    /// both [`LATE_MIN_CUT`] of what triggered it and [`LATE_MIN_CUT_POINTS`]
    /// of the frames, goes back to the scale it left and stops the late rule
    /// there for the late lock (doubling on each repeat) or until the scene
    /// changes. A futile verdict on disturbed windows is put off for fresh
    /// ones first, up to [`MAX_DEFERRALS`] times. Returns that scale (D315,
    /// D316, D318).
    fn judge_late_drop(&mut self) -> Option<f32> {
        let probe = self.late_probe?;
        if probe.windows < JUDGE_WINDOWS || probe.frames == 0 {
            return None;
        }
        self.late_probe = None;
        let share_after = probe.missed as f32 / probe.frames as f32;
        let under_the_line = share_after * (LATE_WINDOW as f32) < LATE_PER_WINDOW as f32;
        let cut = probe.share_before - share_after;
        let pays = cut >= probe.share_before * LATE_MIN_CUT && cut >= LATE_MIN_CUT_POINTS;
        if under_the_line || pays {
            self.late_proven = true;
            // Settled here until the scale changes (D319).
            self.late_settled = true;
            self.late_next_lock = LATE_LOCK_FRAMES;
            return None;
        }
        // A smaller picture cannot cost more: a GPU time up after the step,
        // or a stall in the judged windows, says something else was going
        // on. Judge on fresh windows instead (D318).
        let gpu_rose = probe.gpu_frames > 0
            && probe.gpu_sum / probe.gpu_frames as f32 > probe.before_ms * GPU_RISE;
        if (probe.disturbed || gpu_rose) && probe.deferrals < MAX_DEFERRALS {
            self.late_probe = Some(LateProbe {
                missed: 0,
                frames: 0,
                windows: 0,
                gpu_sum: 0.0,
                gpu_frames: 0,
                disturbed: false,
                deferrals: probe.deferrals + 1,
                ..probe
            });
            return None;
        }
        let from = probe.from.min(self.max);
        let lock_frames = self.late_next_lock;
        self.late_undone = Some(LateFutile {
            scale: from,
            tried: self.scale,
            share_before: probe.share_before,
            share_after,
            lock_frames,
        });
        self.late_floor = Some(from);
        self.late_lock_left = lock_frames;
        self.late_next_lock = lock_frames.saturating_mul(2).min(MAX_LATE_LOCK_FRAMES);
        // The GPU judge's anchor described the drop just undone.
        self.anchor = None;
        self.probe_ms = 0.0;
        self.probe_frames = 0;
        self.scale = from;
        self.over = 0;
        self.under = 0;
        self.cooldown = COOLDOWN_FRAMES;
        self.restart_window();
        self.average_ms = None;
        Some(from)
    }

    /// Counts a lock down by one frame. When it runs out the scene may be
    /// lowered again, and one more futile probe locks it again.
    fn tick_lock(&mut self) {
        // The late lock runs out too (D318); the next one lasts longer.
        if self.late_floor.is_some() {
            self.late_lock_left = self.late_lock_left.saturating_sub(1);
            if self.late_lock_left == 0 {
                self.late_floor = None;
            }
        }
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

    // ------------------------------------------------- missed refreshes (D314)

    /// One refresh period at 60 Hz.
    const PERIOD: f32 = 1000.0 / 60.0;

    /// A controller on a 60 Hz display, as the app builds one.
    fn sixty_hz() -> AdaptiveScale {
        let mut c = AdaptiveScale::new(PERIOD, 0.25, 1.0);
        c.set_display_period(Some(PERIOD));
        c
    }

    /// What a run of [`present`] saw.
    #[derive(Debug, Default)]
    struct Run {
        /// Scale changes.
        changes: u32,
        /// The frame of the first change, if any.
        first_change: Option<u32>,
        /// The late drop reported with the first change that was one.
        missed: Option<Missed>,
        /// The lowest scale reached.
        lowest: f32,
    }

    /// Feeds `frames` frames presented on a vsynced 60 Hz display. For
    /// frame `i` at scale `s`, `frame(s, i)` gives the GPU time and whether
    /// the frame missed its refresh (an interval of two periods, else one).
    fn present(
        c: &mut AdaptiveScale,
        frames: u32,
        mut frame: impl FnMut(f32, u32) -> (f32, bool),
    ) -> Run {
        let mut run = Run {
            lowest: c.scale(),
            ..Run::default()
        };
        for i in 0..frames {
            let (gpu_ms, late) = frame(c.scale(), i);
            c.presented(if late { 2.0 * PERIOD } else { PERIOD });
            if c.observe(gpu_ms).is_some() {
                run.changes += 1;
                run.first_change = run.first_change.or(Some(i));
                run.missed = run.missed.or(c.missed());
            }
            run.lowest = run.lowest.min(c.scale());
        }
        run
    }

    /// The mini PC's scene 13 (M16): 13.6 ms of GPU at full size, area
    /// proportional, and one frame in eleven (9%) a refresh late while the
    /// scale is above 0.92. A slight wobble keeps the mean honest.
    fn chromatic_rise(s: f32, i: u32) -> (f32, bool) {
        let wobble = 1.0 + 0.05 * ((i % 7) as f32 - 3.0) / 3.0;
        (13.6 * s * s * wobble, s > 0.92 && i.is_multiple_of(11))
    }

    /// A repeatable noise in -1..1 for frame `i`.
    fn noise(i: u32) -> f32 {
        let x = i.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        let x = x ^ (x >> 15);
        let x = x.wrapping_mul(0x2c1b_3c6d);
        let x = x ^ (x >> 12);
        (x >> 8) as f32 / (1_u32 << 24) as f32 * 2.0 - 1.0
    }

    #[test]
    fn missed_refreshes_lower_the_scale_with_the_mean_under_budget() {
        // The M16 pattern: the mean (13.6 of 16.7 ms) never trips the old
        // rule, but 9% of the intervals are 33 ms.
        let mut c = sixty_hz();
        let run = present(&mut c, 600, chromatic_rise);
        assert!(c.scale() < 1.0, "lowered within 10 s: {}", c.scale());
        assert!(
            run.first_change
                .is_some_and(|frame| frame <= 2 * LATE_WINDOW + 10),
            "within the two windows it takes: {run:?}"
        );
        assert!(
            run.missed
                .is_some_and(|m| m.late >= 2 * LATE_PER_WINDOW && m.frames == 2 * LATE_WINDOW),
            "and the change says how many frames were late: {run:?}"
        );
        let settled = c.scale();
        assert!(
            (settled - 0.9).abs() < 1e-4,
            "one step, to where the frames stop missing: {settled}"
        );
        let after = present(&mut c, 60 * 60 * 5, chromatic_rise);
        assert_eq!(
            after.changes, 0,
            "and it stays there for five minutes without climbing back into the judder: {after:?}"
        );
    }

    #[test]
    fn missing_refreshes_lower_step_by_step_while_each_step_helps() {
        // One frame in five late at full size, one in twelve at 0.9 (more
        // than halved, and over the settled line of D319), none at 0.8.
        let mut c = sixty_hz();
        let run = present(&mut c, 3600, |s, i| {
            let late = if s > 0.92 {
                i.is_multiple_of(5)
            } else if s > 0.85 {
                i.is_multiple_of(12)
            } else {
                false
            };
            (13.6 * s * s, late)
        });
        assert!(
            (c.scale() - 0.8).abs() < 1e-4 && run.changes == 2,
            "two steps, 1.0 -> 0.9 -> 0.8, both kept: {run:?}, now {}",
            c.scale()
        );
        assert_eq!(c.late_floor(), None, "nothing undone");
    }

    /// Runs `frames` frames of a scene measured on the mini PC (D316): GPU
    /// `cost(s)` and refreshes missed on `share(s)` of the frames at scale
    /// `s`, spread evenly. Returns every change as (frame, from, to, the
    /// late verdict that undid a drop, if this change is one).
    fn measured(
        c: &mut AdaptiveScale,
        frames: u32,
        cost: impl Fn(f32) -> f32,
        share: impl Fn(f32) -> f32,
    ) -> Vec<(u32, f32, f32, Option<LateFutile>)> {
        let mut owed = 0.0_f32;
        let mut changes = Vec::new();
        for i in 0..frames {
            let s = c.scale();
            owed += share(s);
            let late = owed >= 1.0;
            if late {
                owed -= 1.0;
            }
            c.presented(if late { 2.0 * PERIOD } else { PERIOD });
            if let Some(to) = c.observe(cost(s)) {
                changes.push((i, s, to, c.late_undone()));
            }
        }
        changes
    }

    /// The old 13 on the mini PC, as M18 and M19 measured it: refreshes
    /// missed on about 40% of the frames at 1.0, 25% at 0.90, 15% at 0.80
    /// (52 fps), 10-11% at 0.70 and below (54-55 fps). The GPU time falls
    /// less than the area (M18 run 2 dropped from 0.80 at 11.6 ms).
    fn old_13_share(s: f32) -> f32 {
        if s > 0.95 {
            0.40
        } else if s > 0.85 {
            0.25
        } else if s > 0.75 {
            0.15
        } else if s > 0.675 {
            0.105
        } else {
            0.10
        }
    }

    /// Checks that after `changes[settled_at]`, the undo that set the late
    /// lock at `settled`, every change is a retry (one step down, then its
    /// undoing back to `settled`), that each retry waits out the lock before
    /// it, that the locks double up to the cap, and that the run spends no
    /// more than 5% of its `frames` below `settled` (D318: no hunting).
    fn backs_off(
        changes: &[(u32, f32, f32, Option<LateFutile>)],
        settled_at: usize,
        settled: f32,
        frames: u32,
    ) {
        let mut locked_at = changes.get(settled_at).map(|c| c.0).unwrap_or(0);
        let mut lock = changes
            .get(settled_at)
            .and_then(|c| c.3)
            .map_or(0, |u| u.lock_frames);
        let mut below = 0;
        let mut pairs = changes.iter().skip(settled_at + 1);
        while let Some(down) = pairs.next() {
            assert!(
                down.3.is_none() && (down.1 - settled).abs() < 1e-4 && down.2 < settled,
                "a retry steps down from {settled}: {down:?} in {changes:?}"
            );
            assert!(
                down.0 >= locked_at + lock,
                "only once the {lock}-frame lock has run out: {down:?} in {changes:?}"
            );
            let Some(back) = pairs.next() else {
                below += frames - down.0;
                break;
            };
            let undone = back.3;
            assert!(
                undone.is_some() && (back.2 - settled).abs() < 1e-4,
                "and is undone back to {settled}: {back:?} in {changes:?}"
            );
            let next = undone.map_or(0, |u| u.lock_frames);
            assert!(
                next >= lock && next <= MAX_LATE_LOCK_FRAMES,
                "the lock backs off to the cap: {lock} then {next}"
            );
            below += back.0 - down.0;
            locked_at = back.0;
            lock = next;
        }
        assert!(
            below * 20 <= frames,
            "no more than 5% of the time below {settled}: {below} of {frames} frames"
        );
    }

    #[test]
    fn a_verdict_on_a_disturbed_window_is_put_off_and_a_false_lock_heals() {
        // M21 run A: launched seconds after another strobe exited, 13 ran
        // its first 15 s at 42% missed whatever the scale, with 140 ms
        // stalls and its GPU time up from 13.6 to 14.9 ms. D317 locked at
        // 1.00 for the night; now the verdict waits for fresh windows, and
        // the M20 steps follow.
        let mut c = sixty_hz();
        let mut owed = 0.0_f32;
        let mut changes = Vec::new();
        let disturbed_until = 60 * 15;
        for i in 0..60 * 60 * 3_u32 {
            let s = c.scale();
            let disturbed = i < disturbed_until;
            owed += if disturbed {
                0.42
            } else if s > 0.95 {
                0.404
            } else if s > 0.85 {
                0.267
            } else if s > 0.75 {
                0.14
            } else {
                0.026
            };
            let late = owed >= 1.0;
            if late {
                owed -= 1.0;
            }
            let interval = if disturbed && i % 120 == 60 {
                140.0
            } else if late {
                2.0 * PERIOD
            } else {
                PERIOD
            };
            c.presented(interval);
            let gpu = (1.16 + 12.44 * s * s) * if disturbed { 1.1 } else { 1.0 };
            if let Some(to) = c.observe(gpu) {
                changes.push((i, s, to, c.late_undone()));
            }
        }
        assert!(
            changes.iter().all(|change| change.3.is_none()),
            "nothing locked: {changes:?}"
        );
        assert!(
            changes
                .iter()
                .find(|(_, _, to, _)| *to <= 0.8 + 1e-4)
                .is_some_and(|(frame, ..)| *frame < 60 * 90),
            "0.80 or lower within 90 s: {changes:?}"
        );
        assert!(
            (c.scale() - 0.7).abs() < 1e-4,
            "and on to 0.70: {}",
            c.scale()
        );
    }

    #[test]
    fn a_lock_on_an_undetected_disturbance_heals_within_the_minute() {
        // The same, but the disturbance shows only as misses (no stall, no
        // GPU rise): the first verdict locks, and the lock expires.
        let mut c = sixty_hz();
        let mut owed = 0.0_f32;
        let mut changes = Vec::new();
        for i in 0..60 * 60 * 3_u32 {
            let s = c.scale();
            owed += if i < 60 * 15 {
                0.42
            } else if s > 0.95 {
                0.404
            } else if s > 0.85 {
                0.267
            } else if s > 0.75 {
                0.14
            } else {
                0.026
            };
            let late = owed >= 1.0;
            if late {
                owed -= 1.0;
            }
            c.presented(if late { 2.0 * PERIOD } else { PERIOD });
            if let Some(to) = c.observe(1.16 + 12.44 * s * s) {
                changes.push((i, s, to, c.late_undone()));
            }
        }
        assert!(
            changes.get(1).is_some_and(|change| change.3.is_some()),
            "the first verdict locked: {changes:?}"
        );
        assert!(
            changes
                .iter()
                .find(|(_, _, to, _)| *to <= 0.8 + 1e-4)
                .is_some_and(|(frame, ..)| *frame < 60 * 90),
            "0.80 or lower within 90 s: {changes:?}"
        );
        assert!(
            (c.scale() - 0.7).abs() < 1e-4,
            "ends at 0.70: {}",
            c.scale()
        );
    }

    #[test]
    fn the_old_13_steps_while_each_step_pays_and_stops_at_0_80() {
        // M19 run 2 locked at 1.00 (40% -> 26% failed D315's halving) and ran
        // 42.6 fps; M18 run 2 hunted 0.65-0.80. Two steps pay, the third
        // (15% -> 10.5%, 4.5 points) does not, and it holds at 0.80.
        let mut c = sixty_hz();
        let frames = 60 * 60 * 10;
        let changes = measured(&mut c, frames, |s| 6.0 + 7.8 * s * s, old_13_share);
        let scales: Vec<f32> = changes.iter().map(|(_, _, to, _)| *to).collect();
        assert!(
            changes.len() >= 4
                && (scales[0] - 0.9).abs() < 1e-4
                && (scales[1] - 0.8).abs() < 1e-4
                && (scales[2] - 0.7).abs() < 1e-4
                && (scales[3] - 0.8).abs() < 1e-4
                && changes[3].3.is_some()
                && changes[3].0 < 60 * 30,
            "1.00 -> 0.90 -> 0.80 and a try at 0.70 undone, in the first 30 s: {changes:?}"
        );
        // After that only the lock's retries (D318), backing off.
        backs_off(&changes, 3, 0.8, frames);
        assert!((c.scale() - 0.8).abs() < 1e-4, "holds 0.80: {}", c.scale());
    }

    #[test]
    fn the_third_step_follows_a_paid_step_without_waiting_for_the_load_line() {
        // M20 run 2 on the mini PC: 40% of frames missed at 1.00, 26.7% at
        // 0.90, 14% at 0.80 (52.6 fps), 2.6% at 0.70 (58.5 fps); GPU 13.6,
        // 11.2, 9.1 and 7.3 ms. At 0.80 the GPU is under the 10 ms load line,
        // and the step to 0.70 waited three to four minutes for a loud
        // passage. Once a step has paid, the next follows its verdict.
        let mut c = sixty_hz();
        let mut owed = 0.0_f32;
        let mut changes = Vec::new();
        for i in 0..60 * 60 * 10_u32 {
            let s = c.scale();
            owed += if s > 0.95 {
                0.404
            } else if s > 0.85 {
                0.267
            } else if s > 0.75 {
                0.14
            } else {
                0.026
            };
            let late = owed >= 1.0;
            if late {
                owed -= 1.0;
            }
            c.presented(if late { 2.0 * PERIOD } else { PERIOD });
            if let Some(to) = c.observe(1.16 + 12.44 * s * s) {
                changes.push((i, s, to, c.missed(), c.late_undone()));
            }
        }
        assert!(
            changes.len() == 3 && changes.iter().all(|change| change.4.is_none()),
            "three steps, none undone: {changes:?}"
        );
        assert!(
            changes
                .last()
                .is_some_and(|(frame, _, to, ..)| (*to - 0.7).abs() < 1e-4 && *frame < 60 * 30),
            "0.70 within 30 s: {changes:?}"
        );
        assert!(
            changes.iter().all(|(.., missed, _)| missed
                .is_some_and(|m| m.frames == LATE_WINDOWS_TO_DROP * LATE_WINDOW)),
            "each step reports the two windows that triggered it: {changes:?}"
        );
        assert!(
            (c.scale() - 0.7).abs() < 1e-4,
            "and holds 0.70: {}",
            c.scale()
        );
    }

    /// A repeatable pseudo-random stream in 0..1, seeded (D319).
    struct Chance(u64);

    impl Chance {
        fn next(&mut self) -> f32 {
            self.0 = self
                .0
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (self.0 >> 40) as f32 / (1_u64 << 24) as f32
        }
    }

    /// Takes the old 13 down to 0.70 as M20 run 2 did, then runs `frames`
    /// more frames with each frame missing a refresh at random with chance
    /// `share(frame)`. Returns the changes after 0.70 was reached.
    fn held_at_0_70(
        c: &mut AdaptiveScale,
        seed: u64,
        frames: u32,
        share: impl Fn(u32) -> f32,
    ) -> Vec<(u32, f32, f32)> {
        let mut owed = 0.0_f32;
        for _ in 0..60 * 40 {
            let s = c.scale();
            owed += if s > 0.95 {
                0.404
            } else if s > 0.85 {
                0.267
            } else if s > 0.75 {
                0.14
            } else {
                0.026
            };
            let late = owed >= 1.0;
            if late {
                owed -= 1.0;
            }
            c.presented(if late { 2.0 * PERIOD } else { PERIOD });
            c.observe(1.16 + 12.44 * s * s);
        }
        let mut chance = Chance(seed);
        let mut changes = Vec::new();
        for i in 0..frames {
            let s = c.scale();
            let late = chance.next() < share(i);
            c.presented(if late { 2.0 * PERIOD } else { PERIOD });
            if let Some(to) = c.observe(1.16 + 12.44 * s * s) {
                changes.push((i, s, to));
            }
        }
        changes
    }

    #[test]
    fn a_settled_scale_does_not_creep_down_on_noise() {
        // M23 run B: two minutes steady at 0.70 with 2.6-2.7% missed, then
        // a step to 0.60 on 13 misses in 240 frames, which chance gives two
        // windows in a row about one time in twenty. Thirty minutes at 2.7%,
        // drawn at random, five seeds: no step.
        for seed in [1, 2, 3, 2026, 1017] {
            let mut c = sixty_hz();
            let changes = held_at_0_70(&mut c, seed, 60 * 60 * 30, |_| 0.027);
            assert!(
                (c.scale() - 0.7).abs() < 1e-4 && changes.is_empty(),
                "seed {seed}: no step from 0.70 in 30 minutes: {changes:?}"
            );
        }
    }

    #[test]
    fn a_settled_scale_still_steps_for_a_real_change() {
        // Two minutes settled at 0.70, then the scene gets heavier (or
        // another app loads the GPU): 14% missed, as at 0.80 before.
        let settled = 60 * 120;
        for seed in [1, 2, 3] {
            let mut c = sixty_hz();
            let changes = held_at_0_70(&mut c, seed, settled + 60 * 30, |i| {
                if i < settled { 0.027 } else { 0.14 }
            });
            assert!(
                changes
                    .first()
                    .is_some_and(|(frame, from, to)| *frame >= settled
                        && *frame < settled + 60 * 10
                        && (*from - 0.7).abs() < 1e-4
                        && *to < 0.7),
                "seed {seed}: a step from 0.70 within 10 s of the change: {changes:?}"
            );
        }
    }

    #[test]
    fn a_long_run_of_missing_windows_reports_only_the_recent_ones() {
        // The load line holds the step (8 ms), and 15% of frames miss for a
        // minute: the trigger reported is the recent windows, not thirty
        // (D317, D319).
        let mut c = sixty_hz();
        let mut owed = 0.0_f32;
        for _ in 0..3600 {
            owed += 0.15;
            let late = owed >= 1.0;
            if late {
                owed -= 1.0;
            }
            c.presented(if late { 2.0 * PERIOD } else { PERIOD });
            assert_eq!(c.observe(8.0), None, "under the load line, no step");
        }
        // Over the line now: the step reports the last two windows.
        let mut reported = None;
        for _ in 0..600 {
            owed += 0.15;
            let late = owed >= 1.0;
            if late {
                owed -= 1.0;
            }
            c.presented(if late { 2.0 * PERIOD } else { PERIOD });
            if c.observe(12.0).is_some() {
                reported = reported.or(c.missed());
            }
        }
        assert!(
            reported.is_some_and(|m| m.frames == RECENT_WINDOWS as u32 * LATE_WINDOW
                && m.late >= 75
                && m.late <= 105),
            "the recent windows at about 15%, not the minute's run (D319: a minute at one \
             scale is settled, so the recent windows are the evidence): {reported:?}"
        );
    }

    #[test]
    fn a_scene_whose_misses_clear_by_0_80_steps_twice_and_holds() {
        // 16-acid-mirror (M18 run 5, M19 run 5): 23% at 1.0, 14% at 0.90
        // (D315 undid this step: 59.4 fps became 49.2), none at 0.80. GPU
        // 12.0 ms at 1.0 and 10.3 at 0.90, as logged.
        let mut c = sixty_hz();
        let changes = measured(
            &mut c,
            60 * 60 * 10,
            |s| 3.0 + 9.0 * s * s,
            |s| {
                if s > 0.95 {
                    0.23
                } else if s > 0.85 {
                    0.14
                } else {
                    0.002
                }
            },
        );
        assert!(
            changes.len() == 2 && changes.iter().all(|(.., undone)| undone.is_none()),
            "1.00 -> 0.90 -> 0.80, both kept: {changes:?}"
        );
        assert!((c.scale() - 0.8).abs() < 1e-4, "holds 0.80: {}", c.scale());
        assert_eq!(c.late_floor(), None, "nothing undone");
    }

    #[test]
    fn a_step_that_cuts_too_little_is_undone_by_either_test() {
        // 30% -> 26%: 4 points and 13%, under both lines.
        let mut c = sixty_hz();
        let changes = measured(
            &mut c,
            3600,
            |s| 13.6 * s * s,
            |s| if s > 0.95 { 0.30 } else { 0.26 },
        );
        assert!(
            changes.len() == 2 && changes[1].3.is_some() && c.late_floor() == Some(1.0),
            "one try, undone, and stopped at 1.00: {changes:?}"
        );
        // 8% -> 6%: a quarter, but 2 points.
        let mut c = sixty_hz();
        let changes = measured(
            &mut c,
            3600,
            |s| 13.6 * s * s,
            |s| if s > 0.95 { 0.08 } else { 0.06 },
        );
        assert!(
            changes.len() == 2 && changes[1].3.is_some(),
            "a small share's small cut is not worth the pixels: {changes:?}"
        );
    }

    #[test]
    fn the_miss_count_is_refreshes_passed_minus_frames_shown() {
        let mut count = MissCount::default();
        for _ in 0..90 {
            count.add(PERIOD, PERIOD);
        }
        for _ in 0..10 {
            count.add(2.0 * PERIOD, PERIOD);
        }
        assert_eq!(count.frames(), 100, "frames");
        assert_eq!(count.missed(), 10, "one per double interval");
        assert!(
            (count.share() - 0.1).abs() < 1e-6,
            "share {}",
            count.share()
        );
        count.add(1000.0, PERIOD);
        assert_eq!(count.missed(), 12, "a stall counts two at most");
        count.add(f32::NAN, PERIOD);
        assert_eq!(count.frames(), 101, "nonsense is ignored");
        assert!(
            (cadence_for(1000.0 / 144.0) - FASTEST_CADENCE_MS).abs() < 1e-4,
            "144 Hz is judged at 60"
        );
    }

    #[test]
    fn misses_that_do_not_follow_the_scale_stop_the_late_rule() {
        // M18 run 2 on the mini PC: 13 at 13.6 ms, a tenth of the frames
        // late at full size, and about 4% late at every scale below it while
        // the GPU time keeps falling. Without the verdict it hunted between
        // 0.65 and 0.80 for minutes.
        let mut c = sixty_hz();
        let mut changes = Vec::new();
        for i in 0..60 * 60 * 10_u32 {
            let s = c.scale();
            let late = if s > 0.92 {
                i.is_multiple_of(10)
            } else {
                i.is_multiple_of(23)
            };
            c.presented(if late { 2.0 * PERIOD } else { PERIOD });
            if let Some(to) = c.observe(13.6 * s * s) {
                changes.push((i, s, to, c.late_undone()));
            }
        }
        assert!(
            changes.len() == 1 && (changes[0].2 - 0.9).abs() < 1e-4,
            "one step, and none from 0.9: 4.3% is over the drop line but under the settled \
             one (D319), so 0.8 is not even tried: {changes:?}"
        );
        assert!((c.scale() - 0.9).abs() < 1e-4, "ends at 0.9: {}", c.scale());
    }

    #[test]
    fn a_late_drop_is_not_climbed_back_from_until_the_misses_have_stayed_away() {
        // M18 run 2's other half: the mean rule climbed straight back from a
        // late drop. Here the cost halves right after the drop and nothing is
        // late: the climb waits for half a minute of clean windows.
        let mut c = sixty_hz();
        let first = present(&mut c, 600, chromatic_rise);
        assert!((c.scale() - 0.9).abs() < 1e-4, "dropped: {}", c.scale());
        let dropped_at = first.first_change.unwrap_or(0);
        let cheap = present(&mut c, 60 * 60, |s, _| (6.8 * s * s, false));
        let climbed_after = cheap.first_change.map(|frame| 600 - dropped_at + frame);
        assert!(
            climbed_after.is_some_and(|frames| frames >= CLEAR_WINDOWS_TO_RAISE * LATE_WINDOW),
            "it climbs, but only after half a minute of clean windows: {climbed_after:?} \
             frames after the drop"
        );
    }

    #[test]
    fn intervals_that_are_not_quantised_still_count_as_missed_refreshes() {
        // M18 run 3: 52 fps on a 60 Hz display with almost no interval over
        // 25 ms, because the timestamps are taken on the CPU. 19.2 ms every
        // frame is 13% of the refreshes missed, whatever the threshold.
        let mut c = sixty_hz();
        let mut changes = 0;
        for _ in 0..600 {
            c.presented(19.2);
            if c.observe(12.0 * c.scale() * c.scale()).is_some() {
                changes += 1;
            }
        }
        assert!(changes > 0, "the shortfall is seen");
        // And a frame that only jittered (20 ms, then 13.4) misses nothing.
        let mut c = sixty_hz();
        for i in 0..1200_u32 {
            c.presented(if i.is_multiple_of(2) { 20.0 } else { 13.33 });
            assert_eq!(c.observe(12.0), None, "jitter is not a miss ({i})");
        }
    }

    #[test]
    fn a_late_scene_under_the_raise_line_at_full_size_is_still_lowered() {
        // 11 ms is 0.66 of the budget: the raise rule keeps asking to go
        // above the ceiling, which must not keep restarting the late count.
        // Half a window of GPU samples first, so the two counts are out of
        // step, as the timer's lag makes them in the app.
        let mut c = sixty_hz();
        for _ in 0..LATE_WINDOW / 2 {
            c.observe(11.0);
        }
        let run = present(&mut c, 600, |s, i| {
            (11.0 * s * s, s > 0.92 && i.is_multiple_of(11))
        });
        assert!(
            (c.scale() - 0.9).abs() < 1e-4,
            "one step down: {run:?}, now {}",
            c.scale()
        );
    }

    #[test]
    fn a_scene_whose_misses_and_cost_ignore_the_scale_ends_where_it_started() {
        // Frames late whatever the scale and a cost that ignores it: the
        // first late drop is judged on the misses and undone, and the late
        // rule stops at full size.
        let mut c = sixty_hz();
        let frames = 60 * 60 * 10;
        let changes = measured(&mut c, frames, |_| 13.6, |_| 1.0 / 11.0);
        assert_eq!(c.scale(), 1.0, "back where it started: {changes:?}");
        assert!(
            changes.len() >= 2 && changes[1].3.is_some(),
            "one probe, one undo: {changes:?}"
        );
        // Retried rarely: after 60, 120 and 240 s locks, about 8 changes
        // in ten minutes, never more than one step down (D318).
        backs_off(&changes, 1, 1.0, frames);
        assert!(changes.len() <= 10, "rare retries: {changes:?}");
        assert_eq!(c.late_floor(), Some(1.0), "and the late rule is stopped");
        c.reset_measurements();
        assert_eq!(c.late_floor(), None, "until the scene changes");
    }

    #[test]
    fn a_steady_ten_ms_scene_stays_at_full_resolution() {
        let mut c = sixty_hz();
        let run = present(&mut c, 60 * 60 * 5, |_, _| (10.0, false));
        assert_eq!(run.changes, 0, "no change: {run:?}");
        assert_eq!(c.scale(), 1.0, "full resolution");
    }

    #[test]
    fn a_cheap_scene_is_not_lowered_for_late_frames_it_does_not_cause() {
        // 8 ms of a 16.7 ms cadence, and 20% of the frames late anyway:
        // something else is missing the refresh, not this GPU.
        let mut c = sixty_hz();
        let run = present(&mut c, 60 * 60, |_, i| (8.0, i.is_multiple_of(5)));
        assert_eq!(run.changes, 0, "no change: {run:?}");
        assert_eq!(c.scale(), 1.0, "full resolution");
    }

    #[test]
    fn a_stall_counts_as_two_missed_refreshes_at_most() {
        // A 400 ms stall would be 23 refreshes; it is one event.
        let mut c = sixty_hz();
        let mut changes = 0;
        for i in 0..1200_u32 {
            c.presented(if i % 120 == 60 { 400.0 } else { PERIOD });
            if c.observe(13.6).is_some() {
                changes += 1;
            }
        }
        assert_eq!(changes, 0, "a stall every two seconds is not a trend");
    }

    #[test]
    fn an_isolated_hitch_does_not_change_the_scale() {
        // The worst case for the mean: a scene already at 13.6 ms.
        let mut c = sixty_hz();
        let mut run = present(&mut c, 600, |_, _| (13.6, false));
        c.presented(200.0);
        if c.observe(200.0).is_some() {
            run.changes += 1;
        }
        let after = present(&mut c, 600, |_, _| (13.6, false));
        assert_eq!(
            run.changes + after.changes,
            0,
            "one 200 ms frame changes nothing: {run:?} {after:?}"
        );
        assert_eq!(c.scale(), 1.0, "full resolution");
    }

    #[test]
    fn a_short_burst_of_late_frames_does_not_change_the_scale() {
        // Thirty late frames in a row (a heavy second, a transition) fill
        // one window, not two.
        let mut c = sixty_hz();
        let run = present(&mut c, 1200, |_, i| (13.6, (300..330).contains(&i)));
        assert_eq!(run.changes, 0, "a burst is not a trend: {run:?}");
    }

    #[test]
    fn a_lowered_scene_whose_cost_drops_climbs_back() {
        let mut c = sixty_hz();
        present(&mut c, 1200, chromatic_rise);
        assert!(c.scale() < 1.0, "lowered first: {}", c.scale());
        // A quiet passage: half the cost, nothing late. Each step back up
        // waits for half a minute of clean windows (D315).
        let run = present(&mut c, 60 * 75, |s, _| (6.8 * s * s, false));
        assert_eq!(c.scale(), 1.0, "back to full resolution: {run:?}");
    }

    #[test]
    fn a_borderline_scene_does_not_oscillate() {
        // 12.4 ms with a 20% spread, late when the GPU time plus 2 ms of
        // present overhead crosses the period: about 4% late at full size,
        // the drop line, and none at 0.9.
        let mut c = sixty_hz();
        let run = present(&mut c, 60 * 60 * 10, |s, i| {
            let ms = 12.4 * s * s * (1.0 + 0.2 * noise(i));
            (ms, ms + 2.0 > PERIOD)
        });
        assert!(
            run.changes <= 1,
            "at most one drop in ten minutes, never back and forth: {run:?}"
        );
        assert!(run.lowest >= 0.9 - 1e-4, "and not far: {run:?}");
    }

    #[test]
    fn two_period_frames_on_a_144_hz_display_are_not_late() {
        // The budget is the 144 Hz period; the late rule's cadence is held
        // at 60 Hz, so a frame shown every second refresh (13.9 ms) is fine.
        let period = 1000.0 / 144.0;
        let mut c = AdaptiveScale::new(period, 0.25, 1.0);
        c.set_display_period(Some(period));
        assert!(
            c.cadence_ms()
                .is_some_and(|ms| (ms - FASTEST_CADENCE_MS).abs() < 1e-4),
            "cadence held at 60 Hz: {:?}",
            c.cadence_ms()
        );
        let mut changes = 0;
        for _ in 0..3600 {
            c.presented(2.0 * period);
            if c.observe(6.5).is_some() {
                changes += 1;
            }
        }
        assert_eq!(changes, 0, "no change");
        assert_eq!(c.scale(), 1.0, "full resolution");
    }

    #[test]
    fn with_no_display_period_late_frames_are_ignored() {
        // An unknown refresh rate: the mean alone decides, as before D314.
        let mut c = AdaptiveScale::new(PERIOD, 0.25, 1.0);
        let run = present(&mut c, 3600, chromatic_rise);
        assert_eq!(run.changes, 0, "no change: {run:?}");
        c.set_display_period(Some(PERIOD));
        c.set_display_period(None);
        assert_eq!(c.cadence_ms(), None, "and the rule can be switched off");
    }

    #[test]
    fn a_new_scene_forgets_the_late_ceiling() {
        let mut c = sixty_hz();
        present(&mut c, 1200, chromatic_rise);
        assert!(c.scale() < 1.0, "lowered: {}", c.scale());
        c.reset_measurements();
        // The same cost with nothing late, which the first scene's ceiling
        // held at 0.9: as a new scene it climbs on ordinary headroom (to
        // 0.95, where the mean is no longer under the raise line).
        let run = present(&mut c, 600, |s, _| (13.6 * s * s, false));
        assert!(
            run.changes > 0 && c.scale() > 0.9,
            "climbs once the ceiling is forgotten: {run:?}, now {}",
            c.scale()
        );
    }
}
