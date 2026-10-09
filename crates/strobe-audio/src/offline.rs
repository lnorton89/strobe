// ============================================================================
// Module:       strobe_audio::offline
// Description:  Drives an Analyzer over a finished buffer at a fixed frame
//               rate, the way the render loop would, for tests and tuning.
//
// Dependencies: crate::analyzer, crate::frame
// ============================================================================

//! Offline analysis.
//!
//! Live, samples arrive in whatever chunks the device delivers and
//! [`Analyzer::analyze`] is called once per rendered frame. This reproduces
//! that for a buffer already in memory: each simulated frame pushes the
//! samples that elapsed during it (carrying the fractional remainder, so
//! 48 000 Hz at 60 fps is exactly 800 per frame and 44 100 Hz is not) and
//! then analyses with `dt = 1 / fps`. The tuning harness and the tests both
//! use it, so what they measure is what a scene would see.

use crate::analyzer::Analyzer;
use crate::frame::AudioFrame;

/// Pushes `samples` into `analyzer` at `fps` frames a second and calls
/// `on_frame` with the elapsed time in seconds and the frame after each.
///
/// A non-positive or non-finite `fps` does nothing.
pub fn run(
    analyzer: &mut Analyzer,
    samples: &[f32],
    fps: f32,
    mut on_frame: impl FnMut(f32, &AudioFrame),
) {
    if !fps.is_finite() || fps <= 0.0 {
        return;
    }
    let per_frame = f64::from(analyzer.sample_rate()) / f64::from(fps);
    let dt = 1.0 / fps;
    let mut consumed = 0_usize;
    let mut frame = 0_u64;
    while consumed < samples.len() {
        frame += 1;
        let target = ((frame as f64 * per_frame).round() as usize).min(samples.len());
        if let Some(chunk) = samples.get(consumed..target) {
            analyzer.push(chunk);
        }
        consumed = target;
        let latest = analyzer.analyze(dt);
        on_frame(frame as f32 * dt, latest);
    }
}
