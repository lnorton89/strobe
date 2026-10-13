// ============================================================================
// Module:       strobe_shadertoy::convert::write
// Description:  Puts a converted scene on disk atomically: a hidden temporary
//               folder, then one rename.
//
// Dependencies: std::fs; super::Converted
// ============================================================================

//! Writing a scene folder.
//!
//! A half-written scene must never be visible to the scene library, which
//! lists any folder holding a `main.glsl`. So the files go into a hidden
//! sibling folder first (`main.glsl` last of all, in case something lists
//! that folder anyway), and a single `rename` moves it to its real name.
//! A crash leaves at worst a `.name.importing` folder, which the next
//! import of the same scene clears.

use super::Converted;
use std::fs;
use std::path::{Path, PathBuf};

/// Why a scene could not be written.
#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    /// The scene folder is already there and replacing was not asked for.
    #[error("{0} already exists (use --force to replace it)")]
    AlreadyExists(PathBuf),
    /// A file-system operation failed.
    #[error("{action} {path}: {source}")]
    Io {
        /// What was being done.
        action: &'static str,
        /// To what.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// The new scene could not be moved in, and the old one could not be
    /// put back either: it is still whole, under a hidden name.
    #[error(
        "rename {staging}: {source}; the previous scene could not be put back \
         ({restore}) and is kept at {kept}"
    )]
    NotRestored {
        /// The new scene that could not be moved in.
        staging: PathBuf,
        /// Why it could not.
        source: std::io::Error,
        /// Where the previous scene is.
        kept: PathBuf,
        /// Why it could not be put back.
        restore: std::io::Error,
    },
}

fn io(action: &'static str, path: &Path) -> impl FnOnce(std::io::Error) -> WriteError {
    let path = path.to_path_buf();
    move |source| WriteError::Io {
        action,
        path,
        source,
    }
}

/// Writes `converted` as a folder inside `library`, returning its path.
/// With `replace`, an existing scene of the same name is swapped out.
pub fn write(converted: &Converted, library: &Path, replace: bool) -> Result<PathBuf, WriteError> {
    let target = library.join(&converted.folder_name);
    if target.exists() && !replace {
        return Err(WriteError::AlreadyExists(target));
    }
    fs::create_dir_all(library).map_err(io("create", library))?;

    let staging = library.join(format!(".{}.importing", converted.folder_name));
    if staging.exists() {
        fs::remove_dir_all(&staging).map_err(io("clear", &staging))?;
    }
    fs::create_dir_all(&staging).map_err(io("create", &staging))?;

    let result = fill(converted, &staging).and_then(|()| swap_in(&staging, &target));
    if result.is_err() {
        // Best effort: the error being returned matters more.
        let _ = fs::remove_dir_all(&staging);
    }
    result.map(|()| target)
}

/// Writes every file under `staging`, `main.glsl` last.
fn fill(converted: &Converted, staging: &Path) -> Result<(), WriteError> {
    let ordered = converted
        .files
        .iter()
        .filter(|(path, _)| path.as_str() != "main.glsl")
        .chain(
            converted
                .files
                .iter()
                .filter(|(path, _)| path.as_str() == "main.glsl"),
        );
    for (relative, bytes) in ordered {
        let path = staging.join(relative);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(io("create", parent))?;
        }
        fs::write(&path, bytes).map_err(io("write", &path))?;
    }
    Ok(())
}

/// Moves `staging` to `target`, setting any existing `target` aside first
/// so a failure puts it back.
fn swap_in(staging: &Path, target: &Path) -> Result<(), WriteError> {
    if !target.exists() {
        return fs::rename(staging, target).map_err(io("rename", staging));
    }
    let old = target.with_file_name(format!(
        ".{}.replaced",
        target
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    ));
    if old.exists() {
        fs::remove_dir_all(&old).map_err(io("clear", &old))?;
    }
    fs::rename(target, &old).map_err(io("rename", target))?;
    if let Err(error) = fs::rename(staging, target) {
        // Put the previous scene back. If even that fails, say where it is:
        // it is whole, only renamed, and must not look lost.
        return Err(match fs::rename(&old, target) {
            Ok(()) => io("rename", staging)(error),
            Err(restore) => WriteError::NotRestored {
                staging: staging.to_path_buf(),
                source: error,
                kept: old,
                restore,
            },
        });
    }
    fs::remove_dir_all(&old).map_err(io("remove", &old))
}
