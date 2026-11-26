// ============================================================================
// Module:       strobe_app::live
// Description:  Live sources behind syn_Media (webcams, Spout senders, NDI
//               sources): the registry the player asks for a source by name,
//               the capture sink over the live mailbox, and listing each kind
//               off the frame thread.
//
// Dependencies: strobe-capture (capture threads), strobe-gfx (live mailbox),
//               crate::share_input (the Spout and NDI backends)
// ============================================================================

//! Live sources for `syn_Media` (backlog M-04, D528; Spout and NDI input,
//! D575).
//!
//! The player resolves a media choice of `camera:<name>`, `spout:<name>` or
//! `ndi:<name>` by asking the process's [`LiveSources`], which is
//! [`Sources`], installed once at start ([`install`]). It keeps a weak handle
//! on each running source's mailbox, so two players showing one source (the
//! live scene and the next cue warming up) share one capture thread, and a
//! source nobody shows any more closes by itself: its capture asks
//! [`Sink::wanted`], which is false once the capture holds the mailbox's
//! last clone.
//!
//! All three kinds run on `strobe-capture`'s supervisor: a reader thread
//! that lists, opens and reads, reopening by name after a failure, and a
//! watchdog. Only the backend differs: Media Foundation for a camera, and
//! [`crate::share_input`]'s for Spout and NDI. A failure is said once per
//! outage, with an action worded for its kind ([`action`]).
//!
//! [`Listing`] lists one kind on a thread of its own: Media Foundation can
//! take tens of milliseconds, and NDI's first answers take a second.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};
use strobe_capture::{Cause, Failure, Picture, Sink, Timing};
use strobe_gfx::live::{Kind, LiveFrame, LiveSources, Mailbox, Source, WeakMailbox};

/// The running live sources, by kind and name.
#[derive(Debug, Default)]
pub struct Sources {
    running: Mutex<HashMap<(Kind, String), WeakMailbox>>,
}

impl LiveSources for Sources {
    fn open(&self, source: Source<'_>) -> Mailbox {
        let key = (source.kind, source.name.to_string());
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(mailbox) = running.get(&key).and_then(WeakMailbox::upgrade) {
            return mailbox;
        }
        let mailbox = Mailbox::new();
        running.insert(key, mailbox.downgrade());
        log::info!("{source}: chosen for syn_Media");
        let feed = Arc::new(Feed {
            mailbox: mailbox.clone(),
            kind: source.kind,
        });
        match source.kind {
            Kind::Camera => strobe_capture::start(source.name, feed),
            Kind::Spout => strobe_capture::start_with(
                crate::share_input::spout(),
                Timing::default(),
                source.name,
                feed,
            ),
            Kind::Ndi => strobe_capture::start_with(
                crate::share_input::ndi(),
                Timing::default(),
                source.name,
                feed,
            ),
        }
        mailbox
    }
}

/// Makes [`Sources`] the player's live sources, for the whole process.
pub fn install() {
    strobe_gfx::live::install(Arc::new(Sources::default()));
}

/// What the person is told happens and what to do, for a source of `kind`
/// that failed for `cause`: the failure record's action line.
pub fn action(kind: Kind, cause: Cause) -> &'static str {
    match kind {
        Kind::Camera => cause.action(),
        Kind::Spout => spout_action(cause),
        Kind::Ndi => ndi_action(cause),
    }
}

fn spout_action(cause: Cause) -> &'static str {
    match cause {
        Cause::NotFound => {
            "syn_Media is black until the sender starts; check that the sending app is running \
             and sharing under that name (Media > Spout lists the senders running now)"
        }
        Cause::Unplugged => {
            "syn_Media is black; the sending app stopped sharing, and the picture comes back by \
             itself when it shares again under the same name"
        }
        Cause::Stalled => {
            "syn_Media is black; the sender stopped answering, and Strobe keeps reconnecting by \
             name"
        }
        Cause::Unreadable => "syn_Media is black; the sender's picture could not be read",
        Cause::Unsupported => {
            "syn_Media is black; Spout input needs Windows and Spout's library: install Spout from \
             spout.zeal.co, or put SpoutLibrary.dll beside Strobe"
        }
        Cause::Busy | Cause::Blocked | Cause::Other => {
            "syn_Media is black, and Strobe keeps trying the sender by name"
        }
    }
}

fn ndi_action(cause: Cause) -> &'static str {
    match cause {
        Cause::NotFound => {
            "syn_Media is black until the source appears; check that the sending app is running \
             and that both machines are on the same network (Media > NDI lists what is found)"
        }
        Cause::Unplugged => {
            "syn_Media is black; the source stopped sending, and it comes back by itself when it \
             sends again"
        }
        Cause::Stalled => {
            "syn_Media is black; no frame came from the source (the network, or the sender), and \
             Strobe keeps reconnecting"
        }
        Cause::Unreadable => {
            "syn_Media is black; the source sends a picture format Strobe cannot read"
        }
        Cause::Unsupported => {
            "syn_Media is black; NDI input needs the NDI 6 runtime: install NDI Tools from \
             ndi.video/tools, then restart Strobe"
        }
        Cause::Busy | Cause::Blocked | Cause::Other => {
            "syn_Media is black, and Strobe keeps trying the source"
        }
    }
}

/// A Spout sender's or an NDI source's failure in a word: a camera's labels
/// speak of plugs, which these have none of.
fn label(cause: Cause) -> &'static str {
    match cause {
        Cause::NotFound => "not running",
        Cause::Unplugged => "closed",
        Cause::Stalled => "stopped sending",
        Cause::Unreadable => "unreadable",
        Cause::Unsupported => "unavailable",
        Cause::Busy | Cause::Blocked | Cause::Other => "failed",
    }
}

/// A capture's pictures into a mailbox.
#[derive(Debug)]
struct Feed {
    mailbox: Mailbox,
    kind: Kind,
}

impl Sink for Feed {
    fn buffer(&self) -> Vec<u8> {
        self.mailbox.buffer()
    }

    fn put(&self, picture: Picture) {
        self.mailbox.put(LiveFrame {
            width: picture.width,
            height: picture.height,
            rgba: picture.rgba,
            captured: picture.captured,
        });
    }

    fn failed(&self, failure: &Failure) {
        let message = match self.kind {
            Kind::Camera => failure.to_string(),
            Kind::Spout | Kind::Ndi => format!(
                "{} \"{}\" {}: {}",
                self.kind,
                failure.camera,
                label(failure.cause),
                failure.detail
            ),
        };
        self.mailbox
            .fail(message, action(self.kind, failure.cause).to_string());
    }

    fn recovered(&self, name: &str) {
        log::info!("{} \"{name}\": showing again", self.kind);
    }

    fn wanted(&self) -> bool {
        !self.mailbox.retire_if_unshown()
    }

    fn kind(&self) -> &'static str {
        self.kind.noun()
    }
}

/// The sources of `kind` this machine lists now, by name. May take a second
/// (NDI's first search), so a UI calls it off its frame thread.
///
/// # Errors
///
/// The listing's own failure, in a person's words.
pub fn list(kind: Kind) -> Result<Vec<String>, String> {
    match kind {
        Kind::Camera => strobe_capture::list().map_err(|problem| problem.to_string()),
        Kind::Spout => crate::share_input::spout_list(),
        Kind::Ndi => crate::share_input::ndi_list(),
    }
}

/// What a listing came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Listed {
    /// Still asking.
    Asking,
    /// The sources' names; empty when there are none.
    Found(Vec<String>),
    /// The listing failed, in a person's words.
    Failed(String),
}

/// One kind's listing on a thread of its own.
#[derive(Debug)]
pub struct Listing {
    kind: Kind,
    receiver: Option<Receiver<Listed>>,
    result: Listed,
}

impl Listing {
    /// Starts listing sources of `kind`.
    pub fn start(kind: Kind) -> Self {
        let (sender, receiver) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("live-list".to_string())
            .spawn(move || {
                let listed = match list(kind) {
                    Ok(names) => Listed::Found(names),
                    Err(why) => Listed::Failed(why),
                };
                // The UI may have stopped waiting; nothing to tell then.
                let _ = sender.send(listed);
            });
        match spawned {
            Ok(_) => Self {
                kind,
                receiver: Some(receiver),
                result: Listed::Asking,
            },
            Err(error) => Self {
                kind,
                receiver: None,
                result: Listed::Failed(format!("the listing did not start: {error}")),
            },
        }
    }

    /// The kind being listed.
    pub fn kind(&self) -> Kind {
        self.kind
    }

    /// The result so far, taking it in if it has come. Never waits.
    pub fn poll(&mut self) -> &Listed {
        if let Some(receiver) = &self.receiver {
            match receiver.try_recv() {
                Ok(listed) => {
                    self.result = listed;
                    self.receiver = None;
                }
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.result = Listed::Failed("the listing stopped".to_string());
                    self.receiver = None;
                }
            }
        }
        &self.result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, bail};
    use std::time::{Duration, Instant};

    fn registered(sources: &Sources, kind: Kind, name: &str) -> Option<Mailbox> {
        sources.running.lock().ok().and_then(|running| {
            running
                .get(&(kind, name.to_string()))
                .and_then(WeakMailbox::upgrade)
        })
    }

    fn source(kind: Kind, name: &str) -> Source<'_> {
        Source { kind, name }
    }

    #[test]
    fn one_source_is_one_mailbox_until_no_one_shows_it() -> Result<()> {
        const NAME: &str = "Not A Real Source";
        for kind in Kind::ALL {
            let sources = Sources::default();
            let a = sources.open(source(kind, NAME));
            let b = sources.open(source(kind, NAME));
            b.put(LiveFrame::default());
            assert!(a.take().is_some(), "{kind}: both players share one mailbox");
            drop((a, b));
            // Only the capture's own clone is left: it retires within a tick,
            // and the registry no longer hands it out.
            let until = Instant::now() + Duration::from_secs(10);
            while registered(&sources, kind, NAME).is_some() {
                if Instant::now() > until {
                    bail!("{kind}: the capture never let the source go");
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            let again = sources.open(source(kind, NAME));
            assert_eq!(
                again.stats().put,
                0,
                "{kind}: a fresh mailbox: {:?}",
                again.stats()
            );
        }
        Ok(())
    }

    #[test]
    fn the_same_name_of_two_kinds_is_two_sources() {
        let sources = Sources::default();
        let camera = sources.open(source(Kind::Camera, "Feed"));
        let spout = sources.open(source(Kind::Spout, "Feed"));
        spout.put(LiveFrame::default());
        assert!(camera.take().is_none(), "the camera was not fed");
        assert!(spout.take().is_some(), "the sender was");
    }

    #[test]
    fn a_missing_source_reads_black_and_is_said_once_in_its_own_words() -> Result<()> {
        // A Spout sender and an NDI source that cannot exist: with or without
        // the library or runtime on this machine, the capture fails, and the
        // mailbox says so once, worded for its kind.
        for kind in [Kind::Spout, Kind::Ndi] {
            let sources = Sources::default();
            let player = sources.open(source(kind, "Strobe Test Missing Source 7f3a"));
            let until = Instant::now() + Duration::from_secs(15);
            let (why, said) = loop {
                if let Some(reason) = player.take_reason() {
                    break reason;
                }
                if Instant::now() > until {
                    bail!("{kind}: the failure was never said");
                }
                std::thread::sleep(Duration::from_millis(20));
            };
            assert!(player.failed(), "{kind}: black while it fails");
            assert!(
                why.starts_with(kind.noun()),
                "{kind}: the message names the kind: {why}"
            );
            let expected = [
                action(kind, Cause::NotFound),
                action(kind, Cause::Unsupported),
            ];
            assert!(
                expected.contains(&said.as_str()),
                "{kind}: not found, or no library/runtime here: {said}"
            );
            // Retries keep failing; the failure is not said again.
            std::thread::sleep(Duration::from_millis(2500));
            assert_eq!(player.take_reason(), None, "{kind}: said once");
            assert!(player.failed(), "{kind}: still black");
        }
        Ok(())
    }

    #[test]
    fn every_kind_and_cause_has_its_own_words() {
        for kind in [Kind::Spout, Kind::Ndi] {
            for cause in [
                Cause::NotFound,
                Cause::Unplugged,
                Cause::Stalled,
                Cause::Unreadable,
                Cause::Unsupported,
                Cause::Other,
            ] {
                let words = action(kind, cause);
                assert!(
                    words.starts_with("syn_Media is black") && !words.contains("camera"),
                    "{kind} {cause:?}: {words}"
                );
                assert!(
                    !words.contains("  "),
                    "{kind} {cause:?}: no run of spaces from a broken line: {words}"
                );
            }
        }
        assert_eq!(
            action(Kind::Camera, Cause::Busy),
            Cause::Busy.action(),
            "a camera keeps its own words"
        );
    }

    #[test]
    fn a_listing_comes_back_without_waiting_on_the_caller() -> Result<()> {
        for kind in Kind::ALL {
            let mut listing = Listing::start(kind);
            assert_eq!(listing.kind(), kind, "the kind asked for");
            let until = Instant::now() + Duration::from_secs(20);
            loop {
                match listing.poll() {
                    Listed::Asking if Instant::now() < until => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Listed::Asking => bail!("{kind}: the listing never came back"),
                    Listed::Found(_) | Listed::Failed(_) => break,
                }
            }
        }
        Ok(())
    }
}
