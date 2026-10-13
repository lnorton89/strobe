// ============================================================================
// Module:       strobe_scene::json_edit
// Description:  Setting one top-level member of scene.json in its text, so
//               the rest of the file (order, spacing, a BOM) is untouched.
//
// Dependencies: serde_json
// ============================================================================

//! Editing `scene.json` without rewriting it (D205, D222).
//!
//! The app writes a scene's `TAGS` and `IMAGE_PATH` back into the scene's
//! own file. Parsing and re-serialising would reorder keys and reflow an
//! author's formatting, so [`set_member`] works on the text: the file is
//! first checked to be a JSON object with `serde_json`, then the member's
//! value is replaced where it stands, or the key is added after the last
//! one. A file that is not a JSON object is refused, never "repaired".

use serde_json::Value;

/// Why [`set_member`] refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WriteError {
    /// The text is not valid JSON.
    #[error("scene.json is not valid JSON: {0}")]
    Invalid(String),
    /// Valid JSON, but not an object.
    #[error("scene.json is not a JSON object")]
    NotAnObject,
}

/// `json` with its top-level `key` set to `value` (JSON text, written as
/// given), and nothing else changed: the value is replaced in place, or the
/// key is added after the last one with the file's indent. See the module
/// docs.
pub fn set_member(json: &str, key: &str, value: &str) -> Result<String, WriteError> {
    let body = json.trim_start_matches('\u{feff}');
    match serde_json::from_str::<Value>(body) {
        Ok(Value::Object(_)) => {}
        Ok(_) => return Err(WriteError::NotAnObject),
        Err(error) => return Err(WriteError::Invalid(error.to_string())),
    }
    let prefix = json.len() - body.len();
    let members = scan::members(body).ok_or(WriteError::NotAnObject)?;
    let mut out = String::with_capacity(json.len() + value.len() + 16);
    if let Some(found) = members.members.iter().find(|m| m.key == key) {
        out.push_str(json.get(..prefix + found.value.start).unwrap_or_default());
        out.push_str(value);
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
            out.push_str(&format!(",\n{indent}\"{key}\": {value}"));
            out.push_str(json.get(at..).unwrap_or_default());
        }
        None => {
            let at = prefix + members.open + 1;
            out.push_str(json.get(..at).unwrap_or_default());
            out.push_str(&format!("\n  \"{key}\": {value}\n"));
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
    fn a_member_is_replaced_in_place_or_added_last() -> Result<()> {
        let text = "{\n    \"TITLE\": \"x\",\n    \"IMAGE_PATH\": \"old.png\"\n}\n";
        let replaced = set_member(text, "IMAGE_PATH", "\"thumbnail.jpg\"")?;
        assert_eq!(
            replaced, "{\n    \"TITLE\": \"x\",\n    \"IMAGE_PATH\": \"thumbnail.jpg\"\n}\n",
            "only the value changed"
        );
        let added = set_member("{\n  \"TITLE\": \"x\"\n}", "IMAGE_PATH", "\"t.jpg\"")?;
        assert_eq!(
            added, "{\n  \"TITLE\": \"x\",\n  \"IMAGE_PATH\": \"t.jpg\"\n}",
            "added after the last key with its indent"
        );
        assert!(
            set_member("[1]", "IMAGE_PATH", "\"t.jpg\"").is_err(),
            "an array is refused"
        );
        Ok(())
    }
}
