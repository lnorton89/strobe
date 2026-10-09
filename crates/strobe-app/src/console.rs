// ============================================================================
// Module:       strobe_app::console
// Description:  The console's lines: a bounded, level-tagged view of
//               strobe-diag's log ring, refreshed only when it changes.
//
// Dependencies: strobe-diag (logger ring)
// ============================================================================

//! The console model.
//!
//! The console is not a second log. It is a *view* of the ring buffer
//! `strobe-diag` keeps for exactly this purpose, so what the person sees in
//! the bottom panel is what the session file and a crash report contain,
//! including structured failures (`SHADER` / `scene=` / `phase=` blocks),
//! script output and GL driver messages. The app writes to it by logging.
//!
//! [`Console::refresh`] compares the logger's record count with the value it
//! last saw and copies the tail only when it moved, so an idle console costs
//! one atomic load per control-window repaint. The control window is not on
//! the output frame's path in any case.

/// Most lines kept; older ones fall off the front.
const CAPACITY: usize = 200;

/// How serious a line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Detail for developers (`debug` and `trace` records).
    Debug,
    /// Progress and results.
    Info,
    /// Something is off but the show goes on.
    Warn,
    /// Something failed.
    Error,
}

/// One console line (a log record, possibly several text lines long).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// Severity.
    pub level: Level,
    /// The record as `strobe-diag` formatted it.
    pub text: String,
}

/// The level of a formatted record: its second word, as in
/// `   12.345s WARN  [main] target: message`. Session header lines carry
/// no level and read as info.
fn level_of(record: &str) -> Level {
    match record.split_whitespace().nth(1) {
        Some("ERROR") => Level::Error,
        Some("WARN") => Level::Warn,
        Some("DEBUG" | "TRACE") => Level::Debug,
        Some(_) | None => Level::Info,
    }
}

/// The console's lines, oldest first.
#[derive(Debug, Default)]
pub struct Console {
    lines: Vec<Line>,
    /// The logger's record count at the last refresh.
    seen: u64,
    /// The record count when the console was last cleared.
    hidden_before: u64,
}

impl Console {
    /// Brings the console up to date with the logger.
    pub fn refresh(&mut self) {
        let written = strobe_diag::logger::records_written();
        self.sync(written, strobe_diag::logger::recent_lines);
    }

    /// Updates from a logger that has written `written` records in total;
    /// `fetch(n)` returns its newest `n` records, oldest first.
    fn sync(&mut self, written: u64, fetch: impl FnOnce(usize) -> Vec<String>) {
        if written == self.seen {
            return;
        }
        self.seen = written;
        let visible = written.saturating_sub(self.hidden_before);
        let wanted = usize::try_from(visible).map_or(CAPACITY, |n| n.min(CAPACITY));
        self.lines = fetch(wanted)
            .into_iter()
            .map(|text| Line {
                level: level_of(&text),
                text,
            })
            .collect();
    }

    /// Hides every line logged so far. The log itself is untouched.
    pub fn clear(&mut self) {
        self.lines.clear();
        self.hidden_before = self.seen;
    }

    /// The lines, oldest first.
    pub fn lines(&self) -> impl Iterator<Item = &Line> {
        self.lines.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn records(count: usize) -> Vec<String> {
        (0..count)
            .map(|i| format!("{i:>9.3}s INFO  [main] t: line {i}"))
            .collect()
    }

    #[test]
    fn levels_are_read_from_the_record() {
        let cases = [
            ("    1.000s ERROR [main] t: x", Level::Error),
            ("    1.000s WARN  [main] t: x", Level::Warn),
            ("    1.000s INFO  [main] t: x", Level::Info),
            ("    1.000s DEBUG [main] t: x", Level::Debug),
            ("    1.000s TRACE [main] t: x", Level::Debug),
            ("=== Strobe session started 2026 ===", Level::Info),
            ("", Level::Info),
        ];
        for (record, expected) in cases {
            assert_eq!(level_of(record), expected, "record `{record}`");
        }
    }

    #[test]
    fn an_unchanged_logger_is_not_asked_again() {
        let mut console = Console::default();
        console.sync(3, records);
        let mut asked = false;
        console.sync(3, |_| {
            asked = true;
            Vec::new()
        });
        assert!(!asked, "same record count must not refetch");
        assert_eq!(console.lines().count(), 3, "lines kept");
    }

    #[test]
    fn the_console_is_bounded() {
        let mut console = Console::default();
        console.sync(1000, records);
        assert_eq!(console.lines().count(), CAPACITY, "bounded");
    }

    #[test]
    fn clearing_hides_old_records_but_not_new_ones() {
        let mut console = Console::default();
        console.sync(10, records);
        console.clear();
        assert_eq!(console.lines().count(), 0, "cleared");
        console.sync(12, records);
        assert_eq!(
            console.lines().count(),
            2,
            "only the two records logged after the clear are shown"
        );
    }
}
