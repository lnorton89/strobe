// ============================================================================
// Module:       strobe_remote::perf
// Description:  The live performance feed (D567): one sample a second of the
//               host's frame timing, sent to desktop controllers that ask.
//
// Dependencies: serde
// ============================================================================

//! The live performance feed (D567, backlog L-10).
//!
//! Once a minute the host logs a `perf:` line (D206). This is the same
//! counting over a short window, sent over the remote connection so a second
//! machine can watch a show's frame timing as it happens, without the JPEG
//! stream of the control window (which costs the host time of its own).
//!
//! - **Opt in.** A controller sends `perf_open`; only then does the host send
//!   `perf` messages on that connection. An older controller would close the
//!   connection on a message type it does not know, so nothing is sent
//!   unasked. An older host answers `perf_open` with `invalid_request`, and
//!   the controller carries on without the feed (the pattern of D513).
//! - **Desktop controllers only.** A phone's pairing is limited to show
//!   commands; a controller paired for desktop control (by approval or by the
//!   venue trust file) may watch without taking the input lease.
//! - **Latest only.** The app publishes into one slot of the bridge
//!   ([`crate::bridge::AppEnd::publish_perf`]); a connection sends the newest
//!   sample it has not sent. Nothing queues.
//! - **Fields may be added.** Unlike the client messages, a sample tolerates
//!   fields it does not know, so a later host can say more to this
//!   controller without breaking it.

use serde::{Deserialize, Serialize};

/// How often the host publishes a sample, and so the window each covers.
pub const PERIOD_MS: u32 = 1000;

/// One window of the host's frame timing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PerfSample {
    /// The milliseconds this sample covers.
    pub window_ms: u32,
    /// Output frames per second over the window.
    pub fps: f32,
    /// Output frames drawn in the window.
    pub frames: u32,
    /// Frames whose gap was over 1.5 frame budgets (the log's "late").
    pub late: u32,
    /// Refreshes missed, counted as the adaptive controller counts them
    /// (D316).
    pub missed: u32,
    /// The longest gap between two frames, in milliseconds.
    pub worst_gap_ms: f32,
    /// The scene's mean GPU milliseconds a frame; absent without a GPU timer.
    pub gpu_ms: Option<f32>,
    /// The smoothed CPU milliseconds the app spends producing a frame.
    pub cpu_ms: f32,
    /// The frame budget: one refresh period of the output's monitor.
    pub budget_ms: f32,
    /// The render scale in force at the end of the window.
    pub scale: f32,
    /// Whether adaptive resolution is moving the scale.
    pub adaptive: bool,
    /// The control window's redraws in the window.
    pub control: ControlPerf,
    /// Whether the control window is slowed to 10 Hz because making its
    /// context current waits on the output's swap (D216).
    pub control_slowed: bool,
    /// Whether the fullscreen output has lost the focus to another window
    /// over it (D453), which can make Windows compose it.
    pub output_not_foreground: bool,
}

/// The control window's redraws over one window, and where their time went.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ControlPerf {
    /// Redraws in the window.
    pub redraws: u32,
    /// Mean milliseconds a redraw took, swap included.
    pub mean_ms: f32,
    /// The longest redraw, in milliseconds.
    pub worst_ms: f32,
    /// Redraws the event loop ran because Windows sent no paint (D290).
    pub unpainted: u32,
    /// Mean milliseconds making the context current.
    pub make_current_ms: f32,
    /// Mean milliseconds of egui's layout.
    pub layout_ms: f32,
    /// Mean milliseconds of egui's GL paint.
    pub paint_ms: f32,
    /// Mean milliseconds of the buffer swap.
    pub swap_ms: f32,
}

/// `value`, or 0 if it is not finite: JSON has no NaN or infinity, and a
/// sample that cannot be written would drop the connection.
fn finite(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

impl PerfSample {
    /// This sample with every float made finite ([`finite`]), as the bridge
    /// stores it.
    pub fn sanitized(self) -> Self {
        Self {
            fps: finite(self.fps),
            worst_gap_ms: finite(self.worst_gap_ms),
            gpu_ms: self.gpu_ms.filter(|ms| ms.is_finite()),
            cpu_ms: finite(self.cpu_ms),
            budget_ms: finite(self.budget_ms),
            scale: finite(self.scale),
            control: ControlPerf {
                mean_ms: finite(self.control.mean_ms),
                worst_ms: finite(self.control.worst_ms),
                make_current_ms: finite(self.control.make_current_ms),
                layout_ms: finite(self.control.layout_ms),
                paint_ms: finite(self.control.paint_ms),
                swap_ms: finite(self.control.swap_ms),
                ..self.control
            },
            ..self
        }
    }

    /// The CSV header row for [`Self::csv_row`].
    pub const CSV_HEADER: &'static str = "seconds,window_ms,fps,frames,late,missed,worst_gap_ms,\
         gpu_ms,cpu_ms,budget_ms,scale,adaptive,control_redraws,control_mean_ms,control_worst_ms,\
         control_unpainted,make_current_ms,layout_ms,paint_ms,swap_ms,control_slowed,\
         output_not_foreground";

    /// One CSV row, `seconds` being the watcher's time since it started.
    pub fn csv_row(&self, seconds: f64) -> String {
        let c = &self.control;
        format!(
            "{seconds:.1},{},{:.2},{},{},{},{:.2},{},{:.3},{:.2},{:.3},{},{},{:.3},{:.2},{},{:.3},\
             {:.3},{:.3},{:.3},{},{}",
            self.window_ms,
            self.fps,
            self.frames,
            self.late,
            self.missed,
            self.worst_gap_ms,
            self.gpu_ms.map_or(String::new(), |ms| format!("{ms:.3}")),
            self.cpu_ms,
            self.budget_ms,
            self.scale,
            u8::from(self.adaptive),
            c.redraws,
            c.mean_ms,
            c.worst_ms,
            c.unpainted,
            c.make_current_ms,
            c.layout_ms,
            c.paint_ms,
            c.swap_ms,
            u8::from(self.control_slowed),
            u8::from(self.output_not_foreground),
        )
    }

    /// One line for a person, in the order and words of the log's `perf:`
    /// line, so the two read alike.
    pub fn line(&self) -> String {
        let c = &self.control;
        let gpu = self
            .gpu_ms
            .map_or_else(|| "n/a".to_string(), |ms| format!("{ms:.2} ms"));
        let mut line = format!(
            "{:.1} fps, missed {}, {} late of {} frames, worst gap {:.1} ms, gpu {gpu}, cpu \
             {:.2} ms, scale {:.2}{}, control {} redraws {:.2} ms (worst {:.1} ms, {} unpainted): \
             make-current {:.2}, layout {:.2}, paint {:.2}, swap {:.2} ms",
            self.fps,
            self.missed,
            self.late,
            self.frames,
            self.worst_gap_ms,
            self.cpu_ms,
            self.scale,
            if self.adaptive { " (adaptive)" } else { "" },
            c.redraws,
            c.mean_ms,
            c.worst_ms,
            c.unpainted,
            c.make_current_ms,
            c.layout_ms,
            c.paint_ms,
            c.swap_ms,
        );
        if self.control_slowed {
            line.push_str(", control slowed");
        }
        if self.output_not_foreground {
            line.push_str(", output not foreground");
        }
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn a_sample_with_no_finite_number_is_still_written() -> Result<()> {
        let sample = PerfSample {
            fps: f32::NAN,
            gpu_ms: Some(f32::INFINITY),
            control: ControlPerf {
                swap_ms: f32::NEG_INFINITY,
                redraws: 3,
                ..ControlPerf::default()
            },
            frames: 60,
            ..PerfSample::default()
        }
        .sanitized();
        let json = serde_json::to_string(&sample)?;
        assert!(json.contains(r#""fps":0.0"#), "{json}");
        assert!(json.contains(r#""gpu_ms":null"#), "{json}");
        assert_eq!(sample.control.redraws, 3, "counts are kept");
        assert_eq!(sample.frames, 60, "and so are frames");
        Ok(())
    }

    #[test]
    fn a_sample_reads_fields_a_later_host_adds() -> Result<()> {
        let text = serde_json::to_string(&PerfSample::default())?;
        let later = text.replacen('{', r#"{"pass_gpu_ms":[1.0,2.0],"#, 1);
        let read: PerfSample = serde_json::from_str(&later)?;
        assert_eq!(read, PerfSample::default(), "the unknown field is ignored");
        Ok(())
    }

    #[test]
    fn the_csv_row_has_a_cell_per_header_column() {
        let header = PerfSample::CSV_HEADER.split(',').count();
        for sample in [
            PerfSample::default(),
            PerfSample {
                gpu_ms: Some(2.5),
                adaptive: true,
                ..PerfSample::default()
            },
        ] {
            let row = sample.csv_row(12.0);
            assert_eq!(row.split(',').count(), header, "{row}");
        }
    }

    #[test]
    fn the_line_reads_like_the_logs() {
        let line = PerfSample {
            fps: 59.94,
            frames: 60,
            gpu_ms: Some(2.1),
            scale: 1.0,
            control_slowed: true,
            ..PerfSample::default()
        }
        .line();
        assert!(
            line.starts_with("59.9 fps, missed 0, 0 late of 60 frames"),
            "{line}"
        );
        assert!(line.contains("gpu 2.10 ms"), "{line}");
        assert!(line.ends_with(", control slowed"), "{line}");
    }
}
