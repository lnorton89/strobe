// ============================================================================
// Module:       strobe_conventions::sources
// Description:  Finds the workspace root and every Rust source file the house
//               rules apply to.
//
// Dependencies: anyhow, std::fs
// ============================================================================

//! Source discovery shared by every rule in this crate.
//!
//! The rules are workspace-wide, so the walk starts at the workspace root
//! (two directories above this crate's manifest) and visits `src/`,
//! `tests/` and `examples/` of every crate under `crates/`, and the crate's
//! `build.rs` where it has one (D497). Paths are reported relative to the
//! root with forward slashes, so a failure reads the same on Windows as on
//! Linux.
//!
//! [`enclosing_function`] is the one piece of Rust "parsing" the rules
//! share: which `fn` a line belongs to, found by looking upwards.

use anyhow::{Result, anyhow};
use std::fs;
use std::path::{Path, PathBuf};

/// The directories inside each crate that hold Rust sources.
pub const AREAS: [&str; 3] = ["src", "tests", "examples"];

/// The build script a crate may have beside its manifest.
pub const BUILD_SCRIPT: &str = "build.rs";

/// The workspace root: `crates/strobe-conventions/../..`.
pub fn workspace_root() -> Result<PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("{} has no workspace root above it", manifest.display()))
}

/// Every `.rs` file under `root`, sorted, found iteratively.
pub fn rust_files(root: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(dir) = pending.pop() {
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rs") {
                found.push(path);
            }
        }
    }
    found.sort();
    Ok(found)
}

/// One source file: its root-relative path and its text.
#[derive(Debug)]
pub struct Source {
    /// Path relative to the workspace root, `/`-separated.
    pub relative: String,
    /// The file's contents.
    pub text: String,
}

/// Every Rust source in every crate's `src/`, `tests/` and `examples/`,
/// and every crate's `build.rs`.
pub fn workspace_sources() -> Result<Vec<Source>> {
    let root = workspace_root()?;
    let mut sources = Vec::new();
    for entry in fs::read_dir(root.join("crates"))? {
        let krate = entry?.path();
        if !krate.is_dir() {
            continue;
        }
        let mut paths = Vec::new();
        for area in AREAS {
            let dir = krate.join(area);
            if dir.is_dir() {
                paths.extend(rust_files(&dir)?);
            }
        }
        let build = krate.join(BUILD_SCRIPT);
        if build.is_file() {
            paths.push(build);
        }
        for path in paths {
            let relative = path
                .strip_prefix(&root)?
                .to_string_lossy()
                .replace('\\', "/");
            let text = fs::read_to_string(&path)?;
            sources.push(Source { relative, text });
        }
    }
    sources.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(sources)
}

/// The line of the nearest named function at or above `line`: the `fn` a
/// line belongs to, for a file written in the workspace's style (one item
/// per line, no nested named functions after the code that uses them).
pub fn enclosing_function(lines: &[&str], line: usize) -> Option<usize> {
    (0..=line).rev().find(|index| {
        let trimmed = lines.get(*index).map_or("", |text| text.trim_start());
        [
            "fn ",
            "pub fn ",
            "pub(crate) fn ",
            "pub(super) fn ",
            "const fn ",
        ]
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
    })
}

/// The name of the function declared on `line` (`fn name(` or `fn name<`).
pub fn function_name(line: &str) -> Option<&str> {
    let (_, rest) = line.split_once("fn ")?;
    let end = rest.find(|c: char| !(c.is_alphanumeric() || c == '_'))?;
    rest.get(..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_workspace_root_holds_the_workspace_manifest() -> Result<()> {
        let root = workspace_root()?;
        let manifest = fs::read_to_string(root.join("Cargo.toml"))?;
        assert!(
            manifest.contains("[workspace]"),
            "{} is not the workspace root",
            root.display()
        );
        Ok(())
    }

    #[test]
    fn the_walk_finds_this_file() -> Result<()> {
        let sources = workspace_sources()?;
        assert!(
            sources
                .iter()
                .any(|source| source.relative == "crates/strobe-conventions/src/sources.rs"),
            "the source walk should find its own file"
        );
        assert!(
            sources
                .iter()
                .any(|source| source.relative == "crates/strobe-diag/build.rs"),
            "the source walk should find the crates' build scripts"
        );
        Ok(())
    }

    #[test]
    fn a_line_belongs_to_the_function_above_it() {
        let lines = [
            "impl Gl {",
            "    pub fn error(&self) -> Option<u32> {",
            "        let code = 0;",
            "    }",
            "    fn get_i32<T>(&self) -> i32 {",
            "        0",
            "    }",
        ];
        assert_eq!(enclosing_function(&lines, 2), Some(1), "inside `error`");
        assert_eq!(enclosing_function(&lines, 5), Some(4), "inside `get_i32`");
        assert_eq!(enclosing_function(&lines, 0), None, "above every function");
        assert_eq!(function_name(lines[1]), Some("error"), "a plain name");
        assert_eq!(function_name(lines[4]), Some("get_i32"), "a generic one");
    }
}
