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

use crate::audio::SCALARS;

/// Beats per minute of the synthetic track.
const BPM: f32 = 120.0;

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
    let beat_len = 60.0 / BPM;
    let beats = time / beat_len;
    let phase = beats.rem_euclid(1.0);
    let beat_index = beats.floor();

    let kick = (-phase * 9.0).exp();
    let hat = (-((phase + 0.5).fract()) * 18.0).exp();
    let swell = 0.5 + 0.5 * (time * std::f32::consts::TAU / (beat_len * 64.0)).sin();
    let bass = (0.35 + 0.65 * kick) * (0.6 + 0.4 * swell);
    let mid = 0.3 + 0.3 * swell + 0.2 * (time * 1.7).sin().abs();
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
            "syn_Time" | "syn_MidTime" | "syn_MidHighTime" | "syn_HighTime" => time * (0.6 + level),
            "syn_BassTime" => beat_index + 1.0 - (-phase * 4.0).exp(),
            "syn_CurvedTime" => time * (0.5 + level * level * 1.5),
            "syn_Presence"
            | "syn_BassPresence"
            | "syn_MidPresence"
            | "syn_MidHighPresence"
            | "syn_HighPresence" => swell,
            "syn_OnBeat" => kick,
            "syn_ToggleOnBeat" => (beat_index % 2.0).abs(),
            "syn_RandomOnBeat" => random,
            "syn_BeatTime" | "syn_BPMTwitcher" => beat_index + phase.powf(0.25),
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
}
