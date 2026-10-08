// ============================================================================
// Module:       strobe_app::console
// Description:  The console's messages: scene load and compile results, kept
//               in a bounded ring for the bottom panel to show.
//
// Dependencies: std only
// ============================================================================

//! The console model.
//!
//! The UI only reads this; the app writes to it. Compile errors arrive
//! already mapped to `main.glsl:N` by the renderer, so a message is shown
//! exactly as it was produced.

use std::collections::VecDeque;

/// Most lines kept; older ones fall off the front.
const CAPACITY: usize = 200;

/// How serious a line is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Progress and results.
    Info,
    /// Something failed.
    Error,
}

/// One console line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// Severity.
    pub level: Level,
    /// The text, possibly several lines long.
    pub text: String,
}

/// The console's lines, oldest first.
#[derive(Debug, Default)]
pub struct Console {
    lines: VecDeque<Line>,
}

impl Console {
    /// Appends an informational line.
    pub fn info(&mut self, text: impl Into<String>) {
        let text = text.into();
        log::info!("{text}");
        self.push(Level::Info, text);
    }

    /// Appends an error.
    pub fn error(&mut self, text: impl Into<String>) {
        let text = text.into();
        log::error!("{text}");
        self.push(Level::Error, text);
    }

    fn push(&mut self, level: Level, text: String) {
        if self.lines.len() == CAPACITY {
            self.lines.pop_front();
        }
        self.lines.push_back(Line { level, text });
    }

    /// Removes every line.
    pub fn clear(&mut self) {
        self.lines.clear();
    }

    /// The lines, oldest first.
    pub fn lines(&self) -> impl Iterator<Item = &Line> {
        self.lines.iter()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_lines_fall_off_when_full() {
        let mut console = Console::default();
        for i in 0..CAPACITY + 5 {
            console.info(format!("line {i}"));
        }
        assert_eq!(console.lines().count(), CAPACITY, "bounded");
        assert_eq!(
            console.lines().next().map(|l| l.text.as_str()),
            Some("line 5"),
            "oldest dropped"
        );
    }
}
