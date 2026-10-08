// ============================================================================
// Module:       strobe_audio::tempo
// Description:  Tempo estimation from the onset envelope, tap tempo, manual
//               nudging, and the phase-locked beat grid.
//
// Dependencies: std only
// ============================================================================

//! Tempo: how fast, how sure, and where the beats fall.
//!
//! **Estimation.** The analyser feeds one onset-strength value per FFT hop.
//! Every half second this module autocorrelates the last eight seconds of
//! that envelope. A steady pulse train shows peaks at one beat period and at
//! every multiple of it, so instead of picking the single tallest peak (which
//! can land on any harmonic) each candidate tempo from 50 to 220 BPM is
//! scored by the *comb* of autocorrelation values at its first four
//! multiples. Candidates are checked every quarter BPM, with the lag
//! interpolated linearly, which is what gets sub-hop accuracy out of an
//! envelope sampled at about 94 Hz.
//!
//! **Octave choice.** A pulse train at 87 BPM also scores well as 174 BPM,
//! and the reverse. The score is multiplied by a soft prior centred on
//! 120 BPM (one standard deviation is [`PRIOR_OCTAVES`] octaves), and the
//! autocorrelation is the biased kind, which falls with lag and so favours
//! the shorter period when two candidates are otherwise equal. The result
//! is that a clean track resolves to the tempo a person would tap, in the
//! 80-160 range, and a track near the edges may report double or half time.
//! That ambiguity is inherent; tap tempo and nudge exist to override it.
//!
//! **Confidence** is how far the best comb stands above the average comb
//! (peak sharpness), multiplied by a stability term that rises while
//! successive estimates agree and halves when they do not.

use std::f32::consts::LN_2;

/// Slowest tempo reported.
pub const BPM_MIN: f32 = 50.0;
/// Fastest tempo reported.
pub const BPM_MAX: f32 = 220.0;
/// The tempo reported before anything has been heard.
pub const BPM_DEFAULT: f32 = 120.0;

/// Seconds of onset envelope the autocorrelation sees.
const WINDOW_SECONDS: f32 = 8.0;

/// Seconds of envelope required before the first estimate.
const MIN_SECONDS: f32 = 3.0;

/// Seconds between estimates.
const ESTIMATE_INTERVAL: f32 = 0.5;

/// Spacing of candidate tempos, in BPM.
const BPM_STEP: f32 = 0.25;

/// Autocorrelation multiples that vote for a candidate tempo.
const HARMONICS: usize = 4;

/// Only lags up to this fraction of the available envelope are trusted:
/// beyond it the correlation averages too few pairs.
const USABLE_LAG_FRACTION: f32 = 0.6;

/// Standard deviation of the tempo prior, in octaves around 120 BPM.
const PRIOR_OCTAVES: f32 = 1.0;

/// An estimate whose sharpness is below this is treated as "no tempo here".
const MIN_SHARPNESS: f32 = 0.12;

/// The sharpness that counts as full confidence. A clean pulse train
/// scores about this far above the average comb.
const FULL_SHARPNESS: f32 = 0.45;

/// Estimates closer than this (as a natural-log ratio, about 4%) agree.
const AGREE_TOLERANCE: f32 = 0.04;

/// Fraction of the way a new agreeing estimate moves the tracked tempo.
const TRACK_GAIN: f32 = 0.3;

/// Disagreeing estimates needed in a row before the tempo jumps.
const JUMP_AFTER: u32 = 2;

/// A tempo and how sharply it stood out.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Estimate {
    /// Beats per minute, within [`BPM_MIN`] to [`BPM_MAX`].
    pub(crate) bpm: f32,
    /// How far the winning comb stood above the average, roughly 0 to 1.
    pub(crate) sharpness: f32,
}

/// Autocorrelation tempo estimator over a ring of onset strengths.
#[derive(Debug)]
pub(crate) struct Estimator {
    /// Onset envelope samples per second.
    rate: f32,
    /// The envelope, newest sample at `head - 1`.
    ring: Vec<f32>,
    /// Next write position.
    head: usize,
    /// Valid samples in the ring.
    filled: usize,
    /// The envelope in time order with its mean removed.
    linear: Vec<f32>,
    /// Normalised autocorrelation by lag.
    acf: Vec<f32>,
}

impl Estimator {
    /// An estimator for an envelope sampled `rate` times a second.
    pub(crate) fn new(rate: f32) -> Self {
        let len = (WINDOW_SECONDS * rate).ceil() as usize + 1;
        let slowest_lag = (rate * 60.0 / BPM_MIN).ceil() as usize;
        let acf_len =
            (HARMONICS * slowest_lag + 2).min((USABLE_LAG_FRACTION * len as f32) as usize + 2);
        Self {
            rate,
            ring: vec![0.0; len],
            head: 0,
            filled: 0,
            linear: vec![0.0; len],
            acf: vec![0.0; acf_len],
        }
    }

    /// Records the next onset-strength sample.
    pub(crate) fn push(&mut self, x: f32) {
        if let Some(slot) = self.ring.get_mut(self.head) {
            *slot = x;
        }
        self.head = (self.head + 1) % self.ring.len();
        self.filled = (self.filled + 1).min(self.ring.len());
    }

    /// The best tempo for the envelope so far, or `None` when there is too
    /// little of it or it is flat.
    pub(crate) fn estimate(&mut self) -> Option<Estimate> {
        let n = self.filled;
        if (n as f32) < MIN_SECONDS * self.rate {
            return None;
        }
        let len = self.ring.len();
        let oldest = (self.head + len - n) % len;
        let mut mean = 0.0;
        for i in 0..n {
            mean += self.ring.get((oldest + i) % len).copied().unwrap_or(0.0);
        }
        mean /= n as f32;
        let mut energy = 0.0;
        for i in 0..n {
            let v = self.ring.get((oldest + i) % len).copied().unwrap_or(0.0) - mean;
            if let Some(slot) = self.linear.get_mut(i) {
                *slot = v;
            }
            energy += v * v;
        }
        if energy / (n as f32) < 1e-4 {
            return None;
        }

        let usable =
            ((USABLE_LAG_FRACTION * n as f32) as usize).min(self.acf.len().saturating_sub(1));
        let signal = self.linear.get(..n)?;
        if let Some(first) = self.acf.first_mut() {
            *first = 1.0;
        }
        for lag in 1..=usable {
            let (head, tail) = (signal.get(..n - lag)?, signal.get(lag..)?);
            let sum: f32 = head.iter().zip(tail).map(|(a, b)| a * b).sum();
            if let Some(slot) = self.acf.get_mut(lag) {
                *slot = sum / energy;
            }
        }

        let steps = ((BPM_MAX - BPM_MIN) / BPM_STEP) as usize;
        let mut best = (0.0_f32, BPM_DEFAULT, 0.0_f32);
        let mut total = 0.0_f32;
        for step in 0..=steps {
            let bpm = BPM_MIN + step as f32 * BPM_STEP;
            let score = self.comb(bpm, usable);
            total += score;
            let ranked = score * prior(bpm);
            if ranked > best.0 {
                best = (ranked, bpm, score);
            }
        }
        let average = total / (steps + 1) as f32;
        let sharpness = (best.2 - average).max(0.0);
        Some(Estimate {
            bpm: best.1,
            sharpness,
        })
    }

    /// The mean autocorrelation at the first multiples of `bpm`'s period.
    fn comb(&self, bpm: f32, usable: usize) -> f32 {
        let period = self.rate * 60.0 / bpm;
        let mut sum = 0.0;
        let mut votes = 0.0;
        for m in 1..=HARMONICS {
            let lag = period * m as f32;
            if lag >= usable as f32 {
                break;
            }
            let low = lag.floor() as usize;
            let t = lag - low as f32;
            let a = self.acf.get(low).copied().unwrap_or(0.0);
            let b = self.acf.get(low + 1).copied().unwrap_or(a);
            sum += (a + (b - a) * t).max(0.0);
            votes += 1.0;
        }
        if votes > 0.0 { sum / votes } else { 0.0 }
    }
}

/// The soft preference for tempos near 120 BPM.
fn prior(bpm: f32) -> f32 {
    let octaves = (bpm / BPM_DEFAULT).ln() / LN_2;
    (-0.5 * (octaves / PRIOR_OCTAVES).powi(2)).exp()
}

/// Tap-tempo state: the last few tap times.
#[derive(Clone, Debug, Default)]
pub(crate) struct TapTempo {
    /// Recent tap times in seconds, oldest first.
    taps: Vec<f64>,
}

impl TapTempo {
    /// Taps further apart than this start a new count.
    const RESET_AFTER: f64 = 2.5;
    /// The most taps remembered.
    const MAX_TAPS: usize = 8;

    /// Registers a tap at `now` seconds. With two or more taps in a row,
    /// returns the tempo they imply.
    pub(crate) fn tap(&mut self, now: f64) -> Option<f32> {
        let restart = self
            .taps
            .last()
            .is_some_and(|last| now <= *last || now - *last > Self::RESET_AFTER);
        if restart {
            self.taps.clear();
        }
        if self.taps.len() == Self::MAX_TAPS {
            self.taps.remove(0);
        }
        self.taps.push(now);
        let first = *self.taps.first()?;
        let intervals = (self.taps.len() - 1) as f64;
        if intervals < 1.0 {
            return None;
        }
        let period = (now - first) / intervals;
        Some(((60.0 / period) as f32).clamp(BPM_MIN, BPM_MAX))
    }
}

/// Tempo tracker: turns raw estimates into a stable tempo and confidence,
/// and lets the operator override it.
#[derive(Debug)]
pub(crate) struct Tempo {
    /// The autocorrelation estimator.
    estimator: Estimator,
    /// Onset samples since the last estimate.
    since_estimate: u32,
    /// Onset samples between estimates.
    estimate_every: u32,
    /// The tracked automatic tempo.
    auto_bpm: f32,
    /// Whether any estimate has been accepted.
    have_estimate: bool,
    /// A disagreeing estimate waiting for confirmation, and its count.
    pending: Option<(f32, u32)>,
    /// 0..1, rises while estimates agree.
    stability: f32,
    /// The latest sharpness-derived confidence.
    confidence: f32,
    /// Operator-set tempo, if any.
    manual: Option<f32>,
    /// Tap tempo state.
    taps: TapTempo,
}

impl Tempo {
    /// A tracker for an onset envelope sampled `rate` times a second.
    pub(crate) fn new(rate: f32) -> Self {
        Self {
            estimator: Estimator::new(rate),
            since_estimate: 0,
            estimate_every: (ESTIMATE_INTERVAL * rate).round().max(1.0) as u32,
            auto_bpm: BPM_DEFAULT,
            have_estimate: false,
            pending: None,
            stability: 0.0,
            confidence: 0.0,
            manual: None,
            taps: TapTempo::default(),
        }
    }

    /// Feeds one onset-strength sample.
    pub(crate) fn observe(&mut self, onset: f32) {
        self.estimator.push(onset);
        self.since_estimate += 1;
        if self.since_estimate < self.estimate_every {
            return;
        }
        self.since_estimate = 0;
        match self.estimator.estimate() {
            Some(estimate) if estimate.sharpness >= MIN_SHARPNESS => self.accept(estimate),
            Some(_) | None => {
                self.confidence = 0.0;
                self.stability = 0.0;
            }
        }
    }

    /// Folds in an estimate.
    fn accept(&mut self, estimate: Estimate) {
        self.confidence = (estimate.sharpness / FULL_SHARPNESS).clamp(0.0, 1.0);
        if !self.have_estimate {
            self.auto_bpm = estimate.bpm;
            self.have_estimate = true;
            self.stability = 0.4;
            return;
        }
        let off = (estimate.bpm / self.auto_bpm).ln().abs();
        if off < AGREE_TOLERANCE {
            self.auto_bpm += TRACK_GAIN * (estimate.bpm - self.auto_bpm);
            self.pending = None;
            self.stability += 0.25 * (1.0 - self.stability);
            return;
        }
        self.stability *= 0.5;
        self.pending = match self.pending {
            Some((candidate, count)) if (estimate.bpm / candidate).ln().abs() < AGREE_TOLERANCE => {
                if count + 1 >= JUMP_AFTER {
                    self.auto_bpm = estimate.bpm;
                    None
                } else {
                    Some((candidate, count + 1))
                }
            }
            Some(_) | None => Some((estimate.bpm, 1)),
        };
    }

    /// The tempo to use now: the operator's if set, else the tracked one.
    pub(crate) fn bpm(&self) -> f32 {
        self.manual.unwrap_or(self.auto_bpm)
    }

    /// Confidence 0..1. An operator-set tempo is fully trusted.
    pub(crate) fn confidence(&self) -> f32 {
        if self.manual.is_some() {
            1.0
        } else {
            self.confidence * self.stability
        }
    }

    /// Whether the operator has taken over the tempo.
    pub(crate) fn is_manual(&self) -> bool {
        self.manual.is_some()
    }

    /// Registers a tap; two or more in a row set the tempo.
    pub(crate) fn tap(&mut self, now: f64) {
        if let Some(bpm) = self.taps.tap(now) {
            self.manual = Some(bpm);
        }
    }

    /// Moves the tempo by `delta` BPM and holds it there.
    pub(crate) fn nudge(&mut self, delta: f32) {
        if delta.is_finite() {
            self.manual = Some((self.bpm() + delta).clamp(BPM_MIN, BPM_MAX));
        }
    }

    /// Hands the tempo back to the estimator.
    pub(crate) fn clear_manual(&mut self) {
        self.manual = None;
    }

    /// Drops the tracked tempo's confidence; the tempo itself is kept so a
    /// pause in the music does not forget it.
    pub(crate) fn silence(&mut self) {
        self.confidence = 0.0;
        self.stability = 0.0;
    }
}

/// A phase accumulator in beats, kept in step with detected onsets.
///
/// Between onsets the grid free-runs at the current tempo. Each detected
/// beat nudges it toward the nearest whole beat, so small tempo errors
/// cannot accumulate into drift, while a one-off stray onset (a snare
/// ghost note) moves it only a little.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct BeatGrid {
    /// Position in beats since the grid started.
    beats: f64,
    /// Whether the grid has been aligned to a real beat yet.
    locked: bool,
    /// Consecutive onsets that missed the grid by a wide margin.
    stray: u32,
}

impl BeatGrid {
    /// Seconds between a real onset and its detection (window centring and
    /// the one-hop peak-pick delay).
    const DETECTION_LATENCY: f64 = 0.03;
    /// Fraction of the error removed per aligned onset.
    const GAIN: f64 = 0.25;
    /// Onsets further than this many beats from the grid count as strays.
    const STRAY_BEATS: f64 = 0.3;
    /// Strays in a row that make the grid re-align outright.
    const STRAYS_TO_SNAP: u32 = 3;

    /// Position in beats.
    pub(crate) fn beats(&self) -> f64 {
        self.beats
    }

    /// Runs the grid for `dt` seconds at `bpm`.
    pub(crate) fn advance(&mut self, dt: f32, bpm: f32) {
        self.beats += f64::from(dt.max(0.0)) * f64::from(bpm) / 60.0;
    }

    /// Aligns to a detected beat.
    pub(crate) fn on_beat(&mut self, bpm: f32) {
        let real = self.beats - Self::DETECTION_LATENCY * f64::from(bpm) / 60.0;
        let error = real.round() - real;
        if !self.locked {
            self.beats += error;
            self.locked = true;
            return;
        }
        if error.abs() <= Self::STRAY_BEATS {
            self.beats += Self::GAIN * error;
            self.stray = 0;
        } else {
            self.stray += 1;
            if self.stray >= Self::STRAYS_TO_SNAP {
                self.beats += error;
                self.stray = 0;
            }
        }
    }

    /// Aligns exactly to a tap, which is a human saying "this is a beat".
    pub(crate) fn on_tap(&mut self) {
        self.beats += self.beats.round() - self.beats;
        self.locked = true;
        self.stray = 0;
    }

    /// Forgets alignment, so the next beat snaps the grid outright.
    pub(crate) fn unlock(&mut self) {
        self.locked = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tap_tempo_averages_the_intervals() {
        let mut taps = TapTempo::default();
        assert_eq!(taps.tap(10.0), None, "one tap implies nothing");
        let bpm = taps.tap(10.5).unwrap_or(0.0);
        assert!((bpm - 120.0).abs() < 0.01, "0.5 s apart is 120 BPM: {bpm}");
        let bpm = taps.tap(11.0).unwrap_or(0.0);
        assert!((bpm - 120.0).abs() < 0.01, "still 120 BPM: {bpm}");
    }

    #[test]
    fn a_long_pause_restarts_the_count() {
        let mut taps = TapTempo::default();
        taps.tap(1.0);
        taps.tap(1.5);
        assert_eq!(taps.tap(9.0), None, "a tap after a long gap is a first tap");
    }

    #[test]
    fn tap_tempo_is_clamped_to_the_reported_range() {
        let mut taps = TapTempo::default();
        taps.tap(0.0);
        let bpm = taps.tap(0.1).unwrap_or(0.0);
        assert_eq!(bpm, BPM_MAX, "600 BPM taps are clamped");
    }

    #[test]
    fn nudge_overrides_and_clear_restores() {
        let mut tempo = Tempo::new(94.0);
        tempo.nudge(8.0);
        assert!((tempo.bpm() - 128.0).abs() < 1e-4, "120 + 8");
        assert!(tempo.is_manual(), "nudging takes manual control");
        assert_eq!(tempo.confidence(), 1.0, "a manual tempo is trusted");
        tempo.clear_manual();
        assert!((tempo.bpm() - 120.0).abs() < 1e-4, "back to automatic");
    }

    #[test]
    fn the_grid_locks_to_the_first_beat_then_follows_gently() {
        let mut grid = BeatGrid::default();
        grid.advance(0.3, 120.0);
        grid.on_beat(120.0);
        let offset = grid.beats() - BeatGrid::DETECTION_LATENCY * 2.0;
        assert!(
            (offset - offset.round()).abs() < 1e-9,
            "the first beat snaps the real onset to a whole beat: {offset}"
        );
    }

    #[test]
    fn the_prior_peaks_at_120() {
        assert!(prior(120.0) > prior(80.0), "120 beats 80");
        assert!(prior(120.0) > prior(200.0), "120 beats 200");
    }
}
