// ============================================================================
// Module:       strobe_gfx::live
// Description:  A live picture source behind syn_Media, such as a webcam:
//               the one-picture mailbox between a capture thread and the
//               render thread, and the host's hook that opens a source by name.
//
// Dependencies: std::sync
// ============================================================================

//! Live sources for `syn_Media` (backlog M-04, D528).
//!
//! A capture thread [`Mailbox::put`]s each new picture; the render thread
//! [`Mailbox::take`]s the newest at most once a frame and uploads it into
//! one reused texture. A picture not taken by the time the next arrives is
//! dropped and counted, so a slow frame never queues lag behind it. The
//! render thread hands each buffer back with [`Mailbox::give_back`], so
//! neither side allocates once both have one.
//!
//! A source is named in a media request as `camera:<name>`
//! ([`strobe_scene::camera`]). The player does not know how to open one:
//! the host installs [`LiveSources`] once at start ([`install`]), and a
//! request for a camera asks it for that camera's mailbox. Without it (the
//! bench, the tests, a build with no capture) a camera reads black and says
//! so once.
//!
//! The mailbox also tells the capture side whether anyone still shows it:
//! every player showing a camera holds a clone, and the capture keeps one.
//! [`Mailbox::retire_if_unshown`] is true once only the capture's is left,
//! and the host's registry then opens a fresh one the next time the camera
//! is asked for.

use std::sync::{Arc, Mutex, PoisonError, RwLock, Weak};
use std::time::Instant;

/// What `syn_MediaType` holds while a camera shows, as SSF documents for a
/// webcam (Q22).
pub const MEDIA_TYPE_WEBCAM: f32 = 3.0;

/// One live picture.
#[derive(Debug, Default)]
pub struct LiveFrame {
    /// Width in texels.
    pub width: u32,
    /// Height in texels.
    pub height: u32,
    /// RGBA8, bottom row first (GL's order, so `syn_Media` reads upright).
    pub rgba: Vec<u8>,
    /// When the capture thread had it.
    pub captured: Option<Instant>,
}

/// Counters, for the log and the bench.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LiveStats {
    /// Pictures the source put.
    pub put: u64,
    /// Pictures replaced before the render thread took them.
    pub dropped: u64,
    /// Pictures taken.
    pub taken: u64,
}

#[derive(Debug, Default)]
struct Slot {
    fresh: Option<LiveFrame>,
    spare: Option<Vec<u8>>,
    /// Set by a failure, cleared by the next picture.
    failed: bool,
    /// Why, and what to tell the person, until a player takes it to
    /// report.
    reason: Option<(String, String)>,
    /// No one shows this any more; the capture is stopping.
    retired: bool,
    stats: LiveStats,
}

/// The one-picture hand-over between a capture thread and the render thread.
#[derive(Debug, Clone, Default)]
pub struct Mailbox(Arc<Mutex<Slot>>);

/// A [`Mailbox`] that does not keep it alive, for a host's registry.
#[derive(Debug, Clone, Default)]
pub struct WeakMailbox(Weak<Mutex<Slot>>);

impl WeakMailbox {
    /// The mailbox, if it is still alive and its capture still running.
    pub fn upgrade(&self) -> Option<Mailbox> {
        let mailbox = Mailbox(self.0.upgrade()?);
        (!mailbox.with(|slot| slot.retired)).then_some(mailbox)
    }
}

impl Mailbox {
    /// An empty mailbox.
    pub fn new() -> Self {
        Self::default()
    }

    fn with<T>(&self, f: impl FnOnce(&mut Slot) -> T) -> T {
        let mut slot = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        f(&mut slot)
    }

    /// A handle that does not keep the mailbox alive.
    pub fn downgrade(&self) -> WeakMailbox {
        WeakMailbox(Arc::downgrade(&self.0))
    }

    /// A buffer to fill, reused when the render thread has given one back.
    pub fn buffer(&self) -> Vec<u8> {
        self.with(|slot| slot.spare.take()).unwrap_or_default()
    }

    /// Offers the newest picture, replacing one not yet taken, and ends a
    /// failure.
    pub fn put(&self, frame: LiveFrame) {
        self.with(|slot| {
            slot.stats.put += 1;
            slot.failed = false;
            slot.reason = None;
            if let Some(old) = slot.fresh.replace(frame) {
                slot.stats.dropped += 1;
                slot.spare = Some(old.rgba);
            }
        });
    }

    /// Says the source stopped, why, and what the person is told (the
    /// failure record's action): the picture reads black until the next
    /// [`Self::put`]. Drops a picture not yet taken.
    pub fn fail(&self, why: String, action: String) {
        self.with(|slot| {
            slot.failed = true;
            slot.reason = Some((why, action));
            if let Some(old) = slot.fresh.take() {
                slot.spare = Some(old.rgba);
            }
        });
    }

    /// Takes the newest picture, if one arrived since the last take.
    pub fn take(&self) -> Option<LiveFrame> {
        self.with(|slot| {
            let frame = slot.fresh.take();
            if frame.is_some() {
                slot.stats.taken += 1;
            }
            frame
        })
    }

    /// Returns a buffer for the source to fill again.
    pub fn give_back(&self, rgba: Vec<u8>) {
        self.with(|slot| slot.spare = Some(rgba));
    }

    /// Whether the source is in a failure (black until its next picture).
    pub fn failed(&self) -> bool {
        self.with(|slot| slot.failed)
    }

    /// Takes why the source failed and what to say, once, for the one
    /// player that reports.
    pub fn take_reason(&self) -> Option<(String, String)> {
        self.with(|slot| slot.reason.take())
    }

    /// The counters so far.
    pub fn stats(&self) -> LiveStats {
        self.with(|slot| slot.stats)
    }

    /// For the capture side: true, and the mailbox retired for good, when
    /// this is the last clone, so no player shows it. Checked and set under
    /// the lock, so a registry that upgrades a [`WeakMailbox`] either sees
    /// it retired or keeps it alive.
    pub fn retire_if_unshown(&self) -> bool {
        self.with(|slot| {
            if !slot.retired && Arc::strong_count(&self.0) == 1 {
                slot.retired = true;
            }
            slot.retired
        })
    }
}

/// How a host opens a live source by name.
pub trait LiveSources: Send + Sync {
    /// The mailbox of the camera called `name`, starting its capture if
    /// none is running. Must return at once: the camera opens in the
    /// background, and a camera that is not there fails through the mailbox.
    fn camera(&self, name: &str) -> Mailbox;
}

static SOURCES: RwLock<Option<Arc<dyn LiveSources>>> = RwLock::new(None);

/// Installs the host's live sources, replacing any before.
pub fn install(sources: Arc<dyn LiveSources>) {
    *SOURCES.write().unwrap_or_else(PoisonError::into_inner) = Some(sources);
}

/// The installed live sources, if any.
pub(crate) fn sources() -> Option<Arc<dyn LiveSources>> {
    SOURCES
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(width: u32) -> LiveFrame {
        LiveFrame {
            width,
            height: 1,
            rgba: vec![0; width as usize * 4],
            captured: None,
        }
    }

    #[test]
    fn the_newest_picture_wins_and_buffers_come_back() {
        let mailbox = Mailbox::new();
        mailbox.put(frame(1));
        mailbox.put(frame(2));
        let taken = mailbox.take();
        assert_eq!(taken.as_ref().map(|f| f.width), Some(2), "the newest");
        assert!(mailbox.take().is_none(), "taken once");
        assert_eq!(mailbox.buffer().len(), 4, "the dropped one's buffer");
        if let Some(taken) = taken {
            mailbox.give_back(taken.rgba);
        }
        assert_eq!(mailbox.buffer().len(), 8, "the shown one's buffer");
        let stats = mailbox.stats();
        assert_eq!(
            (stats.put, stats.dropped, stats.taken),
            (2, 1, 1),
            "{stats:?}"
        );
    }

    #[test]
    fn a_failure_holds_until_the_next_picture_and_is_told_once() {
        let mailbox = Mailbox::new();
        mailbox.put(frame(1));
        mailbox.fail("unplugged".to_string(), "replug it".to_string());
        assert!(mailbox.failed(), "black now");
        assert!(mailbox.take().is_none(), "the stale picture went");
        assert_eq!(
            mailbox.take_reason(),
            Some(("unplugged".to_string(), "replug it".to_string())),
            "told"
        );
        assert_eq!(mailbox.take_reason(), None, "once");
        assert!(mailbox.failed(), "still black");
        mailbox.put(frame(1));
        assert!(!mailbox.failed(), "a picture ends it");
    }

    #[test]
    fn a_mailbox_retires_when_no_one_else_holds_it() {
        let capture = Mailbox::new();
        let registry = capture.downgrade();
        let player = registry.upgrade();
        assert!(player.is_some(), "alive while the capture runs");
        assert!(!capture.retire_if_unshown(), "a player shows it");
        drop(player);
        assert!(capture.retire_if_unshown(), "no one shows it");
        assert!(
            registry.upgrade().is_none(),
            "retired: a fresh one is needed"
        );
        drop(capture);
        assert!(registry.upgrade().is_none(), "and gone");
    }
}
