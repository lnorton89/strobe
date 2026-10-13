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

/// Why [`write`] refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WriteError {
    /// The text is not valid JSON.
    #[error("scene.json is not valid JSON: {0}")]
    Invalid(String),
    /// Valid JSON, but not an object.
    #[error("scene.json is not a JSON object")]
    NotAnObject,
}

/// `json` with its `TAGS` set to `tags` (normalised), and nothing else
/// changed. See the module docs.
pub fn write(json: &str, tags: &[String]) -> Result<String, WriteError> {
    let body = json.trim_start_matches('\u{feff}');
    match serde_json::from_str::<Value>(body) {
        Ok(Value::Object(_)) => {}
        Ok(_) => return Err(WriteError::NotAnObject),
        Err(error) => return Err(WriteError::Invalid(error.to_string())),
    }
    let tags = normalized(tags.iter().map(String::as_str));
    let value = format!(
        "[{}]",
        tags.iter()
            .map(|tag| Value::String(tag.clone()).to_string())
            .collect::<Vec<_>>()
            .join(", ")
    );
    let prefix = json.len() - body.len();
    let members = scan::members(body).ok_or(WriteError::NotAnObject)?;
    let mut out = String::with_capacity(json.len() + value.len() + 16);
    if let Some(found) = members.members.iter().find(|m| m.key == KEY) {
        out.push_str(json.get(..prefix + found.value.start).unwrap_or_default());
        out.push_str(&value);
        out.push_str(json.get(prefix + found.value.end..).unwrap_or_default());
        return Ok(out);
    }
    match members.members.last() {
        Some(last) => {
            let first = members
                .members
                .first()
                .map_or(last.key_start, |m| m.key_start);
            let indent = indent_before(body, first);
            let at = prefix + last.value.end;
            out.push_str(json.get(..at).unwrap_or_default());
            out.push_str(&format!(",\n{indent}\"{KEY}\": {value}"));
            out.push_str(json.get(at..).unwrap_or_default());
        }
        None => {
            let at = prefix + members.open + 1;
            out.push_str(json.get(..at).unwrap_or_default());
            out.push_str(&format!("\n  \"{KEY}\": {value}\n"));
            let rest = json.get(at..).unwrap_or_default();
            out.push_str(rest.trim_start());
        }
    }
    Ok(out)
}

/// The spaces and tabs before byte `at` on its line; two spaces when the
/// key shares its line with the brace.
fn indent_before(text: &str, at: usize) -> String {
    let line = text.get(..at).unwrap_or_default();
    let start = line.rfind('\n').map_or(0, |n| n + 1);
    let lead = line.get(start..).unwrap_or_default();
    if !lead.is_empty() && lead.chars().all(|c| c == ' ' || c == '\t') {
        lead.to_string()
    } else {
        "  ".to_string()
    }
}

/// Where each top-level member of a JSON object sits in its text. Run only
/// on text `serde_json` has already accepted as an object, so it does not
/// re-validate; it only needs to find boundaries.
mod scan {
    use std::ops::Range;

    /// One `"key": value` pair.
    pub(super) struct Member {
        /// The key, unescaped only as far as plain ASCII keys need.
        pub(super) key: String,
        /// Byte offset of the key's opening quote.
        pub(super) key_start: usize,
        /// The value's bytes.
        pub(super) value: Range<usize>,
    }

    /// The object's members and where its `{` is.
    pub(super) struct Members {
        /// Byte offset of the opening brace.
        pub(super) open: usize,
        /// The members, in file order.
        pub(super) members: Vec<Member>,
    }

    fn skip_ws(bytes: &[u8], mut at: usize) -> usize {
        while bytes.get(at).is_some_and(u8::is_ascii_whitespace) {
            at += 1;
        }
        at
    }

    /// The byte after the string starting at `at` (its opening quote).
    fn skip_string(bytes: &[u8], mut at: usize) -> Option<usize> {
        at += 1;
        loop {
            match bytes.get(at)? {
                b'\\' => at += 2,
                b'"' => return Some(at + 1),
                _ => at += 1,
            }
        }
    }

    /// The byte after the value starting at `at`.
    fn skip_value(bytes: &[u8], at: usize) -> Option<usize> {
        match bytes.get(at)? {
            b'"' => skip_string(bytes, at),
            b'{' | b'[' => {
                let mut depth = 0usize;
                let mut i = at;
                loop {
                    match bytes.get(i)? {
                        b'"' => {
                            i = skip_string(bytes, i)?;
                            continue;
                        }
                        b'{' | b'[' => depth += 1,
                        b'}' | b']' => {
                            depth = depth.saturating_sub(1);
                            if depth == 0 {
                                return Some(i + 1);
                            }
                        }
                        _ => {}
                    }
                    i += 1;
                }
            }
            _ => {
                let mut i = at;
                while bytes
                    .get(i)
                    .is_some_and(|b| !matches!(b, b',' | b'}' | b']') && !b.is_ascii_whitespace())
                {
                    i += 1;
                }
                Some(i)
            }
        }
    }

    /// The members of the object in `text`.
    pub(super) fn members(text: &str) -> Option<Members> {
        let bytes = text.as_bytes();
        let open = skip_ws(bytes, 0);
        if bytes.get(open) != Some(&b'{') {
            return None;
        }
        let mut members = Vec::new();
        let mut at = skip_ws(bytes, open + 1);
        if bytes.get(at) == Some(&b'}') {
            return Some(Members { open, members });
        }
        loop {
            let key_start = at;
            let key_end = skip_string(bytes, key_start)?;
            let key = text.get(key_start + 1..key_end - 1)?.to_string();
            at = skip_ws(bytes, key_end);
            if bytes.get(at) != Some(&b':') {
                return None;
            }
            let start = skip_ws(bytes, at + 1);
            let end = skip_value(bytes, start)?;
            members.push(Member {
                key,
                key_start,
                value: start..end,
            });
            at = skip_ws(bytes, end);
            match bytes.get(at)? {
                b',' => at = skip_ws(bytes, at + 1),
                b'}' => return Some(Members { open, members }),
                _ => return None,
            }
        }
    }
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
