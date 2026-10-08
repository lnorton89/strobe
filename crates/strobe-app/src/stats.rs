// ============================================================================
// Module:       strobe_app::stats
// Description:  Frame-time readout: smoothed CPU time per frame and the
//               frame rate the output window is actually achieving.
//
// Dependencies: std only
// ============================================================================

//! Frame statistics.
//!
//! "CPU ms" is the time the app spends producing a frame, up to the point
//! it hands the finished frame to the driver; it excludes the wait for
//! vertical blank inside the swap. It is a cost figure, not a latency
//! one. The frame rate comes from the interval between frame starts, so a
//! vsync wait does count there, which is what makes it the rate the
//! audience sees. Both are exponential moving averages.

use std::time::{Duration, Instant};

/// Weight of the newest sample in the averages.
const SMOOTHING: f32 = 0.1;

/// Longest frame interval reported as `dt`; a stall (window drag, a long
/// compile) must not fling every smoothed control across its range.
const MAX_DT: f32 = 0.1;

/// Smoothed frame timings.
#[derive(Debug, Default)]
pub struct FrameStats {
    last_start: Option<Instant>,
    interval_ms: f32,
    cpu_ms: f32,
}

impl FrameStats {
    /// Marks the start of a frame at `now` and returns the seconds since
    /// the previous one, clamped.
    pub fn begin(&mut self, now: Instant) -> f32 {
        let dt = match self.last_start {
            Some(previous) => now.duration_since(previous).as_secs_f32(),
            None => 1.0 / 60.0,
        };
        self.last_start = Some(now);
        if self.interval_ms == 0.0 {
            self.interval_ms = dt * 1000.0;
        } else {
            self.interval_ms += (dt * 1000.0 - self.interval_ms) * SMOOTHING;
        }
        dt.min(MAX_DT)
    }

    /// Records the CPU time one frame took.
    pub fn finish(&mut self, cpu: Duration) {
        let ms = cpu.as_secs_f32() * 1000.0;
        if self.cpu_ms == 0.0 {
            self.cpu_ms = ms;
        } else {
            self.cpu_ms += (ms - self.cpu_ms) * SMOOTHING;
        }
    }

    /// Smoothed CPU milliseconds per frame.
    pub fn cpu_ms(&self) -> f32 {
        self.cpu_ms
    }

    /// Smoothed frames per second.
    pub fn fps(&self) -> f32 {
        if self.interval_ms > 0.0 {
            1000.0 / self.interval_ms
        } else {
            0.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steady_sixty_hertz_reads_sixty() {
        let mut stats = FrameStats::default();
        let start = Instant::now();
        for i in 0..120_u32 {
            stats.begin(start + Duration::from_micros(16_667) * i);
        }
        assert!((stats.fps() - 60.0).abs() < 0.5, "fps was {}", stats.fps());
    }

    #[test]
    fn a_stall_is_clamped_for_the_scene_clock() {
        let mut stats = FrameStats::default();
        let start = Instant::now();
        stats.begin(start);
        let dt = stats.begin(start + Duration::from_secs(5));
        assert!((dt - MAX_DT).abs() < f32::EPSILON, "dt was {dt}");
    }
}
