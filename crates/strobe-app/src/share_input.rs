// ============================================================================
// Module:       strobe_app::share_input
// Description:  Spout and NDI input as capture backends: listing senders and
//               sources, and a stream that reads one into a reused buffer,
//               paced, holding the last picture while the sender idles.
//
// Dependencies: strobe-capture (Backend, Stream), strobe-output (the Spout and
//               NDI receivers)
// ============================================================================

//! Spout and NDI behind `syn_Media` (D575).
//!
//! `strobe-capture`'s supervisor already does what a live source needs (a
//! reader thread that reopens by name, a watchdog, a failure said once), so
//! Spout and NDI are two more [`Backend`]s for it, beside Media Foundation.
//! The receivers themselves are `strobe-output`'s; this module adapts them.
//!
//! **Pacing.** A Spout read is a poll: [`SpoutStream::read`] asks for a new
//! frame every [`POLL`] and hands one over at most every [`MIN_GAP`], so a
//! sender that does not count frames (every read looks new) is read at most
//! 125 times a second, not as fast as the thread can spin. An NDI read waits
//! on NDI for the next frame.
//!
//! **Idle senders.** A sender that is still there but sends nothing new (a
//! paused app) keeps its last picture on screen: the stream hands the same
//! picture over again every [`HOLD`], so the watchdog, which counts a read
//! busy past its stall time as a stopped source, never fires on a sender
//! that is merely idle. A sender that never sends a first picture within
//! [`FIRST_PICTURE`] is a failure, said once. A sender that closes is a
//! failure at once.
//!
//! **Causes.** A missing library or runtime is [`Cause::Unsupported`];
//! anything a read says is classed by its wording, as a camera's is ("not
//! found", "end of stream").

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};
use strobe_capture::{Backend, Cause, PixelFormat, Problem, RawFrame, Stream};
use strobe_output::{NdiFinder, NdiReceiver, Order, Received, SpoutReceiver};

/// How often a Spout stream asks for a new frame while none has come.
pub const POLL: Duration = Duration::from_millis(4);

/// The least time between two pictures handed over.
pub const MIN_GAP: Duration = Duration::from_millis(8);

/// How often an idle sender's last picture is handed over again: well inside
/// the watchdog's stall time (5 s).
pub const HOLD: Duration = Duration::from_secs(1);

/// How long a stream waits for a sender's first picture before failing.
pub const FIRST_PICTURE: Duration = Duration::from_secs(3);

/// How long the first NDI listing waits for discovery's first answers.
const NDI_DISCOVERY: Duration = Duration::from_millis(2000);

/// How long one NDI read waits for a frame before looking at the clock.
const NDI_WAIT: Duration = Duration::from_millis(250);

/// The Spout backend.
pub fn spout() -> Arc<dyn Backend> {
    Arc::new(Spout)
}

/// The NDI backend.
pub fn ndi() -> Arc<dyn Backend> {
    Arc::new(Ndi)
}

/// The Spout senders running now, for a listing.
///
/// # Errors
///
/// No Spout library, worded for a person.
pub fn spout_list() -> Result<Vec<String>, String> {
    strobe_output::spout_senders().map_err(|problem| problem.to_string())
}

/// The NDI sources found so far, for a listing; the first call waits a
/// moment for discovery.
///
/// # Errors
///
/// No NDI runtime, worded for a person.
pub fn ndi_list() -> Result<Vec<String>, String> {
    finder()?
        .sources(NDI_DISCOVERY)
        .map_err(|problem| problem.to_string())
}

/// NDI's discovery, made once for the process: it searches in the background
/// from then on. A runtime that failed to load is not retried until the next
/// run (installing NDI Tools needs a restart).
fn finder() -> Result<&'static NdiFinder, String> {
    static FINDER: OnceLock<Result<NdiFinder, String>> = OnceLock::new();
    FINDER
        .get_or_init(|| NdiFinder::new().map_err(|problem| problem.to_string()))
        .as_ref()
        .map_err(Clone::clone)
}

fn unsupported(detail: String) -> Problem {
    Problem {
        cause: Cause::Unsupported,
        detail,
    }
}

/// Spout through its library.
#[derive(Debug, Clone, Copy)]
struct Spout;

impl Backend for Spout {
    fn list(&self) -> Result<Vec<String>, Problem> {
        spout_list().map_err(unsupported)
    }

    fn open(&self, name: &str) -> Result<Box<dyn Stream>, Problem> {
        let receiver = SpoutReceiver::open(name).map_err(|problem| unsupported(problem.note))?;
        Ok(Box::new(SpoutStream {
            receiver,
            held: Held::new(),
        }))
    }
}

/// NDI through its runtime.
#[derive(Debug, Clone, Copy)]
struct Ndi;

impl Backend for Ndi {
    fn list(&self) -> Result<Vec<String>, Problem> {
        ndi_list().map_err(unsupported)
    }

    fn open(&self, name: &str) -> Result<Box<dyn Stream>, Problem> {
        let finder = finder().map_err(unsupported)?;
        let receiver =
            NdiReceiver::open(finder, name).map_err(|problem| Problem::from_text(problem.note))?;
        Ok(Box::new(NdiStream {
            receiver,
            held: Held::new(),
        }))
    }
}

/// The picture a stream last handed over, in its own reused buffer, and the
/// clock that paces it.
#[derive(Debug)]
pub struct Held {
    /// The receiver writes here; a picture is handed over from here.
    pub pixels: Vec<u8>,
    /// The last picture received, while `pixels` still holds it.
    pub last: Option<Received>,
    /// When a picture was last handed over, or the stream opened.
    pub since: Instant,
}

/// What a stream does next after one read of its receiver.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// Hand this picture over.
    Hand(Received),
    /// Nothing to hand over yet; read again.
    Wait,
    /// No first picture in time.
    Starved,
}

impl Held {
    /// Nothing held, the clock starting now.
    pub fn new() -> Self {
        Self {
            pixels: Vec::new(),
            last: None,
            since: Instant::now(),
        }
    }

    /// The pacing policy, after one read that brought `fresh` (or nothing),
    /// at `now`: a new picture is handed over; an idle sender's last one
    /// again every [`HOLD`]; no first picture within [`FIRST_PICTURE`] is
    /// starved.
    pub fn next(&mut self, fresh: Option<Received>, now: Instant) -> Next {
        let waited = now.saturating_duration_since(self.since);
        if let Some(fresh) = fresh {
            self.last = Some(fresh);
            self.since = now;
            return Next::Hand(fresh);
        }
        // The buffer may have been resized for a new size since; only a
        // picture it still holds whole is handed over again.
        let held = self.last.filter(|last| last.len() == self.pixels.len());
        match held {
            Some(last) if waited >= HOLD => {
                self.since = now;
                Next::Hand(last)
            }
            Some(_) => Next::Wait,
            None if waited >= FIRST_PICTURE => Next::Starved,
            None => Next::Wait,
        }
    }

    /// How long to sleep before the next read, so pictures are at least
    /// [`MIN_GAP`] apart.
    pub fn gap(&self, now: Instant) -> Duration {
        MIN_GAP.saturating_sub(now.saturating_duration_since(self.since))
    }

    /// The held picture as the capture thread reads it.
    fn frame(&self, received: Received) -> RawFrame<'_> {
        RawFrame {
            format: match received.order {
                Order::Rgba => PixelFormat::Rgba,
                Order::Bgra => PixelFormat::Bgra,
            },
            width: received.width,
            height: received.height,
            data: &self.pixels,
        }
    }
}

impl Default for Held {
    fn default() -> Self {
        Self::new()
    }
}

fn starved() -> Problem {
    Problem {
        cause: Cause::Stalled,
        detail: format!(
            "connected, but no picture came in {:.0} s",
            FIRST_PICTURE.as_secs_f32()
        ),
    }
}

/// One Spout sender, read on the capture thread that opened it.
#[derive(Debug)]
struct SpoutStream {
    receiver: SpoutReceiver,
    held: Held,
}

impl Stream for SpoutStream {
    fn describe(&self) -> String {
        "Spout, RGBA read through Spout's own GL context".to_string()
    }

    fn read(&mut self) -> Result<RawFrame<'_>, Problem> {
        std::thread::sleep(self.held.gap(Instant::now()));
        loop {
            let fresh = self
                .receiver
                .receive(&mut self.held.pixels)
                .map_err(|problem| Problem::from_text(problem.note))?;
            match self.held.next(fresh, Instant::now()) {
                Next::Hand(received) => return Ok(self.held.frame(received)),
                Next::Wait => std::thread::sleep(POLL),
                Next::Starved => return Err(starved()),
            }
        }
    }
}

/// One NDI source, read on the capture thread that opened it.
#[derive(Debug)]
struct NdiStream {
    receiver: NdiReceiver,
    held: Held,
}

impl Stream for NdiStream {
    fn describe(&self) -> String {
        "NDI, RGBA decoded by NDI".to_string()
    }

    fn read(&mut self) -> Result<RawFrame<'_>, Problem> {
        loop {
            let fresh = self
                .receiver
                .receive(&mut self.held.pixels, NDI_WAIT)
                .map_err(|problem| Problem::from_text(problem.note))?;
            match self.held.next(fresh, Instant::now()) {
                Next::Hand(received) => return Ok(self.held.frame(received)),
                Next::Wait => {}
                Next::Starved => return Err(starved()),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(width: u32) -> Received {
        Received {
            width,
            height: 1,
            order: Order::Rgba,
        }
    }

    #[test]
    fn a_new_picture_is_handed_over_at_once() {
        let mut held = Held::new();
        let now = held.since;
        held.pixels.resize(8, 0);
        assert_eq!(
            held.next(Some(picture(2)), now),
            Next::Hand(picture(2)),
            "fresh"
        );
        assert_eq!(held.since, now, "the clock restarts");
    }

    #[test]
    fn an_idle_sender_keeps_its_last_picture() {
        let mut held = Held::new();
        let start = held.since;
        held.pixels.resize(8, 0);
        held.next(Some(picture(2)), start);
        assert_eq!(
            held.next(None, start + HOLD / 2),
            Next::Wait,
            "not yet again"
        );
        assert_eq!(
            held.next(None, start + HOLD),
            Next::Hand(picture(2)),
            "the same picture again, so the watchdog sees a live source"
        );
        assert_eq!(
            held.next(None, start + HOLD + FIRST_PICTURE * 10),
            Next::Hand(picture(2)),
            "idle for as long as it likes: never starved once it has sent"
        );
    }

    #[test]
    fn a_resized_buffer_is_not_handed_over_as_the_old_picture() {
        let mut held = Held::new();
        let start = held.since;
        held.pixels.resize(8, 0);
        held.next(Some(picture(2)), start);
        // The sender changed size: the receiver resized the buffer and has
        // not filled it yet.
        held.pixels.resize(16, 0);
        assert_eq!(
            held.next(None, start + HOLD * 2),
            Next::Wait,
            "nothing whole to show"
        );
    }

    #[test]
    fn no_first_picture_in_time_is_starved() {
        let mut held = Held::new();
        let start = held.since;
        assert_eq!(
            held.next(None, start + FIRST_PICTURE / 2),
            Next::Wait,
            "waiting"
        );
        assert_eq!(
            held.next(None, start + FIRST_PICTURE),
            Next::Starved,
            "said, once, by the supervisor"
        );
        assert_eq!(starved().cause, Cause::Stalled, "a stalled source");
    }

    #[test]
    fn pictures_are_paced_at_least_the_gap_apart() {
        let mut held = Held::new();
        let start = held.since;
        held.next(Some(picture(1)), start);
        assert_eq!(held.gap(start), MIN_GAP, "just handed one over");
        assert_eq!(held.gap(start + MIN_GAP), Duration::ZERO, "free to read");
        assert!(
            FIRST_PICTURE < Duration::from_secs(5) && HOLD < FIRST_PICTURE,
            "both inside the watchdog's 5 s stall"
        );
    }

    #[test]
    fn a_held_picture_reads_in_its_own_order() {
        let mut held = Held::new();
        held.pixels = vec![1, 2, 3, 4];
        let frame = held.frame(Received {
            width: 1,
            height: 1,
            order: Order::Bgra,
        });
        assert_eq!(
            (frame.format, frame.data.len()),
            (PixelFormat::Bgra, 4),
            "BGRA, from the held buffer"
        );
    }
}
