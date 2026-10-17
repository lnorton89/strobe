// ============================================================================
// Module:       strobe_audio::smoothing
// Description:  Time-constant smoothing primitives: followers, peak trackers,
//               and the logistic transition behind the beat-toggled uniforms.
//
// Dependencies: std only
// ============================================================================

//! Smoothing that depends on elapsed time, never on call count.
//!
//! Every filter here takes the time step as an argument and converts it to a
//! blend factor with `1 - exp(-dt / tau)`. That is the exact solution of a
//! one-pole low-pass for any step, so a value smoothed in 16 ms steps and
//! the same value smoothed in 33 ms steps land in the same place. It is the
//! audio-side twin of the control smoothing rule in `strobe-scene` (D9).

/// The fraction of the remaining distance covered in `dt` seconds by a
/// one-pole filter with time constant `tau`.
///
/// A non-positive `tau` means "instant"; a non-positive or non-finite `dt`
/// means no time passed.
pub(crate) fn blend(dt: f32, tau: f32) -> f32 {
    if !dt.is_finite() || dt <= 0.0 {
        return 0.0;
    }
    if tau <= 0.0 {
        return 1.0;
    }
    1.0 - (-dt / tau).exp()
}

/// The multiplier that an exponential decay with time constant `tau`
/// applies over `dt` seconds.
pub(crate) fn decay(dt: f32, tau: f32) -> f32 {
    1.0 - blend(dt, tau)
}

/// A one-pole follower with different time constants for rising and falling.
///
/// Loudness meters feel right when they jump up quickly and fall slowly:
/// a fast attack keeps a kick visible, a slow release stops the picture
/// flickering between hits.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Follower {
    /// The current output.
    pub(crate) value: f32,
}

impl Follower {
    /// Moves toward `target` over `dt` seconds and returns the new value.
    pub(crate) fn step(&mut self, target: f32, dt: f32, attack: f32, release: f32) -> f32 {
        let tau = if target > self.value { attack } else { release };
        self.value += (target - self.value) * blend(dt, tau);
        self.value
    }
}

/// A running maximum with exponential decay and a floor.
///
/// Normalising by a running maximum makes "loud" mean "loud for this
/// track", but divides by a tiny number in a silent room. The floor stops
/// that: a signal below it is reported as small, not amplified to full
/// scale.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PeakTracker {
    /// The decayed maximum so far.
    peak: f32,
    /// The smallest divisor [`PeakTracker::normalise`] will use.
    floor: f32,
}

impl PeakTracker {
    /// A tracker that never divides by less than `floor`.
    pub(crate) fn new(floor: f32) -> Self {
        Self { peak: 0.0, floor }
    }

    /// Folds in `x` after letting the old peak decay by `decay_factor`.
    pub(crate) fn update(&mut self, x: f32, decay_factor: f32) {
        self.peak = x.max(self.peak * decay_factor);
    }

    /// The divisor to use: the peak, but never below the floor.
    pub(crate) fn divisor(&self) -> f32 {
        self.peak.max(self.floor)
    }
}

/// A value that glides from one level to another along a logistic curve.
///
/// `syn_ToggleOnBeat` and `syn_RandomOnBeat` are specified to transition
/// "using a logistic curve": slow start, fast middle, slow arrival, which
/// reads as a deliberate swing rather than a fade.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Logistic {
    /// Where the transition started.
    from: f32,
    /// Where it is heading.
    to: f32,
    /// Progress along the curve, 0 to 1.
    progress: f32,
}

impl Logistic {
    /// Seconds a full transition takes.
    const DURATION: f32 = 0.25;
    /// Steepness of the S-curve; 10 puts the ends within 1% of flat.
    const STEEPNESS: f32 = 10.0;

    /// At rest at `value`.
    pub(crate) fn at(value: f32) -> Self {
        Self {
            from: value,
            to: value,
            progress: 1.0,
        }
    }

    /// Starts a transition from the current value to `target`.
    pub(crate) fn retarget(&mut self, target: f32) {
        self.from = self.value();
        self.to = target;
        self.progress = 0.0;
    }

    /// Advances the transition by `dt` seconds.
    pub(crate) fn advance(&mut self, dt: f32) {
        if dt.is_finite() && dt > 0.0 {
            self.progress = (self.progress + dt / Self::DURATION).min(1.0);
        }
    }

    /// The current value.
    pub(crate) fn value(&self) -> f32 {
        let curve = |p: f32| 1.0 / (1.0 + (-Self::STEEPNESS * (p - 0.5)).exp());
        let (low, high) = (curve(0.0), curve(1.0));
        let shaped = (curve(self.progress) - low) / (high - low);
        self.from + (self.to - self.from) * shaped
    }
}

/// A one-pole high-pass that removes a constant offset from the input.
///
/// Cheap microphones and line inputs often sit a little off zero. The
/// offset is not sound, but through the analysis window it leaks into the
/// lowest FFT bins, which are the bass band's, and read as bass that never
/// stops (D327). `y[n] = x[n] - x[n-1] + r * y[n-1]` with the corner at
/// [`DcBlocker::CORNER_HZ`], far below the 20 Hz bass edge.
#[derive(Clone, Copy, Debug)]
pub(crate) struct DcBlocker {
    /// The pole, `exp(-2 pi corner / rate)`.
    pole: f32,
    /// The previous input, once one has been seen.
    last_in: Option<f32>,
    /// The previous output.
    last_out: f32,
}

impl DcBlocker {
    /// The corner frequency in Hz: a 55 Hz kick loses under 2 %.
    const CORNER_HZ: f32 = 10.0;
    /// Outputs smaller than this are flushed to zero, so a decaying tail in
    /// digital silence never reaches the slow subnormal range.
    const FLUSH: f32 = 1.0e-20;

    /// A blocker for audio at `sample_rate` Hz.
    pub(crate) fn new(sample_rate: f32) -> Self {
        let pole = (-std::f32::consts::TAU * Self::CORNER_HZ / sample_rate.max(1.0)).exp();
        Self {
            pole,
            last_in: None,
            last_out: 0.0,
        }
    }

    /// Filters one sample. The first sample primes the filter instead of
    /// stepping from zero, so a stream that opens off-centre gives no click.
    pub(crate) fn step(&mut self, x: f32) -> f32 {
        let previous = self.last_in.unwrap_or(x);
        let mut y = x - previous + self.pole * self.last_out;
        if y.abs() < Self::FLUSH {
            y = 0.0;
        }
        self.last_in = Some(x);
        self.last_out = y;
        y
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dc_blocker_removes_an_offset_and_keeps_a_tone() {
        let rate = 48_000.0;
        let mut blocker = DcBlocker::new(rate);
        let mut peak = 0.0_f32;
        let mut last = 0.0_f32;
        for n in 0..48_000 {
            let tone = 0.5 * (std::f32::consts::TAU * 55.0 * n as f32 / rate).sin();
            last = blocker.step(0.2 + tone);
            if n > 24_000 {
                peak = peak.max(last.abs());
            }
        }
        assert!(
            (peak - 0.5).abs() < 0.02,
            "a 55 Hz tone keeps its amplitude, got {peak}"
        );
        let mut silent = DcBlocker::new(rate);
        let mut out = 1.0_f32;
        for _ in 0..48_000 {
            out = silent.step(0.2);
        }
        assert!(
            out == 0.0,
            "a constant offset is removed outright, got {out}"
        );
        assert!(last.is_finite(), "the output stays finite");
    }

    #[test]
    fn blend_is_step_size_independent() {
        let once = blend(0.1, 0.2);
        let twice = 1.0 - (1.0 - blend(0.05, 0.2)) * (1.0 - blend(0.05, 0.2));
        assert!(
            (once - twice).abs() < 1e-6,
            "one 100 ms step ({once}) should equal two 50 ms steps ({twice})"
        );
    }

    #[test]
    fn blend_ignores_nonsense_steps() {
        assert_eq!(blend(f32::NAN, 0.1), 0.0, "NaN dt means no time passed");
        assert_eq!(blend(-1.0, 0.1), 0.0, "negative dt means no time passed");
        assert_eq!(blend(0.1, 0.0), 1.0, "zero tau is instant");
    }

    #[test]
    fn follower_attacks_faster_than_it_releases() {
        let mut f = Follower::default();
        let up = f.step(1.0, 0.05, 0.05, 1.0);
        let down = f.step(0.0, 0.05, 0.05, 1.0) - up;
        assert!(up > 0.6, "attack covers most of the gap, got {up}");
        assert!(down > -0.2, "release is slow, moved {down}");
    }

    #[test]
    fn logistic_runs_from_start_to_target_exactly() {
        let mut l = Logistic::at(0.0);
        l.retarget(1.0);
        assert!(l.value().abs() < 1e-6, "starts at the old value");
        l.advance(0.125);
        assert!((l.value() - 0.5).abs() < 1e-3, "halfway is the midpoint");
        l.advance(1.0);
        assert!((l.value() - 1.0).abs() < 1e-6, "arrives exactly");
    }

    #[test]
    fn peak_tracker_never_divides_below_its_floor() {
        let mut p = PeakTracker::new(0.01);
        p.update(0.0001, 1.0);
        assert_eq!(p.divisor(), 0.01, "a tiny signal is not amplified");
        p.update(0.5, 1.0);
        assert_eq!(p.divisor(), 0.5, "a real peak is the divisor");
    }
}
