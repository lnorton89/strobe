// ============================================================================
// Module:       strobe_app::show::run
// Description:  Show: a loaded playlist being played. Precompiles every
//               scene, ticks the clock, warms the next scene, composites
//               transitions, saves the resume point.
//
// Dependencies: strobe-playlist; strobe-gfx (Compositor, Gl); crate::{audio,
//               session, library, failures}; super::{beat, cache, ...}
// ============================================================================

//! The show.
//!
//! A [`Show`] owns the playlist's [`PlaylistClock`] and every compiled scene
//! and decides, each frame, which scenes render and what the output shows.
//! It does not own the window, the context or the live scene's slot: the app
//! lends those for the length of a call as an [`Env`], so the app's existing
//! machinery (the preview, the resize logic, screenshots, context recovery)
//! keeps treating "the live scene" as one `Option<Session>`.
//!
//! # Where a scene is
//!
//! At any moment each scene of the playlist is in exactly one place:
//!
//! | Place | What it holds | Render size |
//! |---|---|---|
//! | the cache | a compiled, **parked** session | 1x1 |
//! | `Env::active` | the session of the clock's current cue | full |
//! | `Show::outgoing` | the session of the cue being left, during a transition | full |
//! | `Show::warming` | the next cue's session, from `WarmUp` to its start | half each edge |
//!
//! Parking is what keeps video memory bounded (D151): every distinct scene
//! is compiled once at load, because a compile is the only thing that costs
//! frames, but its render targets exist only while it is active, outgoing
//! or warming. Compiling needs no targets and a 4-hour, 80-scene show would
//! otherwise ask a 2012 laptop for gigabytes it does not have. Allocating
//! targets is a few textures, not a compile.
//!
//! # A frame
//!
//! 1. Read the beat from the audio frame and tick the clock.
//! 2. Drain its events into a buffer kept between frames, and act on each:
//!    `CueStarted` swaps the incoming session in, `WarmUp` starts the next
//!    scene rendering small, `TransitionStarted` picks the blend, failures
//!    are logged, blackout is passed to the app.
//! 3. Advance and render the active scene, the outgoing one and the warming
//!    one.
//! 4. During a transition, mix the active and outgoing frames with the
//!    compositor at the clock's `progress`.
//!
//! None of it compiles a shader. [`Show::runtime_compiles`] counts, from
//! the context's own counter, any compile that happens after the load.

use super::beat::BeatReader;
use super::cache::{Progress, SceneCache};
use super::resolve::{self, Resolved};
use super::resume::{ResumeWriter, SaveTimer};
use super::start::{self, Start};
use super::view::{self, CueRow};
use super::watch::{self, Watch};
use crate::audio::AudioInput;
use crate::library::Library;
use crate::session::{SceneClock, Session};
use std::path::{Path, PathBuf};
use strobe_audio::events::{Event, EventDetector};
use strobe_audio::song::SongDetector;
use strobe_diag::failure::{self, Failure, Subsystem};
use strobe_gfx::gl::Gl;
use strobe_gfx::{Blend, Caps, Compositor, MediaRequest, Mouse};
use strobe_playlist::{
    AudioEventKind, ClockConfig, ClockEvent, ClockState, Command, Playlist, PlaylistClock,
    ResolvedCue, ResumePoint, SceneRef, SkipReason, Timeline, TimelineConfig, TransitionKind,
};

/// Whether `new` has the same look as `old`, the cue that was live before a
/// replace: what [`Show::prepare`] would apply is unchanged. Its position
/// may differ, since an edit above it moves it (D432).
fn same_cue(old: &ResolvedCue, new: &ResolvedCue) -> bool {
    old.scene == new.scene
        && old.preset == new.preset
        && old.media == new.media
        && old.overrides == new.overrides
        && old.meta == new.meta
}

/// The longest real-time step one frame gives the playlist clock (D300).
pub const MAX_WALL_STEP: f32 = 1.0;

/// The size a parked scene's targets are cut down to.
pub const PARKED: [i32; 2] = [1, 1];

/// The smallest edge a warm-up renders at.
const MIN_WARM_EDGE: i32 = 16;

/// The render size for a warming scene: half of each edge (a quarter of the
/// pixels), the spec's "quarter resolution".
pub fn warm_size(full: [i32; 2]) -> [i32; 2] {
    [
        (full[0] / 2).max(MIN_WARM_EDGE).min(full[0].max(1)),
        (full[1] / 2).max(MIN_WARM_EDGE).min(full[1].max(1)),
    ]
}

/// What the app lends the show for one call.
pub struct Env<'a> {
    /// The context's GL functions.
    pub gl: &'a Gl,
    /// What the context can do.
    pub caps: &'a Caps,
    /// The live scene's slot (the app's `Stack::session`).
    pub active: &'a mut Option<Session>,
    /// The transition mixer.
    pub compositor: &'a mut Compositor,
    /// The audio input.
    pub audio: &'a mut AudioInput,
    /// The full render size.
    pub full_size: [i32; 2],
    /// Where a scene's clock starts (nonzero in screenshot mode).
    pub start_time: f32,
    /// The pointer.
    pub mouse: Mouse,
    /// `strobe_Date` in local time; its fourth element is seconds since
    /// local midnight.
    pub date: [f32; 4],
    /// The media the operator chose for `syn_Media` (D217), shown in a cue
    /// that names none of its own; `None` leaves each scene its own.
    pub media: Option<&'a Path>,
}

/// How far loading has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Compiling scenes (and transitions); not yet playing.
    Compiling,
    /// The clock is running.
    Ready,
}

/// What one compile step did, for the app to record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepReport {
    /// The scene's name.
    pub scene: String,
    /// Its library entry, when it is one.
    pub library_index: Option<usize>,
    /// Why it failed, in one line, if it did.
    pub failure: Option<String>,
}

/// A message for the operator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Notice {
    /// Whether it is a warning rather than information.
    pub warning: bool,
    /// What to say.
    pub text: String,
}

/// What a frame decided that the app must act on.
#[derive(Debug, Default)]
pub struct FrameReport {
    /// Blackout was switched (by a command or the clock).
    pub blackout: Option<bool>,
    /// A message for the banner.
    pub notice: Option<Notice>,
}

/// The session being warmed.
struct Warm {
    scene: SceneRef,
    /// The cue whose settings it was prepared with.
    index: usize,
    session: Session,
}

/// The session being left.
struct Outgoing {
    scene: SceneRef,
    session: Session,
}

/// A custom transition file and what became of it.
struct Custom {
    path: PathBuf,
    handle: Option<usize>,
    done: bool,
}

/// A playlist being played. See the module docs.
pub struct Show {
    source: Option<PathBuf>,
    base: PathBuf,
    playlist: Playlist,
    clock: PlaylistClock,
    rows: Vec<CueRow>,
    rev: u64,
    /// One resolution per cache slot.
    resolved: Vec<Result<Resolved, String>>,
    cache: SceneCache<Session>,
    customs: Vec<Custom>,
    phase: Phase,
    total_steps: usize,
    warming: Option<Warm>,
    outgoing: Option<Outgoing>,
    active_scene: Option<SceneRef>,
    blend: Option<Blend>,
    composited: bool,
    reader: BeatReader,
    /// Breakdowns and drops heard in the audio, for cues that advance on
    /// them (D226).
    structure: EventDetector,
    /// Hears the next track start (D237).
    song: SongDetector,
    /// The audio source the song detector last heard; a change resets it.
    song_source: Option<crate::audio::Source>,
    /// The show's own clock, which the audio the playlist hears is
    /// evaluated at: summed in `f64`, as a scene's is (D392).
    show_time: SceneClock,
    events: Vec<ClockEvent>,
    waiting: Option<f64>,
    resume_in: Option<ResumePoint>,
    /// The cue that was live when the playlist was replaced, so the restore
    /// can leave the live scene alone if the save did not change that cue.
    carried: Option<ResolvedCue>,
    /// The point to restore is from an earlier version of this playlist.
    lenient: bool,
    writer: Option<ResumeWriter>,
    save: SaveTimer,
    watch: Option<Watch>,
    compiles_baseline: u64,
    runtime_compiles: u64,
    ended_noted: bool,
    notice: Option<Notice>,
}

impl std::fmt::Debug for Show {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Show")
            .field("name", &self.playlist.name)
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}

impl Show {
    /// Prepares to play `playlist`: lays out its timeline, builds the clock,
    /// resolves every scene reference against `library` and marks the ones
    /// that cannot be found as failed. Compiles nothing: that is
    /// [`Self::compile_step`], one scene at a time.
    ///
    /// `source` is the file it came from (relative scene paths and custom
    /// transitions are looked for beside it). `resume` is the saved point to
    /// try (see [`super::start`]); `writer` saves new ones.
    pub fn new(
        source: Option<PathBuf>,
        playlist: Playlist,
        library: &Library,
        resume: Option<ResumePoint>,
        writer: Option<ResumeWriter>,
    ) -> Self {
        let base = source
            .as_deref()
            .and_then(Path::parent)
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        let timeline = Timeline::from_playlist(&playlist, &TimelineConfig::default());
        let clock = PlaylistClock::new(timeline, ClockConfig::default());
        let watch = source.as_deref().map(|path| Watch::new(watch::stamp(path)));
        let mut show = Self {
            source,
            base,
            playlist,
            clock,
            rows: Vec::new(),
            rev: 1,
            resolved: Vec::new(),
            cache: SceneCache::new(Vec::new()),
            customs: Vec::new(),
            phase: Phase::Compiling,
            total_steps: 0,
            warming: None,
            outgoing: None,
            active_scene: None,
            blend: None,
            composited: false,
            reader: BeatReader::new(),
            structure: EventDetector::default(),
            song: SongDetector::new(),
            song_source: None,
            show_time: SceneClock::at(0.0),
            events: Vec::with_capacity(64),
            waiting: None,
            resume_in: resume,
            carried: None,
            lenient: false,
            writer,
            save: SaveTimer::new(),
            watch,
            compiles_baseline: 0,
            runtime_compiles: 0,
            ended_noted: false,
            notice: None,
        };
        show.rebuild_tables(library, None);
        show
    }

    /// Resolves scenes and custom transitions for the current playlist and
    /// clock, reusing `old`'s compiled scenes where the playlist still has
    /// them.
    fn rebuild_tables(&mut self, library: &Library, old: Option<&mut SceneCache<Session>>) {
        let scenes = strobe_playlist::distinct_scenes(&self.playlist);
        let mut cache = SceneCache::new(scenes);
        if let Some(old) = old {
            cache.adopt_from(old);
        }
        self.resolved = (0..cache.len())
            .map(|slot| match cache.scene(slot) {
                Some(scene) => resolve::scene_dir(library, &self.base, scene),
                None => Err("no scene".to_string()),
            })
            .collect();
        // A scene that cannot be found is failed now, without a compile.
        for slot in 0..cache.len() {
            let (Some(scene), Some(Err(reason))) =
                (cache.scene(slot).cloned(), self.resolved.get(slot))
            else {
                continue;
            };
            let reason = reason.clone();
            cache.fill(slot, Err(reason.clone()));
            self.note_unplayable(&scene, &reason);
        }
        // Scenes the cache learned are failed (from an earlier load) are
        // failed on the new clock too.
        for slot in 0..cache.len() {
            if let Some(scene) = cache.scene(slot).cloned()
                && cache.failure(&scene).is_some()
            {
                self.clock.mark_scene_failed(&scene);
            }
        }
        let mut customs: Vec<Custom> = Vec::new();
        for cue in self.clock.timeline().cues() {
            if let TransitionKind::Shader(path) = &cue.resolved.transition.kind
                && !customs.iter().any(|c| c.path == *path)
            {
                customs.push(Custom {
                    path: path.clone(),
                    handle: None,
                    done: false,
                });
            }
        }
        // Custom transitions already compiled for this compositor stay.
        for custom in &mut customs {
            if let Some(previous) = self.customs.iter().find(|c| c.path == custom.path) {
                custom.handle = previous.handle;
                custom.done = previous.done;
            }
        }
        self.customs = customs;
        self.cache = cache;
        self.rows = view::rows(self.clock.timeline());
        self.rev += 1;
        self.total_steps = self.cache.len() + self.customs.len();
        self.phase = Phase::Compiling;
    }

    /// Records that a scene cannot play: its cues are skipped, with a
    /// console entry.
    fn note_unplayable(&mut self, scene: &SceneRef, reason: &str) {
        let cues = self.clock.mark_scene_failed(scene);
        failure::report(
            Failure::new(Subsystem::Playlist, "precompile")
                .scene(scene)
                .message(reason)
                .action(format!("{cues} cue(s) using this scene will be skipped")),
        );
    }

    // ---- queries -----------------------------------------------------------

    /// The playlist's name.
    pub fn name(&self) -> &str {
        &self.playlist.name
    }

    /// The file it was loaded from.
    pub fn source(&self) -> Option<&Path> {
        self.source.as_deref()
    }

    /// The authored playlist.
    pub fn playlist(&self) -> &Playlist {
        &self.playlist
    }

    /// The clock, for the transport and the LIST pane to read.
    pub fn clock(&self) -> &PlaylistClock {
        &self.clock
    }

    /// The timeline rows, rebuilt when the playlist changes.
    pub fn rows(&self) -> &[CueRow] {
        &self.rows
    }

    /// Bumped whenever the rows change.
    pub fn rev(&self) -> u64 {
        self.rev
    }

    /// Loading or playing.
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Precompile progress, counting custom transitions as steps.
    pub fn progress(&self) -> Progress {
        let scenes = self.cache.progress();
        let customs_done = self.customs.iter().filter(|c| c.done).count();
        Progress {
            done: scenes.done + customs_done,
            total: self.total_steps,
            failed: scenes.failed,
        }
    }

    /// Why cue `index` cannot play, if its scene failed.
    pub fn cue_failure(&self, index: usize) -> Option<&str> {
        let cue = self.clock.timeline().cues().get(index)?;
        self.cache.failure(&cue.resolved.scene)
    }

    /// Whether the output should show the compositor's frame rather than the
    /// live scene's.
    pub fn composited(&self) -> bool {
        self.composited
    }

    /// Seconds until the anchored start, while the show waits for it.
    pub fn waiting_for_anchor(&self) -> Option<f64> {
        self.waiting
    }

    /// Shader compiles seen since the load finished. Always 0 in a healthy
    /// show (D152).
    pub fn runtime_compiles(&self) -> u64 {
        self.runtime_compiles
    }

    /// Whether the cue playing names its own `media` (D217), which the
    /// operator's choice does not replace.
    pub fn live_cue_has_media(&self) -> bool {
        self.clock
            .current_cue()
            .is_some_and(|cue| cue.resolved.media.is_some())
    }

    /// The warming scene's cue, while one is warming.
    pub fn warming_cue(&self) -> Option<usize> {
        self.clock.warming().filter(|_| self.warming.is_some())
    }

    /// Takes the pending message for the operator.
    pub fn take_notice(&mut self) -> Option<Notice> {
        self.notice.take()
    }

    /// The name of cue `index`'s scene.
    pub fn cue_scene(&self, index: usize) -> Option<&str> {
        self.rows.get(index).map(|row| row.scene.as_str())
    }

    // ---- loading -----------------------------------------------------------

    /// Does one step of the precompile: one scene's compile, or one custom
    /// transition's. `None` when there is nothing left to compile.
    ///
    /// The app calls this once per control-window frame, after painting, so
    /// the frame that shows the progress is never the one that blocks.
    pub fn compile_step(&mut self, env: &mut Env<'_>) -> Option<StepReport> {
        if let Some(slot) = self.cache.next_pending() {
            return Some(self.compile_scene(slot, env));
        }
        if let Some(position) = self.customs.iter().position(|c| !c.done) {
            return Some(self.compile_custom(position, env));
        }
        if self.phase == Phase::Compiling {
            self.finish_compile(env);
        }
        None
    }

    fn compile_scene(&mut self, slot: usize, env: &mut Env<'_>) -> StepReport {
        let scene = self.cache.scene(slot).cloned();
        let resolved = self.resolved.get(slot).cloned();
        let (Some(scene), Some(Ok(found))) = (scene, resolved) else {
            return StepReport {
                scene: "?".to_string(),
                library_index: None,
                failure: Some("unresolved".to_string()),
            };
        };
        let started = std::time::Instant::now();
        let loaded = Session::load(
            env.gl,
            env.caps,
            &found.dir,
            &found.name,
            PARKED,
            env.start_time,
            env.audio,
        );
        let failure_text = match loaded {
            Ok(mut session) => {
                // One draw at 1x1 makes the driver do its deferred work
                // (Mesa's llvmpipe and several Windows drivers finish a
                // program on its first draw), so it is not left for the
                // first cue.
                session.prime(env.audio, env.start_time);
                self.check_cue_presets(&scene, &session);
                log::info!(
                    "playlist: compiled {} ({}/{}) in {:.0} ms",
                    found.name,
                    self.cache.progress().done + 1,
                    self.cache.len(),
                    started.elapsed().as_secs_f64() * 1000.0
                );
                self.cache.fill(slot, Ok(session));
                None
            }
            Err(error) => {
                let summary = error.to_string();
                self.cache.fill(slot, Err(summary.clone()));
                self.note_unplayable(&scene, &summary);
                Some(summary)
            }
        };
        StepReport {
            scene: found.name,
            library_index: found.library_index,
            failure: failure_text,
        }
    }

    /// Warns, once and early, about a cue that names a preset its scene does
    /// not have, instead of at the cue's warm-up in the middle of a show.
    fn check_cue_presets(&self, scene: &SceneRef, session: &Session) {
        for (index, cue) in self.clock.timeline().cues().iter().enumerate() {
            if cue.resolved.scene != *scene {
                continue;
            }
            if let Some(name) = cue.resolved.preset.as_deref()
                && !session.has_preset(name)
            {
                log::warn!(
                    "playlist: cue {} names preset \"{name}\", which {} does not have; the cue will play with the scene's defaults",
                    index + 1,
                    session.name
                );
            }
        }
    }

    fn compile_custom(&mut self, position: usize, env: &mut Env<'_>) -> StepReport {
        let Some(custom) = self.customs.get(position) else {
            return StepReport {
                scene: "?".to_string(),
                library_index: None,
                failure: None,
            };
        };
        let path = custom.path.clone();
        let full = if path.is_absolute() {
            path.clone()
        } else {
            self.base.join(&path)
        };
        let result = std::fs::read_to_string(&full)
            .map_err(|e| format!("{}: {e}", full.display()))
            .and_then(|source| {
                env.compositor
                    .add_custom(&source)
                    .map_err(|e| format!("{}: {e}", full.display()))
            });
        let label = format!("transition {}", path.display());
        let failure_text = match result {
            Ok(handle) => {
                if let Some(custom) = self.customs.get_mut(position) {
                    custom.handle = Some(handle);
                    custom.done = true;
                }
                None
            }
            Err(reason) => {
                if let Some(custom) = self.customs.get_mut(position) {
                    custom.done = true;
                }
                failure::report(
                    Failure::new(Subsystem::Playlist, "transition")
                        .asset(path.display())
                        .message(&reason)
                        .action("cues using it crossfade instead"),
                );
                Some(reason)
            }
        };
        StepReport {
            scene: label,
            library_index: None,
            failure: failure_text,
        }
    }

    /// Everything is compiled: remember the compile counter, then decide
    /// where the show starts.
    fn finish_compile(&mut self, env: &mut Env<'_>) {
        self.compiles_baseline = env.gl.shader_compiles();
        self.phase = Phase::Ready;
        if let Err(error) = env.compositor.resize(env.full_size) {
            log::warn!("playlist: could not size the compositor: {error}");
        }
        let progress = self.cache.progress();
        log::info!(
            "playlist \"{}\": {} scene(s) compiled, {} failed; shader compiles from here on are counted ({} so far)",
            self.playlist.name,
            progress.total - progress.failed,
            progress.failed,
            self.compiles_baseline
        );
        let resume = self.resume_in.take();
        let outcome = start::plan(
            &mut self.clock,
            resume.as_ref(),
            f64::from(env.date[3]),
            self.lenient,
        );
        self.lenient = false;
        match outcome {
            Start::Fresh => log::info!("playlist: starting at the first cue"),
            Start::Resumed { offset } => log::info!(
                "playlist: resumed the saved position ({:.1} s into the cue)",
                offset
            ),
            Start::ResumeRefused(reason) => {
                log::info!("playlist: not resuming: {reason}; starting at the first cue");
            }
            Start::OnSchedule { offset } => log::info!(
                "playlist: anchored: the schedule says {offset:.1} s into the cue that should be playing, and the clock is there (or on the next playable cue)"
            ),
            Start::WaitingForAnchor { seconds } => {
                self.waiting = Some(seconds);
                self.notice = Some(Notice {
                    warning: false,
                    text: format!(
                        "Waiting for the anchor: the show starts in {}.",
                        view::clock(seconds)
                    ),
                });
            }
            Start::Finished => {
                self.notice = Some(Notice {
                    warning: true,
                    text: "The schedule says this show already finished today.".to_string(),
                });
            }
        }
    }

    /// Swaps in a new version of the playlist (a file reload, or a
    /// generated section). Compiled scenes are kept and only new ones are
    /// compiled; the show returns to where it was (leniently: the playlist
    /// is meant to be the same show, D158). The old clock's position is
    /// restored once the new scenes are compiled.
    pub fn replace_playlist(&mut self, playlist: Playlist, library: &Library) {
        // An edit above the live cue shifts its authored position, so the
        // point is carried to where that cue now is (D432).
        let point = self
            .clock
            .resume_point()
            .or_else(|| self.resume_in.take())
            .map(|point| point.follow(&self.playlist, &playlist));
        if let Some(live) = self.clock.current_cue() {
            self.carried = Some(live.resolved.clone());
        }
        // Scenes in flight go back to the cache so they can be kept.
        if let Some(out) = self.outgoing.take() {
            self.park(out.scene, out.session);
        }
        if let Some(warm) = self.warming.take() {
            self.park(warm.scene, warm.session);
        }
        self.blend = None;
        self.composited = false;
        let mut old = std::mem::replace(&mut self.cache, SceneCache::new(Vec::new()));
        let timeline = Timeline::from_playlist(&playlist, &TimelineConfig::default());
        let held = self.clock.is_held();
        self.clock = PlaylistClock::new(timeline, ClockConfig::default());
        // A held cue stays held through an edit: the operator let go of
        // nothing (D433).
        if held {
            self.clock.hold();
        }
        self.playlist = playlist;
        self.rebuild_tables(library, Some(&mut old));
        self.waiting = None;
        self.resume_in = point;
        self.lenient = true;
        self.ended_noted = false;
        self.save.reset();
        // Anything the old cache still holds belongs to scenes the new
        // playlist dropped; it is freed with `old`.
        drop(old);
    }

    // ---- the frame ---------------------------------------------------------

    /// Runs one frame of the show: tick, react, render. Does nothing until
    /// the precompile is finished.
    ///
    /// `dt` is the scenes' step, which the app clamps so a stall does not
    /// fling their smoothed controls (`stats::MAX_DT`); `wall_dt` is the
    /// real time since the last frame, which the playlist clock, the audio
    /// listeners and the resume timer follow, so a scene drawing at 2 fps
    /// does not stretch a 2-minute cue to 8 (D300). It is capped at
    /// [`MAX_WALL_STEP`]: a frame that waited on a compile or a window drag
    /// does not eat that much of the cue.
    pub fn frame(&mut self, env: &mut Env<'_>, dt: f32, wall_dt: f32) -> FrameReport {
        let mut report = FrameReport::default();
        self.composited = false;
        if self.phase != Phase::Ready {
            return report;
        }
        let wall = if wall_dt.is_finite() && wall_dt > 0.0 {
            wall_dt.min(MAX_WALL_STEP)
        } else {
            dt
        };
        self.show_time.advance(wall);
        if !self.pace_anchor(env, wall) {
            let frame = env.audio.frame_at(self.show_time.seconds());
            let beat = self.reader.read(&frame.scalars, frame.downbeat);
            let [whole, bass] = frame.energy;
            let source = env.audio.source();
            self.clock.tick(f64::from(wall), &beat);
            self.hear(wall, whole, bass);
            self.hear_song(wall, whole, &beat, source);
        }

        self.events.clear();
        self.events.extend(self.clock.drain_events());
        let mut next = 0;
        while let Some(event) = self.events.get(next).copied() {
            next += 1;
            self.handle(event, env, &mut report);
        }
        if self.clock.transition().is_none()
            && let Some(out) = self.outgoing.take()
        {
            self.park(out.scene, out.session);
            self.blend = None;
        }

        if let Some(session) = env.active.as_mut() {
            session.advance(dt, env.audio, env.mouse, env.date);
        }
        if let Some(out) = self.outgoing.as_mut() {
            out.session.advance(dt, env.audio, env.mouse, env.date);
        }
        if let Some(warm) = self.warming.as_mut() {
            warm.session.advance(dt, env.audio, env.mouse, env.date);
        }
        self.composite(env);
        self.account(env, wall);
        if report.notice.is_none() {
            report.notice = self.notice.take();
        }
        report
    }

    /// Says so when cue `index` can end only on an audio event and there is
    /// no live audio to hear it in: it would wait until a person moves on.
    fn warn_if_deaf(&mut self, index: usize, env: &Env<'_>) {
        if env.audio.source() == crate::audio::Source::Live {
            return;
        }
        let Some(advance) = self
            .clock
            .timeline()
            .cues()
            .get(index)
            .map(|cue| &cue.resolved.advance)
        else {
            return;
        };
        let audio_only = !advance.has_timer()
            && (advance.listens_for(AudioEventKind::Drop)
                || advance.listens_for(AudioEventKind::Breakdown)
                || advance.listens_for(AudioEventKind::SongChange));
        if audio_only {
            self.notice = Some(Notice {
                warning: true,
                text: format!(
                    "Cue {} waits for a drop, a breakdown or a new song, but the audio is \
                     synthetic and has none. Press Next to move on.",
                    index + 1
                ),
            });
        }
    }

    /// Listens for a breakdown or a drop and hands it to the clock, which
    /// moves on only if the playing cue advances on that event (D226).
    fn hear(&mut self, dt: f32, whole: f32, bass: f32) {
        let Some(event) = self.structure.step(dt, whole, bass) else {
            return;
        };
        let kind = match event {
            Event::Breakdown => AudioEventKind::Breakdown,
            Event::Drop => AudioEventKind::Drop,
        };
        let moved = self.clock.apply(Command::AudioEvent(kind));
        log::info!(
            "audio: {} heard{}",
            match event {
                Event::Breakdown => "a breakdown",
                Event::Drop => "a drop",
            },
            if moved { "; the cue moves on" } else { "" }
        );
    }

    /// Listens for the next track and hands it to the clock (D237). Only
    /// live audio counts, and any change of source (a device lost, the
    /// synthetic fallback, a device back) starts the detector over, so a
    /// reconnect is never a song change.
    fn hear_song(
        &mut self,
        dt: f32,
        whole: f32,
        beat: &strobe_playlist::BeatInfo,
        source: crate::audio::Source,
    ) {
        if self.song_source != Some(source) {
            self.song_source = Some(source);
            self.song.reset();
        }
        match source {
            crate::audio::Source::Live => {}
            crate::audio::Source::Synthetic => return,
        }
        if !self.song.step(dt, whole, beat.bpm, beat.confidence) {
            return;
        }
        let moved = self
            .clock
            .apply(Command::AudioEvent(AudioEventKind::SongChange));
        log::info!(
            "audio: a new song{}",
            if moved { "; the cue moves on" } else { "" }
        );
    }

    /// While the show waits for its anchor, counts the wait down and starts
    /// the schedule when it is over. Returns whether the clock is held back.
    fn pace_anchor(&mut self, env: &Env<'_>, dt: f32) -> bool {
        let Some(remaining) = self.waiting.as_mut() else {
            return false;
        };
        *remaining -= f64::from(dt);
        if *remaining > 0.0 {
            return true;
        }
        self.waiting = None;
        if let Some(placement) = self.clock.sync_to_wall_clock(f64::from(env.date[3])) {
            log::info!("playlist: the anchor has arrived ({placement:?})");
        }
        false
    }

    fn handle(&mut self, event: ClockEvent, env: &mut Env<'_>, report: &mut FrameReport) {
        match event {
            ClockEvent::CueStarted { index, pass } => {
                log::info!(
                    "playlist: cue {} started: {} (loop pass {pass})",
                    index + 1,
                    self.cue_scene(index).unwrap_or("?")
                );
                // A jump can bring an ended show back; its next end is
                // noted (and its resume point cleared) again.
                self.ended_noted = false;
                self.begin_cue(index, env);
                self.warn_if_deaf(index, env);
            }
            ClockEvent::TransitionStarted { id, from, to } => {
                let blend = self.blend_for(to);
                log::info!(
                    "playlist: transition {id} started: cue {} to cue {} ({blend:?})",
                    from + 1,
                    to + 1
                );
                self.blend = Some(blend);
                if let Err(error) = env.compositor.resize(env.full_size) {
                    log::warn!("playlist: could not size the compositor: {error}");
                }
            }
            ClockEvent::TransitionFinished { id, completed } => {
                log::info!(
                    "playlist: transition {id} {}",
                    if completed { "finished" } else { "cut short" }
                );
            }
            ClockEvent::WarmUp { index } => self.warm_up(index, env),
            ClockEvent::CueSkipped { index, reason } => {
                let why = match reason {
                    SkipReason::SceneFailed => "its scene failed to compile",
                };
                log::warn!(
                    "playlist: cue {} ({}) skipped: {why}",
                    index + 1,
                    self.cue_scene(index).unwrap_or("?")
                );
            }
            ClockEvent::HoldChanged(held) => {
                log::info!("playlist: {}", if held { "held" } else { "released" });
            }
            ClockEvent::BlackoutChanged(on) => report.blackout = Some(on),
            ClockEvent::PlaylistEnded => {
                if !self.ended_noted {
                    self.ended_noted = true;
                    log::info!("playlist: ended");
                    if let Some(writer) = &self.writer {
                        writer.clear();
                    }
                    report.notice = Some(Notice {
                        warning: false,
                        text: format!(
                            "\"{}\" has ended. The last scene stays up.",
                            self.playlist.name
                        ),
                    });
                }
            }
        }
    }

    /// The compositor blend for the transition into cue `to`.
    fn blend_for(&self, to: usize) -> Blend {
        let kind = self
            .clock
            .timeline()
            .cues()
            .get(to)
            .map(|cue| &cue.resolved.transition.kind);
        match kind {
            Some(TransitionKind::FadeThroughBlack) => Blend::FadeThroughBlack,
            Some(TransitionKind::Shader(path)) => self
                .customs
                .iter()
                .find(|c| c.path == *path)
                .and_then(|c| c.handle)
                .map_or(Blend::Crossfade, Blend::Custom),
            Some(TransitionKind::Crossfade | TransitionKind::Cut) | None => Blend::Crossfade,
        }
    }

    /// Starts the next scene rendering small, so a feedback scene is not
    /// black when its cue starts.
    fn warm_up(&mut self, index: usize, env: &mut Env<'_>) {
        let Some(scene) = self
            .clock
            .timeline()
            .cues()
            .get(index)
            .map(|cue| cue.resolved.scene.clone())
        else {
            return;
        };
        if self.active_scene.as_ref() == Some(&scene) {
            return;
        }
        // Already warming for another cue of the same scene (a jump): it
        // takes this cue's settings.
        if let Some(mut warm) = self.warming.take_if(|w| w.scene == scene) {
            if warm.index != index {
                self.prepare(&mut warm.session, index, env.start_time, env.media);
                warm.index = index;
            }
            self.warming = Some(warm);
            return;
        }
        if let Some(old) = self.warming.take() {
            self.park(old.scene, old.session);
        }
        let Some(mut session) = self.unpark(&scene, env) else {
            return;
        };
        self.prepare(&mut session, index, env.start_time, env.media);
        let size = warm_size(env.full_size);
        if let Err(error) = session.player.resize(size) {
            log::warn!("playlist: could not warm up {scene}: {error}");
            self.park(scene, session);
            return;
        }
        log::info!(
            "playlist: warming up cue {} ({}) at {}x{}",
            index + 1,
            session.name,
            size[0],
            size[1]
        );
        self.warming = Some(Warm {
            scene,
            index,
            session,
        });
    }

    /// Restarts a session's clock and applies a cue's control settings and
    /// media.
    fn prepare(&self, session: &mut Session, index: usize, start_time: f32, media: Option<&Path>) {
        session.restart_clock(start_time);
        resolve::reset_controls(&mut session.controls);
        resolve::reset_controls(&mut session.meta);
        let Some(cue) = self.clock.timeline().cues().get(index) else {
            return;
        };
        // Settings layer, later over earlier: the scene's defaults, then its
        // `default` preset (or the cue's own preset, which replaces it), then
        // the cue's explicit overrides. Presets were read when the scene was
        // compiled, so this touches no disk (D176).
        match cue.resolved.preset.as_deref() {
            Some(name) => match session.apply_preset(name) {
                Ok(problems) => {
                    for problem in problems {
                        log::warn!("playlist: cue {}: preset \"{name}\": {problem}", index + 1);
                    }
                }
                Err(problem) => {
                    // The cue was warned about at compile time; it still
                    // gets the scene's own starting look, not bare defaults.
                    log::warn!("playlist: cue {}: {problem}", index + 1);
                    session.apply_startup_preset();
                }
            },
            None => session.apply_startup_preset(),
        }
        // The cue's own media, then the operator's, then the preset's, then
        // the scene's (D217, D223). Asking for what already shows starts
        // nothing, so a cue that restarts its scene does not reload its
        // pictures.
        let preset_media = match cue.resolved.preset.as_deref() {
            Some(name) => session.preset_media(name),
            None => session.startup_media(),
        };
        let chosen = match &cue.resolved.media {
            Some(path) => Some(self.base.join(path)),
            None => media.map(Path::to_path_buf).or(preset_media),
        };
        session
            .player
            .choose_media(chosen.map(MediaRequest::for_path));
        for problem in resolve::apply_overrides(&mut session.controls, &cue.resolved.overrides) {
            log::warn!("playlist: cue {}: {problem}", index + 1);
        }
        // Meta values go to the session's meta bank (D160); nothing renders
        // from it yet, but a script or a later post stage will read them.
        for problem in resolve::apply_overrides(&mut session.meta, &cue.resolved.meta.values) {
            log::warn!("playlist: cue {} meta: {problem}", index + 1);
        }
    }

    /// Makes cue `index`'s scene the live one.
    fn begin_cue(&mut self, index: usize, env: &mut Env<'_>) {
        let Some(scene) = self
            .clock
            .timeline()
            .cues()
            .get(index)
            .map(|cue| cue.resolved.scene.clone())
        else {
            return;
        };
        let carried = self.carried.take();
        // The same scene again (a restart, or two cues in a row): keep the
        // session, give it the cue's settings. Unless this is a reload
        // putting the clock back on the cue that was live, which the save
        // did not change: then the live scene carries on as it was, its
        // clock and the operator's changes included (D158).
        let media = env.media;
        if self.active_scene.as_ref() == Some(&scene)
            && let Some(session) = env.active.as_mut()
        {
            let untouched = carried
                .as_ref()
                .zip(self.clock.timeline().cues().get(index))
                .is_some_and(|(old, cue)| same_cue(old, &cue.resolved));
            if !untouched {
                self.prepare_active(session, index, env.start_time, media);
            }
            return;
        }
        // A transition that was still running is over.
        if let Some(out) = self.outgoing.take() {
            self.park(out.scene, out.session);
        }
        // A warmed session was prepared for the cue it was warmed for; a
        // jump or a step back can start another cue of the same scene.
        let warmed = self
            .warming
            .take_if(|warm| warm.scene == scene)
            .map(|mut warm| {
                if warm.index != index {
                    self.prepare(&mut warm.session, index, env.start_time, media);
                }
                warm.session
            });
        let incoming = match warmed {
            Some(mut session) => match session.player.resize_keeping(env.full_size) {
                Ok(()) => Some(session),
                Err(error) => {
                    log::warn!("playlist: could not grow the warmed-up scene: {error}");
                    Some(session)
                }
            },
            None => self.unpark(&scene, env).and_then(|mut session| {
                self.prepare(&mut session, index, env.start_time, media);
                match session.player.resize(env.full_size) {
                    Ok(()) => Some(session),
                    Err(error) => {
                        log::error!("playlist: could not allocate {}: {error}", session.name);
                        self.park(scene.clone(), session);
                        None
                    }
                }
            }),
        };
        let Some(mut session) = incoming else {
            log::error!(
                "playlist: cue {} has no scene to play; skipping it",
                index + 1
            );
            self.clock.mark_failed(index);
            return;
        };
        // The operator's output settings carry over, under whatever the
        // cue, its preset or the scene's script set (D207).
        if let Some(live) = env.active.as_ref() {
            session.meta.carry_unchanged_from(&live.meta);
        }
        let previous = env.active.replace(session);
        let previous_scene = self.active_scene.replace(scene);
        if let (Some(session), Some(scene)) = (previous, previous_scene) {
            self.outgoing = Some(Outgoing { scene, session });
        }
    }

    fn prepare_active(
        &self,
        session: &mut Session,
        index: usize,
        start_time: f32,
        media: Option<&Path>,
    ) {
        // Keep the picture (a restart should not flash black), start the
        // clock over and apply the cue's settings. The output settings carry
        // over as they do between scenes (D207); one copy per cue, not per
        // frame.
        let before = session.meta.clone();
        self.prepare(session, index, start_time, media);
        session.meta.carry_unchanged_from(&before);
    }

    /// Takes `scene` out of the cache for a new cue, every pass cleared and
    /// its script started over. Parking keeps what a resize does not touch:
    /// a literal-size or `FIXED_SIZE` pass (D287), and all of a scene that
    /// pins its final pass (D280). A cue starts its scene's clock at the
    /// beginning, so its buffers start there too, as they did before D287
    /// (D297), and so does its script's state (D323).
    fn unpark(&mut self, scene: &SceneRef, env: &mut Env<'_>) -> Option<Session> {
        let mut session = self.cache.take(scene)?;
        if let Err(error) = session.player.restart() {
            log::warn!(
                "playlist: could not clear {}'s passes: {error}",
                session.name
            );
        }
        session.restart_script(env.audio, env.start_time);
        Some(session)
    }

    /// Returns a session to the cache, its targets cut down to nothing.
    fn park(&mut self, scene: SceneRef, mut session: Session) {
        if let Err(error) = session.player.resize(PARKED) {
            log::warn!("playlist: could not park {}: {error}", session.name);
        }
        self.cache.put_back(&scene, session);
    }

    /// Mixes the live and outgoing frames while a transition runs.
    fn composite(&mut self, env: &mut Env<'_>) {
        let (Some(progress), Some(out), Some(active), Some(blend)) = (
            self.clock.transition(),
            self.outgoing.as_ref(),
            env.active.as_ref(),
            self.blend,
        ) else {
            return;
        };
        env.compositor.render(
            out.session.player.output_texture(),
            active.player.output_texture(),
            blend,
            progress.progress as f32,
        );
        self.composited = true;
    }

    /// Per-frame bookkeeping that is not rendering: the compile counter and
    /// the resume point.
    fn account(&mut self, env: &Env<'_>, dt: f32) {
        let compiles = env.gl.shader_compiles();
        if compiles != self.compiles_baseline {
            let extra = compiles - self.compiles_baseline;
            self.runtime_compiles += extra;
            self.compiles_baseline = compiles;
            log::warn!(
                "playlist: {extra} shader compile(s) happened during playback ({} in all); a scene's script created a uniform after load",
                self.runtime_compiles
            );
        }
        if let Some(point) = self.save.tick(f64::from(dt), self.clock.resume_point())
            && let Some(writer) = &self.writer
        {
            writer.save(point);
        }
    }

    /// Resizes what the show renders at the full size: the active scene is
    /// the app's, so this is the outgoing scene, the warming one and the
    /// compositor.
    pub fn resize(&mut self, full: [i32; 2], compositor: &mut Compositor) {
        if let Some(out) = self.outgoing.as_mut()
            && let Err(error) = out.session.player.resize(full)
        {
            log::warn!("playlist: resizing the outgoing scene failed: {error}");
        }
        if let Some(warm) = self.warming.as_mut()
            && let Err(error) = warm.session.player.resize(warm_size(full))
        {
            log::warn!("playlist: resizing the warming scene failed: {error}");
        }
        if let Err(error) = compositor.resize(full) {
            log::warn!("playlist: resizing the compositor failed: {error}");
        }
    }

    // ---- commands ----------------------------------------------------------

    /// Runs a transport command on the clock. Its effects arrive as events
    /// on the next frame.
    pub fn command(&mut self, command: Command) -> bool {
        self.clock.apply(command)
    }

    /// Whether the clock has nothing left to play.
    pub fn ended(&self) -> bool {
        matches!(self.clock.state(), ClockState::Ended)
    }

    // ---- the file ----------------------------------------------------------

    /// Counts `dt` and, once a second, looks at the playlist file. Returns
    /// its path when it has been saved and has settled (D158).
    pub fn poll_file(&mut self, dt: f64) -> Option<PathBuf> {
        let source = self.source.clone()?;
        let watch = self.watch.as_mut()?;
        if !watch.due(dt) {
            return None;
        }
        watch.check(watch::stamp(&source)).then_some(source)
    }

    /// Records the file's current version as the one the show has.
    pub fn accept_file(&mut self) {
        if let (Some(source), Some(watch)) = (self.source.as_deref(), self.watch.as_mut()) {
            watch.accept(watch::stamp(source));
        }
    }

    /// Sets the file the playlist is saved to (after a save-as).
    pub fn set_source(&mut self, path: PathBuf) {
        self.base = path
            .parent()
            .map_or_else(|| PathBuf::from("."), Path::to_path_buf);
        self.watch = Some(Watch::new(watch::stamp(&path)));
        self.source = Some(path);
    }

    // ---- ending ------------------------------------------------------------

    /// What a reload after a lost context needs: the playlist, where it was
    /// loaded from and where playback was.
    pub fn reload_spec(&self) -> (Option<PathBuf>, Playlist, Option<ResumePoint>) {
        (
            self.source.clone(),
            self.playlist.clone(),
            self.clock.resume_point().or(self.resume_in),
        )
    }

    /// Writes the last position and stops the resume thread. Call with the
    /// context still current, then drop the show: its sessions free their
    /// GL objects when they drop.
    pub fn finish(&mut self) {
        if let Some(writer) = self.writer.take() {
            if !self.ended()
                && let Some(point) = self.clock.resume_point()
            {
                writer.save(point);
            }
            writer.finish();
        }
        let progress = self.progress();
        log::info!(
            "playlist \"{}\": closed with {} runtime shader compile(s) ({} scene(s) compiled at load)",
            self.playlist.name,
            self.runtime_compiles,
            progress.done - progress.failed.min(progress.done)
        );
    }

    /// The parked-and-active bookkeeping, for tests: scenes the cache holds
    /// parked.
    pub fn parked_count(&self) -> usize {
        (0..self.cache.len())
            .filter_map(|slot| self.cache.scene(slot))
            .filter(|scene| self.cache.is_parked(scene))
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn warm_up_renders_half_of_each_edge_but_never_below_the_floor() {
        assert_eq!(
            warm_size([1920, 1080]),
            [960, 540],
            "a quarter of the pixels"
        );
        assert_eq!(warm_size([1280, 720]), [640, 360], "720p");
        assert_eq!(warm_size([20, 20]), [16, 16], "the floor");
        assert_eq!(warm_size([8, 8]), [8, 8], "never larger than the full size");
    }
}
