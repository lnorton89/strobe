// ============================================================================
// Module:       strobe_autocontrol::output
// Description:  Writes the result as a new scene folder: a copy of the source
//               with the rewritten GLSL and CONTROLS spliced into scene.json.
//
// Dependencies: std::fs, crate::source, crate::generate
// ============================================================================

//! Writing the result.
//!
//! The original scene is **never modified**. The result is a copy:
//!
//! 1. every file of the source folder is copied into a hidden staging
//!    folder beside the destination;
//! 2. the rewritten GLSL files overwrite their copies;
//! 3. `scene.json` gets the new `CONTROLS` entries spliced in **as text**,
//!    so authored keys, their order, comments in the surrounding layout and
//!    the `SOURCE` block of an import all survive untouched (parsing and
//!    re-serialising would sort the keys);
//! 4. the staging folder is renamed into place in one step, so a crash
//!    leaves either the whole scene or nothing.
//!
//! The destination is `<library>/<name>-auto.synScene`. It must not exist
//! unless the caller asks to replace it, and it must not be the source.

use crate::generate::Generated;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Why writing failed.
#[derive(Debug, thiserror::Error)]
pub enum OutputError {
    /// A file operation failed.
    #[error("{path}: {source}")]
    Io {
        /// Where.
        path: PathBuf,
        /// What happened.
        source: io::Error,
    },
    /// The destination folder is already there.
    #[error("{0} already exists; ask to replace it")]
    Exists(PathBuf),
    /// The destination is the source folder: the original is never edited.
    #[error("{0} is the source scene; the original is never edited")]
    SameAsSource(PathBuf),
    /// `scene.json` is not JSON this tool can splice into.
    #[error("scene.json: {0}")]
    SceneJson(String),
}

fn io_error(path: &Path) -> impl FnOnce(io::Error) -> OutputError + '_ {
    move |source| OutputError::Io {
        path: path.to_path_buf(),
        source,
    }
}

/// The index just past the string that starts at `open` (a `"`).
fn skip_string(bytes: &[u8], open: usize) -> usize {
    let mut i = open + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'\\' => i += 2,
            b'"' => return i + 1,
            _ => i += 1,
        }
    }
    bytes.len()
}

/// Finds the top-level `"CONTROLS": [ ... ]`: the offsets of its `[` and `]`.
fn find_controls(json: &str) -> Option<(usize, usize)> {
    let bytes = json.as_bytes();
    let mut depth = 0_i32;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                let end = skip_string(bytes, i);
                if depth == 1 && json.get(i..end) == Some("\"CONTROLS\"") {
                    let mut j = end;
                    while bytes.get(j).is_some_and(u8::is_ascii_whitespace) {
                        j += 1;
                    }
                    if bytes.get(j) != Some(&b':') {
                        i = end;
                        continue;
                    }
                    j += 1;
                    while bytes.get(j).is_some_and(u8::is_ascii_whitespace) {
                        j += 1;
                    }
                    if bytes.get(j) != Some(&b'[') {
                        return None;
                    }
                    let mut nest = 0_i32;
                    let mut k = j;
                    while k < bytes.len() {
                        match bytes[k] {
                            b'"' => {
                                k = skip_string(bytes, k);
                                continue;
                            }
                            b'[' | b'{' => nest += 1,
                            b']' | b'}' => {
                                nest -= 1;
                                if nest == 0 {
                                    return Some((j, k));
                                }
                            }
                            _ => {}
                        }
                        k += 1;
                    }
                    return None;
                }
                i = end;
                continue;
            }
            b'{' | b'[' => depth += 1,
            b'}' | b']' => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    None
}

/// `scene_json` with `lines` (JSON objects) added to its `CONTROLS`, which
/// is created if the scene has none. An empty or missing file is `{}`.
pub fn splice_controls(scene_json: Option<&str>, lines: &[String]) -> Result<String, OutputError> {
    let text = scene_json.filter(|t| !t.trim().is_empty()).unwrap_or("{}");
    if lines.is_empty() {
        return Ok(text.to_string());
    }
    let body = lines
        .iter()
        .map(|line| format!("    {line}"))
        .collect::<Vec<_>>()
        .join(",\n");
    if let Some((open, close)) = find_controls(text) {
        let inside = text.get(open + 1..close).unwrap_or("");
        let before = text.get(..close).unwrap_or("").trim_end();
        return Ok(if inside.trim().is_empty() {
            format!(
                "{}\n{body}\n  {}",
                text.get(..=open).unwrap_or(""),
                &text[close..]
            )
        } else {
            format!("{before},\n{body}\n  {}", &text[close..])
        });
    }
    let last = text.rfind('}').ok_or_else(|| {
        OutputError::SceneJson("it has no closing brace to add CONTROLS before".to_string())
    })?;
    let head = text.get(..last).unwrap_or("").trim_end();
    let comma = if head.ends_with('{') { "" } else { "," };
    Ok(format!(
        "{head}{comma}\n  \"CONTROLS\": [\n{body}\n  ]\n{}",
        &text[last..]
    ))
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), OutputError> {
    fs::create_dir_all(to).map_err(io_error(to))?;
    for entry in fs::read_dir(from).map_err(io_error(from))? {
        let entry = entry.map_err(io_error(from))?;
        let source = entry.path();
        let target = to.join(entry.file_name());
        let kind = entry.file_type().map_err(io_error(&source))?;
        if kind.is_dir() {
            copy_tree(&source, &target)?;
        } else {
            fs::copy(&source, &target).map_err(io_error(&source))?;
        }
    }
    Ok(())
}

/// The folder name the result is written to: `warm.synScene` becomes
/// `warm-auto.synScene`.
pub fn output_name(source: &Path) -> String {
    let stem = strobe_scene::load::folder_title(source);
    format!("{stem}-auto.synScene")
}

/// Writes `generated` as a new scene next to `library`, copying `source`.
///
/// Returns the new scene's folder. `replace` allows overwriting an earlier
/// result at the same destination; the source itself is never overwritten.
pub fn write_scene(
    source: &Path,
    library: &Path,
    generated: &Generated,
    replace: bool,
) -> Result<PathBuf, OutputError> {
    let name = output_name(source);
    let destination = library.join(&name);
    let same = match (fs::canonicalize(source), fs::canonicalize(&destination)) {
        (Ok(a), Ok(b)) => a == b,
        _ => false,
    };
    if same {
        return Err(OutputError::SameAsSource(destination));
    }
    if destination.exists() && !replace {
        return Err(OutputError::Exists(destination));
    }
    fs::create_dir_all(library).map_err(io_error(library))?;
    let staging = library.join(format!(".{name}.staging"));
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(io_error(&staging))?;
    }
    copy_tree(source, &staging)?;
    for (file, text) in generated.sources.iter() {
        let path = staging.join(file);
        fs::write(&path, text).map_err(io_error(&path))?;
    }
    let json_path = staging.join("scene.json");
    // A scene with no scene.json gets a fresh one; one that exists but
    // cannot be read is an error, never quietly replaced (its other fields
    // would be lost).
    let original = match fs::read_to_string(&json_path) {
        Ok(text) => Some(text),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(io_error(&json_path)(error)),
    };
    let spliced = splice_controls(original.as_deref(), &generated.control_lines)?;
    fs::write(&json_path, spliced).map_err(io_error(&json_path))?;

    if destination.exists() {
        fs::remove_dir_all(&destination).map_err(io_error(&destination))?;
    }
    fs::rename(&staging, &destination).map_err(io_error(&destination))?;
    Ok(destination)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use strobe_scene::scene::SceneDef;

    const LINE: &str =
        r#"{ "NAME": "zoom", "TYPE": "slider", "MIN": 0.0, "MAX": 2.0, "DEFAULT": 1.0 }"#;

    fn spliced(json: Option<&str>) -> Result<SceneDef> {
        let text = splice_controls(json, &[LINE.to_string()])?;
        Ok(SceneDef::parse(&text, "t")?)
    }

    #[test]
    fn controls_are_added_to_an_existing_array() -> Result<()> {
        let def = spliced(Some(
            "{\n  \"TITLE\": \"x\",\n  \"CONTROLS\": [\n    { \"NAME\": \"a\", \"TYPE\": \"slider\" }\n  ],\n  \"PASSES\": []\n}\n",
        ))?;
        let names: Vec<&str> = def.controls.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["a", "zoom"], "authored first, then the new one");
        assert_eq!(def.title, "x", "other keys survive");
        Ok(())
    }

    #[test]
    fn an_empty_array_and_a_missing_array_both_work() -> Result<()> {
        assert_eq!(
            spliced(Some("{ \"CONTROLS\": [] }"))?.controls.len(),
            1,
            "empty array"
        );
        assert_eq!(
            spliced(Some("{ \"TITLE\": \"x\" }"))?.controls.len(),
            1,
            "no CONTROLS key"
        );
        assert_eq!(spliced(Some("{}"))?.controls.len(), 1, "empty object");
        assert_eq!(spliced(None)?.controls.len(), 1, "no file at all");
        Ok(())
    }

    #[test]
    fn a_nested_controls_key_is_not_mistaken_for_the_scenes() -> Result<()> {
        let text = "{ \"SOURCE\": { \"CONTROLS\": [1] }, \"TITLE\": \"x\" }";
        let def = spliced(Some(text))?;
        assert_eq!(def.controls.len(), 1, "a new top-level CONTROLS");
        let out = splice_controls(Some(text), &[LINE.to_string()])?;
        assert!(
            out.contains("\"CONTROLS\": [1]"),
            "the nested one is untouched: {out}"
        );
        Ok(())
    }

    #[test]
    fn strings_containing_brackets_do_not_confuse_the_scan() -> Result<()> {
        let def = spliced(Some(
            "{ \"CONTROLS\": [ { \"NAME\": \"a\", \"TYPE\": \"slider\", \"DESCRIPTION\": \"]\\\"}\" } ] }",
        ))?;
        assert_eq!(def.controls.len(), 2, "brackets inside strings are text");
        Ok(())
    }

    #[test]
    fn no_new_controls_leaves_the_text_alone() -> Result<()> {
        let text = "{ \"TITLE\": \"x\" }";
        assert_eq!(splice_controls(Some(text), &[])?, text, "unchanged");
        Ok(())
    }

    #[test]
    fn the_output_is_named_beside_the_source_name() {
        assert_eq!(
            output_name(Path::new("scenes/warm-tunnel.synScene")),
            "warm-tunnel-auto.synScene",
            "suffix before .synScene"
        );
    }
}
