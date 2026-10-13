// ============================================================================
// Module:       strobe_audio::song
// Description:  Song changes from the raw level and the tempo: a gap between
//               tracks, or a new tempo that holds, each held long enough that
//               a stall or one wrong reading cannot pass for one.
//
// Dependencies: crate::events (ABSENT)
// ============================================================================

//! Song changes (backlog M-05, D237).
//!
//! A playlist cue can end on `audio-song`: the next track has started. The
//! analyser has no idea what a track is, so this listens for the two
//! things a track change leaves behind in what it does measure:
//!
//! - **A gap.** The level stays under [`ABSENT`] for between [`GAP_MIN`]
//!   and [`GAP_MAX`] seconds after at least [`PLAYED_BEFORE`] seconds of
//!   music, and sound then holds for [`RESUME_HOLD`]. A player going from
//!   one track to the next does that; a breath in the music is shorter,
//!   and a stop is longer (a later start is then not a change: nothing was
//!   playing).
//! - **A new tempo.** With the tempo confidence at [`TEMPO_CONFIDENCE`] or
//!   more, a tempo more than [`TEMPO_SHIFT`] away from the settled one
//!   holds (within [`TEMPO_SAME`]) for [`TEMPO_HOLD`] seconds. Double and
//!   half time count as the same tempo, since the estimator may flip
//!   octaves. The first tempo heard settles without an event.
//!
//! Two changes are at least [`MIN_SONG`] apart, and [`SongDetector::reset`]
//! forgets everything; the app calls it whenever the audio source changes
//! (a device lost, synthetic audio, a device back), so a reconnect is never
//! a song change. What it cannot hear: a DJ mix that holds one tempo
//! through the change, with no gap. That needs a cue's timer or a person.

use crate::events::ABSENT;

/// The shortest silence between tracks, in seconds.
pub const GAP_MIN: f32 = 0.8;

/// The longest: past this the music stopped rather than changed track.
pub const GAP_MAX: f32 = 10.0;

/// Seconds of music before a gap for the gap to end a song.
pub const PLAYED_BEFORE: f32 = 30.0;

/// Seconds of sound after a gap before it counts.
pub const RESUME_HOLD: f32 = 1.5;

/// The tempo confidence from which tempo readings count.
pub const TEMPO_CONFIDENCE: f32 = 0.6;

/// A tempo this far from the settled one (as a log ratio, about 4 %) is
/// another tempo.
pub const TEMPO_SHIFT: f32 = 0.04;

/// Readings this close (about 2 %) are the same candidate tempo.
pub const TEMPO_SAME: f32 = 0.02;

/// Seconds a new tempo must hold to be a new song.
pub const TEMPO_HOLD: f32 = 8.0;

/// Two song changes are at least this far apart, in seconds.
pub const MIN_SONG: f32 = 30.0;

/// A frame step longer than this is skipped, as in `events`.
const MAX_STEP: f32 = 0.5;

/// Listens for song changes (see the module docs). A few compares a frame.
#[derive(Debug, Clone, Default)]
pub struct SongDetector {
    /// Seconds of sound since the last stop or song change.
    playing: f32,
    /// Seconds under [`ABSENT`] so far.
    silent: f32,
    /// After a gap that qualifies: seconds of sound since it.
    resumed: Option<f32>,
    /// The tempo the song settled on.
    settled: Option<f32>,
    /// A tempo that differs from it, and how long it has held.
    candidate: Option<(f32, f32)>,
    /// Seconds until another change may be reported.
    quiet: f32,
}

impl SongDetector {
    /// A detector that has heard nothing.
    pub fn new() -> Self {
        Self::default()
    }

    /// Forgets everything (the audio source changed).
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// One frame: the whole signal's raw RMS (`AudioFrame::energy[0]`),
    /// the tempo and its confidence, `dt` seconds after the last. Returns
    /// whether a new song started.
    pub fn step(&mut self, dt: f32, level: f32, bpm: f32, confidence: f32) -> bool {
        if !dt.is_finite() || dt <= 0.0 || dt > MAX_STEP {
            return false;
        }
        self.quiet = (self.quiet - dt).max(0.0);
        let level = if level.is_finite() { level } else { 0.0 };
        if level < ABSENT {
            self.silent += dt;
            self.resumed = None;
            if self.silent > GAP_MAX {
                // The music stopped; whatever comes next starts afresh.
                self.playing = 0.0;
                self.settled = None;
                self.candidate = None;
            }
            return false;
        }
        if self.silent > 0.0 {
            if (GAP_MIN..=GAP_MAX).contains(&self.silent) && self.playing >= PLAYED_BEFORE {
                self.resumed = Some(0.0);
            }
            self.silent = 0.0;
        }
        self.playing += dt;

        if let Some(resumed) = self.resumed.as_mut() {
            *resumed += dt;
            if *resumed >= RESUME_HOLD {
                self.resumed = None;
                return self.change();
            }
        }
        if self.tempo(dt, bpm, confidence) {
            return self.change();
        }
        false
    }

    /// Follows the tempo; returns whether a new one has held.
    fn tempo(&mut self, dt: f32, bpm: f32, confidence: f32) -> bool {
        let usable = bpm.is_finite()
            && bpm > 0.0
            && confidence.is_finite()
            && confidence >= TEMPO_CONFIDENCE;
        if !usable {
            return false;
        }
        if let Some(settled) = self.settled
            && same_tempo(bpm, settled, TEMPO_SHIFT)
        {
            self.candidate = None;
            return false;
        }
        let held = match self.candidate {
            Some((tempo, held)) if same_tempo(bpm, tempo, TEMPO_SAME) => held + dt,
            Some(_) | None => 0.0,
        };
        let tempo = self
            .candidate
            .map_or(bpm, |(tempo, _)| if held > 0.0 { tempo } else { bpm });
        self.candidate = Some((tempo, held));
        if held < TEMPO_HOLD {
            return false;
        }
        self.candidate = None;
        let first = self.settled.is_none();
        self.settled = Some(tempo);
        !first
    }

    /// Reports a change unless one came less than [`MIN_SONG`] ago, and
    /// starts the new song: its tempo is learned again, quietly.
    fn change(&mut self) -> bool {
        self.settled = None;
        self.candidate = None;
        self.playing = 0.0;
        if self.quiet > 0.0 {
            return false;
        }
        self.quiet = MIN_SONG;
        true
    }
}

/// Whether two tempos are within `tolerance` (a log ratio) of each other,
/// allowing for double and half time.
fn same_tempo(a: f32, b: f32, tolerance: f32) -> bool {
    [1.0_f32, 2.0, 0.5]
        .into_iter()
        .any(|factor| (a * factor / b).ln().abs() <= tolerance)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FPS: f32 = 60.0;

    /// One stretch of input: seconds, level, bpm, confidence.
    type Part = (f32, f32, f32, f32);

    /// The times, in seconds, at which `parts` report a song change.
    fn changes(detector: &mut SongDetector, parts: &[Part]) -> Vec<f32> {
        let dt = 1.0 / FPS;
        let mut time = 0.0;
        let mut found = Vec::new();
        for &(seconds, level, bpm, confidence) in parts {
            let frames = (seconds * FPS).round() as usize;
            for _ in 0..frames {
                time += dt;
                if detector.step(dt, level, bpm, confidence) {
                    found.push(time);
                }
            }
        }
        found
    }

    const MUSIC: f32 = 0.1;

    #[test]
    fn a_gap_between_tracks_is_a_new_song_but_a_breath_or_a_stop_is_not() {
        let mut detector = SongDetector::new();
        let found = changes(
            &mut detector,
            &[
                (60.0, MUSIC, 128.0, 0.9),
                (0.3, 0.0, 128.0, 0.0), // a breath
                (40.0, MUSIC, 128.0, 0.9),
                (2.0, 0.0, 0.0, 0.0), // the gap
                (10.0, MUSIC, 128.0, 0.9),
            ],
        );
        assert_eq!(found.len(), 1, "one change: {found:?}");
        assert!(
            (found[0] - (100.3 + 2.0 + RESUME_HOLD)).abs() < 0.1,
            "after the resume hold: {found:?}"
        );

        let mut stopped = SongDetector::new();
        let found = changes(
            &mut stopped,
            &[
                (60.0, MUSIC, 128.0, 0.9),
                (30.0, 0.0, 0.0, 0.0), // the music stopped
                (60.0, MUSIC, 128.0, 0.9),
            ],
        );
        assert!(
            found.is_empty(),
            "a start after a stop is not a change: {found:?}"
        );
    }

    #[test]
    fn a_new_tempo_that_holds_is_a_new_song_and_octave_flips_are_not() {
        let mut detector = SongDetector::new();
        let found = changes(
            &mut detector,
            &[
                (30.0, MUSIC, 124.0, 0.9),
                (10.0, MUSIC, 62.0, 0.9),  // the estimator halves
                (10.0, MUSIC, 124.0, 0.4), // unsure, and wrong would not count
                (5.0, MUSIC, 140.0, 0.9),  // too short to be a song
                (20.0, MUSIC, 124.5, 0.9), // a nudge within 4 %
                (20.0, MUSIC, 132.0, 0.9), // a new track
            ],
        );
        assert_eq!(found.len(), 1, "one change: {found:?}");
        assert!(
            (found[0] - (75.0 + TEMPO_HOLD)).abs() < 0.2,
            "eight seconds into the new tempo: {found:?}"
        );
    }

    #[test]
    fn the_first_tempo_settles_quietly_and_changes_are_spaced() {
        let mut detector = SongDetector::new();
        let found = changes(
            &mut detector,
            &[
                (20.0, MUSIC, 120.0, 0.9),
                (20.0, MUSIC, 140.0, 0.9), // a change at 28 s
                (20.0, MUSIC, 160.0, 0.9), // too soon after it: not reported
            ],
        );
        assert_eq!(found.len(), 1, "the first tempo is not a change: {found:?}");
    }

    #[test]
    fn a_reset_forgets_and_nonsense_is_ignored() {
        let mut detector = SongDetector::new();
        changes(&mut detector, &[(60.0, MUSIC, 120.0, 0.9)]);
        // The device went away and came back: the app resets.
        detector.reset();
        let found = changes(
            &mut detector,
            &[(2.0, 0.0, 0.0, 0.0), (20.0, MUSIC, 150.0, 0.9)],
        );
        assert!(found.is_empty(), "a reconnect is no song change: {found:?}");
        assert!(
            !detector.step(f32::NAN, MUSIC, 150.0, 0.9)
                && !detector.step(1.0 / FPS, f32::NAN, f32::INFINITY, f32::NAN)
                && !detector.step(5.0, MUSIC, 150.0, 0.9),
            "nonsense and stalls report nothing"
        );
    }
}
