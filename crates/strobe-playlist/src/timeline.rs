// ============================================================================
// Module:       strobe_playlist::timeline
// Description:  The static timeline: every enabled cue's start, duration and
//               transition window, with random durations drawn from the
//               seed and beat lengths at a fallback BPM.
//
// Dependencies: crate::resolve, crate::duration, crate::rng, crate::model
// ============================================================================

//! The timeline.
//!
//! A [`Timeline`] answers "what does this playlist look like if nothing
//! surprises it": cue 0 starts at 0, each cue starts when the previous one
//! ends, and the total is the sum of the durations. It is what the editor
//! previews and what the wall-clock scheduler indexes into.
//!
//! # Rules
//!
//! - **Disabled cues** are not in the timeline at all.
//! - **Random durations** are drawn once per cue from
//!   `derive(seed, cue position)`, so a cue's length depends on the seed and
//!   where the cue sits, never on how many other cues drew numbers. A
//!   looping playlist repeats the *same* lengths every pass; that is what
//!   makes the preview exact and the wall-clock lookup a plain modulo.
//! - **Beat lengths** (`Bars`/`Beats`) are converted at the fallback BPM
//!   here. At run time the clock uses the live BPM instead, and ends the cue
//!   on the next downbeat after its computed length; the timeline cannot
//!   know that and does not pretend to.
//! - **Manual** (and audio-event-only) cues have no deadline at run time.
//!   The timeline schedules them for their nominal duration and marks them
//!   [`TimelineCue::open_ended`], so the total is the runtime *if every
//!   such cue is allowed to run its nominal time*.
//! - **A transition** into cue `i` starts when cue `i` starts, and is capped
//!   at cue `i`'s duration, so cues are contiguous and the runtime stays
//!   the plain sum. The first cue has no transition in the static timeline
//!   (there is nothing to come from).
//! - **Very short cues** are stretched to [`MIN_CUE_SECONDS`]: a zero-length
//!   cue would let a looping playlist spin without time passing.

use crate::duration::Length;
use crate::model::{LoopMode, TransitionKind, WallClockAnchor};
use crate::resolve::{ResolvedCue, ResolvedPlaylist};
use crate::rng::SplitMix64;
use std::ops::Range;

/// The shortest a cue can be, in seconds.
pub const MIN_CUE_SECONDS: f64 = 0.5;

/// The tempo beat lengths are previewed at when none is configured.
const DEFAULT_FALLBACK_BPM: f64 = 120.0;

/// Stream numbers within a cue's derived seed space.
const STREAM_DURATION: u64 = 0;
const STREAM_TRANSITION: u64 = 1;

/// How the timeline converts beats to seconds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimelineConfig {
    /// BPM used for `Bars`/`Beats` lengths. The live clock overrides it
    /// with the detected tempo when that is confident.
    pub fallback_bpm: f64,
}

impl Default for TimelineConfig {
    fn default() -> Self {
        Self {
            fallback_bpm: DEFAULT_FALLBACK_BPM,
        }
    }
}

/// One enabled cue, placed in time.
#[derive(Debug, Clone, PartialEq)]
pub struct TimelineCue {
    /// The cue with all its settings resolved.
    pub resolved: ResolvedCue,
    /// The cue's length with randomness drawn (beats stay beats).
    pub length: Length,
    /// The transition into this cue, with randomness drawn. Zero seconds
    /// for a cut.
    pub transition_length: Length,
    /// Seconds from the start of the playlist.
    pub start: f64,
    /// Seconds this cue takes at the fallback BPM, at least
    /// [`MIN_CUE_SECONDS`].
    pub duration: f64,
    /// When the transition into this cue ends (equals `start` for a cut
    /// and for the first cue).
    pub transition_end: f64,
    /// Whether nothing but a person or an audio event ends the cue.
    pub open_ended: bool,
}

impl TimelineCue {
    /// When the cue ends.
    pub fn end(&self) -> f64 {
        self.start + self.duration
    }
}

/// A playlist laid out in time.
#[derive(Debug, Clone, PartialEq)]
pub struct Timeline {
    cues: Vec<TimelineCue>,
    total: f64,
    config: TimelineConfig,
    name: String,
    identity: u64,
    seed: u64,
    loop_mode: LoopMode,
    anchor: Option<WallClockAnchor>,
    section_names: Vec<String>,
    section_ranges: Vec<Range<usize>>,
}

impl Timeline {
    /// Lays out `playlist`'s enabled cues.
    pub fn build(playlist: &ResolvedPlaylist, config: &TimelineConfig) -> Self {
        let bpm = config.fallback_bpm;
        let mut cues: Vec<TimelineCue> = Vec::new();
        let mut section_ranges: Vec<Range<usize>> =
            playlist.section_names.iter().map(|_| 0..0).collect();
        let mut start = 0.0;

        for resolved in playlist.cues.iter().filter(|cue| cue.enabled) {
            let position = cues.len() as u64;
            let length = resolved.duration.resolve(
                &mut SplitMix64::derive(playlist.seed, position * 2 + STREAM_DURATION),
                bpm,
            );
            let transition_length = match resolved.transition.kind {
                TransitionKind::Cut => Length::Seconds(0.0),
                TransitionKind::Crossfade
                | TransitionKind::FadeThroughBlack
                | TransitionKind::Shader(_) => resolved.transition.duration.resolve(
                    &mut SplitMix64::derive(playlist.seed, position * 2 + STREAM_TRANSITION),
                    bpm,
                ),
            };
            let duration = length.seconds(bpm).max(MIN_CUE_SECONDS);
            let transition_end = if cues.is_empty() {
                start
            } else {
                start + transition_length.seconds(bpm).clamp(0.0, duration)
            };
            let open_ended = !resolved.advance.has_timer();

            if let Some(range) = section_ranges.get_mut(resolved.section) {
                if range.start >= range.end {
                    range.start = cues.len();
                }
                range.end = cues.len() + 1;
            }
            cues.push(TimelineCue {
                resolved: resolved.clone(),
                length,
                transition_length,
                start,
                duration,
                transition_end,
                open_ended,
            });
            start += duration;
        }

        Self {
            cues,
            total: start,
            config: *config,
            name: playlist.name.clone(),
            identity: playlist.identity,
            seed: playlist.seed,
            loop_mode: playlist.loop_mode,
            anchor: playlist.anchor,
            section_names: playlist.section_names.clone(),
            section_ranges,
        }
    }

    /// Resolves and lays out an authored playlist in one step.
    pub fn from_playlist(playlist: &crate::model::Playlist, config: &TimelineConfig) -> Self {
        Self::build(&crate::resolve::resolve(playlist), config)
    }

    /// The enabled cues in play order.
    pub fn cues(&self) -> &[TimelineCue] {
        &self.cues
    }

    /// How many enabled cues there are.
    pub fn len(&self) -> usize {
        self.cues.len()
    }

    /// Whether there is nothing to play.
    pub fn is_empty(&self) -> bool {
        self.cues.is_empty()
    }

    /// Total runtime of one pass, in seconds: the sum of the cue
    /// durations.
    pub fn total_runtime(&self) -> f64 {
        self.total
    }

    /// Whether any cue has no deadline of its own, so the real runtime
    /// depends on a person or an audio event.
    pub fn has_open_ended(&self) -> bool {
        self.cues.iter().any(|cue| cue.open_ended)
    }

    /// The tempo settings the timeline was built with.
    pub fn config(&self) -> &TimelineConfig {
        &self.config
    }

    /// The playlist's name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The playlist's resume identity ([`crate::resolve::identity`]).
    pub fn identity(&self) -> u64 {
        self.identity
    }

    /// The seed random durations were drawn with.
    pub fn seed(&self) -> u64 {
        self.seed
    }

    /// What happens after the last cue.
    pub fn loop_mode(&self) -> LoopMode {
        self.loop_mode
    }

    /// The wall-clock anchor, if any.
    pub fn anchor(&self) -> Option<WallClockAnchor> {
        self.anchor
    }

    /// The name of the section at authored index `section`.
    pub fn section_name(&self, section: usize) -> Option<&str> {
        self.section_names.get(section).map(String::as_str)
    }

    /// Timeline indices of the cues in authored section `section` (empty if
    /// every cue in it is disabled).
    pub fn section_range(&self, section: usize) -> Range<usize> {
        self.section_ranges.get(section).cloned().unwrap_or(0..0)
    }

    /// The timeline index of the cue at authored `(section, cue)`, or of
    /// the next enabled cue after it if that one is disabled.
    pub fn find_authored(&self, section: usize, cue: usize) -> Option<usize> {
        self.cues
            .iter()
            .position(|c| (c.resolved.section, c.resolved.cue) >= (section, cue))
    }

    /// The cue playing `offset` seconds into one pass, and how far into it
    /// that is. `None` for an empty timeline or an offset outside it.
    pub fn locate(&self, offset: f64) -> Option<(usize, f64)> {
        if !(0.0..self.total).contains(&offset) {
            return None;
        }
        let after = self.cues.partition_point(|cue| cue.start <= offset);
        let index = after.checked_sub(1)?;
        let cue = self.cues.get(index)?;
        Some((index, offset - cue.start))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duration::DurationSpec;
    use crate::model::{
        AdvanceMode, Cue, Playlist, SceneRef, Section, ShuffleSpec, TransitionSpec,
    };
    use anyhow::{Result, anyhow};

    pub(crate) fn playlist_of(durations: &[DurationSpec]) -> Playlist {
        let mut playlist = Playlist::new("t");
        let mut section = Section::new("s");
        for (i, duration) in durations.iter().enumerate() {
            let mut cue = Cue::new(SceneRef::Library(format!("scene-{i}")));
            cue.settings.duration = Some(duration.clone());
            section.cues.push(cue);
        }
        playlist.sections.push(section);
        playlist
    }

    fn secs(values: &[f64]) -> Vec<DurationSpec> {
        values.iter().map(|v| DurationSpec::Seconds(*v)).collect()
    }

    fn cue_at(timeline: &Timeline, index: usize) -> Result<&TimelineCue> {
        timeline
            .cues()
            .get(index)
            .ok_or_else(|| anyhow!("no cue {index}"))
    }

    #[test]
    fn starts_accumulate_and_the_total_is_the_sum() -> Result<()> {
        let playlist = playlist_of(&secs(&[10.0, 20.0, 30.0]));
        let timeline = Timeline::from_playlist(&playlist, &TimelineConfig::default());
        let starts: Vec<f64> = timeline.cues().iter().map(|c| c.start).collect();
        assert_eq!(starts, vec![0.0, 10.0, 30.0], "cue starts");
        assert_eq!(timeline.total_runtime(), 60.0, "total runtime");
        assert_eq!(
            cue_at(&timeline, 2)?.end(),
            60.0,
            "the last cue ends at the total"
        );
        Ok(())
    }

    #[test]
    fn disabled_cues_leave_no_gap() -> Result<()> {
        let mut playlist = playlist_of(&secs(&[10.0, 20.0, 30.0]));
        if let Some(cue) = playlist
            .sections
            .first_mut()
            .and_then(|s| s.cues.get_mut(1))
        {
            cue.enabled = false;
        }
        let timeline = Timeline::from_playlist(&playlist, &TimelineConfig::default());
        assert_eq!(timeline.len(), 2, "two enabled cues");
        assert_eq!(
            cue_at(&timeline, 1)?.start,
            10.0,
            "the third cue follows the first directly"
        );
        assert_eq!(
            timeline.total_runtime(),
            40.0,
            "disabled time is not counted"
        );
        assert_eq!(
            timeline.find_authored(0, 1),
            Some(1),
            "a disabled cue maps to its successor"
        );
        Ok(())
    }

    #[test]
    fn beats_use_the_fallback_tempo() -> Result<()> {
        let playlist = playlist_of(&[DurationSpec::Bars(8), DurationSpec::Beats(6)]);
        let at_120 = Timeline::from_playlist(
            &playlist,
            &TimelineConfig {
                fallback_bpm: 120.0,
            },
        );
        assert_eq!(cue_at(&at_120, 0)?.duration, 16.0, "8 bars at 120 BPM");
        assert_eq!(cue_at(&at_120, 1)?.duration, 3.0, "6 beats at 120 BPM");
        let at_60 = Timeline::from_playlist(&playlist, &TimelineConfig { fallback_bpm: 60.0 });
        assert_eq!(cue_at(&at_60, 0)?.duration, 32.0, "8 bars at 60 BPM");
        Ok(())
    }

    #[test]
    fn random_durations_follow_the_seed_and_stay_in_range() -> Result<()> {
        let rand = DurationSpec::Random {
            min: Box::new(DurationSpec::Seconds(60.0)),
            max: Box::new(DurationSpec::Seconds(120.0)),
        };
        let mut playlist = playlist_of(&vec![rand; 20]);
        playlist.shuffle = Some(ShuffleSpec {
            seed: 99,
            no_repeat_window: 0,
        });
        let config = TimelineConfig::default();
        let first = Timeline::from_playlist(&playlist, &config);
        let again = Timeline::from_playlist(&playlist, &config);
        assert_eq!(first, again, "same seed, same timeline");
        for cue in first.cues() {
            assert!(
                (60.0..120.0).contains(&cue.duration),
                "{} is outside rand(60s, 120s)",
                cue.duration
            );
        }
        playlist.shuffle = Some(ShuffleSpec {
            seed: 100,
            no_repeat_window: 0,
        });
        let other = Timeline::from_playlist(&playlist, &config);
        assert_ne!(
            first.total_runtime(),
            other.total_runtime(),
            "a new seed changes the draw"
        );
        Ok(())
    }

    #[test]
    fn a_cues_random_length_does_not_depend_on_its_neighbours() -> Result<()> {
        let rand = DurationSpec::Random {
            min: Box::new(DurationSpec::Seconds(10.0)),
            max: Box::new(DurationSpec::Seconds(500.0)),
        };
        let long = playlist_of(&vec![rand.clone(); 6]);
        let short = playlist_of(&vec![rand; 3]);
        let config = TimelineConfig::default();
        let (a, b) = (
            Timeline::from_playlist(&long, &config),
            Timeline::from_playlist(&short, &config),
        );
        for i in 0..3 {
            assert_eq!(
                cue_at(&a, i)?.duration,
                cue_at(&b, i)?.duration,
                "cue {i} changed length when later cues were added"
            );
        }
        Ok(())
    }

    #[test]
    fn transitions_are_capped_at_the_cue_and_absent_for_cut_and_first() -> Result<()> {
        let mut playlist = playlist_of(&secs(&[10.0, 3.0, 10.0]));
        playlist.defaults.transition = Some(TransitionSpec::crossfade(4.0));
        if let Some(cue) = playlist
            .sections
            .first_mut()
            .and_then(|s| s.cues.get_mut(2))
        {
            cue.settings.transition = Some(TransitionSpec::cut());
        }
        let timeline = Timeline::from_playlist(&playlist, &TimelineConfig::default());
        let first = cue_at(&timeline, 0)?;
        assert_eq!(
            first.transition_end, first.start,
            "the first cue has nothing to come from"
        );
        let second = cue_at(&timeline, 1)?;
        assert_eq!(
            second.transition_end,
            second.start + 3.0,
            "capped at the 3 s cue"
        );
        let third = cue_at(&timeline, 2)?;
        assert_eq!(third.transition_end, third.start, "a cut takes no time");
        assert_eq!(
            third.transition_length,
            Length::Seconds(0.0),
            "a cut has zero length"
        );
        Ok(())
    }

    #[test]
    fn manual_cues_are_marked_open_ended_but_keep_their_nominal_time() -> Result<()> {
        let mut playlist = playlist_of(&secs(&[10.0, 20.0]));
        if let Some(cue) = playlist
            .sections
            .first_mut()
            .and_then(|s| s.cues.get_mut(0))
        {
            cue.settings.advance = Some(AdvanceMode::Manual);
        }
        let timeline = Timeline::from_playlist(&playlist, &TimelineConfig::default());
        assert!(cue_at(&timeline, 0)?.open_ended, "manual has no deadline");
        assert!(!cue_at(&timeline, 1)?.open_ended, "timer has one");
        assert!(timeline.has_open_ended(), "the timeline reports it");
        assert_eq!(
            timeline.total_runtime(),
            30.0,
            "nominal durations are summed"
        );
        Ok(())
    }

    #[test]
    fn tiny_durations_are_stretched() -> Result<()> {
        let timeline =
            Timeline::from_playlist(&playlist_of(&secs(&[0.0, 0.1])), &TimelineConfig::default());
        assert_eq!(cue_at(&timeline, 0)?.duration, MIN_CUE_SECONDS, "0 s");
        assert_eq!(cue_at(&timeline, 1)?.duration, MIN_CUE_SECONDS, "0.1 s");
        Ok(())
    }

    #[test]
    fn locate_finds_the_cue_and_offset() -> Result<()> {
        let timeline = Timeline::from_playlist(
            &playlist_of(&secs(&[10.0, 20.0])),
            &TimelineConfig::default(),
        );
        assert_eq!(timeline.locate(0.0), Some((0, 0.0)), "the very start");
        assert_eq!(
            timeline.locate(9.5),
            Some((0, 9.5)),
            "late in the first cue"
        );
        assert_eq!(
            timeline.locate(10.0),
            Some((1, 0.0)),
            "a boundary belongs to the later cue"
        );
        assert_eq!(
            timeline.locate(29.0),
            Some((1, 19.0)),
            "inside the last cue"
        );
        assert_eq!(timeline.locate(30.0), None, "the end is outside");
        assert_eq!(timeline.locate(-1.0), None, "before the start");
        Ok(())
    }

    #[test]
    fn sections_map_to_timeline_ranges() -> Result<()> {
        let mut playlist = Playlist::new("p");
        for (name, count, off) in [("a", 2, false), ("b", 1, true), ("c", 2, false)] {
            let mut section = Section::new(name);
            for i in 0..count {
                let mut cue = Cue::new(SceneRef::Library(format!("{name}{i}")));
                cue.enabled = !off;
                section.cues.push(cue);
            }
            playlist.sections.push(section);
        }
        let timeline = Timeline::from_playlist(&playlist, &TimelineConfig::default());
        assert_eq!(timeline.section_range(0), 0..2, "first section");
        assert!(
            timeline.section_range(1).is_empty(),
            "an all-disabled section is empty"
        );
        assert_eq!(timeline.section_range(2), 2..4, "third section");
        assert_eq!(timeline.section_name(2), Some("c"), "section name");
        assert!(timeline.section_range(9).is_empty(), "unknown section");
        Ok(())
    }

    #[test]
    fn property_starts_are_monotonic_and_runtime_is_the_sum_of_durations() -> Result<()> {
        // A hand-rolled property test: many seeded random playlists with a
        // mix of units, disabled cues and transitions.
        for seed in 0..60u64 {
            let mut rng = SplitMix64::new(seed);
            let count = 1 + rng.below(40) as usize;
            let specs: Vec<DurationSpec> = (0..count)
                .map(|_| match rng.below(4) {
                    0 => DurationSpec::Seconds(rng.range(0.0, 300.0)),
                    1 => DurationSpec::Bars(1 + rng.below(32) as u32),
                    2 => DurationSpec::Beats(1 + rng.below(64) as u32),
                    _ => DurationSpec::Random {
                        min: Box::new(DurationSpec::Seconds(rng.range(1.0, 50.0))),
                        max: Box::new(DurationSpec::Bars(1 + rng.below(16) as u32)),
                    },
                })
                .collect();
            let mut playlist = playlist_of(&specs);
            playlist.shuffle = Some(ShuffleSpec {
                seed,
                no_repeat_window: 0,
            });
            if let Some(section) = playlist.sections.first_mut() {
                for cue in &mut section.cues {
                    cue.enabled = rng.below(5) != 0;
                    if rng.below(3) == 0 {
                        cue.settings.transition =
                            Some(TransitionSpec::crossfade(rng.range(0.0, 40.0)));
                    }
                }
            }
            let bpm = 60.0 + rng.range(0.0, 140.0);
            let timeline =
                Timeline::from_playlist(&playlist, &TimelineConfig { fallback_bpm: bpm });

            let mut previous_start = -1.0;
            let mut sum = 0.0;
            for cue in timeline.cues() {
                assert!(
                    cue.start > previous_start,
                    "seed {seed}: starts are not strictly increasing"
                );
                assert_eq!(
                    cue.start, sum,
                    "seed {seed}: a cue does not start where the last ended"
                );
                assert!(
                    cue.duration >= MIN_CUE_SECONDS,
                    "seed {seed}: cue shorter than the minimum"
                );
                assert!(
                    cue.transition_end >= cue.start && cue.transition_end <= cue.end(),
                    "seed {seed}: transition window escapes its cue"
                );
                previous_start = cue.start;
                sum += cue.duration;
            }
            let total = timeline.total_runtime();
            assert!(
                (total - sum).abs() <= 1e-9 * sum.max(1.0),
                "seed {seed}: total {total} is not the sum of durations {sum}"
            );
        }
        Ok(())
    }
}
