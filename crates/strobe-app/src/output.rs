// ============================================================================
// Module:       strobe_app::output
// Description:  Sharing the finished picture: the Spout and NDI senders the
//               operator turned on, the frame each is given, and what to say
//               when one stops.
//
// Dependencies: strobe-output (the senders, Output and its policy),
//               strobe-gfx (the texture description); log
// ============================================================================

//! Sharing the picture (D570, D571; backlog M-06).
//!
//! Strobe draws one finished picture a frame, after the output post stage and
//! at the picture's size rather than a window's. This module hands that
//! picture to whichever senders the operator turned on:
//!
//! - **Spout** takes the renderer's texture itself, on the frame thread with
//!   the context current, which is the whole point of it: nothing is read
//!   back (D571).
//! - **NDI** takes pixels, so the picture is read back and sent as a frame.
//!   The read is synchronous, which is the first cut's cost and the number
//!   M-06's acceptance test measures; a readback ring is the next step.
//!
//! Both follow `master`, colour and mirrors, because they are given the post
//! stage's output, and neither follows blackout, which is a present-time
//! clear of the shown window only (D135, D570).
//!
//! A sender that fails is said once and dropped, and frames go nowhere until
//! the operator turns it on again or the show reopens it: a shared output is
//! never allowed to stop the show.

use crate::ui::state::SharingUi;
use strobe_gfx::gl::TextureShare;
use strobe_output::{Frame, NdiSender, Output, SharedTexture, SpoutSender, Status};

/// The picture a frame can offer a sender.
pub(crate) struct Picture<'a> {
    /// The final texture, for a sender that shares GPU memory.
    pub(crate) texture: Option<TextureShare>,
    /// The final picture as pixels, top row first, read back only when a
    /// sender needs them.
    pub(crate) frame: Option<&'a Frame>,
}

/// The senders this run has open.
#[derive(Default)]
pub(crate) struct Sharing {
    spout: Option<Output>,
    ndi: Option<Output>,
    /// The last thing said about a sender, for the Settings row.
    note: String,
    /// What was asked for last, so a rebuild happens only on a change.
    asked: Option<Asked>,
}

/// What the operator asked for: two names, each off when absent.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Asked {
    spout: Option<String>,
    ndi: Option<String>,
}

impl Sharing {
    /// A run with nothing shared, which is how every run starts unless a
    /// flag or a saved setting names a sender.
    pub(crate) fn new() -> Self {
        Self::default()
    }

    /// The names this run is asked to share under.
    pub(crate) fn names(&self) -> (Option<&str>, Option<&str>) {
        let spout = self.spout.as_ref().map(Output::name);
        let ndi = self.ndi.as_ref().map(Output::name);
        (spout, ndi)
    }

    /// What to say about the senders: empty while they are fine.
    pub(crate) fn note(&self) -> &str {
        &self.note
    }

    /// Whether a sender failed and has not been reopened: the Settings row
    /// offers to try again.
    pub(crate) fn faulted(&self) -> bool {
        [&self.spout, &self.ndi]
            .into_iter()
            .flatten()
            .any(|output| matches!(output.status(), Status::Down))
    }

    /// Applies `ui`'s two choices: opens a sender that was turned on, closes
    /// one that was turned off, and does nothing when neither changed.
    ///
    /// Opening is where a missing runtime is reported, so this is called
    /// once per change and at launch, never per frame.
    pub(crate) fn apply(&mut self, ui: &SharingUi) {
        let asked = Asked {
            spout: ui.spout.then(|| ui.spout_name.clone()),
            ndi: ui.ndi.then(|| ui.ndi_name.clone()),
        };
        if self.asked.as_ref() == Some(&asked) {
            return;
        }
        self.asked = Some(asked.clone());
        self.note.clear();
        self.spout = open_spout(asked.spout.as_deref(), &mut self.note);
        self.ndi = open_ndi(asked.ndi.as_deref(), &mut self.note);
    }

    /// Hands this frame's picture to every open sender.
    pub(crate) fn show(&mut self, picture: &Picture<'_>) {
        if let (Some(output), Some(texture)) = (self.spout.as_mut(), picture.texture) {
            let shared = SharedTexture::new(
                texture.id,
                texture.width.max(0) as u32,
                texture.height.max(0) as u32,
            );
            output.show_texture(shared);
        }
        if let (Some(output), Some(frame)) = (self.ndi.as_mut(), picture.frame) {
            output.show(frame);
        }
        self.report();
    }

    /// Says what a stopped sender stopped for, once per outage.
    fn report(&mut self) {
        if !self.note.is_empty() {
            return;
        }
        let stopped = [&self.spout, &self.ndi]
            .into_iter()
            .flatten()
            .find(|output| matches!(output.status(), Status::Down))
            .and_then(|output| {
                output
                    .reason()
                    .map(|reason| (output.name().to_owned(), reason.to_owned()))
            });
        if let Some((name, reason)) = stopped {
            self.note = format!(
                "Sharing `{name}` stopped: {reason}. Turn it off and on again, or reopen it."
            );
            log::warn!("{}", self.note);
        }
    }
}

/// Opens a Spout sender named `name`, or nothing when it is off. The failure
/// is worded for the console and the Settings row.
fn open_spout(name: Option<&str>, note: &mut String) -> Option<Output> {
    let name = name?;
    match SpoutSender::open(name) {
        Ok(sender) => {
            log::info!("sharing the output through Spout as `{name}`");
            Some(Output::open(name, Box::new(sender)))
        }
        Err(problem) => {
            *note = problem.to_string();
            log::warn!("Spout output: {problem}");
            None
        }
    }
}

/// Opens an NDI sender named `name`, or nothing when it is off.
fn open_ndi(name: Option<&str>, note: &mut String) -> Option<Output> {
    let name = name?;
    match NdiSender::open(name) {
        Ok(sender) => {
            log::info!("sharing the output over NDI as `{name}`");
            Some(Output::open(name, Box::new(sender)))
        }
        Err(problem) => {
            *note = problem.to_string();
            log::warn!("NDI output: {problem}");
            None
        }
    }
}
