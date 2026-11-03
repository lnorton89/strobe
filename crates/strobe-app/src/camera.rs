// ============================================================================
// Module:       strobe_app::camera
// Description:  Webcams behind syn_Media: the registry the player asks for a
//               camera by name, the capture sink over the live mailbox, and
//               listing cameras off the frame thread.
//
// Dependencies: strobe-capture (capture threads), strobe-gfx (live mailbox)
// ============================================================================

//! Cameras for `syn_Media` (backlog M-04, D528).
//!
//! The player resolves a media choice of `camera:<name>` by asking the
//! process's [`LiveSources`], which is [`Cameras`], installed once at
//! start ([`install`]). It keeps a weak handle on each running camera's
//! mailbox, so two players showing one camera (the live scene and the next
//! cue warming up) share one capture thread, and a camera nobody shows any
//! more closes by itself: its capture asks [`Sink::wanted`], which is
//! false once the capture holds the mailbox's last clone.
//!
//! [`Listing`] lists cameras on a thread of its own, since Media
//! Foundation can take tens of milliseconds to answer and the control
//! window draws the result.

use std::collections::HashMap;
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex, PoisonError};
use strobe_capture::{Failure, Picture, Sink};
use strobe_gfx::live::{LiveFrame, LiveSources, Mailbox, WeakMailbox};

/// The running cameras, by name.
#[derive(Debug, Default)]
pub struct Cameras {
    running: Mutex<HashMap<String, WeakMailbox>>,
}

impl LiveSources for Cameras {
    fn camera(&self, name: &str) -> Mailbox {
        let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(mailbox) = running.get(name).and_then(WeakMailbox::upgrade) {
            return mailbox;
        }
        let mailbox = Mailbox::new();
        running.insert(name.to_string(), mailbox.downgrade());
        log::info!("camera \"{name}\": chosen for syn_Media");
        strobe_capture::start(name, Arc::new(Feed(mailbox.clone())));
        mailbox
    }
}

/// Makes [`Cameras`] the player's live sources, for the whole process.
pub fn install() {
    strobe_gfx::live::install(Arc::new(Cameras::default()));
}

/// A capture's pictures into a mailbox.
#[derive(Debug)]
struct Feed(Mailbox);

impl Sink for Feed {
    fn buffer(&self) -> Vec<u8> {
        self.0.buffer()
    }

    fn put(&self, picture: Picture) {
        self.0.put(LiveFrame {
            width: picture.width,
            height: picture.height,
            rgba: picture.rgba,
            captured: picture.captured,
        });
    }

    fn failed(&self, failure: &Failure) {
        self.0
            .fail(failure.to_string(), failure.cause.action().to_string());
    }

    fn recovered(&self, camera: &str) {
        log::info!("camera \"{camera}\": showing again");
    }

    fn wanted(&self) -> bool {
        !self.0.retire_if_unshown()
    }
}

/// What a camera listing came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Listed {
    /// Still asking.
    Asking,
    /// The cameras' names; empty when there are none.
    Cameras(Vec<String>),
    /// The listing failed, in a person's words.
    Failed(String),
}

/// A camera listing on a thread of its own.
#[derive(Debug)]
pub struct Listing {
    receiver: Option<Receiver<Listed>>,
    result: Listed,
}

impl Listing {
    /// Starts listing.
    pub fn start() -> Self {
        let (sender, receiver) = mpsc::channel();
        let spawned = std::thread::Builder::new()
            .name("camera-list".to_string())
            .spawn(move || {
                let listed = match strobe_capture::list() {
                    Ok(names) => Listed::Cameras(names),
                    Err(problem) => Listed::Failed(problem.to_string()),
                };
                // The UI may have stopped waiting; nothing to tell then.
                let _ = sender.send(listed);
            });
        match spawned {
            Ok(_) => Self {
                receiver: Some(receiver),
                result: Listed::Asking,
            },
            Err(error) => Self {
                receiver: None,
                result: Listed::Failed(format!("the listing did not start: {error}")),
            },
        }
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

    fn registered(cameras: &Cameras, name: &str) -> Option<Mailbox> {
        cameras
            .running
            .lock()
            .ok()
            .and_then(|running| running.get(name).and_then(WeakMailbox::upgrade))
    }

    #[test]
    fn one_camera_is_one_mailbox_until_no_one_shows_it() -> Result<()> {
        const NAME: &str = "Not A Real Camera";
        let cameras = Cameras::default();
        let a = cameras.camera(NAME);
        let b = cameras.camera(NAME);
        b.put(LiveFrame::default());
        assert!(a.take().is_some(), "both players share one mailbox");
        drop((a, b));
        // Only the capture's own clone is left: it retires within a tick,
        // and the registry no longer hands it out.
        let until = Instant::now() + Duration::from_secs(10);
        while registered(&cameras, NAME).is_some() {
            if Instant::now() > until {
                bail!("the capture never let the camera go");
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let again = cameras.camera(NAME);
        assert_eq!(again.stats().put, 0, "a fresh mailbox: {:?}", again.stats());
        Ok(())
    }

    #[test]
    fn a_listing_comes_back_without_waiting_on_the_caller() -> Result<()> {
        let mut listing = Listing::start();
        let until = Instant::now() + Duration::from_secs(20);
        while Instant::now() < until {
            match listing.poll() {
                Listed::Asking => std::thread::sleep(Duration::from_millis(5)),
                Listed::Cameras(_) | Listed::Failed(_) => return Ok(()),
            }
        }
        bail!("the listing never came back")
    }
}
