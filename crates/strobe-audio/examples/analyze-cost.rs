// ============================================================================
// Module:       analyze-cost (example)
// Description:  Times Analyzer::analyze per call on a busy synthetic signal,
//               for the audio line of the per-frame cost budget.
//
// Dependencies: strobe-audio (Analyzer)
// ============================================================================

//! `analyze-cost`: how much CPU the analyser takes.
//!
//! ```text
//! cargo run --release -p strobe-audio --example analyze-cost
//! ```
//!
//! Feeds sixty seconds of a kick-plus-hat-plus-tone mix at 48 kHz in
//! 60-frames-a-second slices and times each `push` + `analyze` pair, the
//! work the analysis thread does per render frame. Prints the mean, the
//! 99th percentile and the worst case, and the share of one core that mean
//! represents at 60 calls a second. Run it in release: debug numbers say
//! nothing about the shipped build.

use std::time::Instant;

use strobe_audio::Analyzer;

/// Sample rate of the synthetic signal.
const RATE: u32 = 48_000;
/// Simulated render frames per second.
const FPS: u32 = 60;
/// Seconds of signal.
const SECONDS: u32 = 60;

/// A busy mix: a 128 BPM kick, an off-beat hat burst, and a held chord.
fn signal() -> Vec<f32> {
    let total = (RATE * SECONDS) as usize;
    let beat = f64::from(RATE) * 60.0 / 128.0;
    let mut seed = 0x2545_F491_u32;
    (0..total)
        .map(|n| {
            let t = n as f64 / f64::from(RATE);
            let in_beat = (n as f64 % beat) / f64::from(RATE);
            let kick = (-in_beat / 0.08).exp() * (std::f64::consts::TAU * 55.0 * in_beat).sin();
            let off = ((n as f64 + beat / 2.0) % beat) / f64::from(RATE);
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let hiss = f64::from(seed >> 8) / 8_388_608.0 - 1.0;
            let hat = (-off / 0.02).exp() * hiss;
            let chord = (std::f64::consts::TAU * 220.0 * t).sin()
                + (std::f64::consts::TAU * 277.2 * t).sin()
                + (std::f64::consts::TAU * 329.6 * t).sin();
            (0.6 * kick + 0.15 * hat + 0.05 * chord) as f32
        })
        .collect()
}

fn main() {
    let samples = signal();
    let mut analyzer = Analyzer::new(RATE);
    let per_frame = (RATE / FPS) as usize;
    let mut times = Vec::new();
    for chunk in samples.chunks(per_frame) {
        let started = Instant::now();
        analyzer.push(chunk);
        analyzer.analyze(1.0 / FPS as f32);
        times.push(started.elapsed().as_secs_f64() * 1.0e6);
    }
    times.sort_by(f64::total_cmp);
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    let at = |q: f64| {
        times
            .get(((times.len() as f64 - 1.0) * q) as usize)
            .copied()
    };
    let p99 = at(0.99).unwrap_or(0.0);
    let worst = times.last().copied().unwrap_or(0.0);
    let share = mean * f64::from(FPS) / 1.0e6 * 100.0;
    println!(
        "{} calls: mean {mean:.1} us, p99 {p99:.1} us, worst {worst:.1} us; \
         {share:.2}% of one core at {FPS} calls/s",
        times.len()
    );
}
