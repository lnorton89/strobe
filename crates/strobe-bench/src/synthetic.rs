// ============================================================================
// Module:       strobe_bench::synthetic
// Description:  A deterministic stand-in for the audio analyser: plausible
//               values for every SSF audio uniform as a function of time.
//
// Dependencies: strobe-scene (the audio uniform names)
// ============================================================================

//! Synthetic audio.
//!
//! Audio-reactive scenes look dead at silence, and a benchmark frame of a
//! dead scene measures the wrong thing — branches untaken, feedback never
//! seeded. Until the real analyser exists, this drives every audio uniform
//! from a 120 BPM four-on-the-floor pattern: a kick envelope on each beat,
//! hats on the off-beats, a slow swell over sixteen bars. It is a pure
//! function of time, so two runs render identical frames.

use strobe_scene::audio::SCALARS;

/// Beats per minute of the synthetic track.
const BPM: f32 = 120.0;

/// Fills `out` (in [`SCALARS`] order) with the audio state at `time`.
pub(crate) fn fill(time: f32, out: &mut [f32]) {
    let beat_len = 60.0 / BPM;
    let beats = time / beat_len;
    let phase = beats.fract();
    let beat_index = beats.floor();

    let kick = (-phase * 9.0).exp();
    let hat = (-((phase + 0.5).fract()) * 18.0).exp();
    let swell = 0.5 + 0.5 * (time * std::f32::consts::TAU / (beat_len * 64.0)).sin();
    let bass = (0.35 + 0.65 * kick) * (0.6 + 0.4 * swell);
    let mid = 0.3 + 0.3 * swell + 0.2 * (time * 1.7).sin().abs();
    let high = 0.2 + 0.6 * hat;
    let level = (bass + mid + high) / 3.0;
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
            _ => strobe_scene::audio::silent_value(name),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
