// ============================================================================
// Module:       strobe_scene::tags
// Description:  A scene's TAGS: read leniently from scene.json, normalised
//               so "3D" and "3d" or "audio reactive" and "audio-reactive"
//               are one tag, and written back without disturbing the file.
//
// Dependencies: serde_json
// ============================================================================

//! Scene tags (backlog U-08, D205).
//!
//! Synesthesia's `scene.json` carries `"TAGS": ["dark", "tunnel", ...]`,
//! and most real scenes fill it in. The spellings drift between authors
//! (`3D` and `3d`, `audio reactive` and `audio-reactive`), so a tag is
//! compared in its [`normalize`]d form: lowercase, with runs of spaces,
//! `-` and `_` turned into a single `-`.
//!
//! ## Reading never fails a scene
//!
//! `TAGS` is metadata. A file whose `TAGS` is `null`, a string, a number
//! or an array with a number in it still loads: an array keeps its
//! strings, a string is read as a comma-separated list, and anything else
//! is no tags. Duplicates after normalising are dropped, first one kept.
//!
//! ## Writing keeps the file
//!
//! [`write`] replaces the `TAGS` value in the text, or adds the key after
//! the last one, and leaves every other byte alone: key order, spacing,
//! the scene's own formatting, a byte-order mark. A file that is not a
//! JSON object is refused, never "repaired". The written tags are the
//! normalised ones, so a file round-trips to itself.

use serde_json::Value;

/// The key, as Synesthesia writes it.
pub const KEY: &str = "TAGS";

/// A tag longer than this is cut: it is a label, not a description.
pub const MAX_TAG_CHARS: usize = 48;

/// `tag` as compared and stored: trimmed, lowercase, every run of spaces,
/// `-` and `_` one `-`. `None` when nothing is left.
pub fn normalize(tag: &str) -> Option<String> {
    let mut out = String::new();
    let mut gap = false;
    for c in tag.chars() {
        if c.is_whitespace() || c == '-' || c == '_' {
            gap = !out.is_empty();
            continue;
        }
        if out.chars().count() >= MAX_TAG_CHARS {
            break;
        }
        if gap {
            out.push('-');
            gap = false;
        }
        out.extend(c.to_lowercase());
    }
    (!out.is_empty()).then_some(out)
}

/// Normalises each tag and drops empties and repeats, keeping the first.
pub fn normalized<'a>(tags: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for tag in tags.into_iter().filter_map(normalize) {
        if !out.contains(&tag) {
            out.push(tag);
        }
    }
    out
}

/// What a person typed into a tags field: separated by commas or
/// semicolons (a space is part of a tag, `audio reactive`).
pub fn parse_list(text: &str) -> Vec<String> {
    normalized(text.split([',', ';']))
}

/// The tags in a parsed `TAGS` value (see the module docs).
pub fn from_value(value: &Value) -> Vec<String> {
    match value {
        Value::Array(items) => normalized(items.iter().filter_map(Value::as_str)),
        Value::String(text) => parse_list(text),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::Object(_) => Vec::new(),
    }
}

/// The tags in `scene.json` text. Text that is not JSON has none.
pub fn read(json: &str) -> Vec<String> {
    match serde_json::from_str::<Value>(json.trim_start_matches('\u{feff}')) {
        Ok(Value::Object(map)) => map.get(KEY).map(from_value).unwrap_or_default(),
        Ok(_) | Err(_) => Vec::new(),
    }
}

pub use crate::json_edit::WriteError;

/// `json` with its `TAGS` set to `tags` (normalised), and nothing else
/// changed. See the module docs.
pub fn write(json: &str, tags: &[String]) -> Result<String, WriteError> {
    let tags = normalized(tags.iter().map(String::as_str));
    let value = format!(
        "[{}]",
        tags.iter()
            .map(|tag| Value::String(tag.clone()).to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    crate::json_edit::set_member(json, KEY, &value)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn spellings_of_one_tag_normalise_to_one() -> Result<()> {
        for (input, want) in [
            ("3D", "3d"),
            ("  Audio reactive ", "audio-reactive"),
            ("audio-reactive", "audio-reactive"),
            ("audio__reactive", "audio-reactive"),
            ("-dark-", "dark"),
            ("Path Tracing", "path-tracing"),
        ] {
            assert_eq!(normalize(input).as_deref(), Some(want), "{input:?}");
        }
        assert_eq!(normalize(" - _ "), None, "only separators is no tag");
        let long = "x".repeat(MAX_TAG_CHARS + 10);
        assert_eq!(
            normalize(&long).map(|t| t.chars().count()),
            Some(MAX_TAG_CHARS),
            "a long tag is cut"
        );
        Ok(())
    }

    #[test]
    fn reading_is_lenient_and_drops_repeats() -> Result<()> {
        assert_eq!(
            read(r#"{ "TAGS": ["Dark", "3D", "dark", 7, "audio reactive"] }"#),
            ["dark", "3d", "audio-reactive"],
            "strings kept, numbers skipped, repeats dropped"
        );
        assert_eq!(
            read(r#"{ "TAGS": "calm, slow" }"#),
            ["calm", "slow"],
            "a string is a list"
        );
        assert!(read(r#"{ "TAGS": null }"#).is_empty(), "null is none");
        assert!(read(r#"{ "TAGS": 3 }"#).is_empty(), "a number is none");
        assert!(read(r#"{ "TITLE": "t" }"#).is_empty(), "absent is none");
        assert!(read("not json").is_empty(), "broken JSON is none");
        assert_eq!(
            read("\u{feff}{ \"TAGS\": [\"x\"] }"),
            ["x"],
            "a byte-order mark is skipped"
        );
        Ok(())
    }

    #[test]
    fn typed_lists_split_on_commas_not_spaces() -> Result<()> {
        assert_eq!(
            parse_list("dark, audio reactive;  3D ,,"),
            ["dark", "audio-reactive", "3d"],
            "a space belongs to the tag"
        );
        Ok(())
    }

    #[test]
    fn writing_replaces_only_the_tags_value() -> Result<()> {
        let before =
            "{\n    \"TITLE\": \"Zeta\",\n    \"TAGS\": [ \"old\" ],\n    \"CONTROLS\": []\n}\n";
        let after = write(before, &["Dark".to_string(), "audio reactive".to_string()])?;
        assert_eq!(
            after,
            "{\n    \"TITLE\": \"Zeta\",\n    \"TAGS\": [\"dark\", \"audio-reactive\"],\n    \"CONTROLS\": []\n}\n",
            "every other byte is kept"
        );
        assert_eq!(read(&after), ["dark", "audio-reactive"], "it reads back");
        assert_eq!(
            write(&after, &read(&after))?,
            after,
            "writing what was read changes nothing"
        );
        Ok(())
    }

    #[test]
    fn writing_adds_the_key_after_the_last_one_with_the_files_indent() -> Result<()> {
        let before = "{\n\t\"TITLE\": \"t\",\n\t\"PASSES\": [{ \"TARGET\": \"a\" }]\n}";
        let after = write(before, &["calm".to_string()])?;
        assert_eq!(
            after,
            "{\n\t\"TITLE\": \"t\",\n\t\"PASSES\": [{ \"TARGET\": \"a\" }],\n\t\"TAGS\": [\"calm\"]\n}",
            "appended, tab-indented like its neighbours"
        );
        let empty = write("{}", &["x".to_string()])?;
        assert_eq!(read(&empty), ["x"], "an empty object gets the key");
        let bom = write("\u{feff}{ \"A\": 1 }", &["x".to_string()])?;
        assert!(bom.starts_with('\u{feff}'), "the byte-order mark stays");
        assert_eq!(read(&bom), ["x"], "and the result reads");
        Ok(())
    }

    #[test]
    fn writing_handles_strings_with_braces_and_escapes() -> Result<()> {
        let before = r#"{ "DESCRIPTION": "a \"}\" brace, [", "TAGS": null, "N": -1.5e3 }"#;
        let after = write(before, &["x".to_string()])?;
        assert_eq!(
            after, r#"{ "DESCRIPTION": "a \"}\" brace, [", "TAGS": ["x"], "N": -1.5e3 }"#,
            "the scanner is not fooled by text inside strings"
        );
        Ok(())
    }

    #[test]
    fn writing_refuses_what_is_not_an_object() -> Result<()> {
        assert_eq!(
            write("[1, 2]", &[]),
            Err(WriteError::NotAnObject),
            "an array is refused"
        );
        assert!(
            matches!(write("{ \"A\": ", &[]), Err(WriteError::Invalid(_))),
            "broken JSON is refused, not repaired"
        );
        Ok(())
    }
}
