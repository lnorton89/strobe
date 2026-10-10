// ============================================================================
// Module:       strobe_app::show::resume
// Description:  Saving the playlist's resume point every few seconds on a
//               thread of its own, and reading it back at start-up.
//
// Dependencies: strobe-playlist (ResumePoint); std::thread, std::sync::mpsc
// ============================================================================

//! The resume file.
//!
//! The show saves where it is every [`SAVE_EVERY`] seconds so that a crash,
//! a power cut or a restart picks up on the same cue at the same offset
//! (PLAN section 7.4). Writing is file I/O, and the frame loop does not
//! block on file I/O, so the write happens on a thread:
//!
//! - [`SaveTimer`] decides *when*: it counts the frame's `dt` and says
//!   "now" once every five seconds, and only if the point has changed
//!   since the last one sent (a held cue's point does not change, and a
//!   disk that is asked for the same bytes every five seconds for hours is
//!   a disk being worn for nothing).
//! - [`ResumeWriter`] does the writing: the frame loop sends a
//!   [`ResumePoint`] down a channel (no I/O, no blocking), the thread keeps
//!   only the newest one it has been sent, writes it atomically
//!   (`ResumePoint::save` writes a temporary file and renames it over the
//!   target), and logs a failure once instead of once a write.
//!
//! There is one resume file, `resume.toml` in the data directory. The point
//! carries the playlist's identity, so a point left by another playlist is
//! refused when it is loaded ([`super::start`]) rather than applied; the
//! next save then replaces it.
//!
//! [`ResumeWriter::finish`] sends nothing new and joins the thread, so the
//! point sent last is on disk before the process exits.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::thread::JoinHandle;
use strobe_playlist::ResumePoint;

/// Seconds between saves.
pub const SAVE_EVERY: f64 = 5.0;

/// The resume file inside the data directory.
pub fn path(data_dir: &Path) -> PathBuf {
    data_dir.join("resume.toml")
}

/// What the frame loop can ask the writer to do.
#[derive(Debug)]
enum Message {
    /// Write this point.
    Save(ResumePoint),
    /// Delete the file: the show is over and has nothing to resume.
    Clear,
}

/// The writer thread and the way to reach it.
#[derive(Debug)]
pub struct ResumeWriter {
    sender: Option<Sender<Message>>,
    thread: Option<JoinHandle<()>>,
}

impl ResumeWriter {
    /// Starts the thread, writing to `path`. If the thread cannot start,
    /// saving is off and the show plays on; a warning says so.
    pub fn start(path: PathBuf) -> Self {
        let (sender, receiver) = channel::<Message>();
        let spawned = std::thread::Builder::new()
            .name("strobe-resume".to_string())
            .spawn(move || run(&path, &receiver));
        match spawned {
            Ok(thread) => Self {
                sender: Some(sender),
                thread: Some(thread),
            },
            Err(error) => {
                log::warn!(
                    "the resume writer could not start ({error}); the position will not be saved"
                );
                Self {
                    sender: None,
                    thread: None,
                }
            }
        }
    }

    /// Hands the thread a point to write. Never blocks and never touches
    /// the disk.
    pub fn save(&self, point: ResumePoint) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(Message::Save(point));
        }
    }

    /// Asks the thread to delete the file.
    pub fn clear(&self) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(Message::Clear);
        }
    }

    /// Stops the thread after it has written everything sent so far.
    pub fn finish(mut self) {
        self.sender = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The thread: wait for a message, fold whatever else is waiting into it,
/// do the last thing asked.
fn run(path: &Path, receiver: &Receiver<Message>) {
    let mut reported = false;
    while let Ok(first) = receiver.recv() {
        let mut latest = first;
        while let Ok(next) = receiver.try_recv() {
            latest = next;
        }
        let result = match latest {
            Message::Save(point) => point.save(path).map_err(|e| e.to_string()),
            Message::Clear => match std::fs::remove_file(path) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(format!("{}: {e}", path.display())),
            },
        };
        if let Err(error) = result
            && !reported
        {
            reported = true;
            log::warn!("could not write the resume file (reported once): {error}");
        }
    }
}

/// Reads the saved point, if there is one. A missing file is not a
/// problem; an unreadable one is logged and ignored.
pub fn load(path: &Path) -> Option<ResumePoint> {
    if !path.exists() {
        return None;
    }
    match ResumePoint::load(path) {
        Ok(point) => Some(point),
        Err(error) => {
            log::warn!("ignoring the resume file: {error}");
            None
        }
    }
}

/// Decides when to send a point to the writer.
#[derive(Debug)]
pub struct SaveTimer {
    since: f64,
    last: Option<ResumePoint>,
}

impl SaveTimer {
    /// A timer that has not fired. The first point is due after
    /// [`SAVE_EVERY`] seconds, so a show that is switched off within five
    /// seconds of starting keeps the previous run's point.
    pub fn new() -> Self {
        Self {
            since: 0.0,
            last: None,
        }
    }

    /// Counts `dt` seconds and returns the point to save if one is due:
    /// five seconds have passed and `now` differs from the last one sent.
    pub fn tick(&mut self, dt: f64, now: Option<ResumePoint>) -> Option<ResumePoint> {
        if dt.is_finite() && dt > 0.0 {
            self.since += dt;
        }
        // 300 steps of 1/60 sum to just under 5: allow for the rounding.
        if self.since + 1e-6 < SAVE_EVERY {
            return None;
        }
        self.since = 0.0;
        let point = now?;
        if self.last == Some(point) {
            return None;
        }
        self.last = Some(point);
        Some(point)
    }

    /// Forgets what was last sent, so the next due point is sent even if it
    /// is the same (after a different playlist replaced this one).
    pub fn reset(&mut self) {
        self.since = 0.0;
        self.last = None;
    }
}

impl Default for SaveTimer {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, anyhow};

    fn point(cue: usize, offset: f64) -> ResumePoint {
        ResumePoint {
            identity: 0x1d0b_8b7d_3c5a_6f21,
            section: 0,
            cue,
            pass: 0,
            offset,
        }
    }

    /// A scratch directory under the system temp dir, removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Result<Self> {
            let dir =
                std::env::temp_dir().join(format!("strobe-resume-{tag}-{}", std::process::id()));
            std::fs::create_dir_all(&dir)?;
            Ok(Self(dir))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn a_point_is_due_every_five_seconds_and_only_if_it_changed() {
        let mut timer = SaveTimer::new();
        let mut sent = Vec::new();
        // 20 seconds at 60 Hz with the cue offset following the clock.
        for frame in 0..(20 * 60) {
            let offset = f64::from(frame) / 60.0;
            if let Some(p) = timer.tick(1.0 / 60.0, Some(point(0, offset))) {
                sent.push(p.offset.round() as u32);
            }
        }
        assert_eq!(sent, vec![5, 10, 15, 20], "one point per five seconds");
    }

    #[test]
    fn a_held_cue_does_not_rewrite_the_same_point() {
        let mut timer = SaveTimer::new();
        let held = point(2, 12.0);
        let mut sent = 0;
        for _ in 0..(60 * 60) {
            if timer.tick(1.0 / 60.0, Some(held)).is_some() {
                sent += 1;
            }
        }
        assert_eq!(sent, 1, "a minute of hold writes the point once");
        timer.reset();
        assert!(
            (0..(5 * 60)).any(|_| timer.tick(1.0 / 60.0, Some(held)).is_some()),
            "after a reset the same point is due again"
        );
    }

    #[test]
    fn nothing_playing_sends_nothing_and_nonsense_time_is_ignored() {
        let mut timer = SaveTimer::new();
        assert!(timer.tick(10.0, None).is_none(), "no point to save");
        assert!(
            timer.tick(f64::NAN, Some(point(0, 1.0))).is_none(),
            "NaN dt adds nothing"
        );
        assert!(
            timer.tick(-5.0, Some(point(0, 1.0))).is_none(),
            "negative dt adds nothing"
        );
    }

    #[test]
    fn the_writer_keeps_the_newest_point_and_finish_flushes_it() -> Result<()> {
        let scratch = Scratch::new("writer")?;
        let file = path(&scratch.0);
        let writer = ResumeWriter::start(file.clone());
        for cue in 0..50 {
            writer.save(point(cue, 1.5));
        }
        writer.finish();
        let loaded = load(&file).ok_or_else(|| anyhow!("no resume file was written"))?;
        assert_eq!(
            loaded,
            point(49, 1.5),
            "the last point sent is the one on disk"
        );
        Ok(())
    }

    #[test]
    fn clearing_removes_the_file_and_a_missing_file_loads_as_nothing() -> Result<()> {
        let scratch = Scratch::new("clear")?;
        let file = path(&scratch.0);
        assert!(load(&file).is_none(), "nothing there yet");
        let writer = ResumeWriter::start(file.clone());
        writer.save(point(1, 2.0));
        writer.clear();
        writer.finish();
        assert!(!file.exists(), "the file was cleared");
        Ok(())
    }

    #[test]
    fn a_damaged_file_is_ignored_rather_than_fatal() -> Result<()> {
        let scratch = Scratch::new("damaged")?;
        let file = path(&scratch.0);
        std::fs::write(&file, "identity = \"not hex\"\nsection = 0\ncue = 0\n")?;
        assert!(load(&file).is_none(), "a bad file reads as no file");
        Ok(())
    }
}
