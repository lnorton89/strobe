// ============================================================================
// Module:       strobe_scene::load
// Description:  Reads a .synScene folder from disk into a validated scene plus
//               its shader and script source.
//
// Dependencies: std::fs; crate::scene, crate::error
// ============================================================================

//! Loading a scene folder.
//!
//! The only file access in this crate. A folder needs `main.glsl`;
//! `scene.json` is optional (a bare shader is a valid scene with no
//! controls), and so is `script.js`.

use crate::error::SceneError;
use crate::scene::SceneDef;
use std::fs;
use std::path::{Path, PathBuf};

/// A scene folder, read and validated.
#[derive(Debug, Clone)]
pub struct SceneFolder {
    /// The folder.
    pub dir: PathBuf,
    /// The validated `scene.json`.
    pub def: SceneDef,
    /// `main.glsl`.
    pub shader: String,
    /// `script.js`, if present.
    pub script: Option<String>,
}

impl SceneFolder {
    /// Reads and validates the scene at `dir`.
    pub fn load(dir: &Path) -> Result<Self, SceneError> {
        let read = |name: &str| -> Result<Option<String>, SceneError> {
            let path = dir.join(name);
            match fs::read_to_string(&path) {
                Ok(text) => Ok(Some(text)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(source) => Err(SceneError::Io { path, source }),
            }
        };
        let shader =
            read("main.glsl")?.ok_or_else(|| SceneError::MissingShader(dir.to_path_buf()))?;
        let json = read("scene.json")?.unwrap_or_else(|| "{}".to_string());
        let def = SceneDef::parse(&json, &folder_title(dir))?;
        let script = read("script.js")?;
        Ok(Self {
            dir: dir.to_path_buf(),
            def,
            shader,
            script,
        })
    }
}

/// A title from the folder name: `warm-tunnel.synScene` → `warm-tunnel`.
pub fn folder_title(dir: &Path) -> String {
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    name.strip_suffix(".synScene").unwrap_or(&name).to_string()
}

/// Every scene folder directly inside `library`, sorted by name.
///
/// A scene folder is any directory containing `main.glsl`, with or without
/// the `.synScene` suffix.
pub fn scan(library: &Path) -> Result<Vec<PathBuf>, SceneError> {
    let entries = fs::read_dir(library).map_err(|source| SceneError::Io {
        path: library.to_path_buf(),
        source,
    })?;
    let mut found: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.join("main.glsl").is_file())
        .collect();
    found.sort();
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_comes_from_the_folder_without_its_suffix() {
        assert_eq!(
            folder_title(Path::new("lib/warm-tunnel.synScene")),
            "warm-tunnel",
            "suffix"
        );
        assert_eq!(folder_title(Path::new("lib/plain")), "plain", "no suffix");
    }
}
