// ============================================================================
// Module:       strobe_app::output
// Description:  Sharing the finished picture: the Spout and NDI senders the
//               operator turned on, the GPU stage that sizes, blacks out and
//               packs the picture for them, and the line Settings shows.
//
// Dependencies: strobe-output (the senders, Output and its policy),
//               strobe-gfx (the share stage, the texture); crate::prefs; log
// ============================================================================

//! Sharing the picture (D570, D571, D573; backlog M-06).
//!
//! Strobe draws one finished picture a frame, after the output post stage and
//! at the picture's size rather than a window's. This module hands that
//! picture to whichever senders the operator turned on, the way the VJ tools
//! people already use do it (research in `docs/research/m06-ndi-spout.md`):
//!
//! - **What goes out** is the show's final picture: after the post stage's
//!   master, colour and mirrors, and black while the show is blacked out, so
//!   a receiver on another screen goes dark with the projector.
//! - **Its size** is the picture's own, or a fixed size chosen in Settings
//!   (`--share-size`), so a 720p render can feed a 1080p mixer input.
//! - **Spout** takes the texture itself, on the render thread with the
//!   context current: nothing is read back (D571). A second Spout sender,
//!   the **clean feed**, can carry the picture before the post stage and
//!   never blacked out, under a name of its own (D574), for a mixer that
//!   wants the raw scene beside the show's output.
//! - **NDI** takes UYVY packed on the GPU and read back through a fenced ring
//!   (`strobe_gfx::share`), submitted asynchronously. While NDI reports no
//!   receiver, nothing is packed or read back at all; an optional cap limits
//!   how many frames a second go out.
//!
//! A sender that fails is said once and dropped, and frames go nowhere until
//! the operator changes the setting: a shared output never stops the show.
//!
//! The GPU side ([`GpuShare`]: the stage and the Spout sender, both bound to
//! the GL context) lives in the app's context stack and goes with it; a
//! rebuilt context gets a fresh one, and [`Sharing::context_changed`] makes
//! the Spout sender open again on it.

use crate::prefs::SharingPref;
use std::time::{Duration, Instant};
use strobe_gfx::caps::Caps;
use strobe_gfx::gl::{Gl, Texture};
use strobe_gfx::share::{ShareStage, Want};
use strobe_output::{
    Layout, NdiOptions, NdiSender, Output, Pixels, SharedTexture, SpoutSender, Status,
};

/// How often NDI's receiver count is asked, and the status line redrawn.
const POLL: Duration = Duration::from_millis(500);

/// The share pieces that belong to one GL context: the stage and the Spout
/// sender, whose interop is tied to the context it was made on.
#[derive(Default)]
pub(crate) struct GpuShare {
    stage: Option<ShareStage>,
    /// The stage would not build; said once, then Spout goes direct.
    stage_failed: bool,
    spout: Option<Output>,
    /// The clean feed's own stage, for its size (D574).
    clean_stage: Option<ShareStage>,
    /// The clean feed: a second Spout sender (D574).
    clean: Option<Output>,
}

impl GpuShare {
    /// Lets go of everything after the context was lost: the stage's GL
    /// objects are skipped by their owners on a lost context, and the Spout
    /// sender is leaked rather than released, because releasing it calls
    /// into a context that no longer exists.
    pub(crate) fn abandon(self) {
        let Self {
            stage,
            spout,
            clean_stage,
            clean,
            ..
        } = self;
        drop(stage);
        drop(clean_stage);
        for sender in [spout, clean].into_iter().flatten() {
            std::mem::forget(sender);
        }
    }
}

/// This frame's pictures, as [`Sharing::frame`] takes them.
#[derive(Clone, Copy)]
pub(crate) struct Pictures<'a> {
    /// The show's picture: after the post stage.
    pub(crate) show: &'a Texture,
    /// The clean feed's picture: before the post stage (D574).
    pub(crate) clean: &'a Texture,
    /// The show is blacked out: the show's picture goes out black.
    pub(crate) black: bool,
    /// Nothing is playing: the clean feed goes out black too, so a receiver
    /// that connects before a scene is chosen shows black, not nothing.
    pub(crate) clean_black: bool,
}

/// What the operator asked of NDI, compared without allocating per frame.
#[derive(Debug, Clone, PartialEq, Eq)]
struct NdiAsk {
    name: String,
    groups: String,
    fps: u32,
}

/// The senders' state that outlives a context, and what Settings shows.
pub(crate) struct Sharing {
    ndi: Option<Output>,
    /// The Spout name last applied (`Some(None)` for off); `None` until the
    /// first apply, or after a context change.
    asked_spout: Option<Option<String>>,
    /// The clean feed's name last applied, as for Spout (D574).
    asked_clean: Option<Option<String>>,
    /// The NDI options last applied, as for Spout.
    asked_ndi: Option<Option<NdiAsk>>,
    size: Option<[i32; 2]>,
    /// The least time between two NDI frames, from the cap.
    interval: Option<Duration>,
    spout_note: String,
    clean_note: String,
    ndi_note: String,
    stage_note: String,
    /// When NDI last packed a frame, for the cap.
    last_pack: Option<Instant>,
    receivers: Option<u32>,
    polled: Option<Instant>,
    /// NDI frames sent since `counted_from`, for the measured rate.
    counted: u32,
    counted_from: Option<Instant>,
    ndi_fps: f32,
    /// The Spout picture's size, for the status line.
    spout_size: Option<[u32; 2]>,
    /// The clean feed's size, for the status line.
    clean_size: Option<[u32; 2]>,
    status: String,
    /// What went wrong with Spout, and with NDI or the stage, for each row.
    notes: [String; 2],
}

impl Sharing {
    /// A run with nothing shared yet.
    pub(crate) fn new() -> Self {
        Self {
            ndi: None,
            asked_spout: None,
            asked_clean: None,
            asked_ndi: None,
            size: None,
            interval: None,
            spout_note: String::new(),
            clean_note: String::new(),
            ndi_note: String::new(),
            stage_note: String::new(),
            last_pack: None,
            receivers: None,
            polled: None,
            counted: 0,
            counted_from: None,
            ndi_fps: 0.0,
            spout_size: None,
            clean_size: None,
            status: String::new(),
            notes: [String::new(), String::new()],
        }
    }

    /// What went wrong with Spout, and with NDI, each for its own Settings
    /// row: empty while all is well.
    pub(crate) fn notes(&self) -> [&str; 2] {
        [&self.notes[0], &self.notes[1]]
    }

    /// One line on what is being shared, for the Settings row.
    pub(crate) fn status(&self) -> &str {
        &self.status
    }

    /// The context was rebuilt: the Spout sender went with the old one, so
    /// the next apply opens it again on the new.
    pub(crate) fn context_changed(&mut self) {
        self.asked_spout = None;
        self.asked_clean = None;
    }

    /// Applies `live`: opens a sender that was turned on or renamed, closes
    /// one turned off, and does nothing (and allocates nothing) when neither
    /// changed. Opening is where a missing runtime is reported.
    pub(crate) fn apply(&mut self, live: &SharingPref, gpu: &mut GpuShare) -> bool {
        let mut changed = false;
        let spout_wanted = live.spout.then_some(live.spout_name.as_str());
        let spout_same = self
            .asked_spout
            .as_ref()
            .is_some_and(|asked| asked.as_deref() == spout_wanted);
        if !spout_same {
            gpu.spout = None;
            self.spout_note.clear();
            self.spout_size = None;
            gpu.spout = spout_wanted.and_then(|name| open_spout(name, &mut self.spout_note));
            self.asked_spout = Some(spout_wanted.map(str::to_owned));
            changed = true;
        }
        let clean_wanted = live.spout_clean.then_some(live.spout_clean_name.as_str());
        let clean_same = self
            .asked_clean
            .as_ref()
            .is_some_and(|asked| asked.as_deref() == clean_wanted);
        if !clean_same || !spout_same {
            gpu.clean = None;
            self.clean_note.clear();
            self.clean_size = None;
            gpu.clean = clean_wanted.and_then(|name| {
                if spout_wanted.is_some_and(|main| main.trim() == name.trim()) {
                    self.clean_note = format!(
                        "the clean feed needs a name of its own; `{name}` is the main sender's"
                    );
                    return None;
                }
                open_spout(name, &mut self.clean_note)
            });
            self.asked_clean = Some(clean_wanted.map(str::to_owned));
            changed = true;
        }
        let ndi_same = self.asked_ndi.as_ref().is_some_and(|asked| match asked {
            None => !live.ndi,
            Some(ask) => {
                live.ndi
                    && ask.name == live.ndi_name
                    && ask.groups == live.ndi_groups
                    && ask.fps == live.ndi_fps
            }
        });
        if !ndi_same {
            // The old sender goes first: two of the same name fight.
            self.ndi = None;
            self.ndi_note.clear();
            self.receivers = None;
            self.polled = None;
            self.counted = 0;
            self.counted_from = None;
            self.ndi_fps = 0.0;
            let ask = live.ndi.then(|| NdiAsk {
                name: live.ndi_name.clone(),
                groups: live.ndi_groups.clone(),
                fps: live.ndi_fps,
            });
            self.ndi = ask
                .as_ref()
                .and_then(|ask| open_ndi(ask, &mut self.ndi_note));
            self.asked_ndi = Some(ask);
            changed = true;
        }
        let size = live.size.map(|[w, h]| {
            [
                i32::try_from(w).unwrap_or(i32::MAX),
                i32::try_from(h).unwrap_or(i32::MAX),
            ]
        });
        if size != self.size {
            self.size = size;
            changed = true;
        }
        let interval =
            (live.ndi_fps > 0).then(|| Duration::from_secs_f64(1.0 / f64::from(live.ndi_fps)));
        if interval != self.interval {
            self.interval = interval;
            changed = true;
        }
        if changed {
            self.refresh_note();
            self.refresh_status(gpu);
        }
        changed
    }

    /// Whether anything is being shared, so the frame can skip all of this.
    pub(crate) fn active(&self, gpu: &GpuShare) -> bool {
        gpu.spout.is_some() || gpu.clean.is_some() || self.ndi.is_some()
    }

    /// Hands this frame's picture to every open sender: `pictures.show` (the
    /// post stage's output), black instead when `pictures.black`, and
    /// `pictures.clean` (before the post stage, never black) to the clean
    /// feed. Returns whether the note or the status line changed, so the app
    /// copies them to Settings only then.
    pub(crate) fn frame(
        &mut self,
        gpu: &mut GpuShare,
        gl: &Gl,
        caps: &Caps,
        pictures: Pictures<'_>,
        now: Instant,
    ) -> bool {
        let Pictures {
            show: source,
            clean: clean_source,
            black,
            clean_black,
        } = pictures;
        if !self.active(gpu) {
            return false;
        }
        let poll = self.polled.is_none_or(|at| now.duration_since(at) >= POLL);
        if poll {
            self.polled = Some(now);
            self.receivers = self.ndi.as_ref().and_then(Output::receivers);
        }
        let ndi_live = self
            .ndi
            .as_ref()
            .is_some_and(|ndi| ndi.status() == Status::Live);
        let due = pack_due(self.interval, self.last_pack, now);
        let pixels = ndi_live && self.receivers != Some(0) && due;
        if pixels {
            self.last_pack = Some(now);
        }
        let want = Want {
            size: self.size,
            black,
            texture: gpu.spout.is_some(),
            pixels,
        };
        if gpu.stage.is_none() && !gpu.stage_failed {
            match ShareStage::new(gl, caps)
                .and_then(|main| ShareStage::new(gl, caps).map(|clean| (main, clean)))
            {
                Ok((stage, clean)) => {
                    gpu.stage = Some(stage);
                    gpu.clean_stage = Some(clean);
                }
                Err(error) => {
                    gpu.stage_failed = true;
                    self.stage_note = format!(
                        "the share stage would not build ({error}); Spout shares the picture \
                         as it is, and NDI cannot send"
                    );
                    log::warn!("{}", self.stage_note);
                }
            }
        }
        let texture = match gpu.stage.as_mut() {
            Some(stage) => match stage.prepare(source, want) {
                Ok(texture) => texture,
                Err(error) => {
                    if self.stage_note.is_empty() {
                        self.stage_note = format!("the shared picture could not be sized: {error}");
                        log::warn!("{}", self.stage_note);
                    }
                    None
                }
            },
            None => want.texture.then(|| source.share()),
        };
        if let (Some(spout), Some(texture)) = (gpu.spout.as_mut(), texture) {
            let width = u32::try_from(texture.width).unwrap_or(0);
            let height = u32::try_from(texture.height).unwrap_or(0);
            spout.show_texture(SharedTexture::new(texture.id, width, height));
            self.spout_size = Some([width, height]);
        }
        if let Some(clean) = gpu.clean.as_mut() {
            let want = Want {
                size: self.size,
                black: clean_black,
                texture: true,
                pixels: false,
            };
            let texture = match gpu.clean_stage.as_mut() {
                Some(stage) => stage.prepare(clean_source, want).unwrap_or_default(),
                None => Some(clean_source.share()),
            };
            if let Some(texture) = texture {
                let width = u32::try_from(texture.width).unwrap_or(0);
                let height = u32::try_from(texture.height).unwrap_or(0);
                clean.show_texture(SharedTexture::new(texture.id, width, height));
                self.clean_size = Some([width, height]);
            }
        }
        if let (Some(ndi), Some(stage)) = (self.ndi.as_mut(), gpu.stage.as_mut())
            && let Some((data, [width, height])) = stage.take_pixels()
        {
            let sent = ndi.show(Pixels {
                width,
                height,
                layout: Layout::Uyvy,
                data,
            });
            if sent == strobe_output::Outcome::Sent {
                self.counted += 1;
            }
        }
        let mut changed = self.take_failures(gpu);
        if poll {
            let from = *self.counted_from.get_or_insert(now);
            let span = now.duration_since(from).as_secs_f32();
            if span >= 1.0 {
                self.ndi_fps = self.counted as f32 / span;
                self.counted = 0;
                self.counted_from = Some(now);
            }
            changed |= self.refresh_status(gpu);
        }
        changed
    }

    /// Notes a sender the policy took down, once per outage.
    fn take_failures(&mut self, gpu: &GpuShare) -> bool {
        let mut changed = false;
        if self.spout_note.is_empty()
            && let Some(reason) = gpu.spout.as_ref().and_then(down_reason)
        {
            self.spout_note = format!("Spout stopped: {reason}");
            changed = true;
        }
        if self.clean_note.is_empty()
            && let Some(reason) = gpu.clean.as_ref().and_then(down_reason)
        {
            self.clean_note = format!("The clean feed stopped: {reason}");
            changed = true;
        }
        if self.ndi_note.is_empty()
            && let Some(reason) = self.ndi.as_ref().and_then(down_reason)
        {
            self.ndi_note = format!("NDI stopped: {reason}");
            changed = true;
        }
        if changed {
            self.refresh_note();
        }
        changed
    }

    /// Rebuilds the two notes: Spout's with the clean feed's, and NDI's with
    /// the stage's (which only NDI cannot do without). Only on a change.
    fn refresh_note(&mut self) {
        // Both senders failing for one reason (no SpoutLibrary.dll) say it
        // once, not twice in a row.
        let join = |[first, second]: [&String; 2]| {
            if second.is_empty() || second == first {
                first.clone()
            } else if first.is_empty() {
                second.clone()
            } else {
                format!("{first} {second}")
            }
        };
        let notes = [
            join([&self.spout_note, &self.clean_note]),
            join([&self.ndi_note, &self.stage_note]),
        ];
        for (new, old) in notes.iter().zip(&self.notes) {
            if !new.is_empty() && new != old {
                log::warn!("sharing: {new}");
            }
        }
        self.notes = notes;
    }

    /// Rebuilds the status line; returns whether it changed. Twice a second
    /// at most, so its allocation is not a per-frame one.
    fn refresh_status(&mut self, gpu: &GpuShare) -> bool {
        let mut parts = Vec::with_capacity(3);
        if let Some(spout) = gpu.spout.as_ref()
            && spout.status() == Status::Live
        {
            parts.push(match self.spout_size {
                Some([w, h]) => format!("Spout: `{}` live at {w}x{h}", spout.name()),
                None => format!("Spout: `{}` waiting for the first frame", spout.name()),
            });
        }
        if let Some(clean) = gpu.clean.as_ref()
            && clean.status() == Status::Live
        {
            parts.push(match self.clean_size {
                Some([w, h]) => format!("clean feed `{}` live at {w}x{h}", clean.name()),
                None => format!("clean feed `{}` waiting for the first frame", clean.name()),
            });
        }
        if let Some(ndi) = self.ndi.as_ref()
            && ndi.status() == Status::Live
        {
            parts.push(match self.receivers {
                Some(0) => format!(
                    "NDI: `{}` waiting for a receiver (nothing is read back until one connects)",
                    ndi.name()
                ),
                Some(1) => format!(
                    "NDI: `{}` to 1 receiver at {:.0} fps",
                    ndi.name(),
                    self.ndi_fps
                ),
                Some(n) => format!(
                    "NDI: `{}` to {n} receivers at {:.0} fps",
                    ndi.name(),
                    self.ndi_fps
                ),
                None => format!("NDI: `{}` at {:.0} fps", ndi.name(), self.ndi_fps),
            });
        }
        let status = parts.join(" · ");
        let changed = status != self.status;
        self.status = status;
        changed
    }
}

/// Lays this run's sharing flags over the saved choice: `--spout` and
/// `--ndi` turn a sender on under a name, the others set its options. Only
/// `live` changes, so the flags are never saved (D573).
pub(crate) fn apply_flags(cli: &crate::cli::Cli, sharing: &mut crate::ui::state::SharingUi) {
    let live = &mut sharing.live;
    if let Some(name) = cli.spout.as_deref() {
        live.spout = true;
        name.clone_into(&mut live.spout_name);
    }
    if let Some(name) = cli.spout_clean.as_deref() {
        live.spout_clean = true;
        name.clone_into(&mut live.spout_clean_name);
    }
    if let Some(name) = cli.ndi.as_deref() {
        live.ndi = true;
        name.clone_into(&mut live.ndi_name);
    }
    if let Some(groups) = cli.ndi_groups.as_deref() {
        groups.clone_into(&mut live.ndi_groups);
    }
    if let Some(fps) = cli.ndi_fps {
        live.ndi_fps = fps;
    }
    match cli.share_size {
        Some(crate::cli::ShareSize::Picture) => live.size = None,
        Some(crate::cli::ShareSize::Fixed(width, height)) => live.size = Some([width, height]),
        None => {}
    }
    sharing.reset_drafts();
}

/// Why `output`'s sender was taken down, if it was.
fn down_reason(output: &Output) -> Option<&str> {
    (output.status() == Status::Down)
        .then(|| output.reason())
        .flatten()
}

/// Opens a Spout sender named `name`. The failure is worded for the console
/// and the Settings row.
fn open_spout(name: &str, note: &mut String) -> Option<Output> {
    match SpoutSender::open(name) {
        Ok(sender) => {
            log::info!("sharing the output through Spout as `{name}`");
            Some(Output::open(name, Box::new(sender)))
        }
        Err(problem) => {
            *note = problem.to_string();
            None
        }
    }
}

/// Whether NDI's frame-rate cap lets this frame be packed: always without a
/// cap or before the first pack, otherwise once `interval` has nearly
/// passed. A quarter interval of slack, so a 30 fps cap on a 60 Hz show takes
/// every other frame rather than drifting below 30.
fn pack_due(interval: Option<Duration>, last: Option<Instant>, now: Instant) -> bool {
    match (interval, last) {
        (Some(interval), Some(last)) => now.duration_since(last) + interval / 4 >= interval,
        _ => true,
    }
}

/// Opens an NDI sender for `ask`.
fn open_ndi(ask: &NdiAsk, note: &mut String) -> Option<Output> {
    let options = NdiOptions {
        name: ask.name.clone(),
        groups: ask.groups.clone(),
        fps: ask.fps,
    };
    match NdiSender::open(&options) {
        Ok(sender) => {
            log::info!(
                "sharing the output over NDI as `{}`{}",
                ask.name,
                if ask.groups.is_empty() {
                    String::new()
                } else {
                    format!(" in groups {}", ask.groups)
                }
            );
            Some(Output::open(ask.name.clone(), Box::new(sender)))
        }
        Err(problem) => {
            *note = problem.to_string();
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_opened_or_said_while_both_are_off() {
        let mut sharing = Sharing::new();
        let mut gpu = GpuShare::default();
        let live = SharingPref::default();
        assert!(
            sharing.apply(&live, &mut gpu),
            "the first apply is a change"
        );
        assert!(!sharing.active(&gpu), "nothing is shared");
        assert!(
            sharing.notes().iter().all(|note| note.is_empty()),
            "and nothing is wrong"
        );
        assert!(
            !sharing.apply(&live, &mut gpu),
            "the same settings again change nothing"
        );
    }

    #[test]
    fn a_size_or_a_cap_is_a_change_without_a_sender() {
        let mut sharing = Sharing::new();
        let mut gpu = GpuShare::default();
        let mut live = SharingPref::default();
        sharing.apply(&live, &mut gpu);
        live.size = Some([1920, 1080]);
        assert!(sharing.apply(&live, &mut gpu), "a new size applies");
        assert_eq!(sharing.size, Some([1920, 1080]), "and is kept");
        live.ndi_fps = 30;
        assert!(sharing.apply(&live, &mut gpu), "a new cap applies");
        assert_eq!(
            sharing.interval,
            Some(Duration::from_secs_f64(1.0 / 30.0)),
            "as the least time between frames"
        );
    }

    #[test]
    fn the_clean_feed_refuses_the_main_senders_name() {
        let mut sharing = Sharing::new();
        let mut gpu = GpuShare::default();
        let live = SharingPref {
            spout: true,
            spout_clean: true,
            spout_clean_name: SharingPref::default().spout_name,
            ..SharingPref::default()
        };
        sharing.apply(&live, &mut gpu);
        assert!(gpu.clean.is_none(), "no second sender of the same name");
        assert!(
            sharing.notes()[0].contains("name of its own"),
            "and the Spout row says why: {:?}",
            sharing.notes()
        );
    }

    #[test]
    fn one_reason_for_both_spout_senders_is_said_once() {
        let mut sharing = Sharing::new();
        let reason = "Spout output needs SpoutLibrary.dll".to_owned();
        sharing.spout_note.clone_from(&reason);
        sharing.clean_note.clone_from(&reason);
        sharing.refresh_note();
        assert_eq!(sharing.notes()[0], reason, "the same reason, once");
        sharing.clean_note = "The clean feed stopped: gone".to_owned();
        sharing.refresh_note();
        assert_eq!(
            sharing.notes()[0],
            format!("{reason} The clean feed stopped: gone"),
            "two different reasons are both said"
        );
    }

    /// Frames packed in one second of a `hz` show under a `cap`.
    fn packed_in_a_second(hz: u32, cap: u32) -> u32 {
        let interval = Some(Duration::from_secs_f64(1.0 / f64::from(cap)));
        let start = Instant::now();
        let mut last = None;
        let mut packed = 0;
        for frame in 0..hz {
            let now = start + Duration::from_secs_f64(f64::from(frame) / f64::from(hz));
            if pack_due(interval, last, now) {
                last = Some(now);
                packed += 1;
            }
        }
        packed
    }

    #[test]
    fn the_frame_rate_cap_takes_whole_frames_near_the_cap() {
        for (hz, cap, want) in [
            (60, 30, 30),
            (60, 60, 60),
            (75, 30, 38),
            (144, 30, 36),
            (144, 60, 72),
            (30, 60, 30),
        ] {
            let got = packed_in_a_second(hz, cap);
            assert_eq!(got, want, "a {hz} Hz show capped at {cap} fps packs {want}");
        }
        assert!(
            pack_due(None, Some(Instant::now()), Instant::now()),
            "no cap packs every frame"
        );
    }

    #[test]
    fn a_context_change_asks_for_spout_again() {
        let mut sharing = Sharing::new();
        let mut gpu = GpuShare::default();
        let live = SharingPref::default();
        sharing.apply(&live, &mut gpu);
        sharing.context_changed();
        assert!(
            sharing.apply(&live, &mut gpu),
            "after a rebuild the Spout choice is applied again"
        );
    }
}
