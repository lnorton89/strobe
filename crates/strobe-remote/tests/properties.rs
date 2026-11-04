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

/// One desktop input event, with its numbers inside the limits the server
/// enforces (`in_bounds`) or anywhere at all (`!in_bounds`).
fn event(in_bounds: bool) -> impl Strategy<Value = serde_json::Value> {
    use serde_json::json;
    let point = if in_bounds {
        prop_oneof![0.0f32..=1.0, Just(0.0), Just(1.0)].boxed()
    } else {
        prop_oneof![-1.0e30f32..-0.001, 1.001f32..1.0e30, Just(f32::MAX)].boxed()
    };
    let scroll = if in_bounds {
        (-4096.0f32..=4096.0).boxed()
    } else {
        prop_oneof![4096.5f32..1.0e30, -1.0e30f32..-4096.5].boxed()
    };
    let button = if in_bounds {
        (0u8..=2).boxed()
    } else {
        (3u8..=255).boxed()
    };
    let modifiers = (any::<bool>(), any::<bool>(), any::<bool>())
        .prop_map(|(alt, ctrl, shift)| json!({ "alt": alt, "ctrl": ctrl, "shift": shift }));
    prop_oneof![
        (point.clone(), point.clone()).prop_map(|(x, y)| json!({ "kind": "move", "x": x, "y": y })),
        (point.clone(), point, button, any::<bool>(), modifiers.clone()).prop_map(
            |(x, y, button, down, modifiers)| json!({
                "kind": "button", "x": x, "y": y, "button": button, "down": down, "modifiers": modifiers
            })
        ),
        (scroll.clone(), scroll, modifiers.clone()).prop_map(|(x, y, modifiers)| json!({
            "kind": "scroll", "x": x, "y": y, "modifiers": modifiers
        })),
        ("[!-~]{1,24}", any::<bool>(), modifiers).prop_map(|(key, down, modifiers)| json!({
            "kind": "key", "key": key, "down": down, "modifiers": modifiers
        })),
        "[^\u{0}]{0,200}".prop_map(|text| json!({ "kind": "text", "text": text })),
    ]
}

/// Messages the protocol defines, every field inside the limits the server
/// checks, values from ordinary to the edge of those limits: hello,
/// pairing, authentication, each command, ping, and desktop input.
fn valid_message() -> impl Strategy<Value = String> {
    use serde_json::json;
    let name = "[A-Za-z0-9._-]{1,20}";
    let command = prop::sample::select(vec![
        "next_scene",
        "prev_scene",
        "toggle_hold",
        "restart_cue",
        "extend_cue",
        "skip_section",
        "toggle_blackout",
        "blackout_on",
        "blackout_off",
        "tap_tempo",
    ]);
    prop_oneof![
        any::<u16>()
            .prop_map(|v| json!({ "type": "hello", "protocol_version": v, "features": [] })),
        name.prop_map(|name| json!({ "type": "request_pair", "name": name })),
        "[0-9]{1,8}".prop_map(|code| json!({ "type": "pair", "code": code })),
        "[0-9]{1,8}".prop_map(|code| json!({ "type": "pair_desktop", "code": code })),
        "[A-Za-z0-9_-]{1,64}".prop_map(|token| json!({ "type": "authenticate", "token": token })),
        ("[a-zA-Z0-9]{1,16}", command).prop_map(|(id, command)| json!({
            "type": "command", "request_id": id, "command": command
        })),
        Just(json!({ "type": "ping" })),
        Just(json!({ "type": "desktop_open" })),
        prop::collection::vec(event(true), 1..=32)
            .prop_map(|events| json!({ "type": "desktop_input", "events": events })),
        name.prop_map(|name| json!({ "type": "request_desktop", "name": name })),
    ]
    .prop_map(|message| message.to_string())
}

#[test]
fn desktop_input_outside_the_limits_is_always_refused() -> Result<()> {
    // The other half of the limits: an event list with one event out of
    // range (a point off the window, a scroll past 4096, a fourth mouse
    // button) is refused whole, wherever that event sits.
    use serde_json::json;
    let message = (
        prop::collection::vec(event(true), 0..10),
        event(false),
        prop::collection::vec(event(true), 0..10),
    )
        .prop_filter("non-text events carry numbers to break", |(_, bad, _)| {
            bad["kind"] != json!("text") && bad["kind"] != json!("key")
        })
        .prop_map(|(before, bad, after)| {
            let mut events = before;
            events.push(bad);
            events.extend(after);
            json!({ "type": "desktop_input", "events": events }).to_string()
        });
    check(1024, message, |text| {
        prop_assert!(
            strobe_remote::decode_client(&text).is_err(),
            "accepted: {}",
            text.chars().take(200).collect::<String>()
        );
        Ok(())
    })
}

#[test]
fn a_well_formed_message_decodes_whatever_its_values() -> Result<()> {
    let decoded = std::cell::Cell::new(0_u32);
    let refused = std::cell::RefCell::new(std::collections::BTreeSet::new());
    let cases = 1024;
    check(cases, valid_message(), |text| {
        match strobe_remote::decode_client(&text) {
            Ok(_) => decoded.set(decoded.get() + 1),
            Err(code) => {
                refused.borrow_mut().insert(format!(
                    "{code:?}: {}",
                    text.chars().take(80).collect::<String>()
                ));
            }
        }
        Ok(())
    })?;
    // A generator the decoder mostly refuses would make the properties here
    // pass over nothing.
    let refused = refused.into_inner();
    assert!(
        decoded.get() * 2 >= cases,
        "only {} of {cases} well-formed messages decoded; refusals: {:?}",
        decoded.get(),
        refused.iter().take(6).collect::<Vec<_>>()
    );
    Ok(())
}

#[test]
fn any_client_text_decodes_or_is_refused() -> Result<()> {
    check(
        1024,
        prop_oneof![valid_message(), client_text(), ".{0,200}"],
        |text| {
            // Either outcome is fine; no panic is the property.
            let _ = strobe_remote::decode_client(&text);
            Ok(())
        },
    )
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
