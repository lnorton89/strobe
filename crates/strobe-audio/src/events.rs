// ============================================================================
// Module:       strobe_audio::events
// Description:  Song-structure events from the analyser's levels: a
//               breakdown (the bass falls away and stays away) and a drop
//               (the energy comes back after one), for playlist advance.
//
// Dependencies: crate::smoothing
// ============================================================================

//! Breakdowns and drops (backlog P-01, D226).
//!
//! A playlist cue can end on `audio-drop` or `audio-breakdown`; the clock
//! already acts on those events, and this is where they come from. The
//! detector reads the frame's raw energies (`AudioFrame::energy`: the RMS
//! of the whole signal and of the bass, before the analyser normalises them
//! to a recent peak, which over a long breakdown would lift the quiet back
//! up) and keeps a short and a long average of each. Everything is a ratio
//! between those averages, so the input gain does not matter:
//!
//! - **Breakdown:** the short bass average stays under
//!   [`BREAKDOWN_RATIO`] of the long one for [`BREAKDOWN_HOLD`] seconds,
//!   while there is still sound. Reported once, when the hold is met.
//! - **Drop:** after a breakdown, the short level and bass averages climb
//!   back to [`DROP_RATIO`] of what they were before it.
//!
//! What keeps it from lying:
//!
//! - **One transient is not a structure.** Every condition has to hold for a
//!   while, not a frame, and two events of the same kind are at least
//!   [`MIN_GAP`] apart. A drop may follow its own breakdown sooner: that is
//!   the shape being looked for.
//! - **Silence is not a breakdown.** A level under [`ABSENT`] (a device
//!   unplugged, a paused player) is no audio: nothing is reported, the
//!   averages start over when sound returns, and the first [`MIN_GAP`] after
//!   that reports nothing, so a reconnect cannot pass for a drop.
//! - **Frame-rate independent.** Every average steps by `dt`; a stall
//!   longer than [`MAX_STEP`] is skipped rather than smeared.
//!
//! The thresholds are first estimates against synthetic structure; real
//! sets will tune them (the decision log says how to reopen).

use crate::smoothing::blend;

/// A song-structure event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// The bass fell away and stayed away.
    Breakdown,
    /// The energy came back after a breakdown.
    Drop,
}

/// The short averages' time constant, in seconds.
const SHORT_TAU: f32 = 0.4;

/// The long averages' time constant, in seconds.
const LONG_TAU: f32 = 20.0;

/// A breakdown is the short bass average under this share of the long one.
pub const BREAKDOWN_RATIO: f32 = 0.4;

/// For this long, in seconds.
pub const BREAKDOWN_HOLD: f32 = 2.0;

/// A drop is the energy back to this share of what it was before the
/// breakdown.
pub const DROP_RATIO: f32 = 0.8;

/// Below this RMS (-80 dBFS) there is no audio, not a quiet passage.
pub const ABSENT: f32 = 1e-4;

/// The long bass average must be at least this share of the long whole
/// before a breakdown can be called: a track that never had bass has
/// nothing to break down from.
const ACTIVE_SHARE: f32 = 0.1;

/// Two events of the same kind are at least this far apart, in seconds.
pub const MIN_GAP: f32 = 8.0;

/// The level must stay under [`ABSENT`] this long, in seconds, to be no
/// audio rather than a gap between notes.
const ABSENT_HOLD: f32 = 0.25;

/// A frame step longer than this (a stall, a suspended laptop) is skipped.
pub const MAX_STEP: f32 = 0.5;

/// Where the detector is.
#[derive(Debug, Clone, Copy, PartialEq)]
enum State {
    /// No audio: nothing is measured.
    Absent,
    /// Listening; `low` is how long the bass has been under the line.
    Normal { low: f32 },
    /// In a breakdown, waiting for the energy to return to what it was
    /// (`level` and `bass`, the long averages when it began).
    Breakdown { level: f32, bass: f32 },
}

/// Turns a stream of levels into breakdowns and drops (see the module
/// docs). Cheap: a handful of multiplies a frame, no allocation.
#[derive(Debug, Clone)]
pub struct EventDetector {
    state: State,
    level_short: f32,
    bass_short: f32,
    level_long: f32,
    bass_long: f32,
    /// Seconds until another breakdown, and another drop, may be reported.
    quiet: [f32; 2],
    /// Seconds the level has been under [`ABSENT`].
    silent: f32,
}

impl Default for EventDetector {
    fn default() -> Self {
        Self {
            state: State::Absent,
            level_short: 0.0,
            bass_short: 0.0,
            level_long: 0.0,
            bass_long: 0.0,
            quiet: [MIN_GAP; 2],
            silent: 0.0,
        }
    }
}

/// `event`'s slot in [`EventDetector::quiet`].
const fn slot(event: Event) -> usize {
    match event {
        Event::Breakdown => 0,
        Event::Drop => 1,
    }
}

impl EventDetector {
    /// Takes one frame's raw energies (`AudioFrame::energy`: the whole
    /// signal's RMS and the bass band's), `dt` seconds after the last.
    /// Returns the event this frame completes, if any.
    pub fn step(&mut self, dt: f32, level: f32, bass: f32) -> Option<Event> {
        if !dt.is_finite() || dt <= 0.0 || dt > MAX_STEP {
            return None;
        }
        let level = if level.is_finite() {
            level.max(0.0)
        } else {
            0.0
        };
        let bass = if bass.is_finite() { bass.max(0.0) } else { 0.0 };
        for quiet in &mut self.quiet {
            *quiet = (*quiet - dt).max(0.0);
        }

        // No audio for a moment: forget everything, report nothing.
        if level < ABSENT {
            self.silent += dt;
            if self.silent >= ABSENT_HOLD {
                self.state = State::Absent;
            }
        } else {
            self.silent = 0.0;
        }
        if self.state == State::Absent {
            if self.silent > 0.0 {
                return None;
            }
            // Sound again: start the averages here, and wait before
            // trusting them.
            self.level_short = level;
            self.bass_short = bass;
            self.level_long = level;
            self.bass_long = bass;
            self.quiet = [MIN_GAP; 2];
            self.state = State::Normal { low: 0.0 };
            return None;
        }

        let short = blend(dt, SHORT_TAU);
        self.level_short += (level - self.level_short) * short;
        self.bass_short += (bass - self.bass_short) * short;
        let frozen = matches!(self.state, State::Breakdown { .. });
        if !frozen {
            // A breakdown must not become the new normal it is compared to.
            let long = blend(dt, LONG_TAU);
            self.level_long += (level - self.level_long) * long;
            self.bass_long += (bass - self.bass_long) * long;
        }

        match self.state {
            State::Absent => None,
            State::Normal { low } => {
                let under = self.bass_long >= ACTIVE_SHARE * self.level_long
                    && self.bass_short < BREAKDOWN_RATIO * self.bass_long;
                let low = if under { low + dt } else { 0.0 };
                if low >= BREAKDOWN_HOLD {
                    self.state = State::Breakdown {
                        level: self.level_long,
                        bass: self.bass_long,
                    };
                    return self.report(Event::Breakdown);
                }
                self.state = State::Normal { low };
                None
            }
            State::Breakdown { level, bass } => {
                let back =
                    self.level_short >= DROP_RATIO * level && self.bass_short >= DROP_RATIO * bass;
                if !back {
                    return None;
                }
                self.state = State::Normal { low: 0.0 };
                self.report(Event::Drop)
            }
        }
    }

    /// `event`, unless one of its kind was reported less than [`MIN_GAP`]
    /// ago.
    fn report(&mut self, event: Event) -> Option<Event> {
        let quiet = self.quiet.get_mut(slot(event))?;
        if *quiet > 0.0 {
            return None;
        }
        *quiet = MIN_GAP;
        Some(event)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One section of a synthetic track: seconds, level, bass.
    type Part = (f32, f32, f32);

    /// Every event `parts` produce at `fps`, with the time it came.
    fn run(parts: &[Part], fps: f32) -> Vec<(f32, Event)> {
        let mut detector = EventDetector::default();
        let dt = 1.0 / fps;
        let mut time = 0.0_f32;
        let mut events = Vec::new();
        for (seconds, level, bass) in parts {
            let frames = (seconds * fps).round() as usize;
            for frame in 0..frames {
                // A little movement, as real levels have; silence has none.
                let wobble = if frame % 2 == 0 { 1.05 } else { 0.95 };
                if let Some(event) = detector.step(dt, level * wobble, bass * wobble) {
                    events.push((time, event));
                }
                time += dt;
            }
        }
        events
    }

    const MUSIC: Part = (30.0, 0.7, 0.8);

    #[test]
    fn steady_music_has_no_structure() {
        assert!(
            run(&[MUSIC, MUSIC], 60.0).is_empty(),
            "a minute of the same"
        );
    }

    #[test]
    fn a_breakdown_then_a_drop_are_reported_once_each() {
        let events = run(&[MUSIC, (8.0, 0.25, 0.05), (10.0, 0.7, 0.8)], 60.0);
        let kinds: Vec<Event> = events.iter().map(|(_, e)| *e).collect();
        assert_eq!(kinds, [Event::Breakdown, Event::Drop], "{events:?}");
        let (at, _) = events[0];
        assert!(
            (32.0..33.5).contains(&at),
            "the breakdown is called after its hold: {at}"
        );
        let (at, _) = events[1];
        assert!((38.0..39.5).contains(&at), "the drop when it lands: {at}");
    }

    #[test]
    fn a_short_dip_is_not_a_breakdown() {
        let events = run(&[MUSIC, (1.0, 0.25, 0.05), MUSIC], 60.0);
        assert!(events.is_empty(), "one bar of quiet: {events:?}");
    }

    #[test]
    fn silence_and_a_reconnect_report_nothing() {
        // Music, the device goes away, music again at full energy.
        let events = run(&[MUSIC, (6.0, 0.0, 0.0), MUSIC], 60.0);
        assert!(events.is_empty(), "no breakdown, no drop: {events:?}");
        assert!(run(&[(60.0, 0.0, 0.0)], 60.0).is_empty(), "nothing at all");
    }

    #[test]
    fn the_rate_does_not_change_the_answer() {
        let parts = [MUSIC, (8.0, 0.25, 0.05), (10.0, 0.7, 0.8)];
        let slow: Vec<Event> = run(&parts, 30.0).into_iter().map(|(_, e)| e).collect();
        let fast: Vec<Event> = run(&parts, 144.0).into_iter().map(|(_, e)| e).collect();
        assert_eq!(slow, fast, "30 and 144 fps agree");
    }

    #[test]
    fn events_of_one_kind_are_kept_apart() {
        // Two breakdown-drop pairs in quick succession: the second pair
        // comes too soon after the first to be reported.
        let events = run(
            &[
                MUSIC,
                (5.0, 0.25, 0.05),
                (1.0, 0.7, 0.8),
                (5.0, 0.25, 0.05),
                (10.0, 0.7, 0.8),
            ],
            60.0,
        );
        for kind in [Event::Breakdown, Event::Drop] {
            let times: Vec<f32> = events
                .iter()
                .filter(|(_, e)| *e == kind)
                .map(|(t, _)| *t)
                .collect();
            for pair in times.windows(2) {
                assert!(
                    pair[1] - pair[0] >= MIN_GAP - 0.1,
                    "{kind:?}s at least the gap apart: {events:?}"
                );
            }
        }
        assert!(
            events.iter().any(|(_, e)| *e == Event::Drop),
            "the first drop is reported: {events:?}"
        );
    }

    #[test]
    fn the_analyser_s_energies_show_a_breakdown_and_a_drop() {
        use crate::fixtures::{FPS, RATE, kicks, noise};
        let mix = |a: Vec<f32>, b: Vec<f32>| -> Vec<f32> {
            a.iter().zip(&b).map(|(x, y)| x + y).collect()
        };
        // 30 s of kicks over a hiss, 12 s of the hiss alone (a long
        // breakdown, longer than the analyser's peak memory), then the
        // kicks again.
        let mut signal = mix(kicks(128.0, 30.0, RATE), noise(0.05, 30.0, RATE));
        signal.extend(noise(0.05, 12.0, RATE));
        signal.extend(mix(kicks(128.0, 15.0, RATE), noise(0.05, 15.0, RATE)));
        let mut analyzer = crate::Analyzer::new(RATE);
        let mut detector = EventDetector::default();
        let mut events = Vec::new();
        crate::offline::run(&mut analyzer, &signal, FPS, |time, frame| {
            let [whole, bass] = frame.energy;
            if let Some(event) = detector.step(1.0 / FPS, whole, bass) {
                events.push((time, event));
            }
        });
        let kinds: Vec<Event> = events.iter().map(|(_, e)| *e).collect();
        assert_eq!(kinds, [Event::Breakdown, Event::Drop], "{events:?}");
        assert!(
            events
                .first()
                .is_some_and(|(t, _)| (30.0..34.0).contains(t)),
            "the breakdown early in the hiss: {events:?}"
        );
        assert!(
            events.get(1).is_some_and(|(t, _)| (42.0..44.0).contains(t)),
            "the drop when the kicks return: {events:?}"
        );
    }
}
