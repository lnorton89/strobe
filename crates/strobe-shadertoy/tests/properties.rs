// ============================================================================
// Module:       properties (integration test)
// Description:  Property tests for the Shadertoy importer: any JSON text is
//               a shader or a refusal, and any GLSL in a shader's image pass
//               converts or is refused, with bounded output, without a panic.
//
// Dependencies: anyhow, proptest, serde_json; strobe-shadertoy
// ============================================================================

//! Shadertoy exports are a stranger's code (D539).
//!
//! The importer reads a JSON export and rewrites its GLSL into a scene.
//! Each property feeds it generated input, an arbitrary JSON text, and a
//! real export (`tests/fixtures/single_image.json`) with its image pass's
//! code replaced by generated GLSL-like text. It checks the importer never
//! panics and never grows the code without bound. A failing case is shrunk
//! and kept in `tests/properties.proptest-regressions`, replayed first.

use anyhow::{Result, anyhow};
use proptest::prelude::*;
use proptest::test_runner::{Config, FileFailurePersistence, TestRunner};
use strobe_shadertoy::{Options, Shader, ShaderId, convert};

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

/// GLSL-like text: Shadertoy's entry point and uniforms, the constructs
/// the converter rewrites, and punctuation in any order.
fn glsl() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        Just("void mainImage(out vec4 fragColor, in vec2 fragCoord)".to_owned()),
        Just("{".to_owned()),
        Just("}".to_owned()),
        Just("(".to_owned()),
        Just(")".to_owned()),
        Just(";".to_owned()),
        Just(",".to_owned()),
        Just("=".to_owned()),
        Just("fragColor".to_owned()),
        Just("iResolution".to_owned()),
        Just("iTime".to_owned()),
        Just("iMouse".to_owned()),
        Just("iChannel0".to_owned()),
        Just("texture(".to_owned()),
        Just("#define".to_owned()),
        Just("#if".to_owned()),
        Just("\n".to_owned()),
        Just("//".to_owned()),
        Just("/*".to_owned()),
        Just("*/".to_owned()),
        Just("vec4".to_owned()),
        Just("float".to_owned()),
        any::<u16>().prop_map(|n| format!("{n}.0")),
        "[a-z_][a-z0-9_]{0,6}",
        " ",
    ];
    prop::collection::vec(piece, 0..120).prop_map(|parts| parts.concat())
}

#[test]
fn any_json_is_a_shader_or_a_refusal() -> Result<()> {
    check(512, ".{0,300}", |text| {
        // Either outcome is fine; no panic is the property.
        let _ = Shader::from_json(&text);
        Ok(())
    })
}

#[test]
fn any_image_pass_code_converts_or_is_refused() -> Result<()> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("single_image.json");
    let fixture: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path)?)?;
    let converted = std::cell::Cell::new(0_u32);
    let cases = 256;
    check(cases, glsl(), |code| {
        let mut export = fixture.clone();
        let Some(slot) = export.pointer_mut("/Shader/renderpass/0/code") else {
            return Err(TestCaseError::fail("the fixture has no image pass"));
        };
        *slot = serde_json::Value::String(code.clone());
        let Ok(shader) = Shader::from_value(export) else {
            return Ok(());
        };
        if let Ok(output) = convert(&shader, &Options::default()) {
            converted.set(converted.get() + 1);
            let text = format!("{output:?}");
            prop_assert!(
                text.len() <= code.len() * 16 + 65_536,
                "{} bytes of code became {} of output",
                code.len(),
                text.len()
            );
        }
        Ok(())
    })?;
    // About half the generated passes convert (the rest are refused before
    // the rewrite); a quarter is the floor, far below that rate's spread
    // over 256 random cases, so it fails only when the generator stops
    // reaching the rewrite, not by chance (one run converted 127).
    assert!(
        converted.get() * 4 >= cases,
        "only {} of {cases} generated passes converted",
        converted.get()
    );
    Ok(())
}

#[test]
fn any_text_is_a_shader_id_or_a_refusal() -> Result<()> {
    check(1024, "[ -~]{0,80}", |text| {
        // Either outcome is fine; no panic is the property.
        let _ = ShaderId::parse(&text);
        Ok(())
    })
}
