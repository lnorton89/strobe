// ============================================================================
// Module:       strobe_playlist::file
// Description:  The crate's only file I/O: load and save a playlist, and the
//               atomic write both it and resume points use.
//
// Dependencies: crate::parse, crate::write, crate::error, std::fs
// ============================================================================

//! Loading and saving.
//!
//! Everything else in the crate is pure. These functions are the explicit
//! edge: they read or write one path and say which path when they fail.
//!
//! Saves are atomic: the text goes to a temporary file in the same
//! directory, which is then renamed over the target. The show runs on a
//! machine that may lose power; a half-written playlist or resume file is
//! worse than an old one.

use crate::error::PlaylistError;
use crate::model::Playlist;
use crate::{parse, write};
use std::fs;
use std::path::{Path, PathBuf};

/// Reads and parses a playlist file.
pub fn load(path: &Path) -> Result<Playlist, PlaylistError> {
    let text = fs::read_to_string(path).map_err(|source| PlaylistError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(parse::parse(&text)?)
}

/// Writes `playlist` to `path`, atomically.
pub fn save(playlist: &Playlist, path: &Path) -> Result<(), PlaylistError> {
    write_atomic(path, &write::to_toml(playlist))
}

/// Writes `text` to `path` by way of a sibling temporary file.
pub(crate) fn write_atomic(path: &Path, text: &str) -> Result<(), PlaylistError> {
    let io = |source| PlaylistError::Io {
        path: path.to_path_buf(),
        source,
    };
    let temp = temp_path(path);
    fs::write(&temp, text).map_err(io)?;
    if let Err(source) = fs::rename(&temp, path) {
        // Do not leave the temporary file behind if the rename failed.
        let _ = fs::remove_file(&temp);
        return Err(io(source));
    }
    Ok(())
}

/// `dir/name.ext` becomes `dir/.name.ext.tmp`.
fn temp_path(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    path.with_file_name(format!(".{name}.tmp"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Cue, SceneRef, Section};
    use anyhow::Result;

    /// A scratch directory under the system temp dir, removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Result<Self> {
            let dir =
                std::env::temp_dir().join(format!("strobe-playlist-{tag}-{}", std::process::id()));
            fs::create_dir_all(&dir)?;
            Ok(Self(dir))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn save_then_load_round_trips_and_leaves_no_temp_file() -> Result<()> {
        let scratch = Scratch::new("save")?;
        let path = scratch.0.join("show.toml");
        let mut playlist = Playlist::new("show");
        let mut section = Section::new("s");
        section.cues.push(Cue::new(SceneRef::Library("a".into())));
        playlist.sections.push(section);

        save(&playlist, &path)?;
        assert_eq!(load(&path)?, playlist, "loaded playlist differs");
        let leftovers: Vec<_> = fs::read_dir(&scratch.0)?
            .filter_map(Result::ok)
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            leftovers,
            vec!["show.toml".to_string()],
            "the temp file was left behind"
        );

        // Saving again replaces the file.
        playlist.name = "renamed".into();
        save(&playlist, &path)?;
        assert_eq!(
            load(&path)?.name,
            "renamed",
            "second save did not replace the first"
        );
        Ok(())
    }

    #[test]
    fn errors_name_the_file_or_the_line() -> Result<()> {
        let scratch = Scratch::new("errors")?;
        let missing = scratch.0.join("nope.toml");
        let error = load(&missing)
            .err()
            .map(|e| e.to_string())
            .unwrap_or_default();
        assert!(
            error.contains("nope.toml"),
            "a missing file should be named: {error}"
        );

        let bad = scratch.0.join("bad.toml");
        fs::write(&bad, "[defaults]\nduration = \"3mn\"\n")?;
        let error = load(&bad).err().map(|e| e.to_string()).unwrap_or_default();
        assert!(
            error.contains("line 2") && error.contains("\"3mn\""),
            "a parse error: {error}"
        );
        Ok(())
    }
}
