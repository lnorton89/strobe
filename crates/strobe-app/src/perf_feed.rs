// ============================================================================
// Module:       strobe_app::perf_feed
// Description:  The controller's side of the live performance feed (D567):
//               the host's latest samples, a short history, and the strip
//               the desktop viewer draws from them.
//
// Dependencies: egui; strobe-remote (perf::PerfSample)
// ============================================================================

//! What a desktop controller keeps of a host's performance feed, and how it
//! shows it.
//!
//! The worker thread hands each `perf` message to [`Feed::take`]; the viewer
//! copies the feed out when its revision moves and draws [`strip`]: one line
//! of numbers in the words of the host's `perf:` log line, and a trend of the
//! last [`HISTORY`] samples' frame rate against the host's refresh rate.

use std::collections::VecDeque;
use std::time::{Duration, Instant};
use strobe_remote::perf::PerfSample;

/// How many samples the trend shows: a minute, at one a second.
pub const HISTORY: usize = 60;

/// How long after the newest sample the strip says the numbers are old. The
/// host sends one a second; a frame loop that stalls sends none, while its
/// network thread keeps the connection up (D567).
pub const STALE_AFTER: Duration = Duration::from_secs(3);

/// A host's performance samples as one connection received them.
#[derive(Clone, Debug, Default)]
pub struct Feed {
    /// The host's revision of the newest sample, and the sample.
    latest: Option<(u64, PerfSample)>,
    /// Each sample's frame rate as a share of the host's refresh rate
    /// (1.0 is every refresh drawn), oldest first, at most [`HISTORY`].
    trend: VecDeque<f32>,
    /// Moves with every sample taken, for a viewer copying the feed out.
    taken: u64,
    /// When the newest sample arrived.
    at: Option<Instant>,
}

impl Feed {
    /// Takes one sample. A sample under the revision already held is the
    /// same one again and is ignored; any other is new (a restarted host
    /// counts from 1 again).
    pub fn take(&mut self, revision: u64, sample: PerfSample) {
        if self.latest.is_some_and(|(held, _)| held == revision) {
            return;
        }
        self.latest = Some((revision, sample));
        if self.trend.len() == HISTORY {
            self.trend.pop_front();
        }
        self.trend.push_back(share_of_refresh(&sample));
        self.taken += 1;
        self.at = Some(Instant::now());
    }

    /// Forgets the samples, for a new session with a host that may have
    /// restarted: its trend starts afresh rather than joining the last run's.
    pub fn restart(&mut self) {
        self.latest = None;
        self.trend.clear();
        self.at = None;
        self.taken += 1;
    }

    /// How long ago the newest sample arrived, at `now`.
    pub fn age(&self, now: Instant) -> Option<Duration> {
        self.at.map(|at| now.saturating_duration_since(at))
    }

    /// The newest sample, once one has arrived.
    pub fn latest(&self) -> Option<&PerfSample> {
        self.latest.as_ref().map(|(_, sample)| sample)
    }

    /// How many samples were taken: a viewer copies the feed when it moves.
    pub fn taken(&self) -> u64 {
        self.taken
    }

    /// The trend, oldest first.
    pub fn trend(&self) -> impl ExactSizeIterator<Item = f32> + '_ {
        self.trend.iter().copied()
    }
}

/// `sample`'s frame rate as a share of the host's refresh rate, at most 1.
fn share_of_refresh(sample: &PerfSample) -> f32 {
    if sample.budget_ms <= 0.0 {
        return 0.0;
    }
    let refresh = 1000.0 / sample.budget_ms;
    (sample.fps / refresh).clamp(0.0, 1.0)
}

/// The strip's one line: the frame rate, late and missed frames, GPU and CPU
/// time, the scale and the control window's cost, with the two warnings the
/// host's log gives their own lines.
pub fn summary(sample: &PerfSample) -> String {
    let gpu = sample
        .gpu_ms
        .map_or_else(|| "n/a".to_string(), |ms| format!("{ms:.2} ms"));
    let mut text = format!(
        "Host: {:.1} fps \u{b7} {} late \u{b7} {} missed \u{b7} worst gap {:.1} ms \u{b7} GPU {gpu} \
         \u{b7} CPU {:.2} ms \u{b7} scale {:.2}{} \u{b7} control window {:.2} ms",
        sample.fps,
        sample.late,
        sample.missed,
        sample.worst_gap_ms,
        sample.cpu_ms,
        sample.scale,
        if sample.adaptive { " (adaptive)" } else { "" },
        sample.control.mean_ms,
    );
    if sample.control_slowed {
        text.push_str(" \u{b7} control window slowed");
    }
    if sample.output_not_foreground {
        text.push_str(" \u{b7} output not in the foreground");
    }
    text
}

/// Whether `sample` shows the show suffering: a frame missed its refresh,
/// or one of the two states that cost refreshes is on.
pub fn troubled(sample: &PerfSample) -> bool {
    sample.late > 0 || sample.missed > 0 || sample.control_slowed || sample.output_not_foreground
}

/// The strip's height in points: one line of text and the trend beside it.
const TREND_SIZE: egui::Vec2 = egui::vec2(120.0, 16.0);

/// Draws the strip for `feed` with `text` (its [`summary`], made when the
/// sample arrived): the trend, then the line, in the warning colour while
/// the show is [`troubled`] or the numbers are older than [`STALE_AFTER`],
/// which it then says. Nothing while no sample has arrived.
pub fn strip(ui: &mut egui::Ui, feed: &Feed, text: &str) {
    let Some(sample) = feed.latest() else {
        return;
    };
    let stale = feed
        .age(Instant::now())
        .filter(|age| *age >= STALE_AFTER)
        .map(|age| age.as_secs());
    // A sample that stops coming changes nothing else on screen: look again
    // in a second, so the strip says so.
    ui.ctx().request_repaint_after(Duration::from_secs(1));
    let colour = if troubled(sample) || stale.is_some() {
        ui.visuals().warn_fg_color
    } else {
        ui.visuals().weak_text_color()
    };
    ui.horizontal(|ui| {
        let (rect, response) = ui.allocate_exact_size(TREND_SIZE, egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_stroke(
            rect,
            2.0,
            ui.visuals().widgets.noninteractive.bg_stroke,
            egui::StrokeKind::Inside,
        );
        let count = feed.trend().len();
        if count > 1 {
            let step = rect.width() / (HISTORY - 1) as f32;
            let start = rect.right() - step * (count - 1) as f32;
            let points: Vec<egui::Pos2> = feed
                .trend()
                .enumerate()
                .map(|(i, share)| {
                    egui::pos2(
                        start + step * i as f32,
                        rect.bottom() - 1.0 - share * (rect.height() - 2.0),
                    )
                })
                .collect();
            painter.add(egui::Shape::line(points, egui::Stroke::new(1.5, colour)));
        }
        response.on_hover_text(
            "The host's frame rate over the last minute, against its refresh rate (top). \
             Watching costs the host one small message a second; the picture of its window \
             costs it more.",
        );
        ui.label(egui::RichText::new(text).small().color(colour));
        if let Some(seconds) = stale {
            ui.label(
                egui::RichText::new(format!("\u{b7} no new numbers for {seconds} s"))
                    .small()
                    .color(colour),
            );
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use strobe_remote::perf::ControlPerf;

    fn sample(fps: f32) -> PerfSample {
        PerfSample {
            fps,
            budget_ms: 1000.0 / 60.0,
            gpu_ms: Some(2.1),
            scale: 1.0,
            ..PerfSample::default()
        }
    }

    #[test]
    fn the_trend_keeps_a_minute_and_ignores_a_sample_seen_twice() {
        let mut feed = Feed::default();
        for revision in 1..=(HISTORY as u64 + 10) {
            feed.take(revision, sample(30.0));
        }
        assert_eq!(feed.trend().len(), HISTORY, "a minute and no more");
        let taken = feed.taken();
        feed.take(HISTORY as u64 + 10, sample(60.0));
        assert_eq!(feed.taken(), taken, "the same revision is not new");
        feed.take(1, sample(60.0));
        assert_eq!(feed.taken(), taken + 1, "a restarted host's first is");
        assert!(
            feed.trend()
                .last()
                .is_some_and(|share| (share - 1.0).abs() < 1e-3),
            "every refresh drawn"
        );
        assert!(
            feed.trend()
                .take(HISTORY - 1)
                .all(|share| (share - 0.5).abs() < 1e-3),
            "half the refreshes before it"
        );
    }

    #[test]
    fn the_summary_names_the_numbers_and_the_warnings() {
        let calm = sample(59.9);
        let text = summary(&calm);
        assert!(text.starts_with("Host: 59.9 fps"), "{text}");
        assert!(text.contains("GPU 2.10 ms"), "{text}");
        assert!(!troubled(&calm), "a clean second is calm");
        let bad = PerfSample {
            late: 2,
            control_slowed: true,
            output_not_foreground: true,
            gpu_ms: None,
            control: ControlPerf {
                mean_ms: 16.4,
                ..ControlPerf::default()
            },
            ..calm
        };
        let text = summary(&bad);
        assert!(troubled(&bad), "late frames trouble it");
        assert!(text.contains("GPU n/a"), "{text}");
        assert!(text.contains("control window 16.40 ms"), "{text}");
        assert!(
            text.ends_with("control window slowed \u{b7} output not in the foreground"),
            "{text}"
        );
    }

    #[test]
    fn a_restart_forgets_the_run_before_and_still_counts_as_news() {
        let mut feed = Feed::default();
        feed.take(7, sample(60.0));
        assert!(
            feed.age(Instant::now()).is_some(),
            "an age once a sample came"
        );
        let taken = feed.taken();
        feed.restart();
        assert!(feed.latest().is_none(), "no sample from the last run");
        assert_eq!(feed.trend().len(), 0, "and no trend");
        assert!(feed.age(Instant::now()).is_none(), "and no age");
        assert!(feed.taken() > taken, "a viewer copies the cleared feed");
        feed.take(7, sample(60.0));
        assert!(
            feed.latest().is_some(),
            "a restarted host's sample under an old revision is taken"
        );
    }

    #[test]
    fn a_sample_with_no_budget_does_not_divide_by_zero() {
        let mut feed = Feed::default();
        feed.take(1, PerfSample::default());
        assert_eq!(feed.trend().last(), Some(0.0), "nothing drawn");
    }
}
