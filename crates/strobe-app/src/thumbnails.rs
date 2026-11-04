// ============================================================================
// Module:       strobe_app::thumbnails
// Description:  Thumbnails made by rendering scenes offscreen: which scenes
//               want one, the queue of them, and how long each is rendered
//               before its frame is taken.
//
// Dependencies: std (collections, path); crate::library
// ============================================================================

//! Thumbnails rendered for scenes that have none (D547).
//!
//! The browser shows a scene's `IMAGE_PATH` (D212), and "Use frame as
//! picture" takes one from the live frame (D222). This makes one without
//! the operator: the scene is loaded at the thumbnail's size
//! ([`crate::snapshot::SIZE`]), run for [`RENDER_SECONDS`] of scene time so
//! feedback buffers and slow fades have filled in, and its last frame is
//! written by [`crate::snapshot::save`], as the live capture is.
//!
//! The Library's thumbnail button queues every scene that wants one
//! ([`wanting`]); a finished Shadertoy import queues the new scene. The app
//! works the [`Queue`] one scene at a time, [`FRAMES_PER_STEP`] frames a
//! step, so the output keeps drawing in between; a scene's compile still
//! blocks for its step, as a prerender's does (D542).

use crate::library::{Entry, Source};
use std::collections::VecDeque;
use std::path::PathBuf;

/// The scene time rendered before the frame is taken, in seconds.
pub const RENDER_SECONDS: f32 = 3.0;

/// The time step of a thumbnail render: 30 frames a second of scene time.
pub const DT: f32 = 1.0 / 30.0;

/// Frames rendered per step of the app's loop.
pub const FRAMES_PER_STEP: u32 = 15;

/// Frames rendered before the frame is taken.
pub fn frames() -> u32 {
    (RENDER_SECONDS / DT).round() as u32
}

/// The scenes in `entries` that want a thumbnail: one Strobe may write
/// (not inside Synesthesia's install) whose `IMAGE_PATH` names no file.
pub fn wanting(entries: &[Entry]) -> Vec<PathBuf> {
    entries
        .iter()
        .filter(|entry| writable(entry.source))
        .filter(|entry| !entry.thumbnail.as_ref().is_some_and(|path| path.is_file()))
        .map(|entry| entry.dir.clone())
        .collect()
}

/// Whether Strobe may write a scene from `source`.
fn writable(source: Source) -> bool {
    match source {
        Source::Own | Source::Folder | Source::SynesthesiaFolder => true,
        Source::Synesthesia => false,
    }
}

/// Scene folders waiting for a thumbnail, and how far the run has got.
#[derive(Debug, Default)]
pub struct Queue {
    /// Folders still to render, in order.
    pending: VecDeque<PathBuf>,
    /// Folders handed out by [`Queue::take_next`] and finished since the queue
    /// was last empty.
    done: usize,
    /// The folder being rendered now.
    current: Option<PathBuf>,
}

impl Queue {
    /// Queues `dirs`, skipping any queued or being rendered already.
    /// Returns how many were added.
    pub fn add(&mut self, dirs: impl IntoIterator<Item = PathBuf>) -> usize {
        let mut added = 0;
        for dir in dirs {
            if self.current.as_ref() == Some(&dir) || self.pending.contains(&dir) {
                continue;
            }
            self.pending.push_back(dir);
            added += 1;
        }
        added
    }

    /// The next folder to render, now the current one.
    pub fn take_next(&mut self) -> Option<PathBuf> {
        let next = self.pending.pop_front();
        self.current.clone_from(&next);
        next
    }

    /// The current folder is finished, picture or not. When nothing is
    /// left, the count starts over for the next run.
    pub fn finish(&mut self) {
        self.current = None;
        self.done += 1;
        if self.pending.is_empty() {
            self.done = 0;
        }
    }

    /// `(finished, total)` while a run is going, for the browser.
    pub fn progress(&self) -> Option<(usize, usize)> {
        let busy = usize::from(self.current.is_some());
        let total = self.done + busy + self.pending.len();
        (total > 0).then_some((self.done, total))
    }

    /// Whether a folder has been handed out and not finished.
    pub fn is_rendering(&self) -> bool {
        self.current.is_some()
    }

    /// Whether a folder is being rendered or waits.
    pub fn is_busy(&self) -> bool {
        self.current.is_some() || !self.pending.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(name: &str, source: Source, thumbnail: Option<PathBuf>) -> Entry {
        let mut entry = Entry::new(name.to_string(), PathBuf::from(name)).from_source(source);
        entry.thumbnail = thumbnail;
        entry
    }

    #[test]
    fn scenes_without_a_picture_that_strobe_may_write_want_one() -> anyhow::Result<()> {
        let scratch = crate::library::fake::Scratch::new("thumbs-wanting")?;
        let picture = scratch.0.join("thumbnail.jpg");
        std::fs::write(&picture, b"jpeg")?;
        let entries = [
            entry("none", Source::Own, None),
            entry("has", Source::Folder, Some(picture)),
            entry("gone", Source::Folder, Some(scratch.0.join("missing.jpg"))),
            entry("shipped", Source::Synesthesia, None),
            entry("theirs", Source::SynesthesiaFolder, None),
        ];
        assert_eq!(
            wanting(&entries),
            [
                PathBuf::from("none"),
                PathBuf::from("gone"),
                PathBuf::from("theirs")
            ],
            "no picture or a missing one, never inside Synesthesia's install"
        );
        Ok(())
    }

    #[test]
    fn the_queue_skips_repeats_and_counts_a_run() -> anyhow::Result<()> {
        let mut queue = Queue::default();
        assert_eq!(queue.progress(), None, "idle");
        let added = queue.add([PathBuf::from("a"), PathBuf::from("b"), PathBuf::from("a")]);
        assert_eq!(added, 2, "a repeat is not queued twice");
        assert_eq!(queue.take_next(), Some(PathBuf::from("a")), "in order");
        assert_eq!(queue.add([PathBuf::from("a")]), 0, "nor the one rendering");
        assert_eq!(queue.progress(), Some((0, 2)), "one rendering, one waiting");
        queue.finish();
        assert_eq!(queue.progress(), Some((1, 2)), "one done");
        assert_eq!(queue.take_next(), Some(PathBuf::from("b")), "the next");
        queue.finish();
        assert!(!queue.is_busy() && queue.progress().is_none(), "done");
        queue.add([PathBuf::from("c")]);
        assert_eq!(queue.progress(), Some((0, 1)), "a new run counts afresh");
        assert_eq!(frames(), 90, "three seconds at 30 frames a second");
        Ok(())
    }
}
