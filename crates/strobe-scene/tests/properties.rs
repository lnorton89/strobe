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
use strobe_scene::directives::Branches;
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

/// A scene.json the parser should accept: controls of every kind with
/// numbers from ordinary to extreme, passes with sizes as numbers and as
/// expressions, and images.
fn valid_scene() -> impl Strategy<Value = String> {
    use serde_json::{Value, json};
    let number = prop_oneof![
        -1.0e6f64..1.0e6,
        Just(0.0),
        Just(f64::MAX),
        Just(f64::MIN_POSITIVE),
    ];
    let control = (
        prop::sample::select(vec![
            "slider",
            "slider smooth",
            "knob",
            "knob smooth",
            "xy",
            "toggle",
            "toggle smooth",
            "color",
            "dropdown",
        ]),
        number.clone(),
        number.clone(),
        number,
    )
        .prop_map(|(kind, a, b, c)| {
            // Each kind's fields as real scenes write them: xy takes pairs,
            // colour a DEF_COLOR triple, toggle a number, dropdown labels.
            let (low, high) = (a.min(b), a.max(b));
            let mut control: Value = match kind {
                "xy" | "xy smooth" => {
                    json!({ "MIN": [low, low], "MAX": [high, high], "DEFAULT": [c, c] })
                }
                "color" => json!({ "DEF_COLOR": [a, b, c] }),
                "toggle" | "toggle smooth" => json!({ "DEFAULT": if a > 0.0 { 1.0 } else { 0.0 } }),
                "dropdown" => json!({ "DEFAULT": 0 }),
                _ => json!({ "MIN": low, "MAX": high, "DEFAULT": c }),
            };
            control["TYPE"] = json!(kind);
            if kind == "dropdown" {
                control["LABELS"] = json!(["one", "two"]);
                control["VALUES"] = json!([1, 2]);
            }
            if kind.ends_with("smooth") {
                control["PARAMS"] = json!(0.05);
            }
            control
        });
    let pass = (
        prop_oneof![
            Just(json!("$WIDTH/3")),
            Just(json!("$HEIGHT")),
            (1u32..4096).prop_map(|n| json!(n)),
        ],
        any::<bool>(),
    )
        .prop_map(|(size, float)| json!({ "WIDTH": size, "HEIGHT": size, "FLOAT": float }));
    (
        prop::collection::vec(control, 0..10),
        prop::collection::vec(pass, 0..4),
        any::<bool>(),
    )
        .prop_map(|(controls, passes, image)| {
            let controls: Vec<Value> = controls
                .into_iter()
                .enumerate()
                .map(|(i, mut c)| {
                    c["NAME"] = json!(format!("control{i}"));
                    c
                })
                .collect();
            let passes: Vec<Value> = passes
                .into_iter()
                .enumerate()
                .map(|(i, mut p)| {
                    p["TARGET"] = json!(format!("buffer{i}"));
                    p
                })
                .collect();
            let mut scene = json!({ "TITLE": "generated", "CONTROLS": controls, "PASSES": passes });
            if image {
                scene["IMAGES"] = json!([{ "NAME": "picture", "PATH": "picture.png" }]);
            }
            scene.to_string()
        })
}

#[test]
fn a_valid_scene_parses_with_its_controls_whatever_their_numbers() -> Result<()> {
    let parsed = std::cell::Cell::new(0_u32);
    let refused = std::cell::RefCell::new(std::collections::BTreeSet::new());
    let cases = 512;
    check(cases, valid_scene(), |text| {
        match SceneDef::parse(&text, "fallback") {
            Ok(_) => parsed.set(parsed.get() + 1),
            Err(error) => {
                refused.borrow_mut().insert(error.to_string());
            }
        }
        Ok(())
    })?;
    // A generator the parser mostly refuses would make every property here
    // pass over nothing.
    let refused = refused.into_inner();
    assert!(
        parsed.get() * 2 >= cases,
        "only {} of {cases} generated scenes parsed; refusals: {:?}",
        parsed.get(),
        refused.iter().take(6).collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn any_scene_json_parses_or_is_refused() -> Result<()> {
    check(
        512,
        prop_oneof![valid_scene(), scene_json(), ".{0,200}"],
        |text| {
            // Ok or an error are both fine; reaching here without a panic is
            // the property.
            let _ = SceneDef::parse(&text, "fallback");
            Ok(())
        },
    )
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
            let promoted = promote::rewrite(&source, "", es);
            bounded(&source, &promoted.code)?;
            let lines = u32::try_from(source.lines().count()).unwrap_or(u32::MAX);
            prop_assert!(
                promoted
                    .unsupported
                    .iter()
                    .all(|line| *line >= 1 && *line <= lines.max(1)),
                "unsupported lines {:?} outside 1..={lines}",
                promoted.unsupported
            );
            let (hoisted, moved) =
                fixups::hoist_global_initializers(&source, Branches::dialect(es));
            bounded(&source, &hoisted)?;
            prop_assert!(
                moved.len() <= source.len(),
                "{} initializers hoisted from {} bytes",
                moved.len(),
                source.len()
            );
            bounded(
                &source,
                &fixups::uniform_initializers(&source, Branches::dialect(es)),
            )?;
        }
        Ok(())
    })
}
