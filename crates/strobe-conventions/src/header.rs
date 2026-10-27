// ============================================================================
// Module:       strobe_conventions::header
// Description:  Enforces the file-header convention: every source file opens
//               with a banner naming its module, purpose, and dependencies.
//
// Dependencies: anyhow; crate::sources (the workspace walk)
// ============================================================================

//! The file-header convention.
//!
//! The banner at the top of a file is the one piece of documentation
//! nothing else can imply: a reader opening a module cold gets its path
//! in the workspace, what it is for, and what it leans on, before reading
//! a line of code.
//!
//! The check is structural, not editorial. It verifies that the fields
//! are present and filled in, and that `Module:` matches the path the
//! file actually sits at — which is what catches a header copied from a
//! neighbour and never re-read.
//!
//! The banner is plain `//`, not `//!`, and is deliberately not rustdoc.
//! The `//!` block below it is the documentation, and that is where the
//! reasoning belongs.
//!
//! Module paths are derived per crate: `crates/strobe-scene/src/control.rs`
//! is `strobe_scene::control`, a crate's `lib.rs` is
//! `strobe_scene (library crate root)`, its `main.rs` is
//! `strobe_app (binary crate root)`, and files under `tests/` and
//! `examples/` are `name (integration test)` and `name (example)`. A
//! crate's `build.rs` is `build (build script)` (D497).

use crate::sources::BUILD_SCRIPT;
use anyhow::{Result, anyhow};

/// The rule above and below every header.
pub const RULE: &str =
    "// ============================================================================";

/// Width of the label column, so `Description:` and its continuation
/// lines share one left edge.
const LABEL_WIDTH: usize = 14;

/// Fields every header must carry.
const REQUIRED: [&str; 3] = ["Module", "Description", "Dependencies"];

/// rustfmt's default `max_width`. A header that runs past it wraps in an
/// editor and stops lining up.
const MAX_WIDTH: usize = 100;

/// The `Module:` value a file at `relative` (workspace-relative) must
/// declare.
pub fn expected_module(relative: &str) -> Result<String> {
    let rest = relative
        .strip_prefix("crates/")
        .ok_or_else(|| anyhow!("{relative} is not inside crates/"))?;
    let (krate, rest) = rest
        .split_once('/')
        .ok_or_else(|| anyhow!("{relative} is not inside a crate"))?;
    if rest == BUILD_SCRIPT {
        return Ok("build (build script)".to_string());
    }
    let (area, rest) = rest
        .split_once('/')
        .ok_or_else(|| anyhow!("{relative} is not under a checked directory"))?;
    let stem = rest
        .strip_suffix(".rs")
        .ok_or_else(|| anyhow!("{relative} is not a Rust source file"))?;
    let ident = krate.replace('-', "_");

    match area {
        "tests" => return Ok(format!("{stem} (integration test)")),
        "examples" => return Ok(format!("{stem} (example)")),
        _ => {}
    }
    match stem {
        "lib" => return Ok(format!("{ident} (library crate root)")),
        "main" => return Ok(format!("{ident} (binary crate root)")),
        _ => {}
    }
    // `foo/mod.rs` documents `foo`, not `foo::mod`.
    let module = stem.strip_suffix("/mod").unwrap_or(stem);
    Ok(format!("{ident}::{}", module.replace('/', "::")))
}

/// One field's value, with any aligned continuation lines folded in.
fn field(body: &[&str], label: &str) -> Option<String> {
    let prefix = format!("// {label}:");
    let start = body.iter().position(|line| line.starts_with(&prefix))?;
    let indent = format!("// {}", " ".repeat(LABEL_WIDTH));

    let mut value = body.get(start)?.get(prefix.len()..)?.trim().to_string();
    for line in body.get(start + 1..)? {
        let Some(rest) = line.strip_prefix(&indent) else {
            break;
        };
        value.push(' ');
        value.push_str(rest.trim());
    }
    Some(value)
}

/// Every problem with one file's header, rather than only the first.
pub fn problems(relative: &str, text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let lines: Vec<&str> = text.lines().collect();

    if lines.first() != Some(&RULE) {
        found.push(format!("{relative}: does not open with the header rule"));
        return found;
    }
    let Some(close) = lines.iter().skip(1).position(|line| *line == RULE) else {
        found.push(format!("{relative}: header rule is never closed"));
        return found;
    };
    let Some(body) = lines.get(1..=close) else {
        found.push(format!("{relative}: header body could not be read"));
        return found;
    };

    for line in body {
        if !line.starts_with("// ") && *line != "//" {
            found.push(format!(
                "{relative}: non-comment line inside header: {line}"
            ));
        }
        if line.chars().count() > MAX_WIDTH {
            found.push(format!(
                "{relative}: header line exceeds {MAX_WIDTH} columns"
            ));
        }
    }

    for label in REQUIRED {
        let Some(value) = field(body, label) else {
            found.push(format!("{relative}: header is missing `{label}:`"));
            continue;
        };
        if value.is_empty() {
            found.push(format!("{relative}: `{label}:` is empty"));
        } else if value.starts_with('[') {
            found.push(format!(
                "{relative}: `{label}:` still holds the template placeholder"
            ));
        }
    }

    match (field(body, "Module"), expected_module(relative)) {
        (Some(declared), Ok(expected)) if declared != expected => found.push(format!(
            "{relative}: declares `Module: {declared}` but sits at `{expected}`"
        )),
        (_, Err(error)) => found.push(format!("{relative}: {error}")),
        (Some(_), Ok(_)) | (None, Ok(_)) => {}
    }

    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::workspace_sources;

    #[test]
    fn every_source_file_carries_a_module_header() -> Result<()> {
        let sources = workspace_sources()?;
        let failures: Vec<String> = sources
            .iter()
            .flat_map(|source| problems(&source.relative, &source.text))
            .collect();
        assert!(!sources.is_empty(), "found no source files to check");
        assert!(
            failures.is_empty(),
            "{} file header problem(s) across {} file(s):\n{}",
            failures.len(),
            sources.len(),
            failures.join("\n")
        );
        Ok(())
    }

    #[test]
    fn a_module_path_is_derived_from_where_the_file_sits() -> Result<()> {
        let cases = [
            (
                "crates/strobe-scene/src/lib.rs",
                "strobe_scene (library crate root)",
            ),
            (
                "crates/strobe-app/src/main.rs",
                "strobe_app (binary crate root)",
            ),
            (
                "crates/strobe-scene/src/control.rs",
                "strobe_scene::control",
            ),
            ("crates/strobe-gfx/src/gl/mod.rs", "strobe_gfx::gl"),
            (
                "crates/strobe-gfx/src/gl/context.rs",
                "strobe_gfx::gl::context",
            ),
            (
                "crates/strobe-gfx/tests/render.rs",
                "render (integration test)",
            ),
            ("crates/strobe-app/examples/shot.rs", "shot (example)"),
            ("crates/strobe-diag/build.rs", "build (build script)"),
        ];
        for (path, expected) in cases {
            assert_eq!(
                expected_module(path)?,
                expected,
                "wrong module path derived for {path}"
            );
        }
        Ok(())
    }

    #[test]
    fn a_header_naming_the_wrong_module_is_rejected() {
        let text = format!(
            "{RULE}\n\
             // Module:       strobe_scene::control\n\
             // Description:  Something.\n\
             //\n\
             // Dependencies: None.\n\
             {RULE}\n"
        );
        let found = problems("crates/strobe-scene/src/pass.rs", &text);
        assert!(
            found.iter().any(|problem| problem.contains("but sits at")),
            "a mismatched Module: should be reported, got {found:?}"
        );
    }

    #[test]
    fn a_missing_or_placeholder_field_is_rejected() {
        let missing = format!(
            "{RULE}\n\
             // Module:       strobe_scene::pass\n\
             // Dependencies: None.\n\
             {RULE}\n"
        );
        let found = problems("crates/strobe-scene/src/pass.rs", &missing);
        assert!(
            found.iter().any(|problem| problem.contains("Description:")),
            "a missing Description: should be reported, got {found:?}"
        );

        let placeholder = format!(
            "{RULE}\n\
             // Module:       strobe_scene::pass\n\
             // Description:  [Brief description of the file's purpose]\n\
             //\n\
             // Dependencies: None.\n\
             {RULE}\n"
        );
        let found = problems("crates/strobe-scene/src/pass.rs", &placeholder);
        assert!(
            found.iter().any(|problem| problem.contains("placeholder")),
            "an unfilled template should be reported, got {found:?}"
        );
    }

    #[test]
    fn a_file_without_a_closed_header_is_rejected() {
        let found = problems("crates/strobe-scene/src/pass.rs", "//! Just docs.\n");
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("does not open with")),
            "got {found:?}"
        );
        let unclosed = format!("{RULE}\n// Module:       strobe_scene::pass\n");
        let found = problems("crates/strobe-scene/src/pass.rs", &unclosed);
        assert!(
            found.iter().any(|problem| problem.contains("never closed")),
            "got {found:?}"
        );
    }

    #[test]
    fn a_field_wrapped_onto_the_label_column_reads_as_one_value() {
        let body = [
            "// Module:       strobe_scene::pass",
            "// Description:  First half of the sentence",
            "//               and its continuation.",
            "//",
            "// Dependencies: serde",
        ];
        assert_eq!(
            field(&body, "Description"),
            Some("First half of the sentence and its continuation.".to_string()),
            "continuation lines should fold into the field they belong to"
        );
        assert_eq!(
            field(&body, "Module"),
            Some("strobe_scene::pass".to_string()),
            "a field must not swallow the field below it"
        );
    }
}
