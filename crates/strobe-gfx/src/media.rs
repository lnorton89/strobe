// ============================================================================
// Module:       strobe_gfx::media
// Description:  The picture behind syn_Media on the render thread: a still or
//               an image sequence, uploaded once as its frames arrive from the
//               decoder thread.
//
// Dependencies: crate::assets (decoder thread), crate::gl
// ============================================================================

//! `syn_Media` at run time (D186).
//!
//! The decoder thread ([`crate::assets::spawn_decoder`]) reads the files and
//! sends decoded pictures. [`Media::poll`], called once per rendered frame,
//! takes **at most one** arrived picture and uploads it as a texture of its
//! own, so a frame's cost never includes more than one upload and a slow disk
//! costs nothing. After that a picture is only ever *bound*:
//! [`Media::current`] picks the texture for the scene's clock, with no upload,
//! no allocation and no driver query.
//!
//! A still is one texture. A sequence is one texture per picture, played at
//! the request's rate from the scene's `TIME`, looping or holding its last
//! picture. Until the first picture lands `syn_Media` is black and
//! `syn_MediaType` is 0, which is also what a scene with no media sees.
//!
//! Video and webcam input are not here and are not planned for this stage
//! (ROADMAP): they need a decoder and a capture dependency this engine does
//! not carry.

use crate::assets::{MediaMessage, MediaRequest, spawn_decoder};
use crate::gl::{Gl, Texture};
use crate::player::sampling_for;
use std::sync::mpsc::{Receiver, TryRecvError};
use strobe_scene::scene::Filter;

/// What `syn_MediaType` holds while a still or a sequence is showing. The
/// SSF library only tests `> 0.5` (`_isMediaActive`); the values for video
/// and webcam are left unassigned (Q22).
pub const MEDIA_TYPE_IMAGE: f32 = 1.0;

/// The media a player shows, or the lack of it.
#[derive(Debug)]
pub(crate) struct Media {
    request: Option<MediaRequest>,
    frames: Vec<Texture>,
    receiver: Option<Receiver<MediaMessage>>,
    failure: Option<String>,
    /// When decoding started, for the line that says it finished.
    started: Option<std::time::Instant>,
}

impl Media {
    /// No media: black, type 0.
    pub(crate) fn none() -> Self {
        Self {
            request: None,
            frames: Vec::new(),
            receiver: None,
            failure: None,
            started: None,
        }
    }

    /// Starts decoding `request` in the background.
    pub(crate) fn start(request: MediaRequest) -> Result<Self, String> {
        let receiver = spawn_decoder(&request)?;
        Ok(Self {
            request: Some(request),
            frames: Vec::new(),
            receiver: Some(receiver),
            failure: None,
            started: Some(std::time::Instant::now()),
        })
    }

    /// What this media was asked for; `None` for no media.
    pub(crate) fn request(&self) -> Option<&MediaRequest> {
        self.request.as_ref()
    }

    /// The path this media was asked for, for messages.
    pub(crate) fn label(&self) -> String {
        self.request
            .as_ref()
            .map_or_else(String::new, |r| r.path.display().to_string())
    }

    /// Whether at least one picture is ready to show.
    pub(crate) fn has_picture(&self) -> bool {
        !self.frames.is_empty()
    }

    /// Whether the decoder is still sending.
    pub(crate) fn loading(&self) -> bool {
        self.receiver.is_some()
    }

    /// Takes the reason loading stopped early, once.
    pub(crate) fn take_failure(&mut self) -> Option<String> {
        self.failure.take()
    }

    /// Uploads at most one arrived picture. Never waits.
    pub(crate) fn poll(&mut self, gl: &Gl) {
        let Some(receiver) = self.receiver.as_ref() else {
            return;
        };
        let message = match receiver.try_recv() {
            Ok(message) => message,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                MediaMessage::Failed("the media thread stopped before it finished".to_string())
            }
        };
        self.accept(gl, message);
    }

    /// Waits for the decoder to finish and uploads everything. For tests
    /// and the benchmark, whose frames must not depend on disk speed; the
    /// app never calls it.
    pub(crate) fn finish(&mut self, gl: &Gl) {
        while let Some(receiver) = self.receiver.as_ref() {
            let message = receiver.recv().unwrap_or_else(|_| {
                MediaMessage::Failed("the media thread stopped before it finished".to_string())
            });
            self.accept(gl, message);
        }
    }

    fn accept(&mut self, gl: &Gl, message: MediaMessage) {
        match message {
            MediaMessage::Frame(frame) => {
                let (wrap, filter) = self
                    .request
                    .as_ref()
                    .map_or((strobe_scene::scene::Wrap::Clamp, Filter::Linear), |r| {
                        (r.wrap, r.filter)
                    });
                let uploaded = gl.texture_2d(
                    frame.width as i32,
                    frame.height as i32,
                    crate::gl::Format::Rgba8,
                    sampling_for(wrap, filter),
                    Some(&frame.rgba),
                );
                match uploaded {
                    Ok(texture) => {
                        if filter == Filter::Mipmap {
                            gl.generate_mipmaps(&texture);
                        }
                        self.frames.push(texture);
                    }
                    Err(message) => {
                        self.failure = Some(message);
                        self.receiver = None;
                    }
                }
            }
            MediaMessage::Done => {
                self.receiver = None;
                if let Some(started) = self.started {
                    log::info!(
                        "syn_Media: {} picture(s) of {} loaded in {:.1} s",
                        self.frames.len(),
                        self.label(),
                        started.elapsed().as_secs_f32()
                    );
                }
            }
            MediaMessage::Failed(message) => {
                self.failure = Some(message);
                self.receiver = None;
            }
        }
    }

    /// The picture to show at scene time `time`: the still, or the
    /// sequence's frame for that moment. `None` before any picture lands.
    pub(crate) fn current(&self, time: f32) -> Option<&Texture> {
        let count = self.frames.len();
        if count <= 1 {
            return self.frames.first();
        }
        let request = self.request.as_ref()?;
        let step = (time.max(0.0) * request.fps).floor();
        let step = if step.is_finite() { step as usize } else { 0 };
        let index = if request.looping {
            step % count
        } else {
            step.min(count - 1)
        };
        self.frames.get(index)
    }

    /// The value for `syn_MediaType`.
    pub(crate) fn media_type(&self) -> f32 {
        if self.has_picture() {
            MEDIA_TYPE_IMAGE
        } else {
            0.0
        }
    }
}
