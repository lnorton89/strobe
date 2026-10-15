// ============================================================================
// Module:       strobe_playlist::clock::tests
// Description:  Tests for the playlist clock: advancing, transitions, hold,
//               commands, beat-locked ends, skipping, resume and the wall
//               clock.
//
// Dependencies: crate::clock, crate::timeline, crate::model
// ============================================================================

//! Playlist clock tests.
//!
//! Time is scripted: tests tick at a fixed `dt` and build the music with
//! [`Grid`], a steady beat grid, so every expectation is a number of
//! seconds that can be checked by hand.

use super::*;
use crate::duration::DurationSpec;
use crate::model::{
    AdvanceMode, Cue, Playlist, SceneRef, Section, TransitionKind, TransitionSpec, WallClockAnchor,
};
use crate::timeline::TimelineConfig;
use anyhow::{Result, anyhow};

/// Tick size for the scripted runs.
const DT: f64 = 0.05;

fn sec(seconds: f64) -> DurationSpec {
    DurationSpec::Seconds(seconds)
}

/// A one-section playlist of seconds cues with a default transition.
fn show(durations: &[f64], transition: TransitionSpec, loop_mode: LoopMode) -> Playlist {
    let mut playlist = Playlist::new("t");
    playlist.loop_mode = loop_mode;
    playlist.defaults.transition = Some(transition);
    let mut section = Section::new("s");
    for (i, seconds) in durations.iter().enumerate() {
        let mut cue = Cue::new(SceneRef::Library(format!("c{i}")));
        cue.settings.duration = Some(sec(*seconds));
        section.cues.push(cue);
    }
    playlist.sections.push(section);
    playlist
}

fn clock_for(playlist: &Playlist) -> PlaylistClock {
    PlaylistClock::new(
        Timeline::from_playlist(playlist, &TimelineConfig::default()),
        ClockConfig::default(),
    )
}

fn cuts(durations: &[f64]) -> PlaylistClock {
    clock_for(&show(durations, TransitionSpec::cut(), LoopMode::Stop))
}

/// A steady beat grid: at time `t` the grid is `start + t * bpm / 60` beats.
#[derive(Clone, Copy)]
struct Grid {
    start: f64,
    bpm: f32,
    confidence: f32,
}

impl Grid {
    const SILENT: Self = Self {
        start: 0.0,
        bpm: 120.0,
        confidence: 0.0,
    };

    fn at(&self, t: f64) -> BeatInfo {
        BeatInfo {
            bpm: self.bpm,
            confidence: self.confidence,
            beats: self.start + t * f64::from(self.bpm) / 60.0,
            downbeat: 0.0,
        }
    }
}

/// Ticks for `seconds` in `DT` steps from time `*now`, returning every
/// event with the time it was drained.
fn run(
    clock: &mut PlaylistClock,
    now: &mut f64,
    seconds: f64,
    grid: Grid,
) -> Vec<(f64, ClockEvent)> {
    let mut seen = Vec::new();
    let steps = (seconds / DT).round() as usize;
    for _ in 0..steps {
        *now += DT;
        clock.tick(DT, &grid.at(*now));
        seen.extend(clock.drain_events().map(|event| (*now, event)));
    }
    seen
}

fn only(events: &[(f64, ClockEvent)]) -> Vec<ClockEvent> {
    events.iter().map(|(_, event)| *event).collect()
}

fn current(clock: &PlaylistClock) -> Result<usize> {
    clock
        .current()
        .ok_or_else(|| anyhow!("the clock is not playing"))
}

fn approx(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

// ---- advancing ------------------------------------------------------------

#[test]
fn the_first_tick_starts_the_first_cue() -> Result<()> {
    let mut clock = cuts(&[10.0, 10.0]);
    assert_eq!(clock.state(), ClockState::Idle, "built idle");
    assert_eq!(clock.current(), None, "nothing is active yet");
    clock.tick(0.0, &BeatInfo::SILENT);
    assert_eq!(current(&clock)?, 0, "cue 0 is active");
    assert_eq!(
        clock.drain_events().collect::<Vec<_>>(),
        vec![ClockEvent::CueStarted { index: 0, pass: 0 }],
        "one CueStarted"
    );
    Ok(())
}

#[test]
fn cues_advance_when_their_time_is_up() -> Result<()> {
    let mut clock = cuts(&[10.0, 10.0, 10.0]);
    let mut now = 0.0;
    let events = run(&mut clock, &mut now, 10.5, Grid::SILENT);
    assert_eq!(current(&clock)?, 1, "10.5 s in, the second cue is active");
    assert!(
        approx(clock.elapsed(), 0.5, 1e-6),
        "0.5 s into it, got {}",
        clock.elapsed()
    );
    let starts: Vec<(usize, f64)> = events
        .iter()
        .filter_map(|(t, e)| match e {
            ClockEvent::CueStarted { index, .. } => Some((*index, *t)),
            ClockEvent::TransitionStarted { .. }
            | ClockEvent::TransitionFinished { .. }
            | ClockEvent::WarmUp { .. }
            | ClockEvent::CueSkipped { .. }
            | ClockEvent::HoldChanged(_)
            | ClockEvent::BlackoutChanged(_)
            | ClockEvent::PlaylistEnded => None,
        })
        .collect();
    assert_eq!(starts.len(), 2, "two cue starts: {starts:?}");
    assert!(
        starts
            .get(1)
            .is_some_and(|(index, t)| *index == 1 && approx(*t, 10.0, DT)),
        "cue 1 starts at 10 s: {starts:?}"
    );
    Ok(())
}

#[test]
fn a_long_frame_carries_its_time_across_cues() -> Result<()> {
    let mut clock = cuts(&[10.0, 10.0, 10.0]);
    clock.tick(0.0, &BeatInfo::SILENT);
    clock.tick(25.0, &BeatInfo::SILENT);
    assert_eq!(current(&clock)?, 2, "25 s lands in the third cue");
    assert!(
        approx(clock.elapsed(), 5.0, 1e-9),
        "5 s into it, got {}",
        clock.elapsed()
    );
    Ok(())
}

#[test]
fn a_stopping_playlist_ends_once_and_goes_quiet() -> Result<()> {
    let mut clock = cuts(&[5.0, 5.0]);
    let mut now = 0.0;
    let events = only(&run(&mut clock, &mut now, 12.0, Grid::SILENT));
    assert_eq!(clock.state(), ClockState::Ended, "ended");
    assert_eq!(clock.current(), None, "no active cue");
    assert_eq!(
        events
            .iter()
            .filter(|e| matches!(e, ClockEvent::PlaylistEnded))
            .count(),
        1,
        "PlaylistEnded exactly once: {events:?}"
    );
    assert!(
        run(&mut clock, &mut now, 5.0, Grid::SILENT).is_empty(),
        "no events after the end"
    );
    Ok(())
}

#[test]
fn a_looping_playlist_wraps_and_counts_passes() -> Result<()> {
    let mut clock = clock_for(&show(&[5.0, 5.0], TransitionSpec::cut(), LoopMode::Loop));
    let mut now = 0.0;
    run(&mut clock, &mut now, 12.5, Grid::SILENT);
    assert_eq!(current(&clock)?, 0, "back on the first cue");
    assert_eq!(clock.pass(), 1, "second pass");
    run(&mut clock, &mut now, 10.0, Grid::SILENT);
    assert_eq!(clock.pass(), 2, "third pass");
    Ok(())
}

#[test]
fn a_section_loop_stays_in_its_section() -> Result<()> {
    let mut playlist = Playlist::new("t");
    playlist.loop_mode = LoopMode::LoopSection;
    playlist.defaults.transition = Some(TransitionSpec::cut());
    for (name, count) in [("a", 2), ("b", 2)] {
        let mut section = Section::new(name);
        for i in 0..count {
            let mut cue = Cue::new(SceneRef::Library(format!("{name}{i}")));
            cue.settings.duration = Some(sec(5.0));
            section.cues.push(cue);
        }
        playlist.sections.push(section);
    }
    let mut clock = clock_for(&playlist);
    let mut now = 0.0;
    run(&mut clock, &mut now, 1.0, Grid::SILENT);
    assert!(clock.jump_to(2), "go to the second section");
    run(&mut clock, &mut now, 22.0, Grid::SILENT);
    let index = current(&clock)?;
    assert!(
        (2..4).contains(&index),
        "still in section b, on cue {index}"
    );
    assert!(
        clock.pass() >= 2,
        "it wrapped within the section: pass {}",
        clock.pass()
    );
    Ok(())
}

// ---- transitions ----------------------------------------------------------

#[test]
fn a_crossfade_runs_from_zero_to_one_and_finishes() -> Result<()> {
    let mut clock = clock_for(&show(
        &[10.0, 10.0],
        TransitionSpec::crossfade(2.0),
        LoopMode::Stop,
    ));
    let mut now = 0.0;
    run(&mut clock, &mut now, 10.0 + DT, Grid::SILENT);
    let t = clock
        .transition()
        .ok_or_else(|| anyhow!("no transition after the cue change"))?;
    assert_eq!((t.from, t.to), (0, 1), "from cue 0 to cue 1");
    assert!(t.progress < 0.1, "just started, got {}", t.progress);

    let mid = only(&run(&mut clock, &mut now, 1.0, Grid::SILENT));
    assert!(mid.is_empty(), "nothing happens mid-transition: {mid:?}");
    let t = clock
        .transition()
        .ok_or_else(|| anyhow!("transition ended early"))?;
    assert!(
        approx(t.progress, 0.5, 0.05),
        "about halfway after 1 s of 2, got {}",
        t.progress
    );

    let after = only(&run(&mut clock, &mut now, 1.5, Grid::SILENT));
    assert_eq!(
        after,
        vec![ClockEvent::TransitionFinished {
            id: 0,
            completed: true
        }],
        "it finishes once, completed"
    );
    assert!(clock.transition().is_none(), "no transition afterwards");
    Ok(())
}

#[test]
fn transition_events_pair_up_with_unique_ids() -> Result<()> {
    let mut clock = clock_for(&show(
        &[6.0, 6.0, 6.0, 6.0],
        TransitionSpec::crossfade(2.0),
        LoopMode::Loop,
    ));
    let mut now = 0.0;
    let events = only(&run(&mut clock, &mut now, 100.0, Grid::SILENT));
    let started: Vec<u64> = events
        .iter()
        .filter_map(|e| match e {
            ClockEvent::TransitionStarted { id, .. } => Some(*id),
            ClockEvent::CueStarted { .. }
            | ClockEvent::TransitionFinished { .. }
            | ClockEvent::WarmUp { .. }
            | ClockEvent::CueSkipped { .. }
            | ClockEvent::HoldChanged(_)
            | ClockEvent::BlackoutChanged(_)
            | ClockEvent::PlaylistEnded => None,
        })
        .collect();
    let finished: Vec<u64> = events
        .iter()
        .filter_map(|e| match e {
            ClockEvent::TransitionFinished {
                id,
                completed: true,
            } => Some(*id),
            ClockEvent::TransitionFinished {
                completed: false, ..
            }
            | ClockEvent::CueStarted { .. }
            | ClockEvent::TransitionStarted { .. }
            | ClockEvent::WarmUp { .. }
            | ClockEvent::CueSkipped { .. }
            | ClockEvent::HoldChanged(_)
            | ClockEvent::BlackoutChanged(_)
            | ClockEvent::PlaylistEnded => None,
        })
        .collect();
    assert!(
        started.len() >= 15,
        "about 16 transitions in 100 s, got {}",
        started.len()
    );
    assert!(
        started.iter().enumerate().all(|(i, id)| *id == i as u64),
        "ids count up"
    );
    // The last one may still be in flight.
    assert!(
        finished.len() + 1 >= started.len() && finished.len() <= started.len(),
        "every started transition finishes: {} started, {} finished",
        started.len(),
        finished.len()
    );
    Ok(())
}

#[test]
fn a_command_cuts_a_transition_short_and_says_so() -> Result<()> {
    let mut clock = clock_for(&show(
        &[10.0, 10.0, 10.0],
        TransitionSpec::crossfade(4.0),
        LoopMode::Stop,
    ));
    let mut now = 0.0;
    run(&mut clock, &mut now, 10.0 + 1.0, Grid::SILENT);
    assert!(clock.transition().is_some(), "in the middle of 0 -> 1");
    assert!(clock.next_cue(), "next");
    let events = clock.drain_events().collect::<Vec<_>>();
    assert_eq!(
        events,
        vec![
            ClockEvent::TransitionFinished {
                id: 0,
                completed: false
            },
            ClockEvent::CueStarted { index: 2, pass: 0 },
            ClockEvent::TransitionStarted {
                id: 1,
                from: 1,
                to: 2
            },
        ],
        "the old one is closed as incomplete, then the new one opens from where the show was"
    );
    Ok(())
}

#[test]
fn a_transition_never_outlasts_a_short_cue() -> Result<()> {
    let mut clock = clock_for(&show(
        &[10.0, 1.0, 10.0],
        TransitionSpec::crossfade(4.0),
        LoopMode::Stop,
    ));
    let mut now = 0.0;
    run(&mut clock, &mut now, 10.0 + 0.5, Grid::SILENT);
    let t = clock.transition().ok_or_else(|| anyhow!("no transition"))?;
    assert!(
        t.progress > 0.4 && t.progress < 0.6,
        "a 4 s fade squeezed into 1 s: {}",
        t.progress
    );
    let events = only(&run(&mut clock, &mut now, 1.0, Grid::SILENT));
    let finish = events.iter().position(|e| {
        matches!(
            e,
            ClockEvent::TransitionFinished {
                completed: true,
                ..
            }
        )
    });
    let start = events
        .iter()
        .position(|e| matches!(e, ClockEvent::CueStarted { index: 2, .. }));
    assert!(
        finish.is_some() && start.is_some() && finish < start,
        "the fade completes before the next cue starts: {events:?}"
    );
    Ok(())
}

#[test]
fn cuts_and_the_first_cue_have_no_transition() -> Result<()> {
    let mut clock = clock_for(&show(
        &[5.0, 5.0],
        TransitionSpec::crossfade(2.0),
        LoopMode::Stop,
    ));
    clock.tick(0.0, &BeatInfo::SILENT);
    assert!(
        clock.transition().is_none(),
        "nothing to fade from at the start"
    );
    let mut cut = cuts(&[5.0, 5.0]);
    let mut now = 0.0;
    let events = only(&run(&mut cut, &mut now, 6.0, Grid::SILENT));
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, ClockEvent::TransitionStarted { .. })),
        "a cut never starts a transition: {events:?}"
    );
    Ok(())
}

#[test]
fn a_single_cue_loop_does_not_fade_into_itself() -> Result<()> {
    let mut clock = clock_for(&show(
        &[5.0],
        TransitionSpec::crossfade(2.0),
        LoopMode::Loop,
    ));
    let mut now = 0.0;
    let events = only(&run(&mut clock, &mut now, 12.0, Grid::SILENT));
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, ClockEvent::TransitionStarted { .. })),
        "same scene to same scene: {events:?}"
    );
    assert!(clock.pass() >= 2, "but it does count passes");
    Ok(())
}

// ---- hold, extend, restart, blackout --------------------------------------

#[test]
fn hold_freezes_the_cue_and_unhold_resumes_it() -> Result<()> {
    let mut clock = cuts(&[10.0, 10.0]);
    let mut now = 0.0;
    run(&mut clock, &mut now, 4.0, Grid::SILENT);
    assert!(clock.hold(), "hold");
    assert!(!clock.hold(), "holding twice does nothing");
    assert!(clock.is_held(), "held");
    let before = clock.elapsed();
    let held_events = only(&run(&mut clock, &mut now, 30.0, Grid::SILENT));
    assert_eq!(clock.elapsed(), before, "elapsed does not move while held");
    assert_eq!(current(&clock)?, 0, "and the cue does not end");
    assert_eq!(
        held_events,
        vec![ClockEvent::HoldChanged(true)],
        "only the hold announcement, nothing else while frozen"
    );
    assert!(clock.unhold(), "unhold");
    run(&mut clock, &mut now, 5.0, Grid::SILENT);
    assert_eq!(current(&clock)?, 0, "6 s of the cue have run, 4 to go");
    run(&mut clock, &mut now, 1.5, Grid::SILENT);
    assert_eq!(current(&clock)?, 1, "and now it moves on");
    Ok(())
}

#[test]
fn a_transition_finishes_even_while_held() -> Result<()> {
    let mut clock = clock_for(&show(
        &[5.0, 5.0],
        TransitionSpec::crossfade(2.0),
        LoopMode::Stop,
    ));
    let mut now = 0.0;
    run(&mut clock, &mut now, 5.5, Grid::SILENT);
    assert!(clock.transition().is_some(), "mid-fade");
    clock.hold();
    let events = only(&run(&mut clock, &mut now, 3.0, Grid::SILENT));
    assert!(
        clock.transition().is_none(),
        "the fade completed under hold"
    );
    assert!(
        events.contains(&ClockEvent::TransitionFinished {
            id: 0,
            completed: true
        }),
        "and said so: {events:?}"
    );
    Ok(())
}

#[test]
fn commands_still_work_while_held() -> Result<()> {
    let mut clock = cuts(&[10.0, 10.0, 10.0]);
    clock.tick(0.0, &BeatInfo::SILENT);
    clock.hold();
    assert!(clock.next_cue(), "next while held");
    assert_eq!(current(&clock)?, 1, "moved");
    assert!(clock.is_held(), "and still held");
    Ok(())
}

#[test]
fn hold_and_blackout_announce_changes_only() -> Result<()> {
    let mut clock = cuts(&[10.0]);
    clock.tick(0.0, &BeatInfo::SILENT);
    clock.drain_events().for_each(drop);
    clock.apply(Command::ToggleHold);
    clock.apply(Command::ToggleHold);
    clock.apply(Command::ToggleBlackout);
    clock.set_blackout(true);
    assert!(clock.is_blackout(), "blackout is on");
    assert_eq!(
        clock.drain_events().collect::<Vec<_>>(),
        vec![
            ClockEvent::HoldChanged(true),
            ClockEvent::HoldChanged(false),
            ClockEvent::BlackoutChanged(true)
        ],
        "one event per actual change"
    );
    Ok(())
}

#[test]
fn blackout_does_not_stop_the_show() -> Result<()> {
    let mut clock = cuts(&[5.0, 5.0]);
    clock.tick(0.0, &BeatInfo::SILENT);
    clock.apply(Command::ToggleBlackout);
    let mut now = 0.0;
    run(&mut clock, &mut now, 6.0, Grid::SILENT);
    assert_eq!(current(&clock)?, 1, "the cue clock ran under blackout");
    assert!(clock.is_blackout(), "still dark");
    Ok(())
}

#[test]
fn extend_moves_the_end_later_or_earlier() -> Result<()> {
    let mut clock = cuts(&[10.0, 10.0]);
    let mut now = 0.0;
    run(&mut clock, &mut now, 8.0, Grid::SILENT);
    assert!(clock.extend(5.0), "extend");
    assert!(
        approx(clock.cue_length(), 15.0, 1e-9),
        "15 s now, got {}",
        clock.cue_length()
    );
    assert!(
        approx(clock.remaining().unwrap_or(-1.0), 7.0, 0.1),
        "7 s to go"
    );
    run(&mut clock, &mut now, 6.0, Grid::SILENT);
    assert_eq!(current(&clock)?, 0, "14 s in, still the first cue");
    run(&mut clock, &mut now, 1.5, Grid::SILENT);
    assert_eq!(current(&clock)?, 1, "15 s in, moved on");

    assert!(clock.extend(-100.0), "a huge negative extend");
    assert_eq!(clock.cue_length(), 0.0, "never below zero");
    assert!(!clock.extend(f64::NAN), "NaN is refused");
    Ok(())
}

#[test]
fn restart_cue_begins_it_again_without_a_transition() -> Result<()> {
    let mut clock = clock_for(&show(
        &[10.0, 10.0],
        TransitionSpec::crossfade(2.0),
        LoopMode::Stop,
    ));
    let mut now = 0.0;
    run(&mut clock, &mut now, 13.0, Grid::SILENT);
    clock.drain_events().for_each(drop);
    assert!(clock.restart_cue(), "restart");
    assert_eq!(clock.elapsed(), 0.0, "back to the start");
    assert_eq!(current(&clock)?, 1, "same cue");
    assert_eq!(
        clock.drain_events().collect::<Vec<_>>(),
        vec![ClockEvent::CueStarted { index: 1, pass: 0 }],
        "a fresh CueStarted, no transition"
    );
    Ok(())
}

// ---- navigation -------------------------------------------------------------

#[test]
fn next_and_previous_move_through_the_playlist() -> Result<()> {
    let mut clock = cuts(&[10.0, 10.0, 10.0]);
    clock.tick(0.0, &BeatInfo::SILENT);
    assert_eq!(clock.peek_next(), Some(1), "peek");
    assert!(clock.next_cue(), "next");
    assert_eq!(current(&clock)?, 1, "second");
    assert!(clock.previous_cue(), "previous");
    assert_eq!(current(&clock)?, 0, "back to first");
    clock.tick(3.0, &BeatInfo::SILENT);
    assert!(
        clock.previous_cue(),
        "previous at the start of a stopping playlist"
    );
    assert_eq!(current(&clock)?, 0, "restarts the first cue");
    assert_eq!(clock.elapsed(), 0.0, "from the beginning");
    Ok(())
}

#[test]
fn next_past_the_last_cue_ends_a_stopping_playlist() -> Result<()> {
    let mut clock = cuts(&[10.0, 10.0]);
    clock.tick(0.0, &BeatInfo::SILENT);
    clock.next_cue();
    clock.next_cue();
    assert_eq!(clock.state(), ClockState::Ended, "ended");
    assert!(!clock.next_cue(), "next does nothing once ended");
    Ok(())
}

#[test]
fn previous_wraps_in_a_loop_and_steps_the_pass_back() -> Result<()> {
    let mut clock = clock_for(&show(
        &[5.0, 5.0, 5.0],
        TransitionSpec::cut(),
        LoopMode::Loop,
    ));
    let mut now = 0.0;
    run(&mut clock, &mut now, 16.0, Grid::SILENT);
    assert_eq!((current(&clock)?, clock.pass()), (0, 1), "start of pass 1");
    clock.previous_cue();
    assert_eq!(
        (current(&clock)?, clock.pass()),
        (2, 0),
        "last cue of pass 0"
    );
    clock.next_cue();
    assert_eq!(
        (current(&clock)?, clock.pass()),
        (0, 1),
        "and forward again"
    );
    Ok(())
}

#[test]
fn jump_to_goes_anywhere_valid() -> Result<()> {
    let mut clock = cuts(&[5.0, 5.0, 5.0]);
    clock.tick(0.0, &BeatInfo::SILENT);
    assert!(clock.jump_to(2), "jump");
    assert_eq!(current(&clock)?, 2, "arrived");
    assert!(!clock.jump_to(3), "beyond the end");
    assert!(clock.apply(Command::JumpTo(0)), "through the command form");
    clock.mark_failed(1);
    assert!(!clock.jump_to(1), "a failed cue cannot be jumped to");
    assert_eq!(current(&clock)?, 0, "and the show stays put");
    Ok(())
}

#[test]
fn jump_to_revives_an_ended_playlist() -> Result<()> {
    let mut clock = cuts(&[1.0]);
    let mut now = 0.0;
    run(&mut clock, &mut now, 3.0, Grid::SILENT);
    assert_eq!(clock.state(), ClockState::Ended, "ended");
    assert!(clock.jump_to(0), "jump");
    assert_eq!(clock.state(), ClockState::Playing, "playing again");
    Ok(())
}

#[test]
fn skip_section_goes_to_the_next_sections_first_cue() -> Result<()> {
    let mut playlist = Playlist::new("t");
    playlist.defaults.transition = Some(TransitionSpec::cut());
    for (name, count) in [("a", 3), ("b", 2), ("c", 1)] {
        let mut section = Section::new(name);
        for i in 0..count {
            section
                .cues
                .push(Cue::new(SceneRef::Library(format!("{name}{i}"))));
        }
        playlist.sections.push(section);
    }
    let mut clock = clock_for(&playlist);
    clock.tick(0.0, &BeatInfo::SILENT);
    clock.next_cue();
    assert!(
        clock.apply(Command::SkipSection),
        "skip from the middle of section a"
    );
    assert_eq!(current(&clock)?, 3, "first cue of section b");
    clock.skip_section();
    assert_eq!(current(&clock)?, 5, "section c");
    clock.skip_section();
    assert_eq!(
        clock.state(),
        ClockState::Ended,
        "skipping the last section ends a stopping playlist"
    );
    Ok(())
}

#[test]
fn skip_section_wraps_in_a_loop_even_a_section_loop() -> Result<()> {
    for mode in [LoopMode::Loop, LoopMode::LoopSection] {
        let mut playlist = Playlist::new("t");
        playlist.loop_mode = mode;
        playlist.defaults.transition = Some(TransitionSpec::cut());
        for name in ["a", "b"] {
            let mut section = Section::new(name);
            section
                .cues
                .push(Cue::new(SceneRef::Library(format!("{name}0"))));
            section
                .cues
                .push(Cue::new(SceneRef::Library(format!("{name}1"))));
            playlist.sections.push(section);
        }
        let mut clock = clock_for(&playlist);
        clock.tick(0.0, &BeatInfo::SILENT);
        clock.skip_section();
        assert_eq!(current(&clock)?, 2, "{mode:?}: into section b");
        clock.skip_section();
        assert_eq!(current(&clock)?, 0, "{mode:?}: wrapped to section a");
    }
    Ok(())
}

#[test]
fn skip_section_with_one_section_restarts_it_when_looping() -> Result<()> {
    let mut clock = clock_for(&show(&[5.0, 5.0], TransitionSpec::cut(), LoopMode::Loop));
    clock.tick(0.0, &BeatInfo::SILENT);
    clock.next_cue();
    clock.skip_section();
    assert_eq!(current(&clock)?, 0, "back to the section's first cue");
    Ok(())
}

// ---- manual and audio-event advance -----------------------------------------

#[test]
fn manual_cues_have_no_deadline() -> Result<()> {
    let mut playlist = show(&[5.0, 5.0], TransitionSpec::cut(), LoopMode::Stop);
    playlist.defaults.advance = Some(AdvanceMode::Manual);
    let mut clock = clock_for(&playlist);
    let mut now = 0.0;
    run(&mut clock, &mut now, 600.0, Grid::SILENT);
    assert_eq!(
        current(&clock)?,
        0,
        "ten minutes later, still on the first cue"
    );
    assert_eq!(clock.remaining(), None, "no time remaining to report");
    assert!(clock.elapsed() > 599.0, "but elapsed time keeps counting");
    assert!(clock.next_cue(), "a person moves it on");
    assert_eq!(current(&clock)?, 1, "next cue");
    assert_eq!(
        clock.warming(),
        None,
        "nothing to warm up for an open-ended cue"
    );
    Ok(())
}

#[test]
fn audio_events_advance_only_cues_that_listen_for_them() -> Result<()> {
    let mut playlist = show(
        &[100.0, 100.0, 100.0],
        TransitionSpec::cut(),
        LoopMode::Stop,
    );
    if let Some(cue) = playlist
        .sections
        .first_mut()
        .and_then(|s| s.cues.first_mut())
    {
        cue.settings.advance = Some(AdvanceMode::FirstOf(vec![
            AdvanceMode::Timer,
            AdvanceMode::AudioEvent(AudioEventKind::Drop),
        ]));
    }
    if let Some(cue) = playlist
        .sections
        .first_mut()
        .and_then(|s| s.cues.get_mut(1))
    {
        cue.settings.advance = Some(AdvanceMode::AudioEvent(AudioEventKind::Breakdown));
    }
    let mut clock = clock_for(&playlist);
    clock.tick(0.0, &BeatInfo::SILENT);
    assert!(
        !clock.audio_event(AudioEventKind::Breakdown),
        "cue 0 does not listen for breakdowns"
    );
    assert_eq!(current(&clock)?, 0, "so it stays");
    assert!(
        clock.apply(Command::AudioEvent(AudioEventKind::Drop)),
        "but it does for drops"
    );
    assert_eq!(current(&clock)?, 1, "and moves on");
    assert_eq!(clock.remaining(), None, "an event-only cue has no timer");
    assert!(
        !clock.audio_event(AudioEventKind::Drop),
        "cue 1 ignores drops"
    );
    assert!(
        clock.audio_event(AudioEventKind::Breakdown),
        "and takes breakdowns"
    );
    assert_eq!(current(&clock)?, 2, "advanced");
    assert!(
        !clock.audio_event(AudioEventKind::Drop),
        "a timer-only cue ignores events"
    );
    Ok(())
}

#[test]
fn a_song_change_ends_a_listening_cue_but_not_during_a_transition() -> Result<()> {
    let mut playlist = show(
        &[100.0, 100.0, 100.0],
        TransitionSpec::crossfade(4.0),
        LoopMode::Stop,
    );
    for cue in playlist.sections.iter_mut().flat_map(|s| s.cues.iter_mut()) {
        cue.settings.advance = Some(AdvanceMode::AudioEvent(AudioEventKind::SongChange));
    }
    let mut clock = clock_for(&playlist);
    clock.tick(0.0, &BeatInfo::SILENT);
    assert!(
        clock.audio_event(AudioEventKind::SongChange),
        "the next track started"
    );
    clock.tick(1.0, &BeatInfo::SILENT);
    assert!(clock.transition().is_some(), "crossfading into cue 1");
    assert!(
        !clock.audio_event(AudioEventKind::SongChange),
        "a second event mid-crossfade does not cut it short"
    );
    clock.tick(4.0, &BeatInfo::SILENT);
    assert_eq!(current(&clock)?, 1, "cue 1 plays");
    assert!(clock.transition().is_none(), "the crossfade finished");
    assert!(
        clock.audio_event(AudioEventKind::SongChange),
        "after it, the next song moves on again"
    );
    Ok(())
}

// ---- failed cues ---------------------------------------------------------------

#[test]
fn failed_cues_are_skipped_with_an_event() -> Result<()> {
    let mut clock = cuts(&[5.0, 5.0, 5.0, 5.0]);
    clock.tick(0.0, &BeatInfo::SILENT);
    clock.mark_failed(1);
    clock.mark_failed(2);
    clock.drain_events().for_each(drop);
    assert_eq!(clock.peek_next(), Some(3), "peek sees past the failures");
    assert!(clock.next_cue(), "next");
    assert_eq!(current(&clock)?, 3, "went to the first playable cue");
    assert_eq!(
        clock.drain_events().collect::<Vec<_>>(),
        vec![
            ClockEvent::CueSkipped {
                index: 1,
                reason: SkipReason::SceneFailed
            },
            ClockEvent::CueSkipped {
                index: 2,
                reason: SkipReason::SceneFailed
            },
            ClockEvent::CueStarted { index: 3, pass: 0 },
        ],
        "each skipped cue is reported, in order, before the cue that plays"
    );
    Ok(())
}

#[test]
fn a_failed_first_cue_is_skipped_at_the_start() -> Result<()> {
    let mut clock = cuts(&[5.0, 5.0]);
    clock.mark_failed(0);
    clock.tick(0.0, &BeatInfo::SILENT);
    assert_eq!(current(&clock)?, 1, "starts on the second cue");
    assert_eq!(
        clock.drain_events().collect::<Vec<_>>(),
        vec![
            ClockEvent::CueSkipped {
                index: 0,
                reason: SkipReason::SceneFailed
            },
            ClockEvent::CueStarted { index: 1, pass: 0 }
        ],
        "with the skip reported"
    );
    Ok(())
}

#[test]
fn a_cue_that_fails_while_playing_is_left_at_once_without_a_fade() -> Result<()> {
    let mut clock = clock_for(&show(
        &[10.0, 10.0],
        TransitionSpec::crossfade(2.0),
        LoopMode::Stop,
    ));
    clock.tick(0.0, &BeatInfo::SILENT);
    clock.drain_events().for_each(drop);
    clock.mark_failed(0);
    assert_eq!(current(&clock)?, 1, "moved on");
    assert_eq!(
        clock.drain_events().collect::<Vec<_>>(),
        vec![
            ClockEvent::CueSkipped {
                index: 0,
                reason: SkipReason::SceneFailed
            },
            ClockEvent::CueStarted { index: 1, pass: 0 }
        ],
        "reported, and no transition from a scene that is not there"
    );
    Ok(())
}

#[test]
fn skipping_never_visits_a_failed_cue_in_a_loop() -> Result<()> {
    let mut clock = clock_for(&show(
        &[2.0, 2.0, 2.0],
        TransitionSpec::cut(),
        LoopMode::Loop,
    ));
    clock.mark_failed(1);
    let mut now = 0.0;
    let events = only(&run(&mut clock, &mut now, 30.0, Grid::SILENT));
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, ClockEvent::CueStarted { index: 1, .. })),
        "the failed cue never starts"
    );
    assert!(
        events
            .iter()
            .filter(|e| matches!(e, ClockEvent::CueSkipped { index: 1, .. }))
            .count()
            >= 3,
        "and is reported each time it is passed"
    );
    Ok(())
}

#[test]
fn when_everything_failed_the_playlist_ends() -> Result<()> {
    let mut clock = cuts(&[5.0, 5.0]);
    clock.mark_failed(0);
    clock.mark_failed(1);
    clock.tick(0.0, &BeatInfo::SILENT);
    assert_eq!(clock.state(), ClockState::Ended, "nothing to play");
    let events = clock.drain_events().collect::<Vec<_>>();
    assert_eq!(
        events.last(),
        Some(&ClockEvent::PlaylistEnded),
        "and says so: {events:?}"
    );

    let mut looping = clock_for(&show(&[5.0, 5.0], TransitionSpec::cut(), LoopMode::Loop));
    looping.tick(0.0, &BeatInfo::SILENT);
    looping.mark_failed(0);
    looping.mark_failed(1);
    assert_eq!(
        looping.state(),
        ClockState::Ended,
        "a loop of nothing must not spin forever"
    );
    Ok(())
}

#[test]
fn mark_scene_failed_marks_every_cue_using_the_scene() -> Result<()> {
    let mut playlist = show(&[5.0, 5.0, 5.0], TransitionSpec::cut(), LoopMode::Stop);
    if let Some(cue) = playlist
        .sections
        .first_mut()
        .and_then(|s| s.cues.get_mut(2))
    {
        cue.scene = SceneRef::Library("c0".into());
    }
    let mut clock = clock_for(&playlist);
    assert_eq!(
        clock.mark_scene_failed(&SceneRef::Library("c0".into())),
        2,
        "two cues use c0"
    );
    assert!(
        clock.is_failed(0) && clock.is_failed(2) && !clock.is_failed(1),
        "flags"
    );
    assert_eq!(
        clock.mark_scene_failed(&SceneRef::Library("nope".into())),
        0,
        "unknown scene"
    );
    clock.mark_failed(99);
    assert!(!clock.is_failed(99), "out of range is ignored");
    Ok(())
}

// ---- warm-up -----------------------------------------------------------------

#[test]
fn warm_up_fires_once_two_seconds_before_the_end() -> Result<()> {
    let mut clock = cuts(&[10.0, 10.0, 10.0]);
    let mut now = 0.0;
    let events = run(&mut clock, &mut now, 9.0, Grid::SILENT);
    let warm: Vec<(f64, ClockEvent)> = events
        .iter()
        .filter(|(_, e)| matches!(e, ClockEvent::WarmUp { .. }))
        .copied()
        .collect();
    assert_eq!(warm.len(), 1, "exactly one warm-up: {warm:?}");
    assert!(
        warm.first()
            .is_some_and(|(t, e)| approx(*t, 8.0, DT) && *e == ClockEvent::WarmUp { index: 1 }),
        "at 8 s, for cue 1: {warm:?}"
    );
    assert_eq!(
        clock.warming(),
        Some(1),
        "and the clock still reports who is warming"
    );
    run(&mut clock, &mut now, 2.0, Grid::SILENT);
    assert_eq!(
        clock.warming(),
        None,
        "cleared when the next cue starts, until its own warm-up"
    );
    Ok(())
}

#[test]
fn a_short_cue_warms_up_the_moment_it_starts() -> Result<()> {
    let mut clock = cuts(&[10.0, 1.0, 10.0]);
    let mut now = 0.0;
    run(&mut clock, &mut now, 10.0 + DT, Grid::SILENT);
    assert_eq!(
        clock.warming(),
        Some(2),
        "a 1 s cue is inside the 2 s lead from the start"
    );
    Ok(())
}

#[test]
fn warm_up_targets_the_next_playable_cue_and_wraps_in_a_loop() -> Result<()> {
    let mut clock = clock_for(&show(
        &[5.0, 5.0, 5.0],
        TransitionSpec::cut(),
        LoopMode::Loop,
    ));
    clock.mark_failed(1);
    let mut now = 0.0;
    let events = only(&run(&mut clock, &mut now, 4.0, Grid::SILENT));
    assert!(
        events.contains(&ClockEvent::WarmUp { index: 2 }),
        "skips the failed cue: {events:?}"
    );
    run(&mut clock, &mut now, 6.0, Grid::SILENT);
    assert_eq!(current(&clock)?, 2, "playing the last cue");
    assert_eq!(
        clock.peek_next(),
        Some(0),
        "the next one is the wrap-around"
    );
    Ok(())
}

#[test]
fn the_last_cue_of_a_stopping_playlist_has_nothing_to_warm() -> Result<()> {
    let mut clock = cuts(&[3.0]);
    let mut now = 0.0;
    let events = only(&run(&mut clock, &mut now, 3.5, Grid::SILENT));
    assert!(
        !events
            .iter()
            .any(|e| matches!(e, ClockEvent::WarmUp { .. })),
        "no successor, no warm-up: {events:?}"
    );
    Ok(())
}

// ---- beat-locked ends ---------------------------------------------------------

/// A playlist of one 8-beat cue (4 s at the 120 BPM fallback) then a seconds cue.
fn beat_show() -> PlaylistClock {
    let mut playlist = show(&[1000.0, 1000.0], TransitionSpec::cut(), LoopMode::Stop);
    if let Some(cue) = playlist
        .sections
        .first_mut()
        .and_then(|s| s.cues.first_mut())
    {
        cue.settings.duration = Some(DurationSpec::Beats(8));
    }
    clock_for(&playlist)
}

/// Ticks until cue 1 starts; returns when.
fn time_of_second_cue(clock: &mut PlaylistClock, grid: Grid, limit: f64) -> Option<f64> {
    let mut now = 0.0;
    let events = run(clock, &mut now, limit, grid);
    events
        .iter()
        .find_map(|(t, e)| matches!(e, ClockEvent::CueStarted { index: 1, .. }).then_some(*t))
}

#[test]
fn a_beat_cue_waits_for_the_next_downbeat_after_its_computed_end() -> Result<()> {
    // The grid is 1 beat into a bar when the cue starts. 8 beats at 120 BPM
    // is 4 s, when the grid reads 9 (one beat into bar 2); bar 3 begins at
    // beat 12, 1.5 s later.
    let grid = Grid {
        start: 1.0,
        bpm: 120.0,
        confidence: 0.9,
    };
    let ended = time_of_second_cue(&mut beat_show(), grid, 10.0)
        .ok_or_else(|| anyhow!("the cue never ended"))?;
    assert!(
        (5.5 - 1e-9..=5.5 + DT + 1e-9).contains(&ended),
        "should end on the downbeat at 5.5 s, ended at {ended}"
    );
    Ok(())
}

#[test]
fn a_downbeat_exactly_at_the_computed_end_ends_the_cue_there() -> Result<()> {
    let grid = Grid {
        start: 0.0,
        bpm: 120.0,
        confidence: 0.9,
    };
    let ended = time_of_second_cue(&mut beat_show(), grid, 10.0)
        .ok_or_else(|| anyhow!("the cue never ended"))?;
    assert!(
        (4.0 - 1e-9..=4.0 + DT + 1e-9).contains(&ended),
        "8 beats from a downbeat is a downbeat; ended at {ended}"
    );
    Ok(())
}

#[test]
fn a_known_downbeat_moves_the_bar_line_the_cue_ends_on() -> Result<()> {
    // As the test above (grid 0 at the start, 8 beats end at 4 s on grid
    // beat 8), but bars start on grid beats 4n + 1: the cue waits for
    // beat 9, half a second later.
    let mut clock = beat_show();
    let mut now = 0.0;
    let mut ended = None;
    while now < 10.0 && ended.is_none() {
        now += DT;
        let beat = BeatInfo {
            bpm: 120.0,
            confidence: 0.9,
            beats: now * 2.0,
            downbeat: 1.0,
        };
        clock.tick(DT, &beat);
        ended = clock
            .drain_events()
            .find(|e| matches!(e, ClockEvent::CueStarted { index: 1, .. }))
            .map(|_| now);
    }
    let ended = ended.ok_or_else(|| anyhow!("the cue never ended"))?;
    assert!(
        (4.5 - 1e-9..=4.5 + DT + 1e-9).contains(&ended),
        "should end on the shifted downbeat at 4.5 s, ended at {ended}"
    );
    Ok(())
}

#[test]
fn without_confidence_a_beat_cue_ends_on_time_at_the_fallback_tempo() -> Result<()> {
    let grid = Grid {
        start: 1.0,
        bpm: 120.0,
        confidence: 0.2,
    };
    let ended = time_of_second_cue(&mut beat_show(), grid, 10.0)
        .ok_or_else(|| anyhow!("the cue never ended"))?;
    assert!(
        approx(ended, 4.0, DT + 1e-9),
        "8 beats at 120 BPM, no waiting: {ended}"
    );
    Ok(())
}

#[test]
fn the_live_tempo_sets_the_length_when_confident() -> Result<()> {
    // 8 beats at 150 BPM is 3.2 s; the grid starts on a downbeat, 3.2 s is
    // 8 beats, again a downbeat.
    let grid = Grid {
        start: 0.0,
        bpm: 150.0,
        confidence: 0.9,
    };
    let ended = time_of_second_cue(&mut beat_show(), grid, 10.0)
        .ok_or_else(|| anyhow!("the cue never ended"))?;
    assert!(approx(ended, 3.2, DT + 1e-9), "8 beats at 150 BPM: {ended}");
    Ok(())
}

#[test]
fn a_stalled_grid_cannot_strand_the_show() -> Result<()> {
    // The grid reads confident but never moves, so no downbeat ever comes.
    // The wait is abandoned after six beats (3 s at 120 BPM).
    let frozen = |t: f64| BeatInfo {
        bpm: 120.0,
        confidence: 0.9,
        beats: 1.0 + 0.0 * t,
        downbeat: 0.0,
    };
    let mut clock = beat_show();
    let mut now = 0.0;
    let mut ended = None;
    for _ in 0..400 {
        now += DT;
        clock.tick(DT, &frozen(now));
        if clock
            .drain_events()
            .any(|e| matches!(e, ClockEvent::CueStarted { index: 1, .. }))
        {
            ended = Some(now);
            break;
        }
    }
    let ended = ended.ok_or_else(|| anyhow!("the show stalled"))?;
    assert!(
        approx(ended, 7.0, 2.0 * DT),
        "4 s + 3 s timeout, ended at {ended}"
    );
    Ok(())
}

#[test]
fn losing_confidence_mid_wait_ends_the_cue_at_once() -> Result<()> {
    let mut clock = beat_show();
    let mut now = 0.0;
    let steady = Grid {
        start: 1.0,
        bpm: 120.0,
        confidence: 0.9,
    };
    run(&mut clock, &mut now, 4.5, steady);
    assert_eq!(current(&clock)?, 0, "waiting for the downbeat at 5.5 s");
    let lost = Grid {
        confidence: 0.0,
        ..steady
    };
    run(&mut clock, &mut now, 0.1, lost);
    assert_eq!(current(&clock)?, 1, "the wait is abandoned");
    Ok(())
}

#[test]
fn extending_a_waiting_cue_cancels_the_wait() -> Result<()> {
    let mut clock = beat_show();
    let mut now = 0.0;
    let steady = Grid {
        start: 1.0,
        bpm: 120.0,
        confidence: 0.9,
    };
    run(&mut clock, &mut now, 4.5, steady);
    assert!(clock.extend(10.0), "extend");
    run(&mut clock, &mut now, 8.0, steady);
    assert_eq!(current(&clock)?, 0, "the cue plays its extra ten seconds");
    Ok(())
}

#[test]
fn seconds_cues_ignore_the_beat() -> Result<()> {
    let mut clock = cuts(&[4.0, 4.0]);
    let grid = Grid {
        start: 1.0,
        bpm: 120.0,
        confidence: 1.0,
    };
    let ended = time_of_second_cue(&mut clock, grid, 10.0).ok_or_else(|| anyhow!("never ended"))?;
    assert!(
        approx(ended, 4.0, DT + 1e-9),
        "a seconds cue ends on time even on a confident grid: {ended}"
    );
    Ok(())
}

#[test]
fn remaining_reads_zero_while_waiting_for_the_downbeat() -> Result<()> {
    let mut clock = beat_show();
    let mut now = 0.0;
    run(
        &mut clock,
        &mut now,
        4.5,
        Grid {
            start: 1.0,
            bpm: 120.0,
            confidence: 0.9,
        },
    );
    assert_eq!(
        clock.remaining(),
        Some(0.0),
        "past the computed end, waiting"
    );
    Ok(())
}

// ---- resume and the wall clock ------------------------------------------------

#[test]
fn a_resume_point_restores_the_same_cue_and_offset() -> Result<()> {
    let playlist = show(
        &[30.0, 30.0, 30.0],
        TransitionSpec::crossfade(2.0),
        LoopMode::Loop,
    );
    let mut clock = clock_for(&playlist);
    let mut now = 0.0;
    run(&mut clock, &mut now, 100.0, Grid::SILENT);
    let point = clock
        .resume_point()
        .ok_or_else(|| anyhow!("no resume point while playing"))?;
    // 100 s of a 90 s loop: pass 1, 10 s into its first cue.
    assert_eq!(
        (point.section, point.cue, point.pass),
        (0, 0, 1),
        "authored position"
    );
    assert!(
        approx(point.offset, 10.0, DT + 1e-9),
        "offset {}",
        point.offset
    );

    let mut restored = clock_for(&playlist);
    restored.restore(&point)?;
    assert_eq!(current(&restored)?, 0, "same cue");
    assert!(
        approx(restored.elapsed(), point.offset, 1e-9),
        "same offset"
    );
    assert_eq!(restored.pass(), 1, "same pass");
    assert!(restored.transition().is_none(), "a restore cuts in");
    Ok(())
}

#[test]
fn resume_uses_authored_positions_so_disabled_cues_do_not_shift_it() -> Result<()> {
    let mut playlist = show(&[10.0, 10.0, 10.0], TransitionSpec::cut(), LoopMode::Stop);
    if let Some(cue) = playlist
        .sections
        .first_mut()
        .and_then(|s| s.cues.first_mut())
    {
        cue.enabled = false;
    }
    let mut clock = clock_for(&playlist);
    clock.tick(0.0, &BeatInfo::SILENT);
    clock.next_cue();
    let point = clock.resume_point().ok_or_else(|| anyhow!("no point"))?;
    assert_eq!(
        (point.section, point.cue),
        (0, 2),
        "timeline cue 1 is authored cue 2"
    );
    let mut other = clock_for(&playlist);
    other.restore(&point)?;
    assert_eq!(current(&other)?, 1, "and maps back to timeline cue 1");
    Ok(())
}

#[test]
fn a_point_from_another_playlist_is_refused() -> Result<()> {
    let mut clock = cuts(&[10.0, 10.0]);
    clock.tick(0.0, &BeatInfo::SILENT);
    let point = clock.resume_point().ok_or_else(|| anyhow!("no point"))?;
    let mut other = show(&[10.0, 10.0, 10.0], TransitionSpec::cut(), LoopMode::Stop);
    other.name = "different".into();
    let mut target = clock_for(&other);
    assert!(
        matches!(
            target.restore(&point),
            Err(ResumeError::IdentityMismatch { .. })
        ),
        "identity differs"
    );
    assert_eq!(target.state(), ClockState::Idle, "and nothing was changed");
    target.restore_lenient(&point)?;
    assert_eq!(
        current(&target)?,
        0,
        "unless the caller insists (a hot reload)"
    );
    Ok(())
}

#[test]
fn a_point_beyond_the_playlist_or_on_failed_cues_is_handled() -> Result<()> {
    let mut clock = cuts(&[10.0, 10.0]);
    let identity = clock.timeline().identity();
    let beyond = ResumePoint {
        identity,
        section: 0,
        cue: 9,
        pass: 0,
        offset: 0.0,
    };
    assert_eq!(
        clock.restore(&beyond),
        Err(ResumeError::CueMissing { section: 0, cue: 9 }),
        "beyond"
    );

    clock.mark_failed(0);
    let on_failed = ResumePoint {
        identity,
        section: 0,
        cue: 0,
        pass: 0,
        offset: 5.0,
    };
    clock.restore(&on_failed)?;
    assert_eq!(current(&clock)?, 1, "resumes at the next playable cue");
    assert_eq!(clock.elapsed(), 0.0, "from its start");

    clock.mark_failed(1);
    let mut stuck = cuts(&[10.0]);
    stuck.mark_failed(0);
    let point = ResumePoint {
        identity: stuck.timeline().identity(),
        section: 0,
        cue: 0,
        pass: 0,
        offset: 0.0,
    };
    assert_eq!(
        stuck.restore(&point),
        Err(ResumeError::NothingPlayable),
        "nothing playable"
    );
    Ok(())
}

#[test]
fn a_restored_offset_never_runs_past_the_end() -> Result<()> {
    let mut clock = cuts(&[10.0, 10.0]);
    let identity = clock.timeline().identity();
    let point = ResumePoint {
        identity,
        section: 0,
        cue: 0,
        pass: 0,
        offset: 500.0,
    };
    clock.restore(&point)?;
    assert_eq!(current(&clock)?, 0, "still on the cue it was restored to");
    assert!(
        clock.remaining().is_some_and(|r| r > 0.0),
        "with a moment left to play"
    );
    Ok(())
}

fn anchored(durations: &[f64], loop_mode: LoopMode) -> PlaylistClock {
    let mut playlist = show(durations, TransitionSpec::cut(), loop_mode);
    playlist.anchor = WallClockAnchor::from_hm(9, 0);
    clock_for(&playlist)
}

#[test]
fn a_late_start_jumps_to_the_cue_that_should_be_playing() -> Result<()> {
    let mut clock = anchored(&[100.0, 100.0, 100.0], LoopMode::Stop);
    let nine = 9.0 * 3600.0;
    let placement = clock.sync_to_wall_clock(nine + 250.0);
    assert_eq!(
        placement,
        Some(Placement::Playing {
            pass: 0,
            index: 2,
            offset: 50.0
        }),
        "250 s after 09:00"
    );
    assert_eq!(current(&clock)?, 2, "the clock followed");
    assert!(approx(clock.elapsed(), 50.0, 1e-9), "50 s in");
    Ok(())
}

#[test]
fn before_the_anchor_the_clock_waits_and_after_the_end_it_finishes() -> Result<()> {
    let nine = 9.0 * 3600.0;
    let mut early = anchored(&[100.0], LoopMode::Stop);
    assert_eq!(
        early.sync_to_wall_clock(nine - 60.0),
        Some(Placement::Waiting {
            seconds_until_start: 60.0
        }),
        "a minute early"
    );
    assert_eq!(early.state(), ClockState::Idle, "nothing started");

    let mut late = anchored(&[100.0], LoopMode::Stop);
    late.sync_to_wall_clock(nine + 500.0);
    assert_eq!(late.state(), ClockState::Ended, "the show is over");

    let mut looped = anchored(&[100.0, 100.0], LoopMode::Loop);
    looped.sync_to_wall_clock(nine + 1050.0);
    assert_eq!(
        (current(&looped)?, looped.pass()),
        (0, 5),
        "a loop catches up to pass 5"
    );

    let mut unanchored = cuts(&[10.0]);
    assert_eq!(
        unanchored.sync_to_wall_clock(1000.0),
        None,
        "no anchor, nothing to sync"
    );
    Ok(())
}

#[test]
fn a_show_that_runs_past_midnight_picks_up_after_midnight() -> Result<()> {
    // Four hours from 23:00, and Strobe restarted (a crash, a reboot) at
    // 00:15: the show is 75 minutes in, not 22 h 45 min from starting.
    let mut playlist = show(&[3600.0; 4], TransitionSpec::cut(), LoopMode::Stop);
    playlist.anchor = WallClockAnchor::from_hm(23, 0);
    let mut clock = clock_for(&playlist);
    let quarter_past_midnight = 15.0 * 60.0;
    assert_eq!(
        clock.sync_to_wall_clock(quarter_past_midnight),
        Some(Placement::Playing {
            pass: 0,
            index: 1,
            offset: 900.0
        }),
        "yesterday's 23:00 show is still on at 00:15"
    );
    assert_eq!(current(&clock)?, 1, "the clock followed");

    // Once that run is over, the next anchor is waited for as before.
    let mut after = clock_for(&playlist);
    assert_eq!(
        after.sync_to_wall_clock(4.0 * 3600.0),
        Some(Placement::Waiting {
            seconds_until_start: 19.0 * 3600.0
        }),
        "at 04:00 the 23:00 show is over and the next one is waited for"
    );
    Ok(())
}

#[test]
fn syncing_to_a_failed_cue_moves_to_the_next_playable_one() -> Result<()> {
    let mut clock = anchored(&[100.0, 100.0, 100.0], LoopMode::Stop);
    clock.mark_failed(1);
    clock.sync_to_wall_clock(9.0 * 3600.0 + 150.0);
    assert_eq!(current(&clock)?, 2, "cue 1 should be playing but cannot");
    assert_eq!(clock.elapsed(), 0.0, "so cue 2 starts from its beginning");
    Ok(())
}

// ---- the event queue -------------------------------------------------------------

#[test]
fn draining_empties_the_queue_and_keeps_its_storage() -> Result<()> {
    let mut clock = cuts(&[1.0, 1.0, 1.0]);
    let capacity = clock.event_capacity();
    assert!(
        capacity >= MAX_PENDING_EVENTS,
        "the queue is reserved up front: {capacity}"
    );
    let mut now = 0.0;
    for _ in 0..50 {
        run(&mut clock, &mut now, 1.0, Grid::SILENT);
        clock.jump_to(0);
    }
    assert!(!clock.events().is_empty(), "undrained events accumulate");
    assert!(clock.drain_events().count() > 0, "the app drains them");
    assert!(clock.events().is_empty(), "now empty");
    assert_eq!(
        clock.event_capacity(),
        capacity,
        "the queue never reallocated"
    );
    Ok(())
}

#[test]
fn an_undrained_queue_drops_events_instead_of_growing() -> Result<()> {
    let mut clock = clock_for(&show(&[0.5], TransitionSpec::cut(), LoopMode::Loop));
    let capacity = clock.event_capacity();
    clock.tick(0.0, &BeatInfo::SILENT);
    for _ in 0..4000 {
        clock.tick(0.5, &BeatInfo::SILENT);
    }
    assert_eq!(
        clock.events().len(),
        MAX_PENDING_EVENTS,
        "the queue is full"
    );
    assert!(clock.dropped_events() > 0, "and the overflow is counted");
    assert_eq!(clock.event_capacity(), capacity, "without growing");
    Ok(())
}

#[test]
fn an_empty_playlist_ends_immediately() -> Result<()> {
    let mut clock = clock_for(&Playlist::new("empty"));
    clock.tick(0.1, &BeatInfo::SILENT);
    assert_eq!(clock.state(), ClockState::Ended, "nothing to play");
    assert_eq!(
        clock.drain_events().collect::<Vec<_>>(),
        vec![ClockEvent::PlaylistEnded],
        "and it says so"
    );
    Ok(())
}

#[test]
fn bad_dt_values_are_ignored() -> Result<()> {
    let mut clock = cuts(&[10.0]);
    clock.tick(0.0, &BeatInfo::SILENT);
    for dt in [f64::NAN, f64::INFINITY, -5.0, f64::NEG_INFINITY] {
        clock.tick(dt, &BeatInfo::SILENT);
    }
    assert_eq!(clock.elapsed(), 0.0, "no time passed");
    Ok(())
}

// ---- invariants under random commands ------------------------------------------------

#[test]
fn invariants_hold_under_a_random_command_stream() -> Result<()> {
    use crate::rng::SplitMix64;
    for seed in 0..30u64 {
        let mut rng = SplitMix64::new(seed);
        let mut playlist = show(
            &[3.0, 4.0, 2.0, 6.0, 1.0, 5.0],
            TransitionSpec::crossfade(1.5),
            if seed % 3 == 0 {
                LoopMode::Stop
            } else {
                LoopMode::Loop
            },
        );
        if let Some(section) = playlist.sections.first_mut() {
            if let Some(cue) = section.cues.get_mut(2) {
                cue.settings.advance = Some(AdvanceMode::Manual);
            }
            if let Some(cue) = section.cues.get_mut(4) {
                cue.settings.duration = Some(DurationSpec::Beats(6));
            }
        }
        let mut clock = clock_for(&playlist);
        let count = clock.timeline().len();
        let mut open: Vec<u64> = Vec::new();
        let mut ever_started: Vec<u64> = Vec::new();
        let mut now = 0.0;
        for step in 0..4000 {
            now += DT;
            let grid = Grid {
                start: 0.0,
                bpm: 128.0,
                confidence: if rng.below(4) == 0 { 0.1 } else { 0.9 },
            };
            if rng.below(40) == 0 {
                let command = match rng.below(11) {
                    0 => Command::Next,
                    1 => Command::Previous,
                    2 => Command::Hold,
                    3 => Command::Unhold,
                    4 => Command::Extend(rng.range(-5.0, 20.0)),
                    5 => Command::RestartCue,
                    6 => Command::JumpTo(rng.below(8) as usize),
                    7 => Command::SkipSection,
                    8 => Command::ToggleBlackout,
                    9 => Command::AudioEvent(AudioEventKind::Drop),
                    _ => Command::ToggleHold,
                };
                clock.apply(command);
            }
            if rng.below(300) == 0 {
                clock.mark_failed(rng.below(count as u64) as usize);
            }
            clock.tick(DT, &grid.at(now));
            for event in clock.drain_events() {
                match event {
                    ClockEvent::TransitionStarted { id, from, to } => {
                        assert!(
                            !ever_started.contains(&id),
                            "seed {seed} step {step}: transition {id} started twice"
                        );
                        assert!(
                            from < count && to < count && from != to,
                            "seed {seed}: bad transition {from}->{to}"
                        );
                        ever_started.push(id);
                        open.push(id);
                    }
                    ClockEvent::TransitionFinished { id, .. } => {
                        let at = open.iter().position(|o| *o == id);
                        assert!(
                            at.is_some(),
                            "seed {seed} step {step}: transition {id} finished without being open"
                        );
                        if let Some(at) = at {
                            open.remove(at);
                        }
                    }
                    ClockEvent::CueStarted { index, .. } | ClockEvent::WarmUp { index } => {
                        assert!(
                            index < count,
                            "seed {seed}: event for cue {index} of {count}"
                        );
                    }
                    ClockEvent::CueSkipped { .. }
                    | ClockEvent::HoldChanged(_)
                    | ClockEvent::BlackoutChanged(_)
                    | ClockEvent::PlaylistEnded => {}
                }
            }
            if let Some(index) = clock.current() {
                assert!(index < count, "seed {seed}: current {index} out of range");
                assert!(
                    !clock.is_failed(index),
                    "seed {seed} step {step}: playing failed cue {index}"
                );
                assert!(clock.elapsed() >= 0.0, "seed {seed}: negative elapsed");
                if let Some(remaining) = clock.remaining() {
                    assert!(remaining >= 0.0, "seed {seed}: negative remaining");
                }
            }
            if let Some(t) = clock.transition() {
                assert!(
                    (0.0..=1.0).contains(&t.progress),
                    "seed {seed}: progress {}",
                    t.progress
                );
                assert_eq!(
                    open.last(),
                    Some(&t.id),
                    "seed {seed}: the live transition is the open one"
                );
                assert_eq!(
                    Some(t.to),
                    clock.current(),
                    "seed {seed}: arriving at a cue that is not current"
                );
            } else {
                assert!(
                    open.is_empty(),
                    "seed {seed} step {step}: {open:?} still open with no transition"
                );
            }
        }
    }
    Ok(())
}

#[test]
fn transition_kinds_are_read_from_the_timeline_not_the_events() -> Result<()> {
    // Events carry indices only; the app looks the effect up.
    let mut playlist = show(&[5.0, 5.0], TransitionSpec::crossfade(1.0), LoopMode::Stop);
    if let Some(cue) = playlist
        .sections
        .first_mut()
        .and_then(|s| s.cues.get_mut(1))
    {
        cue.settings.transition = Some(TransitionSpec {
            kind: TransitionKind::FadeThroughBlack,
            duration: sec(1.0),
        });
    }
    let mut clock = clock_for(&playlist);
    let mut now = 0.0;
    run(&mut clock, &mut now, 5.2, Grid::SILENT);
    let t = clock.transition().ok_or_else(|| anyhow!("no transition"))?;
    let kind = clock
        .timeline()
        .cues()
        .get(t.to)
        .map(|cue| cue.resolved.transition.kind.clone());
    assert_eq!(
        kind,
        Some(TransitionKind::FadeThroughBlack),
        "the incoming cue's own effect"
    );
    Ok(())
}
