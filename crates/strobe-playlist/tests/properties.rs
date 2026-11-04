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
use strobe_playlist::DurationSpec;

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

/// Text shaped like a playlist and its duration grammar.
fn playlist_text() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        Just("\n".to_owned()),
        Just("# ".to_owned()),
        Just("scene ".to_owned()),
        Just("01-aurora-gradient".to_owned()),
        Just(" for ".to_owned()),
        Just("rand(".to_owned()),
        Just(")".to_owned()),
        Just(",".to_owned()),
        Just("..".to_owned()),
        Just("bars".to_owned()),
        Just("beats".to_owned()),
        Just("s".to_owned()),
        Just("m".to_owned()),
        Just(":".to_owned()),
        Just("=".to_owned()),
        Just("\"".to_owned()),
        Just("[".to_owned()),
        Just("]".to_owned()),
        any::<u32>().prop_map(|n| n.to_string()),
        any::<f32>().prop_map(|n| n.to_string()),
        "[a-z]{1,8}",
        " ",
    ];
    prop::collection::vec(piece, 0..100).prop_map(|parts| parts.concat())
}

#[test]
fn any_playlist_text_parses_or_is_refused() -> Result<()> {
    check(512, prop_oneof![playlist_text(), ".{0,300}"], |text| {
        // Either outcome is fine; no panic is the property.
        let _ = strobe_playlist::parse::parse(&text);
        Ok(())
    })
}

#[test]
fn any_duration_parses_or_is_refused() -> Result<()> {
    check(1024, prop_oneof![playlist_text(), ".{0,60}"], |text| {
        // Either outcome is fine; no panic is the property.
        let _ = DurationSpec::parse(&text);
        Ok(())
    })
}
