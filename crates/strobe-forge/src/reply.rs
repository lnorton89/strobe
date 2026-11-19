// ============================================================================
// Module:       strobe_forge::reply
// Description:  Parses the model's answer strictly into `main.glsl` and
//               `scene.json`, refusing anything else: other files, media,
//               scripts, keys the scene format does not take.
//
// Dependencies: serde_json; strobe-scene (scene.json validation)
// ============================================================================

//! The answer's format.
//!
//! The model answers with two tagged blocks, `<file name="main.glsl">` and
//! `<file name="scene.json">`, each exactly once. Short prose around them
//! is allowed and ignored. Everything else is refused with a message the
//! next turn hands back to the model:
//!
//! - any other file, `script.js` included (the generated scene has no
//!   script: the model's code runs only as a shader, `docs/LLM_SHADER_CREATION.md`);
//! - a `scene.json` that is not an object, or uses a key outside
//!   [`ALLOWED_KEYS`]; `IMAGES`, `CUBEMAPS`, `MEDIA` and `PASS_SOURCES` are
//!   named in the refusal (procedural, one shader, no reference media);
//! - a `scene.json` the scene format itself rejects ([`SceneDef::parse`]).

use strobe_scene::scene::SceneDef;

/// The two files of a candidate scene.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    /// `main.glsl`.
    pub(crate) glsl: String,
    /// `scene.json`.
    pub(crate) json: String,
}

/// The `scene.json` keys a generated scene may use.
pub(crate) const ALLOWED_KEYS: [&str; 10] = [
    "TITLE",
    "CREDIT",
    "DESCRIPTION",
    "WIDTH",
    "HEIGHT",
    "PASSES",
    "CONTROLS",
    "TAGS",
    "HARD_TRANSITIONS",
    "SMOOTH_TRANSITIONS",
];

/// Keys refused by name, with the reason.
const FORBIDDEN_KEYS: [(&str, &str); 5] = [
    ("IMAGES", "no reference media"),
    ("CUBEMAPS", "no reference media"),
    ("MEDIA", "no reference media"),
    ("PASS_SOURCES", "one main.glsl for every pass"),
    ("IMAGE_PATH", "no thumbnail file"),
];

const OPEN: &str = "<file name=\"";
const CLOSE: &str = "</file>";

/// Parses `text` into a candidate, or says what is wrong with it.
pub(crate) fn parse(text: &str) -> Result<Candidate, String> {
    let mut glsl = None;
    let mut json = None;
    let mut rest = text;
    while let Some(at) = rest.find(OPEN) {
        let after = &rest[at + OPEN.len()..];
        let Some(name_end) = after.find("\">") else {
            return Err("a <file name=\"...\"> tag is not closed".to_owned());
        };
        let name = &after[..name_end];
        let body_start = &after[name_end + 2..];
        let Some(body_end) = body_start.find(CLOSE) else {
            return Err(format!("the block for {name} has no </file>"));
        };
        let body = body_start[..body_end].trim_matches(['\r', '\n']).to_owned();
        let slot = match name {
            "main.glsl" => &mut glsl,
            "scene.json" => &mut json,
            "script.js" => {
                return Err(
                    "no script.js: the scene is a shader and its scene.json only".to_owned(),
                );
            }
            other => {
                return Err(format!(
                    "unexpected file {other}: answer with main.glsl and scene.json only"
                ));
            }
        };
        if slot.replace(body).is_some() {
            return Err(format!("{name} appears twice"));
        }
        rest = &body_start[body_end + CLOSE.len()..];
    }
    let (Some(glsl), Some(json)) = (glsl, json) else {
        return Err(
            "the answer must hold both <file name=\"main.glsl\"> and <file name=\"scene.json\">"
                .to_owned(),
        );
    };
    if glsl.trim().is_empty() {
        return Err("main.glsl is empty".to_owned());
    }
    validate_json(&json)?;
    Ok(Candidate { glsl, json })
}

/// Checks `scene.json`'s keys and that the scene format accepts it.
pub(crate) fn validate_json(json: &str) -> Result<(), String> {
    let value: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("scene.json is not valid JSON: {e}"))?;
    let Some(object) = value.as_object() else {
        return Err("scene.json must be a JSON object".to_owned());
    };
    for key in object.keys() {
        if let Some((_, why)) = FORBIDDEN_KEYS.iter().find(|(k, _)| k == key) {
            return Err(format!("scene.json must not use {key}: {why}"));
        }
        if !ALLOWED_KEYS.contains(&key.as_str()) {
            return Err(format!(
                "scene.json key {key} is not allowed; use only {}",
                ALLOWED_KEYS.join(", ")
            ));
        }
    }
    SceneDef::parse(json, "Generated")
        .map_err(|e| format!("scene.json is not a valid scene: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixture;
    use anyhow::{Result, anyhow, ensure};

    #[test]
    fn a_good_reply_gives_both_files() -> Result<()> {
        let got = parse(&fixture::reply(fixture::GLSL)).map_err(|e| anyhow!(e))?;
        ensure!(
            got.glsl.starts_with("vec4 renderMain"),
            "glsl: {}",
            got.glsl
        );
        ensure!(
            got.json.contains("\"TITLE\": \"Rings\""),
            "json: {}",
            got.json
        );
        Ok(())
    }

    #[test]
    fn malformed_replies_say_what_is_wrong() -> Result<()> {
        let cases = [
            ("no blocks at all", "missing", "must hold both"),
            ("<file name=\"main.glsl\">x", "unclosed", "no </file>"),
            (
                "<file name=\"main.glsl\">x</file>",
                "one file",
                "must hold both",
            ),
            (
                "<file name=\"main.glsl\">x</file><file name=\"main.glsl\">y</file>",
                "twice",
                "appears twice",
            ),
            (
                "<file name=\"main.glsl\">x</file><file name=\"scene.json\">[1]</file>",
                "not an object",
                "JSON object",
            ),
            (
                "<file name=\"main.glsl\">x</file><file name=\"scene.json\">{</file>",
                "bad json",
                "not valid JSON",
            ),
            (
                "<file name=\"main.glsl\">x</file><file name=\"notes.txt\">y</file>",
                "other file",
                "unexpected file notes.txt",
            ),
        ];
        for (text, what, expected) in cases {
            match parse(text) {
                Ok(got) => return Err(anyhow!("{what}: accepted {got:?}")),
                Err(error) => ensure!(error.contains(expected), "{what}: {error}"),
            }
        }
        Ok(())
    }

    #[test]
    fn scripts_media_and_unknown_keys_are_refused() -> Result<()> {
        let script = format!(
            "{}<file name=\"script.js\">setUniform('a', 1)</file>",
            fixture::reply(fixture::GLSL)
        );
        let error = parse(&script).err().unwrap_or_default();
        ensure!(error.contains("no script.js"), "script: {error}");
        for (key, expected) in [
            ("IMAGES", "no reference media"),
            ("MEDIA", "no reference media"),
            ("PASS_SOURCES", "one main.glsl"),
            ("LICENSE", "not allowed"),
        ] {
            let json = format!("{{\"TITLE\": \"x\", \"{key}\": []}}");
            let text =
                format!("<file name=\"main.glsl\">x</file><file name=\"scene.json\">{json}</file>");
            let error = parse(&text).err().unwrap_or_default();
            ensure!(
                error.contains(key) && error.contains(expected),
                "{key}: {error}"
            );
        }
        Ok(())
    }

    #[test]
    fn a_scene_the_format_rejects_is_refused() -> Result<()> {
        let json = r#"{"CONTROLS": [{"NAME": "TIME", "TYPE": "slider"}]}"#;
        let error = validate_json(json).err().unwrap_or_default();
        ensure!(
            error.contains("not a valid scene"),
            "reserved name: {error}"
        );
        Ok(())
    }
}
