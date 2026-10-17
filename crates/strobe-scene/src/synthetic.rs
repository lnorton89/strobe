// ============================================================================
// Module:       strobe_scene::synthetic
// Description:  A deterministic stand-in for the audio analyser: plausible
//               values for every SSF audio uniform as a function of time.
//
// Dependencies: crate::audio (the audio uniform names)
// ============================================================================

//! Synthetic audio.
//!
//! Audio-reactive scenes look dead at silence, and a benchmark frame of a
//! dead scene measures the wrong thing — branches untaken, feedback never
//! seeded. Until the real analyser exists, this drives every audio uniform
//! from a 120 BPM four-on-the-floor pattern: a kick envelope on each beat,
//! hats on the off-beats, a slow swell over sixteen bars. It is a pure
//! function of time, so two runs render identical frames. The benchmark
//! uses it always; the app uses it when no audio device is available.
//!
//! # Clocks
//!
//! A clock is the integral of a rate, never the time times a rate: the
//! levels pulse with the kick, so `time * (0.6 + level)` would step on
//! every beat by an amount that grows with the time (hundreds of units
//! between two frames an hour into a show). Each clock here is the closed
//! form of its integral from 0, evaluated in `f64`, so the track stays a
//! pure function of time and no clock steps or runs back at any hour
//! (D330):
//!
//! | Clock | Rate |
//! |---|---|
//! | `syn_Time`, `syn_MidTime`, `syn_MidHighTime`, `syn_HighTime` | `0.6 + syn_Level` |
//! | `syn_BassTime` | a surge on each kick; one unit per beat |
//! | `syn_CurvedTime` | `0.5 + 1.5 m²`, `m` the level averaged over a beat |
//! | `syn_BeatTime` | the tempo: one per beat, evenly |
//!
//! `syn_BPMTwitcher` keeps its snap on the beat; that is what it is for.

use crate::audio::SCALARS;

/// Beats per minute of the synthetic track.
const BPM: f32 = 120.0;
/// Seconds per beat.
const BEAT_LEN: f32 = 60.0 / BPM;
/// How fast the kick decays within a beat, per beat.
const KICK_DECAY: f32 = 9.0;
/// How fast the off-beat hat decays, per beat.
const HAT_DECAY: f32 = 18.0;
/// The swell's period, in beats (sixteen bars).
const SWELL_BEATS: f32 = 64.0;
/// Angular rate of the mids' wobble, `|sin(MID_WOBBLE * t)|`, per second.
const MID_WOBBLE: f32 = 1.7;

/// The shared envelopes of the synthetic track at one instant.
struct Pulse {
    beats: f32,
    phase: f32,
    beat_index: f32,
    kick: f32,
    hat: f32,
    swell: f32,
    bass: f32,
    mid: f32,
    high: f32,
    level: f32,
}

/// Evaluates the synthetic track at `time`.
fn pulse(time: f32) -> Pulse {
    let beats = time / BEAT_LEN;
    let phase = beats.rem_euclid(1.0);
    let beat_index = beats.floor();

    let kick = (-phase * KICK_DECAY).exp();
    let hat = (-((phase + 0.5).fract()) * HAT_DECAY).exp();
    let swell = 0.5 + 0.5 * (time * std::f32::consts::TAU / (BEAT_LEN * SWELL_BEATS)).sin();
    let bass = (0.35 + 0.65 * kick) * (0.6 + 0.4 * swell);
    let mid = 0.3 + 0.3 * swell + 0.2 * (time * MID_WOBBLE).sin().abs();
    let high = 0.2 + 0.6 * hat;
    Pulse {
        beats,
        phase,
        beat_index,
        kick,
        hat,
        swell,
        bass,
        mid,
        high,
        level: (bass + mid + high) / 3.0,
    }
}

/// The synthetic track's clocks at one instant, each the integral from 0 of
/// its rate (D330). See the module docs for the rates.
#[derive(Debug, Clone, Copy)]
struct Clocks {
    /// `syn_Time` and the Mid, MidHigh and High clocks.
    level: f64,
    /// `syn_BassTime`.
    bass: f64,
    /// `syn_CurvedTime`.
    curved: f64,
    /// `syn_BeatTime`.
    beats: f64,
}

/// A complex number, for the one integral that needs one.
#[derive(Debug, Clone, Copy)]
struct Complex {
    re: f64,
    im: f64,
}

impl Complex {
    fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }

    /// `e^(i angle)`.
    fn cis(angle: f64) -> Self {
        Self::new(angle.cos(), angle.sin())
    }

    /// `e^self`.
    fn exp(self) -> Self {
        let scale = self.re.exp();
        Self::new(scale * self.im.cos(), scale * self.im.sin())
    }

    fn minus(self, real: f64) -> Self {
        Self::new(self.re - real, self.im)
    }

    fn times(self, other: Self) -> Self {
        Self::new(
            self.re * other.re - self.im * other.im,
            self.re * other.im + self.im * other.re,
        )
    }

    fn over(self, other: Self) -> Self {
        let norm = other.re * other.re + other.im * other.im;
        Self::new(
            (self.re * other.re + self.im * other.im) / norm,
            (self.im * other.re - self.re * other.im) / norm,
        )
    }

    fn plus(self, other: Self) -> Self {
        Self::new(self.re + other.re, self.im + other.im)
    }
}

/// `∫₀ᵗ e^(-decay · frac(u)) dt`, `u = t / BEAT_LEN` the beat position
/// `beats`: an envelope that restarts at every whole beat and decays by
/// `decay` per beat, integrated in seconds. Each whole beat adds the same
/// amount, so it is a count plus the current beat's part.
fn decay_integral(beats: f64, decay: f64) -> f64 {
    let whole = beats.floor();
    let part = beats - whole;
    let beat_len = f64::from(BEAT_LEN);
    beat_len / decay * ((1.0 - (-decay).exp()) * whole + 1.0 - (-decay * part).exp())
}

/// `∫₀ᵗ kick · sin(ω t) dt`, `ω` the swell's angular rate.
///
/// On beat `j`, with `x` the time into the beat, the integrand is
/// `Im(e^(iθj) e^(zx))` with `θ = ω · BEAT_LEN` and `z = -λ + iω`
/// (`λ` the kick's decay per second). A whole beat contributes
/// `Im(e^(iθj) F)`, `F = (e^(z · BEAT_LEN) - 1) / z`, so the whole beats
/// before `n` sum geometrically to `Im(F (e^(iθn) - 1) / (e^(iθ) - 1))`.
/// The swell lasts exactly [`SWELL_BEATS`] beats, so `e^(iθn)` repeats
/// with that period and `n` is taken modulo it, which keeps every term
/// small four hours in.
fn kick_sine_integral(beats: f64) -> f64 {
    let beat_len = f64::from(BEAT_LEN);
    let swell_beats = f64::from(SWELL_BEATS);
    let omega = std::f64::consts::TAU / (beat_len * swell_beats);
    let theta = omega * beat_len;
    let z = Complex::new(-f64::from(KICK_DECAY) / beat_len, omega);
    let whole = beats.floor();
    let part = beats - whole;
    let turn = Complex::cis(theta * whole.rem_euclid(swell_beats));
    let full = Complex::new(z.re * beat_len, z.im * beat_len)
        .exp()
        .minus(1.0)
        .over(z);
    let before = full.times(turn.minus(1.0).over(Complex::cis(theta).minus(1.0)));
    let current = Complex::new(z.re * part * beat_len, z.im * part * beat_len)
        .exp()
        .minus(1.0)
        .over(z);
    before.plus(turn.times(current)).im
}

/// Evaluates the synthetic track's clocks at `time`, in closed form.
fn clocks(time: f64) -> Clocks {
    /// A level clock runs at this plus the level.
    const CLOCK_BASE: f64 = 0.6;
    /// `syn_BassTime`'s surge: its rate decays by this per beat.
    const BASS_SURGE: f64 = 4.0;
    let beat_len = f64::from(BEAT_LEN);
    let beats = time / beat_len;
    let kick_decay = f64::from(KICK_DECAY);
    let hat_decay = f64::from(HAT_DECAY);
    let omega = std::f64::consts::TAU / (beat_len * f64::from(SWELL_BEATS));
    let wobble = f64::from(MID_WOBBLE);

    // The integrals from 0 to `time` of each envelope in `pulse`.
    let swell = 0.5 * time + 0.5 * (1.0 - (omega * time).cos()) / omega;
    let kick = decay_integral(beats, kick_decay);
    let kick_swell = 0.5 * kick + 0.5 * kick_sine_integral(beats);
    // The hat is the kick's shape half a beat later.
    let hat = decay_integral(beats + 0.5, hat_decay) - decay_integral(0.5, hat_decay);
    // `|sin|` adds 2 per half turn.
    let turns = (wobble * time / std::f64::consts::PI).floor();
    let rest = wobble * time - turns * std::f64::consts::PI;
    let wobble_abs = (2.0 * turns + 1.0 - rest.cos()) / wobble;

    let bass = 0.35 * (0.6 * time + 0.4 * swell) + 0.65 * (0.6 * kick + 0.4 * kick_swell);
    let mid = 0.3 * time + 0.3 * swell + 0.2 * wobble_abs;
    let high = 0.2 * time + 0.6 * hat;
    let level = CLOCK_BASE * time + (bass + mid + high) / 3.0;

    // The curved clock runs at `0.5 + 1.5 m²`, `m` the level with the kick,
    // the hat and the wobble replaced by their means over a beat, so
    // `m = mean + depth · sin(ω t)` and `m²` integrates in closed form.
    let kick_mean = (1.0 - (-kick_decay).exp()) / kick_decay;
    let hat_mean = (1.0 - (-hat_decay).exp()) / hat_decay;
    let wobble_mean = 2.0 / std::f64::consts::PI;
    let bass_gain = 0.35 + 0.65 * kick_mean;
    let mean = (bass_gain * 0.8 + 0.45 + 0.2 * wobble_mean + 0.2 + 0.6 * hat_mean) / 3.0;
    let depth = (bass_gain * 0.2 + 0.15) / 3.0;
    let square = mean * mean * time
        + 2.0 * mean * depth * (1.0 - (omega * time).cos()) / omega
        + depth * depth * (0.5 * time - (2.0 * omega * time).sin() / (4.0 * omega));
    let curved = 0.5 * time + 1.5 * square;

    let whole = beats.floor();
    let surge = (1.0 - (-BASS_SURGE * (beats - whole)).exp()) / (1.0 - (-BASS_SURGE).exp());
    Clocks {
        level,
        bass: whole + surge,
        curved,
        beats,
    }
}

/// The band energies of the synthetic track at one instant, `0..1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bands {
    /// Overall level.
    pub level: f32,
    /// Bass.
    pub bass: f32,
    /// Mids.
    pub mid: f32,
    /// Highs.
    pub high: f32,
}

/// The band energies at `time`.
pub fn bands(time: f32) -> Bands {
    let pulse = pulse(time);
    Bands {
        level: pulse.level,
        bass: pulse.bass,
        mid: pulse.mid,
        high: pulse.high,
    }
}

/// Fills `spectrum` (`syn_Spectrum`) and `level_trail` (`syn_LevelTrail`)
/// with texel data shaped like the real analyser's, in the same channel
/// meaning as the audio crate documents: spectrum `r` raw, `g` juiced, `b`
/// smooth, `a` waveform; trail `[whole, bass, mid, high]` with index 0 now
/// and higher indices older, spanning `trail_seconds`.
///
/// This exists so a scene that draws the spectrum is not dead when no audio
/// device is present. It is a function of `time` alone, like [`fill`].
pub fn fill_textures(
    time: f32,
    spectrum: &mut [[f32; 4]],
    level_trail: &mut [[f32; 4]],
    trail_seconds: f32,
) {
    let now = bands(time);
    let width = spectrum.len().max(1) as f32;
    let hump = |x: f32, centre: f32, spread: f32| {
        let d = (x - centre) / spread;
        (-d * d).exp()
    };
    for (index, texel) in spectrum.iter_mut().enumerate() {
        let x = (index as f32 + 0.5) / width;
        let body = now.bass * (-x * 7.0).exp()
            + 0.6 * now.mid * hump(x, 0.30, 0.12)
            + 0.5 * now.high * hump(x, 0.65, 0.18);
        let ripple = 0.08 * now.level * (0.5 + 0.5 * (x * 60.0 + time * 3.0).sin());
        let smooth = 0.9 * now.bass * (-x * 3.0).exp()
            + 0.5 * now.mid * hump(x, 0.35, 0.3)
            + 0.4 * now.high * hump(x, 0.7, 0.3);
        let wave = 0.5 + 0.35 * now.level * (x * std::f32::consts::TAU * 8.0 + time * 20.0).sin();
        *texel = [
            (body + ripple).clamp(0.0, 1.0),
            (body.sqrt() * 0.8 + ripple).clamp(0.0, 1.0),
            smooth.clamp(0.0, 1.0),
            wave.clamp(0.0, 1.0),
        ];
    }
    let span = trail_seconds.max(0.0) / level_trail.len().max(1) as f32;
    for (index, texel) in level_trail.iter_mut().enumerate() {
        let past = bands(time - index as f32 * span);
        *texel = [
            past.level.clamp(0.0, 1.0),
            past.bass.clamp(0.0, 1.0),
            past.mid.clamp(0.0, 1.0),
            past.high.clamp(0.0, 1.0),
        ];
    }
}

/// Fills `out` (in [`SCALARS`] order) with the audio state at `time`.
pub fn fill(time: f32, out: &mut [f32]) {
    let Pulse {
        beats,
        phase,
        beat_index,
        kick,
        hat,
        swell,
        bass,
        mid,
        high,
        level,
    } = pulse(time);
    let tri = |cycles: f32| 1.0 - (2.0 * (beats / cycles).fract() - 1.0).abs();
    let sin = |cycles: f32| 0.5 + 0.5 * (beats / cycles * std::f32::consts::TAU).sin();
    let random = ((beat_index * 12.9898).sin() * 43_758.547).fract().abs();
    let clocks = clocks(f64::from(time));

    for (slot, name) in out.iter_mut().zip(SCALARS) {
        *slot = match *name {
            "syn_Level" => level,
            "syn_BassLevel" => bass,
            "syn_MidLevel" | "syn_MidHighLevel" => mid,
            "syn_HighLevel" => high,
            "syn_Hits" => kick.max(hat) * 0.8,
            "syn_BassHits" => kick,
            "syn_MidHits" | "syn_MidHighHits" => kick * 0.5,
            "syn_HighHits" => hat,
            "syn_Time" | "syn_MidTime" | "syn_MidHighTime" | "syn_HighTime" => clocks.level as f32,
            "syn_BassTime" => clocks.bass as f32,
            "syn_CurvedTime" => clocks.curved as f32,
            "syn_Presence"
            | "syn_BassPresence"
            | "syn_MidPresence"
            | "syn_MidHighPresence"
            | "syn_HighPresence" => swell,
            "syn_OnBeat" => kick,
            "syn_ToggleOnBeat" => (beat_index % 2.0).abs(),
            "syn_RandomOnBeat" => random,
            "syn_BeatTime" => clocks.beats as f32,
            "syn_BPMTwitcher" => beat_index + phase.powf(0.25),
            "syn_BPM" => BPM,
            "syn_BPMConfidence" => 0.9,
            "syn_BPMSin" => sin(1.0),
            "syn_BPMSin2" => sin(2.0),
            "syn_BPMSin4" => sin(4.0),
            "syn_BPMSin8" => sin(8.0),
            "syn_BPMTri" => tri(1.0),
            "syn_BPMTri2" => tri(2.0),
            "syn_BPMTri4" => tri(4.0),
            "syn_BPMTri8" => tri(8.0),
            "syn_FadeInOut" | "syn_Intensity" => swell,
            _ => crate::audio::silent_value(name),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_textures_are_finite_in_range_and_follow_the_kick() {
        let mut spectrum = vec![[0.0_f32; 4]; 256];
        let mut trail = vec![[0.0_f32; 4]; 256];
        fill_textures(0.0, &mut spectrum, &mut trail, 4.0);
        let in_range = |texels: &[[f32; 4]]| {
            texels
                .iter()
                .flatten()
                .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
        };
        assert!(in_range(&spectrum), "spectrum texels must be 0..1");
        assert!(in_range(&trail), "trail texels must be 0..1");
        let on_beat = spectrum[2][0];
        fill_textures(0.25, &mut spectrum, &mut trail, 4.0);
        assert!(
            spectrum[2][0] < on_beat,
            "low bins are loudest on the kick: {on_beat} then {}",
            spectrum[2][0]
        );
        assert!(
            (trail[0][1] - bands(0.25).bass).abs() < 1e-6,
            "trail index 0 is now"
        );
        assert!(
            (trail[255][1] - bands(0.25 - 255.0 * 4.0 / 256.0).bass).abs() < 1e-3,
            "the last texel is the oldest"
        );
    }

    #[test]
    fn every_value_is_finite_and_the_kick_lands_on_the_beat() {
        let mut values = vec![0.0; SCALARS.len()];
        fill(0.0, &mut values);
        assert!(values.iter().all(|v| v.is_finite()), "all finite at t = 0");
        let bass_hits = SCALARS
            .iter()
            .position(|n| *n == "syn_BassHits")
            .unwrap_or(0);
        assert!(values[bass_hits] > 0.99, "the kick peaks on the beat");
        fill(0.25, &mut values);
        assert!(values[bass_hits] < 0.2, "and has decayed by the off-beat");
    }

    /// Every clock the synthetic track drives.
    const CLOCKS: [&str; 7] = [
        "syn_Time",
        "syn_BassTime",
        "syn_MidTime",
        "syn_MidHighTime",
        "syn_HighTime",
        "syn_CurvedTime",
        "syn_BeatTime",
    ];

    fn scalar(name: &str) -> anyhow::Result<usize> {
        SCALARS
            .iter()
            .position(|scalar| *scalar == name)
            .ok_or_else(|| anyhow::anyhow!("{name} is not a scalar"))
    }

    #[test]
    fn the_clocks_glide_and_never_run_back_hours_into_a_show() -> anyhow::Result<()> {
        // A clock that jumps on the beat is a beat, and the old
        // `time * (0.6 + level)` jumped by hundreds an hour in (D330). At
        // the start, an hour in and four hours in (a venue show), over
        // twenty beats of 60 fps frames, every clock moves forward by at
        // most a few frames' worth: the level clocks run at up to 1.6
        // units a second, the beat clock at 2, and the bass clock's surge
        // covers about an eighth of a unit in a beat's first frame.
        const MAX_STEP: f32 = 0.15;
        let indices = CLOCKS
            .iter()
            .map(|name| scalar(name))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let mut previous = vec![0.0; SCALARS.len()];
        let mut values = vec![0.0; SCALARS.len()];
        for start in [0.0_f32, 3600.0, 4.0 * 3600.0] {
            fill(start, &mut previous);
            for step in 1..=600 {
                let t = start + step as f32 / 60.0;
                fill(t, &mut values);
                for (name, &index) in CLOCKS.iter().zip(&indices) {
                    let moved = values[index] - previous[index];
                    assert!(
                        moved >= 0.0,
                        "{name} ran back by {moved} in one frame at {t} s"
                    );
                    assert!(
                        moved < MAX_STEP,
                        "{name} jumped by {moved} in one frame at {t} s"
                    );
                }
                previous.copy_from_slice(&values);
            }
        }
        Ok(())
    }

    #[test]
    fn the_level_clock_is_the_integral_of_its_rate() -> anyhow::Result<()> {
        // A midpoint sum of `0.6 + syn_Level` over 40 s (more than a whole
        // swell, so the kick-times-swell term is exercised over its
        // period) against the closed form.
        const STEPS: u32 = 400_000;
        for start in [0.0_f64, 100.0] {
            let span = 40.0;
            let dt = span / f64::from(STEPS);
            let sum: f64 = (0..STEPS)
                .map(|step| {
                    let t = start + (f64::from(step) + 0.5) * dt;
                    (0.6 + f64::from(pulse(t as f32).level)) * dt
                })
                .sum();
            let closed = clocks(start + span).level - clocks(start).level;
            assert!(
                (closed - sum).abs() < 1e-5 * span,
                "from {start} s the clock ran {closed}, the rate's integral is {sum}"
            );
        }
        Ok(())
    }

    #[test]
    fn the_beat_clocks_count_one_per_beat() -> anyhow::Result<()> {
        let (beat_time, twitcher) = (scalar("syn_BeatTime")?, scalar("syn_BPMTwitcher")?);
        let mut values = vec![0.0; SCALARS.len()];
        for t in [0.0_f32, 0.1, 0.37, 12.0, 3600.25, 14_400.5] {
            fill(t, &mut values);
            let expected = f64::from(t) * f64::from(BPM) / 60.0;
            assert!(
                (f64::from(values[beat_time]) - expected).abs() < 1e-3,
                "syn_BeatTime is {} at {t} s, not {expected}",
                values[beat_time]
            );
            assert!(
                values[twitcher].floor() == values[beat_time].floor(),
                "the twitcher counts the same beat at {t} s"
            );
        }
        for beat in [0_u32, 1, 7, 7200, 28_800] {
            let at = clocks(f64::from(beat) * f64::from(BEAT_LEN));
            assert!(
                (at.bass - f64::from(beat)).abs() < 1e-9,
                "syn_BassTime is {} on beat {beat}",
                at.bass
            );
        }
        Ok(())
    }

    #[test]
    fn the_same_time_gives_the_same_values() -> anyhow::Result<()> {
        let mut first = vec![0.0; SCALARS.len()];
        let mut second = vec![1.0; SCALARS.len()];
        for t in [0.0_f32, 0.25, 61.3, 3600.0, 14_399.983] {
            fill(t, &mut first);
            fill(13.0, &mut second);
            fill(t, &mut second);
            assert!(
                first
                    .iter()
                    .zip(&second)
                    .all(|(a, b)| a.to_bits() == b.to_bits()),
                "fill is a pure function of time at {t} s"
            );
            assert!(
                first.iter().all(|value| value.is_finite()),
                "every value is finite at {t} s"
            );
        }
        Ok(())
    }
}
