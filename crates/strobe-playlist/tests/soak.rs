// ============================================================================
// Module:       soak (integration test)
// Description:  Four simulated hours of playback at 60 Hz: a plain run
//               checked against the static timeline, and a run under a
//               seeded random command stream checked for invariants.
//
// Dependencies: strobe_playlist (public API only), anyhow
// ============================================================================

//! The playlist soak test.
//!
//! PLAN Phase 5's acceptance asks for "a 4-hour soak with zero dropped
//! transitions". The clock is pure, so the soak runs in simulated time: 4
//! hours is 864,000 ticks of 1/60 s, which takes a fraction of a second even
//! in a debug build, and every property can be checked on every tick.
//!
//! Two runs:
//!
//! 1. **Undisturbed.** An 80-cue, 4-hour, seconds-only playlist with no
//!    commands. Every cue must start when the static timeline says (within
//!    one frame), every non-cut transition must start and complete exactly
//!    once, and the playlist must end on time.
//! 2. **Disturbed.** A generated 4-hour playlist (random durations, bar
//!    lengths, a manual cue, three kinds of transition, a looping end) under
//!    a seeded random stream of live commands, failures and wobbling beat
//!    confidence. After every tick the invariants are checked: the active
//!    cue exists and is not failed, at most one transition is open, no
//!    transition id is ever reused or finished twice, progress stays in
//!    0..1, and the event queue never grows past its reservation.

use anyhow::{Result, anyhow};
use std::collections::HashSet;
use std::time::Instant;
use strobe_playlist::rng::SplitMix64;
use strobe_playlist::{
    AdvanceMode, BeatInfo, ClockConfig, ClockEvent, ClockState, Command, Cue, DurationSpec,
    GenerateSpec, LibraryScene, LibrarySnapshot, LoopMode, Playlist, PlaylistClock, SceneRef,
    Section, Timeline, TimelineConfig, TransitionKind, TransitionSpec, generate,
};

/// Frame time of the simulated app.
const DT: f64 = 1.0 / 60.0;

/// Four hours.
const FOUR_HOURS: f64 = 4.0 * 3600.0;

/// A generous bound for the whole soak in a debug build; the real figure is
/// a small fraction of it.
const BUDGET_SECONDS: f64 = 5.0;

fn library(count: usize) -> LibrarySnapshot {
    LibrarySnapshot {
        scenes: (0..count)
            .map(|i| LibraryScene::new(&format!("scene-{i:02}"), &["soak"]))
            .collect(),
    }
}

/// 80 cues of exactly 180 s with a mix of transitions, no randomness.
fn plain_playlist() -> Result<Playlist> {
    let mut spec = GenerateSpec::new("Plain", FOUR_HOURS, 2024);
    spec.duration = DurationSpec::Seconds(180.0);
    spec.transitions = vec![
        TransitionSpec::crossfade(4.0),
        TransitionSpec::cut(),
        TransitionSpec {
            kind: TransitionKind::FadeThroughBlack,
            duration: DurationSpec::Seconds(2.0),
        },
    ];
    let generated = generate(&library(30), &spec)?;
    let mut playlist = Playlist::new("plain soak");
    playlist.sections.push(generated.section);
    Ok(playlist)
}

/// A generated, looping playlist with the awkward cue kinds mixed in.
fn disturbed_playlist(seed: u64) -> Result<Playlist> {
    let mut spec = GenerateSpec::new("Generated", FOUR_HOURS, seed);
    spec.duration = DurationSpec::Random {
        min: Box::new(DurationSpec::Seconds(20.0)),
        max: Box::new(DurationSpec::Seconds(240.0)),
    };
    spec.transitions = vec![
        TransitionSpec::crossfade(5.0),
        TransitionSpec::cut(),
        TransitionSpec {
            kind: TransitionKind::FadeThroughBlack,
            duration: DurationSpec::Beats(8),
        },
    ];
    let generated = generate(&library(25), &spec)?;
    let mut section = generated.section;
    for (i, cue) in section.cues.iter_mut().enumerate() {
        match i % 7 {
            3 => cue.settings.duration = Some(DurationSpec::Bars(16)),
            5 => cue.settings.advance = Some(AdvanceMode::Manual),
            _ => {}
        }
    }
    let mut tail = Section::new("Tail");
    tail.cues
        .push(Cue::new(SceneRef::Library("scene-00".into())));
    let mut playlist = Playlist::new("disturbed soak");
    playlist.loop_mode = LoopMode::Loop;
    playlist.sections.push(section);
    playlist.sections.push(tail);
    Ok(playlist)
}

fn clock_for(playlist: &Playlist) -> PlaylistClock {
    PlaylistClock::new(
        Timeline::from_playlist(playlist, &TimelineConfig::default()),
        ClockConfig::default(),
    )
}

#[test]
fn four_undisturbed_hours_follow_the_timeline_exactly() -> Result<()> {
    let playlist = plain_playlist()?;
    let timeline = Timeline::from_playlist(&playlist, &TimelineConfig::default());
    assert_eq!(timeline.len(), 80, "4 h of 3-minute cues is 80 cues");
    assert!(
        (timeline.total_runtime() - FOUR_HOURS).abs() < 1e-6,
        "the timeline is 4 h long"
    );

    let expected_transitions = timeline
        .cues()
        .iter()
        .enumerate()
        .filter(|(i, cue)| *i > 0 && !matches!(cue.resolved.transition.kind, TransitionKind::Cut))
        .count();

    let mut clock = PlaylistClock::new(timeline.clone(), ClockConfig::default());
    let started_at = Instant::now();
    let mut now = 0.0;
    let mut cue_starts: Vec<(usize, f64)> = Vec::new();
    let mut transitions_started = 0;
    let mut transitions_completed = 0;
    let mut warmed: Vec<usize> = Vec::new();
    let mut ended_at = None;
    while now < FOUR_HOURS + 60.0 {
        now += DT;
        clock.tick(DT, &BeatInfo::SILENT);
        for event in clock.drain_events() {
            match event {
                ClockEvent::CueStarted { index, .. } => cue_starts.push((index, now)),
                ClockEvent::TransitionStarted { .. } => transitions_started += 1,
                ClockEvent::TransitionFinished { completed, .. } => {
                    assert!(completed, "no command interrupted anything at {now}");
                    transitions_completed += 1;
                }
                ClockEvent::PlaylistEnded => ended_at = Some(now),
                ClockEvent::WarmUp { index } => warmed.push(index),
                ClockEvent::CueSkipped { .. }
                | ClockEvent::HoldChanged(_)
                | ClockEvent::BlackoutChanged(_) => {}
            }
        }
    }
    let took = started_at.elapsed().as_secs_f64();
    eprintln!(
        "undisturbed 4 h soak: {} ticks in {took:.3} s",
        (now / DT).round()
    );

    assert_eq!(cue_starts.len(), 80, "all 80 cues started, once each");
    for (position, (index, at)) in cue_starts.iter().enumerate() {
        assert_eq!(*index, position, "cues start in order");
        let planned = timeline
            .cues()
            .get(position)
            .map(|c| c.start)
            .unwrap_or(f64::NAN);
        assert!(
            (at - planned).abs() <= DT + 1e-6,
            "cue {position} started at {at}, the timeline says {planned}"
        );
    }
    assert_eq!(
        transitions_started, expected_transitions,
        "no transition was dropped"
    );
    assert_eq!(
        transitions_completed, expected_transitions,
        "and each one finished"
    );
    assert_eq!(
        warmed,
        (1..80).collect::<Vec<usize>>(),
        "every cue but the first was warmed up exactly once, in order, before it started"
    );
    let ended = ended_at.ok_or_else(|| anyhow!("the playlist never ended"))?;
    assert!(
        (ended - FOUR_HOURS).abs() <= DT + 1e-6,
        "ended at {ended}, not {FOUR_HOURS}"
    );
    assert_eq!(clock.state(), ClockState::Ended, "and stays ended");
    assert_eq!(clock.dropped_events(), 0, "no event was lost");
    assert!(took < BUDGET_SECONDS, "the soak took {took:.2} s");
    Ok(())
}

/// Everything the disturbed run counts.
#[derive(Default)]
struct Tally {
    ticks: u64,
    commands: u64,
    cue_starts: u64,
    skips: u64,
    transitions_started: u64,
    transitions_completed: u64,
    transitions_cut_short: u64,
    warmups: u64,
}

fn disturbed_run(seed: u64) -> Result<(Tally, f64)> {
    let playlist = disturbed_playlist(seed)?;
    let mut clock = clock_for(&playlist);
    let count = clock.timeline().len();
    let capacity = clock.event_capacity();
    let mut rng = SplitMix64::new(seed ^ 0xA5A5);

    let mut tally = Tally::default();
    let mut seen_ids: HashSet<u64> = HashSet::new();
    let mut open: Option<u64> = None;
    let mut last_id: Option<u64> = None;
    let mut now = 0.0;
    let mut grid_start = 0.0;
    let mut confidence = 0.9f32;
    let mut failed = 0;
    // Whether this cue was lengthened or shortened by hand since it began:
    // a cue shortened below its elapsed time legitimately waits for a downbeat.
    let mut extended = false;
    let started_at = Instant::now();
    // The app reuses one buffer the same way.
    let mut events: Vec<ClockEvent> = Vec::with_capacity(64);

    while now < FOUR_HOURS {
        now += DT;
        tally.ticks += 1;

        // The music: a steady 124 BPM grid whose confidence wobbles, with
        // the occasional restart of the grid.
        if rng.below(1800) == 0 {
            confidence = if rng.below(3) == 0 { 0.1 } else { 0.9 };
        }
        if rng.below(20_000) == 0 {
            grid_start = -now * 124.0 / 60.0;
        }
        let beat = BeatInfo {
            bpm: 124.0,
            confidence,
            beats: grid_start + now * 124.0 / 60.0,
        };

        // Live controls, roughly one every ten seconds.
        if rng.below(600) == 0 {
            let command = match rng.below(12) {
                0 | 1 => Command::Next,
                2 => Command::Previous,
                3 => Command::Hold,
                4 | 5 => Command::Unhold,
                6 => Command::Extend(rng.range(-30.0, 60.0)),
                7 => Command::RestartCue,
                8 => Command::JumpTo(rng.below(count as u64 + 3) as usize),
                9 => Command::SkipSection,
                10 => Command::ToggleBlackout,
                _ => Command::ToggleHold,
            };
            extended |= matches!(command, Command::Extend(_));
            clock.apply(command);
            tally.commands += 1;
        }
        // A scene "fails to compile" now and then, but never most of them.
        if failed < count / 4 && rng.below(9000) == 0 {
            clock.mark_failed(rng.below(count as u64) as usize);
            failed += 1;
        }
        // A person always un-holds eventually, or the show would freeze.
        if clock.is_held() && rng.below(1200) == 0 {
            clock.unhold();
        }

        clock.tick(DT, &beat);

        events.clear();
        events.extend(clock.drain_events());
        for event in events.drain(..) {
            match event {
                ClockEvent::CueStarted { index, .. } => {
                    assert!(
                        index < count,
                        "seed {seed}: cue {index} of {count} at {now}"
                    );
                    assert!(
                        !clock.is_failed(index),
                        "seed {seed}: started failed cue {index} at {now}"
                    );
                    tally.cue_starts += 1;
                    extended = false;
                }
                ClockEvent::TransitionStarted { id, from, to } => {
                    assert!(
                        open.is_none(),
                        "seed {seed}: transition {id} started while {open:?} was open at {now}"
                    );
                    assert!(
                        seen_ids.insert(id),
                        "seed {seed}: transition id {id} reused at {now}"
                    );
                    assert!(
                        last_id.is_none_or(|last| id > last),
                        "seed {seed}: ids went backwards at {now}"
                    );
                    assert!(
                        from < count && to < count && from != to,
                        "seed {seed}: transition {from}->{to}"
                    );
                    open = Some(id);
                    last_id = Some(id);
                    tally.transitions_started += 1;
                }
                ClockEvent::TransitionFinished { id, completed } => {
                    assert_eq!(
                        open,
                        Some(id),
                        "seed {seed}: transition {id} finished but {open:?} was open at {now}"
                    );
                    open = None;
                    if completed {
                        tally.transitions_completed += 1;
                    } else {
                        tally.transitions_cut_short += 1;
                    }
                }
                ClockEvent::WarmUp { index } => {
                    assert!(index < count, "seed {seed}: warm-up for cue {index}");
                    tally.warmups += 1;
                }
                ClockEvent::CueSkipped { index, .. } => {
                    assert!(
                        clock.is_failed(index),
                        "seed {seed}: skipped cue {index} that had not failed"
                    );
                    tally.skips += 1;
                }
                ClockEvent::HoldChanged(_) | ClockEvent::BlackoutChanged(_) => {}
                ClockEvent::PlaylistEnded => {
                    return Err(anyhow!("seed {seed}: a looping playlist ended at {now}"));
                }
            }
        }

        let index = clock
            .current()
            .ok_or_else(|| anyhow!("seed {seed}: not playing at {now}"))?;
        assert!(
            index < count,
            "seed {seed}: current {index} out of range at {now}"
        );
        assert!(
            !clock.is_failed(index),
            "seed {seed}: playing failed cue {index} at {now}"
        );
        assert!(
            clock.elapsed() >= 0.0,
            "seed {seed}: negative elapsed at {now}"
        );
        if let Some(remaining) = clock.remaining() {
            assert!(remaining >= 0.0, "seed {seed}: negative remaining at {now}");
            // A timed cue is never left running far past its end: the
            // longest legitimate wait is a downbeat timeout (6 beats at the
            // slowest tempo the analysis allows, about 7.2 s), unless held.
            if !clock.is_held() && !extended {
                assert!(
                    clock.elapsed() <= clock.cue_length() + 8.0,
                    "seed {seed}: cue {index} overran: {} of {} at {now}",
                    clock.elapsed(),
                    clock.cue_length()
                );
            }
        }
        match clock.transition() {
            Some(t) => {
                assert_eq!(
                    open,
                    Some(t.id),
                    "seed {seed}: transition state disagrees with events at {now}"
                );
                assert!(
                    (0.0..=1.0).contains(&t.progress),
                    "seed {seed}: progress {}",
                    t.progress
                );
                assert_eq!(t.to, index, "seed {seed}: arriving somewhere else at {now}");
            }
            None => assert!(open.is_none(), "seed {seed}: {open:?} still open at {now}"),
        }
        assert_eq!(
            clock.event_capacity(),
            capacity,
            "seed {seed}: the event queue reallocated"
        );
    }

    let took = started_at.elapsed().as_secs_f64();
    assert_eq!(
        clock.dropped_events(),
        0,
        "seed {seed}: events were dropped"
    );
    Ok((tally, took))
}

#[test]
fn four_disturbed_hours_keep_every_invariant() -> Result<()> {
    let mut total_took = 0.0;
    for seed in [1u64, 2, 3] {
        let (tally, took) = disturbed_run(seed)?;
        eprintln!(
            "disturbed 4 h soak, seed {seed}: {} ticks, {} commands, {} cue starts, {} skips, \
             {} transitions ({} completed, {} cut short), {} warm-ups, in {took:.3} s",
            tally.ticks,
            tally.commands,
            tally.cue_starts,
            tally.skips,
            tally.transitions_started,
            tally.transitions_completed,
            tally.transitions_cut_short,
            tally.warmups,
        );
        let expected_ticks = (FOUR_HOURS / DT).round() as u64;
        assert!(
            tally.ticks.abs_diff(expected_ticks) <= 2,
            "four hours of ticks (give or take rounding): {} vs {expected_ticks}",
            tally.ticks
        );
        assert!(
            tally.commands > 500,
            "the stream really exercised the clock: {} commands",
            tally.commands
        );
        assert!(
            tally.transitions_started > 100,
            "and transitions: {}",
            tally.transitions_started
        );
        assert!(
            tally.transitions_cut_short > 0,
            "some transitions were interrupted"
        );
        assert!(tally.skips > 0, "some failed cues were passed over");
        total_took += took;
    }
    assert!(
        total_took < BUDGET_SECONDS,
        "three soaks took {total_took:.2} s"
    );
    Ok(())
}
