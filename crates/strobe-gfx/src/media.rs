// ============================================================================
// Module:       strobe_gfx::media
// Description:  The picture behind syn_Media on the render thread: a still or
//               an image sequence, uploaded once as its frames arrive from the
//               decoder thread, or a camera, uploaded into one reused texture.
//
// Dependencies: crate::assets (decoder thread), crate::live (cameras),
//               crate::gl
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
//! A **camera** (a request for `camera:<name>`, D528) is a live source: its
//! capture thread puts pictures in a [`Mailbox`], and [`Media::poll`] takes
//! at most one a frame and writes it with `Gl::upload` into **one reused
//! texture**, sampled with the slot's `WRAP` and `FILTER`. Only a new
//! picture size makes a new texture. While it shows, `syn_MediaType` is 3;
//! while the camera fails it reads black and 0, and the failure is said
//! once. Until its first picture lands it counts as loading, so the picture
//! it replaces stays up meanwhile.
//!
//! Video files are not here: they need a decoder decision of their own
//! (backlog M-04).

use crate::assets::{Frame, MediaMessage, MediaRequest, spawn_decoder};
use crate::gl::{Gl, Texture};
use crate::live::{MEDIA_TYPE_WEBCAM, Mailbox};
use crate::player::sampling_for;
use std::sync::mpsc::{Receiver, TryRecvError};
use strobe_scene::scene::{Filter, Wrap};

/// What `syn_MediaType` holds while a still or a sequence is showing. The
/// SSF library only tests `> 0.5` (`_isMediaActive`); a camera is
/// [`MEDIA_TYPE_WEBCAM`] (Q22).
pub const MEDIA_TYPE_IMAGE: f32 = 1.0;

/// The media a player shows, or the lack of it.
#[derive(Debug)]
pub(crate) struct Media {
    request: Option<MediaRequest>,
    frames: Vec<Texture>,
    receiver: Option<Receiver<MediaMessage>>,
    failure: Option<String>,
    /// What to tell the person about `failure`, when the source knows.
    action: Option<String>,
    /// When decoding started, for the line that says it finished.
    started: Option<std::time::Instant>,
    /// A camera, instead of decoded files.
    live: Option<Live>,
}

/// A camera's mailbox and the one texture its pictures go into.
#[derive(Debug)]
struct Live {
    mailbox: Mailbox,
    texture: Option<Texture>,
    /// Whether the texture holds a picture to show: false before the first
    /// and while the camera fails.
    showing: bool,
    /// Whether any picture has come, shown or not.
    seen: bool,
}

impl Media {
    /// No media: black, type 0.
    pub(crate) fn none() -> Self {
        Self {
            request: None,
            frames: Vec::new(),
            receiver: None,
            failure: None,
            action: None,
            started: None,
            live: None,
        }
    }

    /// Starts decoding `request` in the background, or, for a camera,
    /// asks the host's live sources for its mailbox.
    pub(crate) fn start(request: MediaRequest) -> Result<Self, String> {
        if let Some(name) = strobe_scene::camera::name_of(&request.path) {
            let sources = crate::live::sources()
                .ok_or_else(|| format!("camera \"{name}\": this program has no camera input"))?;
            let mailbox = sources.camera(name);
            return Ok(Self {
                request: Some(request),
                live: Some(Live {
                    mailbox,
                    texture: None,
                    showing: false,
                    seen: false,
                }),
                ..Self::none()
            });
        }
        let receiver = spawn_decoder(&request)?;
        Ok(Self {
            request: Some(request),
            receiver: Some(receiver),
            started: Some(std::time::Instant::now()),
            ..Self::none()
        })
    }

    /// A still picture already decoded, uploaded now, with no thread: the
    /// browser sandbox's `MEDIA` (W-01), whose build has no threads and
    /// whose picture is in memory. A failed upload is reported through
    /// [`Self::take_failure`] as a decoder's would be.
    pub(crate) fn still(gl: &Gl, request: MediaRequest, frame: Frame) -> Self {
        let mut media = Self {
            request: Some(request),
            ..Self::none()
        };
        media.accept(gl, MediaMessage::Frame(frame));
        media
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
        match &self.live {
            Some(live) => live.showing,
            None => !self.frames.is_empty(),
        }
    }

    /// Whether the decoder is still sending, or a camera has sent no
    /// picture yet and has not failed.
    pub(crate) fn loading(&self) -> bool {
        match &self.live {
            Some(live) => !live.seen && !live.mailbox.failed(),
            None => self.receiver.is_some(),
        }
    }

    /// Takes the reason loading stopped early, and what to tell the person
    /// about it when the source knows, once.
    pub(crate) fn take_failure(&mut self) -> Option<(String, Option<String>)> {
        let message = self.failure.take()?;
        Some((message, self.action.take()))
    }

    fn sampling(&self) -> (Wrap, Filter) {
        self.request
            .as_ref()
            .map_or((Wrap::Clamp, Filter::Linear), |r| (r.wrap, r.filter))
    }

    /// Uploads at most one arrived picture. Never waits. `read` is whether
    /// the scene samples `syn_Media` at all: a camera's pictures are taken
    /// either way, but uploaded only for a scene that reads them.
    pub(crate) fn poll(&mut self, gl: &Gl, read: bool) {
        if self.live.is_some() {
            self.poll_live(gl, read);
            return;
        }
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

    /// A camera's frame: black while it fails (said once), else the newest
    /// picture, if one came, into the one texture. No allocation and no
    /// driver query unless the picture's size changed.
    fn poll_live(&mut self, gl: &Gl, read: bool) {
        let (wrap, filter) = self.sampling();
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if live.mailbox.failed() {
            live.showing = false;
            if let Some((why, action)) = live.mailbox.take_reason() {
                self.failure = Some(why);
                self.action = Some(action);
            }
            return;
        }
        let Some(frame) = live.mailbox.take() else {
            return;
        };
        live.seen = true;
        if !read {
            live.mailbox.give_back(frame.rgba);
            return;
        }
        let (width, height) = (frame.width as i32, frame.height as i32);
        let reuse = live
            .texture
            .as_ref()
            .filter(|t| t.width() == width && t.height() == height);
        let uploaded = match reuse {
            Some(texture) => gl.upload(texture, &frame.rgba),
            None => gl
                .texture_2d(
                    width,
                    height,
                    crate::gl::Format::Rgba8,
                    sampling_for(wrap, filter),
                    Some(&frame.rgba),
                )
                .map(|texture| live.texture = Some(texture)),
        };
        match (uploaded, live.texture.as_ref()) {
            (Ok(()), Some(texture)) => {
                if filter == Filter::Mipmap {
                    gl.generate_mipmaps(texture);
                }
                live.showing = true;
            }
            (Ok(()), None) => live.showing = false,
            (Err(message), _) => {
                live.showing = false;
                self.failure = Some(message);
            }
        }
        live.mailbox.give_back(frame.rgba);
    }

    /// Waits for the decoder to finish and uploads everything. For tests
    /// and the benchmark, whose frames must not depend on disk speed; the
    /// app never calls it. A camera has nothing to wait for.
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
                let (wrap, filter) = self.sampling();
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
    /// sequence's frame for that moment, or the camera's newest. `None`
    /// before any picture lands, and while a camera fails.
    pub(crate) fn current(&self, time: f32) -> Option<&Texture> {
        if let Some(live) = &self.live {
            return live.texture.as_ref().filter(|_| live.showing);
        }
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
        match (&self.live, self.has_picture()) {
            (Some(_), true) => MEDIA_TYPE_WEBCAM,
            (None, true) => MEDIA_TYPE_IMAGE,
            (Some(_) | None, false) => 0.0,
        }
    }
}
