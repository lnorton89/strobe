// ============================================================================
// Module:       properties (integration test)
// Description:  Property tests for the scene model's untrusted input: any
//               scene.json text parses or is refused without a panic, and
//               the GLSL rewriters return bounded output for any source.
//
// Dependencies: anyhow, proptest; strobe-scene
// ============================================================================

//! Scenes arrive from strangers (D539).
//!
//! A downloaded or community scene is input the app did not write: its
//! `scene.json` and its GLSL go through the parser and the rewriters before
//! anything is drawn. Each property here feeds them generated text, both
//! arbitrary and shaped like the real thing, and checks they never panic
//! (release builds abort on a panic) and that a rewrite
//! does not grow its input without bound. A failing case is shrunk and
//! written to `tests/properties.proptest-regressions`, which is committed
//! and replayed first on every run.

use anyhow::{Result, anyhow};
use proptest::prelude::*;
use proptest::test_runner::{Config, FileFailurePersistence, TestRunner};
use strobe_scene::SceneDef;
use strobe_scene::{fixups, promote, tags};

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

/// Text shaped like a scene.json: braces, keys the parser knows, numbers,
/// strings and lists, in any order and nesting.
fn scene_json() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        Just("{".to_owned()),
        Just("}".to_owned()),
        Just("[".to_owned()),
        Just("]".to_owned()),
        Just(",".to_owned()),
        Just(":".to_owned()),
        Just("\"CONTROLS\"".to_owned()),
        Just("\"PASSES\"".to_owned()),
        Just("\"NAME\"".to_owned()),
        Just("\"TYPE\"".to_owned()),
        Just("\"slider smooth\"".to_owned()),
        Just("\"MIN\"".to_owned()),
        Just("\"MAX\"".to_owned()),
        Just("\"DEFAULT\"".to_owned()),
        Just("\"PARAMS\"".to_owned()),
        Just("\"TARGET\"".to_owned()),
        Just("\"WIDTH\"".to_owned()),
        Just("\"$WIDTH/3\"".to_owned()),
        Just("\"IMAGES\"".to_owned()),
        Just("\"HARD_TRANSITIONS\"".to_owned()),
        Just("\"VALUES\"".to_owned()),
        Just("true".to_owned()),
        Just("null".to_owned()),
        any::<i64>().prop_map(|n| n.to_string()),
        any::<f64>().prop_map(|n| n.to_string()),
        "\"[a-zA-Z_ ]{0,12}\"",
    ];
    prop::collection::vec(piece, 0..80).prop_map(|parts| parts.concat())
}

/// Text shaped like GLSL: identifiers the rewriters look for, operators,
/// numbers in both forms, comments and preprocessor lines.
fn glsl() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        Just("PASSINDEX".to_owned()),
        Just("==".to_owned()),
        Just("pow(".to_owned()),
        Just("float".to_owned()),
        Just("int".to_owned()),
        Just("vec3".to_owned()),
        Just("uniform".to_owned()),
        Just("const".to_owned()),
        Just("=".to_owned()),
        Just(";".to_owned()),
        Just("(".to_owned()),
        Just(")".to_owned()),
        Just("{".to_owned()),
        Just("}".to_owned()),
        Just(",".to_owned()),
        Just("/*".to_owned()),
        Just("*/".to_owned()),
        Just("//".to_owned()),
        Just("\n".to_owned()),
        Just("#define X ".to_owned()),
        Just("#if 1\n".to_owned()),
        Just("#endif\n".to_owned()),
        any::<u16>().prop_map(|n| n.to_string()),
        any::<u16>().prop_map(|n| format!("{n}.5")),
        "[a-z_][a-z0-9_]{0,6}",
        " ",
    ];
    prop::collection::vec(piece, 0..120).prop_map(|parts| parts.concat())
}

/// A rewrite may add declarations and casts, but not without bound.
fn bounded(input: &str, output: &str) -> Result<(), TestCaseError> {
    let limit = input.len() * 8 + 4096;
    prop_assert!(
        output.len() <= limit,
        "{} bytes in, {} out",
        input.len(),
        output.len()
    );
    Ok(())
}

#[test]
fn any_scene_json_parses_or_is_refused() -> Result<()> {
    check(512, prop_oneof![scene_json(), ".{0,200}"], |text| {
        // Ok or an error are both fine; reaching here without a panic is
        // the property.
        let _ = SceneDef::parse(&text, "fallback");
        Ok(())
    })
}

#[test]
fn tag_lists_never_panic_and_stay_bounded() -> Result<()> {
    check(512, ".{0,300}", |text| {
        let tags = tags::parse_list(&text);
        let total: usize = tags.iter().map(String::len).sum();
        // Tags are lower-cased, and a lower-case letter can take more bytes
        // than its capital (`Ⱥ` is two, `ⱥ` three), never more than three
        // times as many.
        prop_assert!(
            total <= text.len() * 3,
            "tags {total} from {} bytes",
            text.len()
        );
        Ok(())
    })
}

#[test]
fn the_glsl_rewriters_are_total_and_bounded() -> Result<()> {
    check(512, glsl(), |source| {
        bounded(&source, &fixups::passindex_float_compares(&source))?;
        bounded(&source, &fixups::pow_as_exp2_log2(&source))?;
        for es in [false, true] {
            bounded(&source, &promote::int_to_float(&source, "", es))?;
        }
        Ok(())
    })
}
