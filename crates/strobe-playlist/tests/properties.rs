// ============================================================================
// Module:       properties (integration test)
// Description:  Property tests for the playlist grammar: any playlist text
//               and any duration text parses or is refused without a panic.
//
// Dependencies: anyhow, proptest; strobe-playlist
// ============================================================================

//! Playlists are typed by people and shared between machines (D539).
//!
//! Each property feeds the parser generated text, arbitrary and shaped like
//! a playlist (scene lines, durations, `rand(...)`, comments), and checks it
//! never panics. A failing case is shrunk and kept in
//! `tests/properties.proptest-regressions`, replayed first on every run.

use anyhow::{Result, anyhow};
use proptest::prelude::*;
use proptest::test_runner::{Config, FileFailurePersistence, TestRunner};
use strobe_playlist::{DurationSpec, Timeline, TimelineConfig};

/// Where shrunk failures are kept, relative to this crate.
const REGRESSIONS: &str = "tests/properties.proptest-regressions";

/// Runs `test` over `strategy` for `cases` generated inputs.
fn check<S: Strategy>(
    cases: u32,
    strategy: S,
    test: impl Fn(S::Value) -> Result<(), TestCaseError>,
) -> Result<()> {
    let mut runner = TestRunner::new(Config {
        cases,
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(REGRESSIONS))),
        ..Config::default()
    });
    runner.run(&strategy, test).map_err(|e| anyhow!("{e}"))
}

/// A duration as a playlist writes it, ordinary and extreme: plain and
/// fractional seconds, minutes, bars, beats, `rand(...)` of two of them, a
/// bare number, zero, and nonsense.
fn duration() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => valid_duration(),
        1 => prop_oneof![
            Just("\"-5s\"".to_owned()),
            Just("\"1e308s\"".to_owned()),
            "\"[a-z0-9 ().,]{0,12}\"",
        ],
    ]
}

/// Durations the grammar accepts, from zero to its limits: seconds up to
/// ten million, fractions, minutes, bars and beats up to a million each,
/// bare numbers, and `rand(...)` of two of them.
fn valid_duration() -> impl Strategy<Value = String> {
    let unit = prop_oneof![
        (0u32..=10_000_000).prop_map(|n| format!("{n}s")),
        (0u32..1000).prop_map(|n| format!("{n}.25s")),
        (0u32..600).prop_map(|n| format!("{n}m")),
        (1u32..=1_000_000).prop_map(|n| format!("{n} bars")),
        (1u32..=1_000_000).prop_map(|n| format!("{n} beats")),
        Just("0s".to_owned()),
    ];
    prop_oneof![
        3 => unit.clone().prop_map(|d| format!("\"{d}\"")),
        // A bare TOML number is seconds; it is not allowed inside `rand`.
        1 => (0u32..100_000).prop_map(|n| n.to_string()),
        1 => (unit.clone(), unit).prop_map(|(a, b)| format!("\"rand({a}, {b})\"")),
    ]
}

/// One cue's inline table.
fn cue() -> impl Strategy<Value = String> {
    (
        "[a-z][a-z0-9-]{0,10}",
        prop::option::of(valid_duration()),
        prop::option::of(prop_oneof![
            Just("{ kind = \"cut\" }".to_owned()),
            valid_duration().prop_map(|d| format!("{{ kind = \"crossfade\", duration = {d} }}")),
        ]),
        any::<bool>(),
    )
        .prop_map(|(scene, length, transition, enabled)| {
            let mut out = format!("{{ scene = \"{scene}\"");
            if let Some(length) = length {
                out.push_str(&format!(", duration = {length}"));
            }
            if let Some(transition) = transition {
                out.push_str(&format!(", transition = {transition}"));
            }
            if !enabled {
                out.push_str(", enabled = false");
            }
            out.push_str(" }");
            out
        })
}

/// A whole playlist in the TOML format: a name, defaults, maybe a shuffle,
/// and one to four sections of up to twelve cues.
fn playlist_toml() -> impl Strategy<Value = String> {
    let section = ("[A-Za-z ]{1,12}", prop::collection::vec(cue(), 0..12));
    (
        valid_duration(),
        prop::option::of(0u64..=i64::MAX as u64),
        prop::collection::vec(section, 1..4),
        prop::sample::select(vec!["loop", "stop", "loop-section"]),
    )
        .prop_map(|(default, shuffle, sections, mode)| {
            let mut out = format!(
                "name = \"generated\"\nloop_mode = \"{mode}\"\n\n[defaults]\nduration = {default}\n"
            );
            if let Some(seed) = shuffle {
                out.push_str(&format!(
                    "\n[shuffle]\nseed = {seed}\nno_repeat_window = 3\n"
                ));
            }
            for (name, cues) in sections {
                out.push_str(&format!("\n[[sections]]\nname = \"{name}\"\ncues = [\n"));
                for cue in cues {
                    out.push_str(&format!("  {cue},\n"));
                }
                out.push_str("]\n");
            }
            out
        })
}

/// The same text with up to three bytes changed, so the parser also meets
/// a real playlist with a typo in it.
fn mutated(text: String, edits: Vec<(usize, u8)>) -> String {
    let mut bytes = text.into_bytes();
    let len = bytes.len().max(1);
    for (at, byte) in edits {
        if let Some(slot) = bytes.get_mut(at % len) {
            *slot = byte;
        }
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

/// Valid playlists, damaged ones, and noise.
fn playlist_text() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => playlist_toml(),
        2 => (playlist_toml(), prop::collection::vec((any::<usize>(), any::<u8>()), 1..4))
            .prop_map(|(text, edits)| mutated(text, edits)),
        1 => ".{0,300}",
    ]
}

#[test]
fn any_playlist_text_parses_or_is_refused() -> Result<()> {
    check(512, playlist_text(), |text| {
        // Either outcome is fine; no panic is the property.
        let _ = strobe_playlist::parse::parse(&text);
        Ok(())
    })
}

#[test]
fn any_playlist_that_parses_plans_a_sane_timeline_at_any_tempo() -> Result<()> {
    // From text to plan: whatever parses, at whatever fallback tempo, gives
    // finite cues that start where the last ended, never shorter than the
    // minimum, and `locate` puts every offset inside the cue it names.
    let tempo = prop_oneof![Just(120.0), Just(1.0), Just(1.0e6), 0.001f64..10_000.0,];
    let parsed = std::cell::Cell::new(0_u32);
    let cases = 512;
    check(cases, (playlist_toml(), tempo), |(text, bpm)| {
        let Ok(playlist) = strobe_playlist::parse::parse(&text) else {
            return Ok(());
        };
        parsed.set(parsed.get() + 1);
        let timeline = Timeline::from_playlist(&playlist, &TimelineConfig { fallback_bpm: bpm });
        let mut end = 0.0;
        for cue in timeline.cues() {
            prop_assert!(
                cue.start.is_finite() && cue.duration.is_finite(),
                "a finite cue at {bpm} bpm: {cue:?}"
            );
            prop_assert!(cue.start == end, "contiguous at {bpm} bpm: {cue:?}");
            prop_assert!(
                cue.duration >= strobe_playlist::timeline::MIN_CUE_SECONDS,
                "at least the minimum: {cue:?}"
            );
            end = cue.end();
        }
        let total = timeline.total_runtime();
        for fraction in [0.0, 0.25, 0.5, 0.999] {
            let offset = total * fraction;
            if let Some((index, into)) = timeline.locate(offset) {
                let Some(cue) = timeline.cues().get(index) else {
                    return Err(TestCaseError::fail(format!(
                        "locate named cue {index} of {}",
                        timeline.len()
                    )));
                };
                prop_assert!(
                    into >= 0.0 && into <= cue.duration,
                    "{offset} is {into} into a {} s cue",
                    cue.duration
                );
            }
        }
        Ok(())
    })?;
    // A generator that stopped producing valid playlists would leave this
    // property passing over nothing.
    assert!(
        parsed.get() * 4 >= cases,
        "only {} of {cases} generated playlists parsed",
        parsed.get()
    );
    Ok(())
}

#[test]
fn any_duration_parses_or_is_refused() -> Result<()> {
    let text = prop_oneof![
        duration().prop_map(|d| d.trim_matches('"').to_owned()),
        ".{0,60}",
    ];
    check(1024, text, |text| {
        // Either outcome is fine; no panic is the property.
        let _ = DurationSpec::parse(&text);
        Ok(())
    })
}
