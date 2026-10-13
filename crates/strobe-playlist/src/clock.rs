// ============================================================================
// Module:       strobe_playlist::clock
// Description:  PlaylistClock, the pure playback state machine: which cue is
//               active, when it ends, transitions, hold, beat-locked ends,
//               auto-skip and the events the app drains.
//
// Dependencies: crate::timeline, crate::tempo, crate::resume, crate::schedule
// ============================================================================

//! The playlist clock.
//!
//! [`PlaylistClock`] is the run-time half of the crate: the timeline says
//! what the plan is, the clock says where the show *is*. It has no GL, no
//! threads, no system clock and no I/O. Time arrives as `dt` in
//! [`tick`](PlaylistClock::tick) and the music as a [`BeatInfo`]; everything
//! it wants the app to do comes back as [`ClockEvent`]s. That makes the
//! whole state machine testable (and soak-testable for four hours of show
//! in a few milliseconds) without a window.
//!
//! # Two clocks, kept apart
//!
//! This is the *scene clock*: it advances by frame time, stops while held
//! and is pushed around by commands. The *wall clock* lives in
//! [`schedule`](crate::schedule), reads the timeline and never touches
//! this state; the app asks it where the show should be and, on a late
//! start, tells the clock with [`sync_to_wall_clock`](PlaylistClock::sync_to_wall_clock).
//!
//! # What the clock answers
//!
//! - [`current`](PlaylistClock::current), [`elapsed`](PlaylistClock::elapsed)
//!   and [`remaining`](PlaylistClock::remaining): the active cue, how long
//!   it has run, how long until it ends (`None` for a manual cue).
//! - [`transition`](PlaylistClock::transition): while the show is arriving
//!   at a cue, `from`, `to` and `progress` in `0..1`.
//! - [`is_held`](PlaylistClock::is_held), [`is_blackout`](PlaylistClock::is_blackout).
//! - [`warming`](PlaylistClock::warming) and the [`ClockEvent::WarmUp`]
//!   event: two seconds before a cue ends, the cue to start rendering
//!   off-screen.
//!
//! # Beat-locked ends
//!
//! A cue whose length is in `Bars` or `Beats` has its computed length fixed
//! when it *starts*: beats times 60 over the BPM then in force, which is the
//! live BPM if the analysis is confident
//! ([`ClockConfig::confidence_threshold`]) and the timeline's fallback
//! otherwise. When that time is up the cue does not end; it waits for the
//! next downbeat (the beat grid entering a new bar) and ends there, so a
//! phrase finishes on the one. The wait is abandoned, and the cue ends at
//! once, if confidence is lost, or if no downbeat arrives within six beats
//! (a stalled grid must not strand the show). Seconds cues end exactly on
//! time. Hold, `extend`, and a timer-less (manual) cue behave as described
//! on the methods.
//!
//! # No allocation in steady state
//!
//! `tick` and the queries allocate nothing. All storage is sized when the
//! clock is built: the failed-cue flags (one `bool` per cue) and the event
//! queue, which reserves its maximum ([`MAX_PENDING_EVENTS`]) up front and
//! never grows. Events carry indices and ids, no strings; the cue data they
//! refer to stays in the timeline and is borrowed, not copied. Candidate
//! search for the next cue is arithmetic over indices. If the app stops
//! draining, further events are counted in
//! [`dropped_events`](PlaylistClock::dropped_events) rather than queued.

use crate::model::{AudioEventKind, LoopMode, SceneRef, TransitionKind};
use crate::resume::{ResumeError, ResumePoint};
use crate::schedule::{Placement, elapsed_since_anchor};
use crate::tempo::BeatInfo;
use crate::timeline::{MIN_CUE_SECONDS, Timeline, TimelineCue};
use std::vec::Drain;

/// Confidence above which the live BPM is trusted.
pub const DEFAULT_CONFIDENCE_THRESHOLD: f32 = 0.5;

/// How long before a cue ends its successor should start warming up.
pub const DEFAULT_WARMUP_LEAD_SECONDS: f64 = 2.0;

/// Events queued before the app has drained any; later ones are dropped
/// and counted. Reserved at construction so the queue never reallocates.
pub const MAX_PENDING_EVENTS: usize = 1024;

/// Beats a cue may wait past its computed end for a downbeat.
const LOCK_TIMEOUT_BEATS: f64 = 6.0;

/// Beats of slack when asking which bar the computed end fell in.
const DOWNBEAT_EPSILON_BEATS: f64 = 1.0e-6;

/// Cue changes one tick may make, so a huge `dt` or a pathological
/// playlist cannot spin.
const MAX_CHANGES_PER_TICK: u32 = 4096;

/// Tuning for the clock.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClockConfig {
    /// `BeatInfo::confidence` must exceed this for the live BPM to be used
    /// and for beat-locking to apply.
    pub confidence_threshold: f32,
    /// How many seconds before a cue ends [`ClockEvent::WarmUp`] fires.
    pub warmup_lead: f64,
}

impl Default for ClockConfig {
    fn default() -> Self {
        Self {
            confidence_threshold: DEFAULT_CONFIDENCE_THRESHOLD,
            warmup_lead: DEFAULT_WARMUP_LEAD_SECONDS,
        }
    }
}

/// Where the clock is in its life.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockState {
    /// Built, not started: the first `tick` starts the first playable cue.
    Idle,
    /// A cue is active.
    Playing,
    /// A non-looping playlist ran out (or nothing was playable).
    Ended,
}

/// Why a cue was passed over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkipReason {
    /// Its scene failed to compile (or load) and was marked failed.
    SceneFailed,
}

/// Something the app should react to. Indices are timeline indices
/// ([`Timeline::cues`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockEvent {
    /// A cue became active.
    CueStarted {
        /// The cue.
        index: usize,
        /// Which pass over the playlist.
        pass: u64,
    },
    /// The show began arriving at `to` from `from`; render both and mix.
    TransitionStarted {
        /// Unique per transition, never reused.
        id: u64,
        /// The outgoing cue.
        from: usize,
        /// The incoming (now active) cue.
        to: usize,
    },
    /// A transition ended. `completed` is false if it was cut short by a
    /// command (the show moved on before it finished).
    TransitionFinished {
        /// The id from `TransitionStarted`.
        id: u64,
        /// Whether it ran to the end.
        completed: bool,
    },
    /// Start rendering this cue off-screen now: it becomes active in about
    /// [`ClockConfig::warmup_lead`] seconds. Fires once per cue.
    WarmUp {
        /// The cue to warm.
        index: usize,
    },
    /// A cue was passed over.
    CueSkipped {
        /// The cue.
        index: usize,
        /// Why.
        reason: SkipReason,
    },
    /// Hold was switched on or off.
    HoldChanged(bool),
    /// Blackout was switched on or off.
    BlackoutChanged(bool),
    /// The playlist is over.
    PlaylistEnded,
}

/// The live controls, as data, for MIDI/OSC/hotkey mapping.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
    /// Go to the next playable cue.
    Next,
    /// Go to the previous playable cue.
    Previous,
    /// Freeze the cue's clock.
    Hold,
    /// Release the hold.
    Unhold,
    /// Flip the hold.
    ToggleHold,
    /// Add (or, if negative, remove) seconds on the current cue.
    Extend(f64),
    /// Start the current cue again.
    RestartCue,
    /// Go to a timeline cue.
    JumpTo(usize),
    /// Go to the first playable cue of the next section.
    SkipSection,
    /// Flip blackout.
    ToggleBlackout,
    /// An audio event happened (Phase 7 detection).
    AudioEvent(AudioEventKind),
}

/// A transition in progress.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TransitionProgress {
    /// Matches `ClockEvent::TransitionStarted::id`.
    pub id: u64,
    /// The outgoing cue.
    pub from: usize,
    /// The incoming cue.
    pub to: usize,
    /// 0 at the start, approaching 1 at the end.
    pub progress: f64,
}

/// A transition's mutable state.
#[derive(Debug, Clone, Copy)]
struct ActiveTransition {
    id: u64,
    from: usize,
    to: usize,
    elapsed: f64,
    length: f64,
}

/// A candidate next position.
#[derive(Debug, Clone, Copy)]
struct Step {
    index: usize,
    pass: u64,
}

/// The playlist playback state machine. See the module docs.
#[derive(Debug)]
pub struct PlaylistClock {
    timeline: Timeline,
    config: ClockConfig,
    state: ClockState,
    index: usize,
    pass: u64,
    /// Seconds the active cue has run (frozen while held).
    elapsed: f64,
    /// The active cue's computed length, fixed when it started.
    base_len: f64,
    /// Seconds added or removed by `extend`.
    extension: f64,
    /// Whether the active cue's length is counted in beats.
    beat_locked: bool,
    /// While waiting for a downbeat: the bar the computed end fell in.
    lock_bar: Option<i64>,
    /// While waiting for a downbeat: seconds waited so far.
    lock_waited: f64,
    held: bool,
    blackout: bool,
    transition: Option<ActiveTransition>,
    next_transition_id: u64,
    warmed: bool,
    last_beat: BeatInfo,
    failed: Vec<bool>,
    events: Vec<ClockEvent>,
    dropped_events: u64,
}

impl PlaylistClock {
    /// A clock over `timeline`, not yet started.
    pub fn new(timeline: Timeline, config: ClockConfig) -> Self {
        let count = timeline.len();
        Self {
            timeline,
            config,
            state: ClockState::Idle,
            index: 0,
            pass: 0,
            elapsed: 0.0,
            base_len: 0.0,
            extension: 0.0,
            beat_locked: false,
            lock_bar: None,
            lock_waited: 0.0,
            held: false,
            blackout: false,
            transition: None,
            next_transition_id: 0,
            warmed: false,
            last_beat: BeatInfo::SILENT,
            failed: vec![false; count],
            events: Vec::with_capacity(MAX_PENDING_EVENTS),
            dropped_events: 0,
        }
    }

    // ---- queries ---------------------------------------------------------

    /// The timeline this clock plays.
    pub fn timeline(&self) -> &Timeline {
        &self.timeline
    }

    /// Idle, playing or ended.
    pub fn state(&self) -> ClockState {
        self.state
    }

    /// The active cue's timeline index, while playing.
    pub fn current(&self) -> Option<usize> {
        matches!(self.state, ClockState::Playing).then_some(self.index)
    }

    /// The active cue, while playing.
    pub fn current_cue(&self) -> Option<&TimelineCue> {
        self.current()
            .and_then(|index| self.timeline.cues().get(index))
    }

    /// Which pass over the playlist this is (0 on the first).
    pub fn pass(&self) -> u64 {
        self.pass
    }

    /// Seconds the active cue has played, not counting time held.
    pub fn elapsed(&self) -> f64 {
        self.elapsed
    }

    /// The active cue's total length: its computed length plus extensions.
    pub fn cue_length(&self) -> f64 {
        (self.base_len + self.extension).max(0.0)
    }

    /// Seconds until the active cue ends, or `None` if nothing but a
    /// person (or an audio event) ends it. Zero while a beat-locked cue
    /// waits for its downbeat.
    pub fn remaining(&self) -> Option<f64> {
        self.deadline()
            .map(|deadline| (deadline - self.elapsed).max(0.0))
    }

    /// Whether the cue clock is frozen.
    pub fn is_held(&self) -> bool {
        self.held
    }

    /// Whether the output should be black.
    pub fn is_blackout(&self) -> bool {
        self.blackout
    }

    /// The transition in progress, if the show is still arriving at the
    /// active cue.
    pub fn transition(&self) -> Option<TransitionProgress> {
        self.transition.map(|t| TransitionProgress {
            id: t.id,
            from: t.from,
            to: t.to,
            progress: if t.length > 0.0 {
                (t.elapsed / t.length).clamp(0.0, 1.0)
            } else {
                1.0
            },
        })
    }

    /// The cue that should be rendering off-screen right now, once the
    /// warm-up lead has been reached.
    pub fn warming(&self) -> Option<usize> {
        if self.warmed && matches!(self.state, ClockState::Playing) {
            self.peek_next()
        } else {
            None
        }
    }

    /// The cue a `next` would go to, without going.
    pub fn peek_next(&self) -> Option<usize> {
        if !matches!(self.state, ClockState::Playing) {
            return None;
        }
        self.scan_forward(self.index, self.pass, false, |_| true)
            .found
            .map(|(_, step)| step.index)
    }

    /// Whether the cue is marked failed.
    pub fn is_failed(&self, index: usize) -> bool {
        self.failed.get(index).copied().unwrap_or(false)
    }

    /// Events not yet drained.
    pub fn events(&self) -> &[ClockEvent] {
        &self.events
    }

    /// Hands the app the pending events and empties the queue, keeping its
    /// storage. Events not consumed from the iterator are still removed.
    pub fn drain_events(&mut self) -> Drain<'_, ClockEvent> {
        self.events.drain(..)
    }

    /// Events lost because the queue was full.
    pub fn dropped_events(&self) -> u64 {
        self.dropped_events
    }

    /// The event queue's reserved capacity; it never changes after
    /// construction.
    pub fn event_capacity(&self) -> usize {
        self.events.capacity()
    }

    // ---- driving ---------------------------------------------------------

    /// Advances the show by `dt` seconds with the music as `beat`.
    ///
    /// Starts the first cue if the clock is idle. Ends cues whose time is up
    /// (carrying leftover time into the next one, so a long frame does not
    /// lose time), starts and finishes transitions and queues events.
    pub fn tick(&mut self, dt: f64, beat: &BeatInfo) {
        self.last_beat = *beat;
        match self.state {
            ClockState::Idle => self.start_first(),
            ClockState::Ended => return,
            ClockState::Playing => {}
        }
        // A clock that has just started goes on to spend this tick's time on
        // its first cue; one that found nothing to play has nothing to do.
        if !matches!(self.state, ClockState::Playing) {
            return;
        }
        let mut dt = if dt.is_finite() && dt > 0.0 { dt } else { 0.0 };
        self.advance_transition(dt);

        for _ in 0..MAX_CHANGES_PER_TICK {
            if self.held {
                break;
            }
            let Some(deadline) = self.deadline() else {
                self.elapsed += dt;
                break;
            };
            let room = deadline - self.elapsed;
            if room > dt {
                self.elapsed += dt;
                break;
            }
            // The deadline falls within this step.
            if self.beat_locked
                && let Some(bpm) = beat.confident_bpm(self.config.confidence_threshold)
            {
                if self.wait_for_downbeat(dt, room, bpm, beat) {
                    self.elapsed += dt;
                    break;
                }
                // The downbeat came: end now and start the next cue clean.
                self.advance_forward(false, None, true);
                break;
            }
            // On time: end exactly at the deadline and carry the rest.
            if room > 0.0 {
                dt -= room;
            }
            self.advance_forward(false, None, true);
            if !matches!(self.state, ClockState::Playing) {
                break;
            }
            self.advance_transition(dt);
        }
        self.check_warmup();
    }

    /// One step of the downbeat wait. Returns whether to keep waiting.
    fn wait_for_downbeat(&mut self, dt: f64, room: f64, bpm: f64, beat: &BeatInfo) -> bool {
        let bar_now = beat.bar();
        match self.lock_bar {
            None => {
                // The deadline passed `over` seconds ago, inside this step;
                // find which bar the grid was in at that moment.
                let over = dt - room.max(0.0);
                let beats_back = over * bpm / 60.0;
                // A downbeat exactly at the computed end counts as "after" it,
                // hence the epsilon: 8 beats from a downbeat ends on the next.
                self.lock_bar = Some(beat.bar_at(beat.beats - beats_back - DOWNBEAT_EPSILON_BEATS));
                self.lock_waited = over;
            }
            Some(bar) => {
                self.lock_waited += dt;
                if bar_now < bar {
                    // The grid restarted; wait for its next bar instead.
                    self.lock_bar = Some(bar_now);
                }
            }
        }
        let timeout = LOCK_TIMEOUT_BEATS * 60.0 / bpm;
        let reached = self.lock_bar.is_some_and(|bar| bar_now > bar);
        !(reached || self.lock_waited > timeout)
    }

    /// Runs one command. Returns whether it did anything.
    pub fn apply(&mut self, command: Command) -> bool {
        match command {
            Command::Next => self.next_cue(),
            Command::Previous => self.previous_cue(),
            Command::Hold => self.hold(),
            Command::Unhold => self.unhold(),
            Command::ToggleHold => {
                if self.held {
                    self.unhold()
                } else {
                    self.hold()
                }
            }
            Command::Extend(seconds) => self.extend(seconds),
            Command::RestartCue => self.restart_cue(),
            Command::JumpTo(index) => self.jump_to(index),
            Command::SkipSection => self.skip_section(),
            Command::ToggleBlackout => {
                self.set_blackout(!self.blackout);
                true
            }
            Command::AudioEvent(kind) => self.audio_event(kind),
        }
    }

    /// Goes to the next playable cue (ending the playlist if there is none
    /// and it does not loop). Failed cues passed over are reported.
    pub fn next_cue(&mut self) -> bool {
        if !matches!(self.state, ClockState::Playing) {
            return false;
        }
        self.advance_forward(false, None, true);
        true
    }

    /// Goes to the previous playable cue. At the start of a non-looping
    /// playlist it restarts the first cue instead.
    pub fn previous_cue(&mut self) -> bool {
        if !matches!(self.state, ClockState::Playing) {
            return false;
        }
        let scan = self.scan_backward();
        self.report_skips(
            scan.tried,
            scan.found.map(|(k, _)| k),
            Direction::Backward,
            false,
            None,
        );
        match scan.found {
            Some((_, step)) => self.begin_cue(step.index, step.pass, Some(self.index), 0.0),
            None => self.begin_cue(self.index, self.pass, None, 0.0),
        }
        true
    }

    /// Freezes the cue's clock. A transition already under way still
    /// finishes. Commands still work.
    pub fn hold(&mut self) -> bool {
        self.set_hold(true)
    }

    /// Releases the hold; the cue resumes from where it stopped.
    pub fn unhold(&mut self) -> bool {
        self.set_hold(false)
    }

    fn set_hold(&mut self, held: bool) -> bool {
        if self.held == held {
            return false;
        }
        self.held = held;
        self.emit(ClockEvent::HoldChanged(held));
        true
    }

    /// Adds `seconds` to the active cue's length (negative shortens it, but
    /// never below zero). Cancels a pending downbeat wait.
    pub fn extend(&mut self, seconds: f64) -> bool {
        if !matches!(self.state, ClockState::Playing) || !seconds.is_finite() {
            return false;
        }
        self.extension = (self.extension + seconds).max(-self.base_len);
        self.lock_bar = None;
        self.lock_waited = 0.0;
        true
    }

    /// Starts the active cue again from its beginning (no transition).
    pub fn restart_cue(&mut self) -> bool {
        if !matches!(self.state, ClockState::Playing) {
            return false;
        }
        self.begin_cue(self.index, self.pass, None, 0.0);
        true
    }

    /// Goes to timeline cue `index`. Refused (returns `false`) if there is
    /// no such cue or it is marked failed. Revives an ended playlist.
    pub fn jump_to(&mut self, index: usize) -> bool {
        if index >= self.timeline.len() || self.is_failed(index) {
            return false;
        }
        let from = self.current();
        self.begin_cue(index, self.pass, from, 0.0);
        true
    }

    /// Goes to the first playable cue of the next section, wrapping if the
    /// playlist loops (and restarting the section if there is only one).
    pub fn skip_section(&mut self) -> bool {
        if !matches!(self.state, ClockState::Playing) {
            return false;
        }
        let section = self.section_of(self.index);
        self.advance_forward(true, Some(section), true);
        true
    }

    /// Switches blackout on or off.
    pub fn set_blackout(&mut self, on: bool) {
        if self.blackout != on {
            self.blackout = on;
            self.emit(ClockEvent::BlackoutChanged(on));
        }
    }

    /// An audio event happened. If the active cue's advance mode listens
    /// for it, goes to the next cue.
    pub fn audio_event(&mut self, kind: AudioEventKind) -> bool {
        let listens = self
            .current_cue()
            .is_some_and(|cue| cue.resolved.advance.listens_for(kind));
        if listens {
            self.advance_forward(false, None, true);
        }
        listens
    }

    // ---- failed scenes ----------------------------------------------------

    /// Marks timeline cue `index` as unplayable. It is skipped from now on;
    /// if it is the active cue, the clock moves on at once.
    pub fn mark_failed(&mut self, index: usize) {
        let Some(flag) = self.failed.get_mut(index) else {
            return;
        };
        if *flag {
            return;
        }
        *flag = true;
        if matches!(self.state, ClockState::Playing) && self.index == index {
            self.emit(ClockEvent::CueSkipped {
                index,
                reason: SkipReason::SceneFailed,
            });
            // No transition away from a scene that is not there.
            self.advance_forward(false, None, false);
        }
    }

    /// Marks every cue that uses `scene` as failed. Returns how many cues
    /// that was.
    pub fn mark_scene_failed(&mut self, scene: &SceneRef) -> usize {
        let hits: Vec<usize> = self
            .timeline
            .cues()
            .iter()
            .enumerate()
            .filter(|(_, cue)| cue.resolved.scene == *scene)
            .map(|(index, _)| index)
            .collect();
        for index in &hits {
            self.mark_failed(*index);
        }
        hits.len()
    }

    // ---- resume and wall clock ---------------------------------------------

    /// Where playback is, for saving. `None` unless playing.
    pub fn resume_point(&self) -> Option<ResumePoint> {
        let cue = self.current_cue()?;
        Some(ResumePoint {
            identity: self.timeline.identity(),
            section: cue.resolved.section,
            cue: cue.resolved.cue,
            pass: self.pass,
            offset: self.elapsed,
        })
    }

    /// Resumes at a saved point, refusing one saved for a different
    /// playlist.
    pub fn restore(&mut self, point: &ResumePoint) -> Result<(), ResumeError> {
        let current = self.timeline.identity();
        if point.identity != current {
            return Err(ResumeError::IdentityMismatch {
                saved: point.identity,
                current,
            });
        }
        self.restore_lenient(point)
    }

    /// Resumes at a saved point without checking the playlist identity: for
    /// a hot reload, where the edited playlist is *meant* to be the same
    /// show. The point's section and cue must still exist (a disabled cue
    /// resumes at the next enabled one).
    pub fn restore_lenient(&mut self, point: &ResumePoint) -> Result<(), ResumeError> {
        let missing = ResumeError::CueMissing {
            section: point.section,
            cue: point.cue,
        };
        let wanted = self
            .timeline
            .find_authored(point.section, point.cue)
            .ok_or(missing)?;
        // A failed cue resumes at the next playable one, from its start.
        let (index, offset) = if self.is_failed(wanted) {
            let scan = self.scan_forward(wanted, point.pass, true, |_| true);
            match scan.found {
                Some((_, step)) => (step.index, 0.0),
                None => return Err(ResumeError::NothingPlayable),
            }
        } else {
            (wanted, point.offset)
        };
        self.begin_cue(index, point.pass, None, offset);
        Ok(())
    }

    /// Jumps to what the wall-clock schedule says should be playing at
    /// `now_seconds_of_day`, and returns that placement. `None` if the
    /// playlist has no anchor. Only [`Placement::Playing`] and
    /// [`Placement::Finished`] change anything; before the anchor the clock
    /// is left alone for the app to keep waiting.
    pub fn sync_to_wall_clock(&mut self, now_seconds_of_day: f64) -> Option<Placement> {
        let anchor = self.timeline.anchor()?;
        let placement = self
            .timeline
            .place(elapsed_since_anchor(anchor, now_seconds_of_day));
        self.apply_placement(&placement);
        Some(placement)
    }

    /// Applies a placement computed by [`Timeline::place`].
    pub fn apply_placement(&mut self, placement: &Placement) {
        match *placement {
            Placement::Waiting { .. } => {}
            Placement::Playing {
                pass,
                index,
                offset,
            } => {
                if self.is_failed(index) {
                    let scan = self.scan_forward(index, pass, true, |_| true);
                    if let Some((_, step)) = scan.found {
                        self.begin_cue(step.index, step.pass, None, 0.0);
                    } else {
                        self.end();
                    }
                } else {
                    self.begin_cue(index, pass, None, offset);
                }
            }
            Placement::Finished { .. } => {
                if !matches!(self.state, ClockState::Ended) {
                    self.end();
                }
            }
        }
    }

    // ---- internals ---------------------------------------------------------

    fn emit(&mut self, event: ClockEvent) {
        if self.events.len() < MAX_PENDING_EVENTS {
            self.events.push(event);
        } else {
            self.dropped_events += 1;
        }
    }

    fn section_of(&self, index: usize) -> usize {
        self.timeline
            .cues()
            .get(index)
            .map_or(0, |cue| cue.resolved.section)
    }

    /// Seconds into the active cue at which it ends, if a timer ends it.
    fn deadline(&self) -> Option<f64> {
        if !matches!(self.state, ClockState::Playing) {
            return None;
        }
        let cue = self.timeline.cues().get(self.index)?;
        cue.resolved.advance.has_timer().then(|| self.cue_length())
    }

    /// The BPM beat lengths are computed at right now.
    fn bpm_now(&self) -> f64 {
        self.last_beat.usable_bpm(
            self.config.confidence_threshold,
            self.timeline.config().fallback_bpm,
        )
    }

    fn start_first(&mut self) {
        let scan = self.scan_from_start();
        // Everything before the first playable cue has failed.
        let before = scan.found.map_or(scan.tried, |(_, step)| step.index);
        for index in 0..before {
            self.emit(ClockEvent::CueSkipped {
                index,
                reason: SkipReason::SceneFailed,
            });
        }
        match scan.found {
            Some((_, step)) => self.begin_cue(step.index, 0, None, 0.0),
            None => self.end(),
        }
    }

    /// Makes `index` the active cue, `offset` seconds in. `from` is the cue
    /// the show is arriving from (for a transition), or `None` for a cut-in.
    fn begin_cue(&mut self, index: usize, pass: u64, from: Option<usize>, offset: f64) {
        self.abort_transition();
        let bpm = self.bpm_now();
        let Some(cue) = self.timeline.cues().get(index) else {
            self.end();
            return;
        };
        let base = cue.length.seconds(bpm).max(MIN_CUE_SECONDS);
        let locked = cue.length.is_beat_based();
        let has_timer = cue.resolved.advance.has_timer();
        let is_cut = matches!(cue.resolved.transition.kind, TransitionKind::Cut);
        let transition_len = if is_cut {
            0.0
        } else {
            cue.transition_length.seconds(bpm).clamp(0.0, base)
        };

        let offset = if offset.is_finite() {
            offset.max(0.0)
        } else {
            0.0
        };
        self.state = ClockState::Playing;
        self.index = index;
        self.pass = pass;
        self.base_len = base;
        self.extension = 0.0;
        self.beat_locked = locked;
        self.lock_bar = None;
        self.lock_waited = 0.0;
        // Never resume past the end of a timed cue: it would end on the
        // first tick, skipping the rest of the show for want of a millisecond.
        self.elapsed = if has_timer {
            offset.min((base - 0.001).max(0.0))
        } else {
            offset
        };
        self.warmed = false;

        self.emit(ClockEvent::CueStarted { index, pass });
        if let Some(from) = from
            && from != index
            && transition_len > 0.0
        {
            let id = self.next_transition_id;
            self.next_transition_id += 1;
            self.transition = Some(ActiveTransition {
                id,
                from,
                to: index,
                elapsed: 0.0,
                length: transition_len,
            });
            self.emit(ClockEvent::TransitionStarted {
                id,
                from,
                to: index,
            });
        }
        self.check_warmup();
    }

    fn end(&mut self) {
        self.abort_transition();
        self.state = ClockState::Ended;
        self.warmed = false;
        self.emit(ClockEvent::PlaylistEnded);
    }

    fn abort_transition(&mut self) {
        if let Some(active) = self.transition.take() {
            self.emit(ClockEvent::TransitionFinished {
                id: active.id,
                completed: false,
            });
        }
    }

    fn advance_transition(&mut self, dt: f64) {
        let Some(active) = self.transition.as_mut() else {
            return;
        };
        active.elapsed += dt;
        if active.elapsed >= active.length {
            let id = active.id;
            self.transition = None;
            self.emit(ClockEvent::TransitionFinished {
                id,
                completed: true,
            });
        }
    }

    fn check_warmup(&mut self) {
        if self.warmed || !matches!(self.state, ClockState::Playing) {
            return;
        }
        let Some(remaining) = self.remaining() else {
            return;
        };
        if remaining <= self.config.warmup_lead {
            self.warmed = true;
            if let Some(index) = self.peek_next() {
                self.emit(ClockEvent::WarmUp { index });
            }
        }
    }

    /// Moves to the next playable cue and starts it, reporting skipped
    /// failures; ends the playlist if there is none. `away_from`, when set,
    /// only accepts cues outside that section (skip section); `global`
    /// ignores a section loop; `transition` says whether to arrive by the
    /// cue's transition or cut in.
    fn advance_forward(&mut self, global: bool, away_from: Option<usize>, transition: bool) {
        let from = self.index;
        let scan = match away_from {
            Some(section) => self.scan_forward(from, self.pass, global, |step| {
                self.section_of(step.index) != section
            }),
            None => self.scan_forward(from, self.pass, global, |_| true),
        };
        self.report_skips(
            scan.tried,
            scan.found.map(|(k, _)| k),
            Direction::Forward,
            global,
            away_from,
        );
        match scan.found {
            Some((_, step)) => {
                self.begin_cue(step.index, step.pass, transition.then_some(from), 0.0);
            }
            None if away_from.is_some() => {
                // Skip section with nowhere else to go: replay this section.
                self.restart_section(from, transition);
            }
            None => self.end(),
        }
    }

    /// Skip-section with no other section to go to: replays this section
    /// from its first playable cue, or ends a playlist that does not loop.
    fn restart_section(&mut self, from: usize, transition: bool) {
        let range = self.timeline.section_range(self.section_of(from));
        let first = range.clone().find(|i| !self.is_failed(*i));
        match (first, self.timeline.loop_mode()) {
            (Some(index), LoopMode::Loop | LoopMode::LoopSection) => {
                self.begin_cue(index, self.pass, transition.then_some(from), 0.0);
            }
            (Some(_) | None, LoopMode::Stop) | (None, LoopMode::Loop | LoopMode::LoopSection) => {
                self.end();
            }
        }
    }

    /// Emits `CueSkipped` for failed cues among the first `tried`
    /// candidates, those before `found_at` (or all, if nothing was found).
    fn report_skips(
        &mut self,
        tried: usize,
        found_at: Option<usize>,
        direction: Direction,
        global: bool,
        away_from: Option<usize>,
    ) {
        let upto = found_at.map_or(tried, |k| k.saturating_sub(1));
        let from = self.index;
        for k in 1..=upto {
            let step = match direction {
                Direction::Forward => self.candidate_forward(from, self.pass, k, global),
                Direction::Backward => self.candidate_backward(from, self.pass, k),
            };
            let Some(step) = step else { break };
            let in_scope = away_from.is_none_or(|section| self.section_of(step.index) != section);
            if in_scope && self.is_failed(step.index) {
                self.emit(ClockEvent::CueSkipped {
                    index: step.index,
                    reason: SkipReason::SceneFailed,
                });
            }
        }
    }

    /// The k-th cue after `from` in play order. `global` ignores a section
    /// loop (used by skip-section and wall-clock catch-up).
    fn candidate_forward(&self, from: usize, pass: u64, k: usize, global: bool) -> Option<Step> {
        let count = self.timeline.len();
        if count == 0 {
            return None;
        }
        match self.timeline.loop_mode() {
            LoopMode::Stop => {
                let index = from + k;
                (index < count).then_some(Step { index, pass })
            }
            LoopMode::Loop => Some(wrap(0, count, from, k, pass)),
            LoopMode::LoopSection if global => Some(wrap(0, count, from, k, pass)),
            LoopMode::LoopSection => {
                let range = self.timeline.section_range(self.section_of(from));
                if range.is_empty() {
                    return None;
                }
                Some(wrap(
                    range.start,
                    range.len(),
                    from.saturating_sub(range.start),
                    k,
                    pass,
                ))
            }
        }
    }

    /// The k-th cue before `from` in play order.
    fn candidate_backward(&self, from: usize, pass: u64, k: usize) -> Option<Step> {
        let count = self.timeline.len();
        if count == 0 {
            return None;
        }
        let (start, len) = match self.timeline.loop_mode() {
            LoopMode::Stop | LoopMode::Loop => (0, count),
            LoopMode::LoopSection => {
                let range = self.timeline.section_range(self.section_of(from));
                (range.start, range.len().max(1))
            }
        };
        let position = from.saturating_sub(start);
        match self.timeline.loop_mode() {
            LoopMode::Stop => position.checked_sub(k).map(|index| Step { index, pass }),
            LoopMode::Loop | LoopMode::LoopSection => {
                let behind = k % len;
                let wraps = (k / len + usize::from(behind > position)) as u64;
                let index = start + (position + len - behind) % len;
                Some(Step {
                    index,
                    pass: pass.saturating_sub(wraps),
                })
            }
        }
    }

    fn scan_forward(
        &self,
        from: usize,
        pass: u64,
        global: bool,
        want: impl Fn(Step) -> bool,
    ) -> Scan {
        let count = self.timeline.len();
        let mut tried = 0;
        for k in 1..=count {
            let Some(step) = self.candidate_forward(from, pass, k, global) else {
                break;
            };
            tried = k;
            if want(step) && !self.is_failed(step.index) {
                return Scan {
                    found: Some((k, step)),
                    tried,
                };
            }
        }
        Scan { found: None, tried }
    }

    fn scan_backward(&self) -> Scan {
        let count = self.timeline.len();
        let mut tried = 0;
        for k in 1..=count {
            let Some(step) = self.candidate_backward(self.index, self.pass, k) else {
                break;
            };
            tried = k;
            if !self.is_failed(step.index) {
                return Scan {
                    found: Some((k, step)),
                    tried,
                };
            }
        }
        Scan { found: None, tried }
    }

    /// Candidates for starting the playlist: cue 0, 1, 2, ... (reported as
    /// k = 1, 2, ...).
    fn scan_from_start(&self) -> Scan {
        let count = self.timeline.len();
        let mut tried = 0;
        for k in 1..=count {
            tried = k;
            let index = k - 1;
            if !self.is_failed(index) {
                return Scan {
                    found: Some((k, Step { index, pass: 0 })),
                    tried,
                };
            }
        }
        Scan { found: None, tried }
    }
}

/// Which way a scan walks.
#[derive(Debug, Clone, Copy)]
enum Direction {
    Forward,
    Backward,
}

/// The result of looking for a playable cue.
#[derive(Debug, Clone, Copy)]
struct Scan {
    /// The candidate number (1 = adjacent) and position that was playable.
    found: Option<(usize, Step)>,
    /// How many candidates were examined.
    tried: usize,
}

/// The k-th step forward in a ring of `len` cues starting at `base`, from
/// ring position `position`, counting wraps as passes.
fn wrap(base: usize, len: usize, position: usize, k: usize, pass: u64) -> Step {
    let raw = position + k;
    Step {
        index: base + raw % len,
        pass: pass + (raw / len) as u64,
    }
}

#[cfg(test)]
mod tests;
