// ============================================================================
// Module:       strobe_playlist::file
// Description:  The crate's only file I/O: load and save a playlist, and the
//               atomic write both it and resume points use.
//
// Dependencies: crate::parse, crate::write, crate::error, std::fs, std::io
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
use crate::{merge, parse, write};
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

/// Writes `playlist` to `path`, atomically. Over an existing file, the
/// file's comments and layout are kept wherever the content still matches
/// ([`merge::keep_layout`], D231); if that text would not read back to
/// exactly `playlist`, the canonical text is written instead.
pub fn save(playlist: &Playlist, path: &Path) -> Result<(), PlaylistError> {
    let fresh = write::to_toml(playlist);
    let text = match fs::read_to_string(path) {
        Ok(original) => {
            let merged = merge::keep_layout(&original, &fresh);
            match parse::parse(&merged) {
                Ok(read) if read == *playlist => merged,
                Ok(_) | Err(_) => fresh,
            }
        }
        Err(_) => fresh,
    };
    write_atomic(path, &text)
}

/// Writes `text` to `path` by way of a sibling temporary file. The
/// temporary file is synced before the rename: without it, a power cut soon
/// after the rename can leave an empty file on NTFS, and the next start then
/// has no show or no resume point (D438).
pub(crate) fn write_atomic(path: &Path, text: &str) -> Result<(), PlaylistError> {
    use std::io::Write as _;
    let io = |source| PlaylistError::Io {
        path: path.to_path_buf(),
        source,
    };
    let temp = temp_path(path);
    let written = fs::File::create(&temp).and_then(|mut file| {
        file.write_all(text.as_bytes())?;
        file.sync_all()
    });
    if let Err(source) = written {
        // Do not leave a partial temporary file behind.
        let _ = fs::remove_file(&temp);
        return Err(io(source));
    }
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
    fn saving_over_a_commented_file_keeps_its_comments() -> Result<()> {
        let scratch = Scratch::new("comments")?;
        let path = scratch.0.join("show.toml");
        fs::write(
            &path,
            "# Friday.\nname = \"show\"\n\n[[sections]]\nname = \"s\"\ncues = [\n  # opener\n  { scene = \"a\" },\n]\n",
        )?;
        let mut playlist = load(&path)?;
        if let Some(section) = playlist.sections.first_mut() {
            section.cues.push(Cue::new(SceneRef::Library("b".into())));
        }
        save(&playlist, &path)?;
        let text = fs::read_to_string(&path)?;
        assert!(text.starts_with("# Friday."), "the header stays: {text}");
        assert!(text.contains("# opener"), "the cue's comment stays: {text}");
        assert_eq!(load(&path)?, playlist, "and it reads back to the edit");
        Ok(())
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
