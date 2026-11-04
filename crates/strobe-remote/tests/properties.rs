// ============================================================================
// Module:       properties (integration test)
// Description:  Property tests for the remote's untrusted input: any client
//               message text and any trust file text decodes or is refused,
//               without a panic.
//
// Dependencies: anyhow, proptest; strobe-remote
// ============================================================================

//! The remote listens on the LAN, so its input is a stranger's (D539).
//!
//! Each property feeds a decoder generated text, arbitrary and shaped like
//! the protocol (JSON objects with the message types and fields the server
//! knows), and checks it never panics. A failing case is shrunk and kept in
//! `tests/properties.proptest-regressions`, replayed first on every run.

use anyhow::{Result, anyhow};
use proptest::prelude::*;
use proptest::test_runner::{Config, FileFailurePersistence, TestRunner};
use strobe_remote::trust::TrustSecret;

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

/// Text shaped like a client message: JSON punctuation, the protocol's
/// types and fields, and values of every JSON kind.
fn client_text() -> impl Strategy<Value = String> {
    let piece = prop_oneof![
        Just("{".to_owned()),
        Just("}".to_owned()),
        Just("[".to_owned()),
        Just("]".to_owned()),
        Just(",".to_owned()),
        Just(":".to_owned()),
        Just("\"type\"".to_owned()),
        Just("\"hello\"".to_owned()),
        Just("\"pair\"".to_owned()),
        Just("\"authenticate\"".to_owned()),
        Just("\"command\"".to_owned()),
        Just("\"desktop_input\"".to_owned()),
        Just("\"events\"".to_owned()),
        Just("\"request_id\"".to_owned()),
        Just("\"protocol_version\"".to_owned()),
        Just("\"code\"".to_owned()),
        Just("\"token\"".to_owned()),
        Just("null".to_owned()),
        Just("true".to_owned()),
        any::<i64>().prop_map(|n| n.to_string()),
        any::<f64>().prop_map(|n| n.to_string()),
        "\"[ -~]{0,16}\"",
    ];
    prop::collection::vec(piece, 0..60).prop_map(|parts| parts.concat())
}

#[test]
fn any_client_text_decodes_or_is_refused() -> Result<()> {
    check(1024, prop_oneof![client_text(), ".{0,200}"], |text| {
        // Either outcome is fine; no panic is the property.
        let _ = strobe_remote::decode_client(&text);
        Ok(())
    })
}

#[test]
fn any_trust_file_parses_or_is_refused() -> Result<()> {
    let shaped = prop::collection::vec(
        prop_oneof![
            Just("# comment\n".to_owned()),
            Just("strobe-trust 1\n".to_owned()),
            Just("secret ".to_owned()),
            "[0-9a-f]{0,70}",
            Just("\n".to_owned()),
            "[ -~]{0,10}",
        ],
        0..12,
    )
    .prop_map(|parts| parts.concat());
    check(1024, prop_oneof![shaped, ".{0,200}"], |text| {
        // Either outcome is fine; no panic is the property.
        let _ = TrustSecret::parse(&text);
        Ok(())
    })
}
