// ============================================================================
// Module:       strobe_audio::fixtures
// Description:  Synthetic test signals: kick patterns, tones, noise, square
//               waves, generated in memory.
//
// Dependencies: crate::analyzer, crate::offline
// ============================================================================

//! Test signals.
//!
//! Everything is generated from arithmetic, so the tests need no files and
//! are bit-for-bit repeatable. The kick is a decaying 55 Hz sine with a
//! short pitch drop, which is what a synthesised kick drum is.

use crate::analyzer::Analyzer;
use crate::frame::AudioFrame;
use crate::offline;

/// Sample rate the tests run at.
pub(crate) const RATE: u32 = 48_000;

/// Frames per second the simulated render loop runs at.
pub(crate) const FPS: f32 = 60.0;

/// A kick pattern: one decaying 55 Hz burst per beat, with the first beat
/// at 0.2 s.
pub(crate) fn kicks(bpm: f32, seconds: f32, rate: u32) -> Vec<f32> {
    let total = (seconds * rate as f32) as usize;
    let period = 60.0 / bpm;
    let mut out = vec![0.0_f32; total];
    let mut beat = 0;
    loop {
        let start = ((0.2 + beat as f32 * period) * rate as f32) as usize;
        if start >= total {
            break;
        }
        // 0.4 s is five decay constants: inaudible by the end.
        for n in 0..(0.4 * rate as f32) as usize {
            let Some(slot) = out.get_mut(start + n) else {
                break;
            };
            let t = n as f32 / rate as f32;
            let phase =
                std::f32::consts::TAU * (55.0 * t + 12.0 * (1.0 - (-t * 40.0).exp()) / 40.0);
            *slot += 0.9 * (-t / 0.08).exp() * phase.sin();
        }
        beat += 1;
    }
    out
}

/// A sine with a 20 ms fade-in. Starting a tone on a non-zero sample is a
/// click, and a click is broadband: it would put energy in every band and
/// make band-isolation tests measure the click instead of the tone.
pub(crate) fn tone(freq: f32, amplitude: f32, seconds: f32, rate: u32) -> Vec<f32> {
    let ramp = 0.02 * rate as f32;
    (0..(seconds * rate as f32) as usize)
        .map(|n| {
            let fade = (n as f32 / ramp).min(1.0);
            // The phase is computed in f64: at 10 kHz the argument passes a
            // million radians, where an f32 is only good to a tenth of one,
            // and the resulting phase noise would smear the tone across
            // every band.
            let phase = std::f64::consts::TAU * f64::from(freq) * n as f64 / f64::from(rate);
            fade * amplitude * phase.sin() as f32
        })
        .collect()
}

/// A full-scale square wave, deliberately clipped hard.
pub(crate) fn square(freq: f32, seconds: f32, rate: u32) -> Vec<f32> {
    (0..(seconds * rate as f32) as usize)
        .map(|n| {
            if (freq * n as f32 / rate as f32).fract() < 0.5 {
                1.0
            } else {
                -1.0
            }
        })
        .collect()
}

/// Deterministic white noise in -amplitude..amplitude.
pub(crate) fn noise(amplitude: f32, seconds: f32, rate: u32) -> Vec<f32> {
    let mut state = 0x1234_5678_u32;
    (0..(seconds * rate as f32) as usize)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            amplitude * ((state >> 8) as f32 / 8_388_608.0 - 1.0)
        })
        .collect()
}

/// Runs `samples` through a fresh analyser and returns the last frame.
pub(crate) fn last_frame(samples: &[f32], rate: u32, fps: f32) -> AudioFrame {
    let mut analyzer = Analyzer::new(rate);
    offline::run(&mut analyzer, samples, fps, |_, _| {});
    analyzer.frame().clone()
}
