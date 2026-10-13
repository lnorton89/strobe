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
    ui_ms: f32,
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

    /// Records the time one control-window egui pass took (building and
    /// laying out every widget, not the GL paint), so a UI that has become expensive shows up in the
    /// diagnostics instead of as an unexplained dropped frame.
    pub fn record_ui(&mut self, took: Duration) {
        let ms = took.as_secs_f32() * 1000.0;
        if self.ui_ms == 0.0 {
            self.ui_ms = ms;
        } else {
            self.ui_ms += (ms - self.ui_ms) * SMOOTHING;
        }
    }

    /// Smoothed milliseconds an egui layout pass takes.
    pub fn ui_ms(&self) -> f32 {
        self.ui_ms
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

/// How long one [`PerfWindow`] summarises.
pub const PERF_WINDOW: Duration = Duration::from_secs(60);

/// A frame counts as late when its interval is this many frame budgets.
const LATE_FACTOR: f32 = 1.5;

/// A minute of frame timings, summarised for the log.
///
/// TORNADO T10 ran a fullscreen show at 48 to 56 fps with the GPU busy for
/// under 3 ms a frame, and nothing in the log said where the time went.
/// One line a minute (frame rate, late frames, the worst gap, GPU time and
/// the control window's whole redraw, swap included) settles that kind of
/// question from a soak's log alone. Recording is a few additions; the
/// line is formatted once a minute, never per frame.
#[derive(Debug, Default)]
pub struct PerfWindow {
    started: Option<Instant>,
    last_frame: Option<Instant>,
    frames: u32,
    late: u32,
    worst_ms: f32,
    gpu_sum: f32,
    gpu_frames: u32,
    control_frames: u32,
    control_sum: f32,
    control_worst: f32,
    /// Per part of a control redraw, in [`ControlTimes`]' order: summed
    /// milliseconds.
    part_sums: [f32; 4],
}

/// Where one control-window redraw spent its time (TORNADO T12: the swap
/// itself returned in 0.03 ms while the redraw took 17.5 ms, so the wait is
/// elsewhere; this says where).
#[derive(Clone, Copy, Debug, Default)]
pub struct ControlTimes {
    /// The whole redraw.
    pub total: Duration,
    /// Making the context current on the control window's surface.
    pub make_current: Duration,
    /// The egui pass: building and laying out the widgets.
    pub layout: Duration,
    /// egui's GL paint, up to the swap.
    pub paint: Duration,
    /// The buffer swap.
    pub swap: Duration,
}

impl PerfWindow {
    /// Records a scene frame starting at `now`, against a frame budget of
    /// `budget_ms`, with the GPU time last measured.
    pub fn frame(&mut self, now: Instant, budget_ms: f32, gpu_ms: Option<f32>) {
        self.started.get_or_insert(now);
        if let Some(previous) = self.last_frame {
            let ms = now.duration_since(previous).as_secs_f32() * 1000.0;
            self.worst_ms = self.worst_ms.max(ms);
            if ms > budget_ms * LATE_FACTOR {
                self.late += 1;
            }
        }
        self.last_frame = Some(now);
        self.frames += 1;
        if let Some(gpu) = gpu_ms.filter(|g| g.is_finite()) {
            self.gpu_sum += gpu;
            self.gpu_frames += 1;
        }
    }

    /// Records one control-window redraw and where its time went.
    pub fn control(&mut self, times: ControlTimes) {
        let ms = |d: Duration| d.as_secs_f32() * 1000.0;
        self.control_frames += 1;
        self.control_sum += ms(times.total);
        self.control_worst = self.control_worst.max(ms(times.total));
        for (sum, part) in self.part_sums.iter_mut().zip([
            times.make_current,
            times.layout,
            times.paint,
            times.swap,
        ]) {
            *sum += ms(part);
        }
    }

    /// The summary line when [`PERF_WINDOW`] has passed since the first
    /// frame recorded, and the window starts over (keeping the last frame's
    /// time so the next interval is measured across the boundary).
    pub fn take_due(&mut self, now: Instant) -> Option<String> {
        let started = self.started?;
        let span = now.duration_since(started).as_secs_f32();
        if span < PERF_WINDOW.as_secs_f32() {
            return None;
        }
        let mean = |sum: f32, n: u32| if n > 0 { sum / n as f32 } else { 0.0 };
        let line = format!(
            "perf: {:.1} fps over {:.0} s, {} late of {} frames, worst gap {:.1} ms, \
             gpu {:.2} ms, control {} redraws {:.2} ms (worst {:.1} ms): make-current {:.2}, \
             layout {:.2}, paint {:.2}, swap {:.2} ms",
            self.frames as f32 / span,
            span,
            self.late,
            self.frames,
            self.worst_ms,
            mean(self.gpu_sum, self.gpu_frames),
            self.control_frames,
            mean(self.control_sum, self.control_frames),
            self.control_worst,
            mean(self.part_sums[0], self.control_frames),
            mean(self.part_sums[1], self.control_frames),
            mean(self.part_sums[2], self.control_frames),
            mean(self.part_sums[3], self.control_frames),
        );
        *self = Self {
            started: Some(now),
            last_frame: self.last_frame,
            ..Self::default()
        };
        Some(line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_minute_is_summarised_once_with_late_frames_counted() {
        let mut window = PerfWindow::default();
        let start = Instant::now();
        let mut at = start;
        for i in 0..3600_u32 {
            // Every tenth frame misses a vertical blank.
            let gap = if i % 10 == 9 { 33_333 } else { 16_667 };
            at += Duration::from_micros(gap);
            window.frame(at, 16.67, Some(2.0));
            if i % 2 == 0 {
                window.control(ControlTimes {
                    total: Duration::from_millis(3),
                    make_current: Duration::from_micros(500),
                    layout: Duration::from_micros(1000),
                    paint: Duration::from_micros(500),
                    swap: Duration::from_millis(1),
                });
            }
            if i < 3000 {
                assert!(window.take_due(at).is_none(), "not a minute yet at {i}");
            }
        }
        let line = window
            .take_due(at + Duration::from_secs(1))
            .unwrap_or_default();
        assert!(line.contains("360 late of 3600"), "{line}");
        assert!(line.contains("worst gap 33.3 ms"), "{line}");
        assert!(line.contains("gpu 2.00 ms"), "{line}");
        assert!(line.contains("control 1800 redraws 3.00 ms"), "{line}");
        assert!(
            line.contains("make-current 0.50, layout 1.00, paint 0.50, swap 1.00 ms"),
            "{line}"
        );
        assert!(
            window.take_due(at + Duration::from_secs(2)).is_none(),
            "the window started over"
        );
    }

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
    fn the_ui_pass_time_is_smoothed_like_the_others() {
        let mut stats = FrameStats::default();
        assert_eq!(stats.ui_ms(), 0.0, "nothing measured yet");
        stats.record_ui(Duration::from_millis(4));
        assert!((stats.ui_ms() - 4.0).abs() < 1e-3, "first sample");
        stats.record_ui(Duration::from_millis(14));
        assert!(
            stats.ui_ms() > 4.0 && stats.ui_ms() < 14.0,
            "{}",
            stats.ui_ms()
        );
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
