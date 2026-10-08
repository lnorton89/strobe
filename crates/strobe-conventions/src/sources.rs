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
//! `tests/` and `examples/` of every crate under `crates/`. Paths are
//! reported relative to the root with forward slashes, so a failure reads
//! the same on Windows as on Linux.

use anyhow::{Result, anyhow};
use std::fs;
use std::path::{Path, PathBuf};

/// The directories inside each crate that hold Rust sources.
pub const AREAS: [&str; 3] = ["src", "tests", "examples"];

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

/// Every Rust source in every crate's `src/`, `tests/` and `examples/`.
pub fn workspace_sources() -> Result<Vec<Source>> {
    let root = workspace_root()?;
    let mut sources = Vec::new();
    for entry in fs::read_dir(root.join("crates"))? {
        let krate = entry?.path();
        if !krate.is_dir() {
            continue;
        }
        for area in AREAS {
            let dir = krate.join(area);
            if !dir.is_dir() {
                continue;
            }
            for path in rust_files(&dir)? {
                let relative = path
                    .strip_prefix(&root)?
                    .to_string_lossy()
                    .replace('\\', "/");
                let text = fs::read_to_string(&path)?;
                sources.push(Source { relative, text });
            }
        }
    }
    sources.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(sources)
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
        Ok(())
    }
}
