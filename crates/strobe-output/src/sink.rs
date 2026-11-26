// ============================================================================
// Module:       strobe_output::sink
// Description:  The sender seam: one `Sink` per destination, and `Output`,
//               which owns one, sends frames to it and takes it down once
//               when it fails.
//
// Dependencies: log; crate::Frame
// ============================================================================

//! The sender seam.
//!
//! A [`Sink`] is one destination (an NDI stream, a Spout sender, a file in a
//! test). [`Output`] owns the current one, sends frames to it and, when it
//! fails, drops it and remembers why. Dropping it is the whole recovery: an
//! NDI sender that lost the runtime, or a Spout sender created before a
//! display existed, is gone from the network and a fresh one is what starts
//! it again. Building that fresh one is the host's job, because only the
//! host knows the name and the options; [`Output::reopen`] takes it.
//!
//! The rules here are the show's, not the sender's: a failure is said once
//! and never stops the frame, and a frame that goes nowhere costs a log line
//! per outage and nothing else.

use crate::Frame;
use crate::texture::SharedTexture;

/// Why a sink could not send. The wording is for a person reading the
/// console or the failure record, and the app acts on none of it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Problem {
    /// What went wrong, in a sentence.
    pub note: String,
}

impl Problem {
    /// A problem described by `note`.
    pub fn new(note: impl Into<String>) -> Self {
        Self { note: note.into() }
    }
}

impl std::fmt::Display for Problem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.note)
    }
}

impl std::error::Error for Problem {}

/// One destination for finished frames.
///
/// `send` is called on the render thread once a frame, so it must not block
/// for long and must not allocate in steady state; the backends queue a copy
/// into the sender's own storage.
pub trait Sink: Send + 'static {
    /// Hands `frame` over. An error takes the sink down
    /// ([`Output::show`] drops it), so a failure is not reported twice and
    /// is not retried against the same broken sender.
    fn send(&mut self, frame: &Frame) -> Result<(), Problem>;

    /// Hands over the renderer's texture instead of pixels, for a
    /// destination that shares GPU memory (Spout). The default refuses, so a
    /// frame-only sender is never handed a handle it cannot use.
    fn send_texture(&mut self, _texture: SharedTexture) -> Result<(), Problem> {
        Err(Problem::new(
            "this destination takes frames of pixels, not a texture",
        ))
    }

    /// Releases the destination now, on the thread that owns it.
    ///
    /// Dropping does the same, so a backend whose release has to happen on a
    /// particular thread implements this and leaves its `Drop` empty.
    fn close(&mut self) {}
}

/// What [`Output::show`] did with a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    /// The sink took it.
    Sent,
    /// There is no sink: it failed earlier, or was never opened.
    Nothing,
}

/// Whether an output is sending.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// A sink is open.
    Live,
    /// No sink: it failed, and [`Output::reopen`] has not replaced it.
    Down,
    /// Closed by the host, on purpose.
    Stopped,
}

/// One destination, with the show's failure policy around it.
pub struct Output {
    name: String,
    sink: Option<Box<dyn Sink>>,
    status: Status,
    /// Why the sink failed, kept after it is dropped.
    reason: Option<String>,
    sent: u64,
    dropped: u64,
}

impl Output {
    /// An output named `name` with no sink yet, as the show starts before
    /// the destination is chosen.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            sink: None,
            status: Status::Down,
            reason: None,
            sent: 0,
            dropped: 0,
        }
    }

    /// An output that sends to `sink` from the next frame.
    pub fn open(name: impl Into<String>, sink: Box<dyn Sink>) -> Self {
        Self {
            name: name.into(),
            sink: Some(sink),
            status: Status::Live,
            reason: None,
            sent: 0,
            dropped: 0,
        }
    }

    /// The name a receiver sees.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Whether it is sending.
    pub fn status(&self) -> Status {
        self.status
    }

    /// Why the last sink failed, if one has.
    pub fn reason(&self) -> Option<&str> {
        self.reason.as_deref()
    }

    /// Frames a sink took.
    pub fn sent(&self) -> u64 {
        self.sent
    }

    /// Frames that went nowhere after the sink was taken down, or before one
    /// was opened.
    pub fn dropped(&self) -> u64 {
        self.dropped
    }

    /// Hands one frame to the sink.
    ///
    /// A failing sink is closed, dropped and said once; the frame is
    /// dropped, not queued, so a slow or broken destination can never
    /// back up inside the show.
    pub fn show(&mut self, frame: &Frame) -> Outcome {
        let Some(sink) = self.sink.as_mut() else {
            self.dropped += 1;
            return Outcome::Nothing;
        };
        match sink.send(frame) {
            Ok(()) => {
                self.sent += 1;
                Outcome::Sent
            }
            Err(problem) => {
                self.dropped += 1;
                self.take_down(&problem.note);
                Outcome::Nothing
            }
        }
    }

    /// Hands one texture to the sink, for a destination that shares GPU
    /// memory. The same policy as [`Self::show`]: a failure takes the sink
    /// down, and a frame that cannot go is counted, never queued.
    pub fn show_texture(&mut self, texture: SharedTexture) -> Outcome {
        let Some(sink) = self.sink.as_mut() else {
            self.dropped += 1;
            return Outcome::Nothing;
        };
        match sink.send_texture(texture) {
            Ok(()) => {
                self.sent += 1;
                Outcome::Sent
            }
            Err(problem) => {
                self.dropped += 1;
                self.take_down(&problem.note);
                Outcome::Nothing
            }
        }
    }

    /// Replaces a failed or stopped sink with `sink`, so the next frame goes
    /// out again. Nothing happens while a sink is live: one sender at a time
    /// or two of the same name fight over the receiver.
    pub fn reopen(&mut self, sink: Box<dyn Sink>) -> bool {
        if self.status == Status::Live {
            return false;
        }
        self.sink = Some(sink);
        self.status = Status::Live;
        self.reason = None;
        true
    }

    /// Closes the sink because the host asked, which is not a failure: the
    /// reason stays empty and no log line is written.
    pub fn shut(&mut self) {
        if let Some(mut sink) = self.sink.take() {
            sink.close();
        }
        self.status = Status::Stopped;
        self.reason = None;
    }

    /// Closes the sink and records why.
    fn take_down(&mut self, reason: &str) {
        log::warn!(
            "output {:?} stopped: {reason}; the picture stops being shared until it is reopened (D570)",
            self.name
        );
        if let Some(mut sink) = self.sink.take() {
            sink.close();
        }
        self.status = Status::Down;
        self.reason = Some(reason.to_owned());
    }
}

impl Drop for Output {
    fn drop(&mut self) {
        if let Some(mut sink) = self.sink.take() {
            sink.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A sink that records frames, and starts failing after `fail_after`.
    struct Fake {
        sent: Arc<AtomicUsize>,
        closes: Arc<AtomicUsize>,
        fail_after: usize,
    }

    impl Sink for Fake {
        fn send(&mut self, _frame: &Frame) -> Result<(), Problem> {
            if self.sent.load(Ordering::SeqCst) >= self.fail_after {
                return Err(Problem::new("the fake said no"));
            }
            self.sent.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }

        fn close(&mut self) {
            self.closes.fetch_add(1, Ordering::SeqCst);
        }
    }

    fn frame() -> Frame {
        Frame {
            width: 1,
            height: 1,
            rgba: vec![0, 0, 0, 255],
        }
    }

    fn fake(sent: &Arc<AtomicUsize>, closes: &Arc<AtomicUsize>, fail_after: usize) -> Box<Fake> {
        Box::new(Fake {
            sent: sent.clone(),
            closes: closes.clone(),
            fail_after,
        })
    }

    #[test]
    fn frames_go_to_a_live_sink_and_are_counted() {
        let sent = Arc::new(AtomicUsize::new(0));
        let closes = Arc::new(AtomicUsize::new(0));
        let mut output = Output::open("test", fake(&sent, &closes, usize::MAX));
        for _ in 0..3 {
            assert_eq!(output.show(&frame()), Outcome::Sent, "the sink takes it");
        }
        assert_eq!(output.sent(), 3, "three frames went out");
        assert_eq!(output.dropped(), 0, "none were lost");
        assert_eq!(output.status(), Status::Live, "still live");
        assert_eq!(closes.load(Ordering::SeqCst), 0, "nothing was closed");
    }

    #[test]
    fn a_failure_takes_the_sink_down_once_and_drops_frames_after_it() {
        let sent = Arc::new(AtomicUsize::new(0));
        let closes = Arc::new(AtomicUsize::new(0));
        let mut output = Output::open("test", fake(&sent, &closes, 2));
        assert_eq!(output.show(&frame()), Outcome::Sent, "the first frame");
        assert_eq!(output.show(&frame()), Outcome::Sent, "the second");
        assert_eq!(output.show(&frame()), Outcome::Nothing, "the failure");
        assert_eq!(output.status(), Status::Down, "it is down");
        assert_eq!(
            output.reason(),
            Some("the fake said no"),
            "the reason is kept after the sink is gone"
        );
        assert_eq!(closes.load(Ordering::SeqCst), 1, "it was closed once");
        assert_eq!(output.show(&frame()), Outcome::Nothing, "nothing after");
        assert_eq!(output.dropped(), 2, "both lost frames were counted");
        assert_eq!(closes.load(Ordering::SeqCst), 1, "and never closed twice");
    }

    #[test]
    fn reopening_replaces_a_failed_sink_and_never_a_live_one() {
        let sent = Arc::new(AtomicUsize::new(0));
        let closes = Arc::new(AtomicUsize::new(0));
        let mut output = Output::new("test");
        assert_eq!(output.status(), Status::Down, "nothing opened yet");
        assert!(
            output.reopen(fake(&sent, &closes, 1)),
            "a down output takes a sink"
        );
        assert!(
            !output.reopen(fake(&sent, &closes, 1)),
            "a live one does not"
        );
        assert_eq!(output.show(&frame()), Outcome::Sent, "and it sends");
        assert_eq!(output.show(&frame()), Outcome::Nothing, "then fails");
        assert!(
            output.reopen(fake(&sent, &closes, usize::MAX)),
            "a failed one is replaced"
        );
        assert_eq!(output.reason(), None, "the old reason is gone");
        assert_eq!(output.show(&frame()), Outcome::Sent, "and pictures flow");
    }

    #[test]
    fn shutting_closes_on_purpose_and_is_not_a_failure() {
        let sent = Arc::new(AtomicUsize::new(0));
        let closes = Arc::new(AtomicUsize::new(0));
        let mut output = Output::open("test", fake(&sent, &closes, usize::MAX));
        output.shut();
        assert_eq!(output.status(), Status::Stopped, "stopped, not down");
        assert_eq!(output.reason(), None, "no failure to report");
        assert_eq!(closes.load(Ordering::SeqCst), 1, "the sink was closed");
        assert_eq!(output.show(&frame()), Outcome::Nothing, "and sends nothing");
        drop(output);
        assert_eq!(
            closes.load(Ordering::SeqCst),
            1,
            "dropping a shut output closes nothing twice"
        );
    }

    #[test]
    fn dropping_an_open_output_closes_its_sink() {
        let sent = Arc::new(AtomicUsize::new(0));
        let closes = Arc::new(AtomicUsize::new(0));
        let output = Output::open("test", fake(&sent, &closes, usize::MAX));
        drop(output);
        assert_eq!(
            closes.load(Ordering::SeqCst),
            1,
            "a sender never outlives its output"
        );
    }
}
