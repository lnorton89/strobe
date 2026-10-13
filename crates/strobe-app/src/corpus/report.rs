// ============================================================================
// Module:       strobe_app::corpus::report
// Description:  Turns a run's records into report.json (machine-readable) and
//               report.md (totals, failures by category, slowest scenes).
//
// Dependencies: serde, serde_json; crate::corpus
// ============================================================================

//! The two output files.
//!
//! `report.json` is the record of truth: settings, the driver each dialect
//! ran on (so a result can be repeated and compared), and one [`Record`]
//! per scene and dialect. `report.md` is derived from the same data and is
//! for reading: totals per dialect, failures grouped by cause with counts
//! and scene names, scenes that fail in only one dialect, the ten slowest,
//! and the scenes whose last frame was blank.
//!
//! A report names scenes and quotes one error line each. When the corpus
//! is third-party, keep both files under `target/`.

use super::cause;
use super::{Dialect, Driver, Record, Settings, Status};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// How many scenes the "slowest" tables list.
const SLOWEST: usize = 10;

/// The longest error text a table cell holds.
const CELL: usize = 140;

/// A finished run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Report {
    /// This tool and its version.
    pub tool: String,
    /// The folder that was measured.
    pub library: String,
    /// What each scene was measured with.
    pub settings: Settings,
    /// The per-scene time budget, in seconds.
    pub timeout_secs: u64,
    /// The driver each dialect ran on.
    pub drivers: Vec<Driver>,
    /// One record per scene and dialect.
    pub records: Vec<Record>,
}

impl Report {
    /// A report for `records`, stamped with this build's version.
    pub fn new(
        library: &str,
        settings: Settings,
        timeout_secs: u64,
        drivers: Vec<Driver>,
        records: Vec<Record>,
    ) -> Self {
        Self {
            tool: format!("strobe-corpus {}", env!("CARGO_PKG_VERSION")),
            library: library.to_string(),
            settings,
            timeout_secs,
            drivers,
            records,
        }
    }

    /// `report.json`.
    ///
    /// # Errors
    ///
    /// Only if serialisation fails, which these types cannot cause.
    pub fn to_json(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// The dialects that have at least one record, in report order.
    fn dialects(&self) -> Vec<Dialect> {
        Dialect::ALL
            .into_iter()
            .filter(|dialect| self.records.iter().any(|r| r.dialect == *dialect))
            .collect()
    }

    fn of(&self, dialect: Dialect) -> impl Iterator<Item = &Record> {
        self.records.iter().filter(move |r| r.dialect == dialect)
    }

    /// `report.md`.
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        let dialects = self.dialects();
        out.push_str("# Corpus compatibility report\n\n");
        out.push_str(&format!(
            "- tool: {}\n- library: `{}`\n- per scene: {} frame(s) at {}x{}, {} s budget\n\n",
            self.tool,
            self.library,
            self.settings.frames,
            self.settings.size[0],
            self.settings.size[1],
            self.timeout_secs,
        ));
        self.drivers_section(&mut out);
        self.totals_section(&mut out, &dialects);
        for dialect in &dialects {
            self.failures_section(&mut out, *dialect);
        }
        self.one_dialect_section(&mut out, &dialects);
        self.slowest_section(&mut out);
        self.blank_section(&mut out, &dialects);
        for dialect in &dialects {
            self.notes_section(&mut out, *dialect);
        }
        self.agreement_section(&mut out, &dialects);
        self.every_failure_section(&mut out);
        out
    }

    fn drivers_section(&self, out: &mut String) {
        out.push_str(
            "## Drivers\n\n| dialect | renderer | version | context |\n|---|---|---|---|\n",
        );
        for driver in &self.drivers {
            out.push_str(&format!(
                "| {} | {} | {} | {} |\n",
                driver.dialect.label(),
                cell(&driver.renderer),
                cell(&driver.version),
                cell(&driver.context),
            ));
        }
        out.push('\n');
    }

    fn totals_section(&self, out: &mut String, dialects: &[Dialect]) {
        out.push_str("## Totals\n\n| status |");
        for dialect in dialects {
            out.push_str(&format!(" {} |", dialect.label()));
        }
        out.push_str("\n|---|");
        for _ in dialects {
            out.push_str("---:|");
        }
        out.push('\n');
        for status in Status::ALL {
            out.push_str(&format!("| {} |", status.label()));
            for dialect in dialects {
                let count = self.of(*dialect).filter(|r| r.status == status).count();
                out.push_str(&format!(" {count} |"));
            }
            out.push('\n');
        }
        out.push_str("| **scenes** |");
        for dialect in dialects {
            out.push_str(&format!(" **{}** |", self.of(*dialect).count()));
        }
        out.push_str("\n\n");
    }

    fn failures_section(&self, out: &mut String, dialect: Dialect) {
        let failing: Vec<&Record> = self
            .of(dialect)
            .filter(|r| r.status != Status::Ok)
            .collect();
        out.push_str(&format!(
            "## Failures, {} ({} of {})\n\n",
            dialect.label(),
            failing.len(),
            self.of(dialect).count()
        ));
        if failing.is_empty() {
            out.push_str("None.\n\n");
            return;
        }
        // kind -> label -> scenes
        let mut kinds: BTreeMap<String, BTreeMap<String, Vec<&str>>> = BTreeMap::new();
        for record in &failing {
            let label = record
                .cause
                .clone()
                .unwrap_or_else(|| record.status.label().to_string());
            kinds
                .entry(cause::kind(&label).to_string())
                .or_default()
                .entry(label)
                .or_default()
                .push(&record.scene);
        }
        let mut ordered: Vec<(&String, &BTreeMap<String, Vec<&str>>)> = kinds.iter().collect();
        ordered.sort_by_key(|(kind, labels)| {
            (
                std::cmp::Reverse(labels.values().map(Vec::len).sum::<usize>()),
                (*kind).clone(),
            )
        });
        out.push_str("| category | scenes |\n|---|---:|\n");
        for (kind, labels) in &ordered {
            out.push_str(&format!(
                "| {kind} | {} |\n",
                labels.values().map(Vec::len).sum::<usize>()
            ));
        }
        out.push('\n');
        for (kind, labels) in ordered {
            out.push_str(&format!(
                "### {kind} ({})\n\n",
                labels.values().map(Vec::len).sum::<usize>()
            ));
            let mut by_size: Vec<(&String, &Vec<&str>)> = labels.iter().collect();
            by_size
                .sort_by_key(|(label, scenes)| (std::cmp::Reverse(scenes.len()), (*label).clone()));
            for (label, scenes) in by_size {
                out.push_str(&format!(
                    "- `{label}` ({}): {}\n",
                    scenes.len(),
                    scenes.join(", ")
                ));
            }
            out.push('\n');
        }
    }

    fn one_dialect_section(&self, out: &mut String, dialects: &[Dialect]) {
        if dialects.len() < 2 {
            return;
        }
        out.push_str("## Fails in only one dialect\n\n");
        let mut any = false;
        for (works, fails) in [
            (Dialect::Desktop, Dialect::Gles),
            (Dialect::Gles, Dialect::Desktop),
        ] {
            let scenes: Vec<&str> = self
                .of(fails)
                .filter(|r| r.status != Status::Ok)
                .filter(|r| {
                    self.of(works)
                        .any(|other| other.scene == r.scene && other.status == Status::Ok)
                })
                .map(|r| r.scene.as_str())
                .collect();
            if !scenes.is_empty() {
                any = true;
                out.push_str(&format!(
                    "- ok on {}, failing on {} ({}): {}\n",
                    works.label(),
                    fails.label(),
                    scenes.len(),
                    scenes.join(", ")
                ));
            }
        }
        if !any {
            out.push_str("None: every scene has the same status in both dialects.\n");
        }
        out.push('\n');
    }

    fn slowest_section(&self, out: &mut String) {
        for (title, pick) in [
            (
                "Slowest per frame",
                (|r: &Record| r.frame_ms) as fn(&Record) -> Option<f64>,
            ),
            ("Slowest to compile", |r: &Record| r.compile_ms),
        ] {
            let mut timed: Vec<(&Record, f64)> = self
                .records
                .iter()
                .filter_map(|r| pick(r).map(|ms| (r, ms)))
                .collect();
            timed.sort_by(|a, b| b.1.total_cmp(&a.1));
            out.push_str(&format!(
                "## {title} (top {SLOWEST})\n\n| scene | dialect | compile ms | first frame ms | frame ms |\n|---|---|---:|---:|---:|\n"
            ));
            for (record, _) in timed.into_iter().take(SLOWEST) {
                out.push_str(&format!(
                    "| {} | {} | {} | {} | {} |\n",
                    record.scene,
                    record.dialect.label(),
                    ms(record.compile_ms),
                    ms(record.first_frame_ms),
                    ms(record.frame_ms),
                ));
            }
            out.push('\n');
        }
    }

    fn blank_section(&self, out: &mut String, dialects: &[Dialect]) {
        out.push_str("## Blank final frames (scenes that compiled)\n\n");
        let mut any = false;
        for dialect in dialects {
            for (label, black) in [("all black", true), ("all white", false)] {
                let scenes: Vec<&str> = self
                    .of(*dialect)
                    .filter(|r| {
                        r.status == Status::Ok
                            && if black {
                                r.all_black == Some(true)
                            } else {
                                r.all_white == Some(true)
                            }
                    })
                    .map(|r| r.scene.as_str())
                    .collect();
                if !scenes.is_empty() {
                    any = true;
                    out.push_str(&format!(
                        "- {}, {label} ({}): {}\n",
                        dialect.label(),
                        scenes.len(),
                        scenes.join(", ")
                    ));
                }
            }
        }
        if !any {
            out.push_str("None.\n");
        }
        out.push('\n');
    }

    fn notes_section(&self, out: &mut String, dialect: Dialect) {
        let mut by_label: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for record in self.of(dialect) {
            for note in &record.notes {
                by_label
                    .entry(note.cause.as_str())
                    .or_default()
                    .push(record.scene.as_str());
            }
        }
        if by_label.is_empty() {
            return;
        }
        out.push_str(&format!(
            "## Loaded with problems, {}\n\nThese scenes drew, but something they ask for \
             was missing (an image that would not decode, a script error that was not a \
             fault).\n\n",
            dialect.label()
        ));
        for (label, mut scenes) in by_label {
            scenes.sort_unstable();
            scenes.dedup();
            out.push_str(&format!(
                "- `{label}` ({}): {}\n",
                scenes.len(),
                scenes.join(", ")
            ));
        }
        out.push('\n');
    }

    fn agreement_section(&self, out: &mut String, dialects: &[Dialect]) {
        if dialects.len() < 2 {
            return;
        }
        let mut same = 0;
        let mut differ: Vec<&str> = Vec::new();
        for desktop in self.of(Dialect::Desktop).filter(|r| r.status == Status::Ok) {
            let Some(gles) = self
                .of(Dialect::Gles)
                .find(|r| r.scene == desktop.scene && r.status == Status::Ok)
            else {
                continue;
            };
            if desktop.checksum == gles.checksum {
                same += 1;
            } else {
                differ.push(desktop.scene.as_str());
            }
        }
        out.push_str(&format!(
            "## Dialect agreement\n\nOf the scenes that drew in both dialects, {same} drew a \
             bit-identical last frame and {} did not. Differences are expected from float \
             precision and rounding; this is a pointer for the image diff in Stage 4, not a \
             verdict.\n\n",
            differ.len()
        ));
        if !differ.is_empty() {
            out.push_str(&format!("Differing: {}\n\n", differ.join(", ")));
        }
    }

    fn every_failure_section(&self, out: &mut String) {
        out.push_str("## Every failure\n\n| scene | dialect | status | cause | first error line |\n|---|---|---|---|---|\n");
        let mut failing: Vec<&Record> = self
            .records
            .iter()
            .filter(|r| r.status != Status::Ok)
            .collect();
        failing.sort_by(|a, b| (&a.scene, a.dialect).cmp(&(&b.scene, b.dialect)));
        for record in failing {
            out.push_str(&format!(
                "| {} | {} | {} | {} | {} |\n",
                record.scene,
                record.dialect.label(),
                record.status.label(),
                record.cause.as_deref().unwrap_or(""),
                cell(record.error.as_deref().unwrap_or("")),
            ));
        }
        out.push('\n');
    }
}

/// A table cell: one line, no pipes, capped.
fn cell(text: &str) -> String {
    let flat: String = text
        .chars()
        .map(|c| if c == '|' { '/' } else { c })
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if flat.chars().count() > CELL {
        let mut cut: String = flat.chars().take(CELL).collect();
        cut.push('…');
        cut
    } else {
        flat
    }
}

/// Milliseconds for a table, or a dash.
fn ms(value: Option<f64>) -> String {
    value.map_or_else(|| "-".to_string(), |v| format!("{v:.1}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(scene: &str, dialect: Dialect, frame_ms: f64) -> Record {
        Record {
            compile_ms: Some(frame_ms * 10.0),
            first_frame_ms: Some(frame_ms * 2.0),
            frame_ms: Some(frame_ms),
            all_black: Some(false),
            all_white: Some(false),
            checksum: Some("00".to_string()),
            ..Record::new(scene, dialect)
        }
    }

    fn sample() -> Report {
        let mut records = vec![
            ok("fast", Dialect::Desktop, 1.0),
            ok("fast", Dialect::Gles, 1.5),
            ok("slow", Dialect::Desktop, 9.0),
            Record::failed(
                "slow",
                Dialect::Gles,
                Status::CompileError,
                "ERROR: 0:3: 'clamp' : no matching overloaded function found",
            ),
            Record::failed(
                "ghost",
                Dialect::Desktop,
                Status::CompileError,
                "0:1(1): error: 'ghostName' undeclared",
            ),
            Record::failed(
                "ghost",
                Dialect::Gles,
                Status::CompileError,
                "ERROR: 0:1: 'ghostName' : undeclared identifier",
            ),
            Record::failed("hang", Dialect::Desktop, Status::Timeout, "no result"),
        ];
        if let Some(first) = records.first_mut() {
            first.all_black = Some(true);
        }
        Report::new(
            "lib",
            Settings::default(),
            60,
            vec![Driver {
                dialect: Dialect::Desktop,
                renderer: "llvmpipe | test".to_string(),
                version: "4.5".to_string(),
                vendor: "Mesa".to_string(),
                context: "headless".to_string(),
            }],
            records,
        )
    }

    #[test]
    fn json_round_trips_with_one_record_per_scene_and_dialect() -> anyhow::Result<()> {
        let report = sample();
        let back: Report = serde_json::from_str(&report.to_json()?)?;
        assert_eq!(back, report, "the report survives its own JSON");
        assert!(
            report.to_json()?.contains("\"status\": \"compile_error\""),
            "statuses are snake_case words"
        );
        Ok(())
    }

    #[test]
    fn the_markdown_has_totals_categories_and_slowest() {
        let md = sample().to_markdown();
        assert!(md.contains("| ok | 2 | 1 |"), "totals per dialect:\n{md}");
        assert!(
            md.contains("| timeout | 1 | 0 |"),
            "the timeout is counted:\n{md}"
        );
        assert!(
            md.contains("- `undeclared:ghostName` (1): ghost"),
            "failures grouped by cause with scene names:\n{md}"
        );
        assert!(
            md.contains("- `implicit_conversion` (1): slow"),
            "a built-in with no match is the conversion family:\n{md}"
        );
        assert!(
            md.contains("ok on desktop, failing on gles (1): slow"),
            "a one-dialect failure is called out:\n{md}"
        );
        assert!(
            md.contains("| slow | desktop | 90.0 | 18.0 | 9.0 |"),
            "the slowest table:\n{md}"
        );
        assert!(
            md.contains("desktop, all black (1): fast"),
            "blank frames are listed:\n{md}"
        );
        assert!(
            !md.contains("llvmpipe | test"),
            "pipes are escaped in cells:\n{md}"
        );
    }

    #[test]
    fn the_slowest_table_stops_at_ten() {
        let records: Vec<Record> = (0..15)
            .map(|i| {
                ok(
                    &format!("scene{i:02}"),
                    Dialect::Desktop,
                    f64::from(i) + 1.0,
                )
            })
            .collect();
        let md = Report::new("lib", Settings::default(), 60, Vec::new(), records).to_markdown();
        let section = md.split("## Slowest per frame").nth(1).unwrap_or_default();
        let table = section.split("## ").next().unwrap_or_default();
        assert_eq!(
            table.matches("| scene").count(),
            SLOWEST + 1,
            "ten rows and the header:\n{table}"
        );
        assert!(table.contains("scene14"), "the slowest is first:\n{table}");
    }

    #[test]
    fn cells_are_one_line_and_capped() {
        assert_eq!(cell("a|b\nc"), "a/b c", "pipe and newline");
        assert_eq!(cell(&"x".repeat(500)).chars().count(), CELL + 1, "cap");
    }
}
