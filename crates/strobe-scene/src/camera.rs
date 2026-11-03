// ============================================================================
// Module:       strobe_scene::camera
// Description:  How a media choice names a camera instead of a file:
//               `camera:<name>`, shared by the operator's choice, cues and
//               presets.
//
// Dependencies: std only
// ============================================================================

//! A camera as a media choice (backlog M-04, D528).
//!
//! Everywhere a media choice is a path (the operator's, a cue's `media`, a
//! preset's media channel), a camera is the path `camera:<name>`, `<name>`
//! being what the system lists it as. One spelling keeps every store of a
//! media choice, and the precedence between them (D217), as it is. No file
//! is looked for at such a path, and it is never joined to a folder: a
//! cue's or a preset's camera is the same camera wherever the playlist or
//! scene lives.
//!
//! A scene's own `MEDIA` key cannot name a camera; it names a file in the
//! scene's folder, as in Synesthesia.

use std::path::{Path, PathBuf};

/// The prefix that makes a media path a camera.
pub const PREFIX: &str = "camera:";

/// The camera `path` names, or `None` for a file or folder.
pub fn name_of(path: &Path) -> Option<&str> {
    let name = path.to_str()?.strip_prefix(PREFIX)?.trim();
    (!name.is_empty()).then_some(name)
}

/// The media path for the camera called `name`.
pub fn path_for(name: &str) -> PathBuf {
    PathBuf::from(format!("{PREFIX}{}", name.trim()))
}

/// `relative` resolved against `dir`, unless it names a camera, which is
/// returned as it is.
pub fn resolve(dir: &Path, relative: &Path) -> PathBuf {
    match name_of(relative) {
        Some(_) => relative.to_path_buf(),
        None => dir.join(relative),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_camera_round_trips_and_a_file_is_not_one() {
        let path = path_for("Integrated Camera");
        assert_eq!(name_of(&path), Some("Integrated Camera"), "{path:?}");
        assert_eq!(name_of(Path::new("loops/rain")), None, "a folder");
        assert_eq!(name_of(Path::new("camera:")), None, "no name");
        assert_eq!(
            name_of(Path::new("cameras/x.png")),
            None,
            "a folder called cameras"
        );
    }

    #[test]
    fn a_camera_is_never_joined_to_a_folder() {
        let dir = Path::new("scenes/a");
        assert_eq!(
            resolve(dir, Path::new("camera:USB Cam")),
            PathBuf::from("camera:USB Cam"),
            "as it is"
        );
        assert_eq!(
            resolve(dir, Path::new("pic.png")),
            dir.join("pic.png"),
            "a file is"
        );
    }
}
