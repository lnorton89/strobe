// ============================================================================
// Module:       strobe_app::intake
// Description:  What a file or folder dropped on a window is for: a Shadertoy
//               export, a playlist, media for syn_Media, or a scene folder,
//               and copying a scene folder into the library.
//
// Dependencies: strobe-gfx (assets::list_sources); std (fs)
// ============================================================================

//! Dropped files (backlog I-02, D218).
//!
//! Anything can be dropped on either window, and [`classify`] decides what
//! it is for from the path alone, so a drop and a typed path take the same
//! route:
//!
//! - **a `.json` file** is a Shadertoy export, for the import worker
//!   (unless it is a scene's own `scene.json`, which stands for its folder);
//! - **a `.toml` file** is a playlist, loaded as the LIST pane's Load would;
//! - **a picture, or a folder of pictures**, goes behind `syn_Media`, as the
//!   Inspector's Choose would (D217);
//! - **a scene folder** (one with a `main.glsl`) is played: from where it is
//!   if it is already in the library, else after [`copy_scene`] has copied
//!   it in.
//!
//! Anything else is refused with a sentence saying what can be dropped.

use std::path::{Path, PathBuf};

/// The most files a dropped scene folder may hold to be copied.
pub const MAX_SCENE_FILES: usize = 2000;

/// The most bytes a dropped scene folder may hold to be copied. Scenes are
/// small; a folder past this is more likely a drop by mistake, and the copy
/// runs on the UI thread.
pub const MAX_SCENE_BYTES: u64 = 256 * 1024 * 1024;

/// What a dropped path is for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Dropped {
    /// An exported Shadertoy shader.
    Shadertoy,
    /// A playlist file.
    Playlist,
    /// A picture or a folder of pictures, for `syn_Media`.
    Media,
    /// A scene folder (the dropped folder, or a dropped `scene.json` or
    /// `main.glsl`'s folder).
    Scene(PathBuf),
    /// Nothing Strobe opens; the sentence says what can be dropped.
    Unknown(String),
}

/// What `path` is for (see the module docs). Reads the disk only to tell a
/// folder's kind and whether a `scene.json` has a shader beside it.
pub fn classify(path: &Path) -> Dropped {
    if path.is_dir() {
        if is_scene(path) {
            return Dropped::Scene(path.to_path_buf());
        }
        return match strobe_gfx::assets::list_sources(path) {
            Ok(_) => Dropped::Media,
            Err(_) => Dropped::Unknown(format!(
                "{} holds no scene (main.glsl) and no .png or .jpg pictures.",
                name_of(path)
            )),
        };
    }
    let extension = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let parent = path.parent().filter(|dir| is_scene(dir));
    match extension.as_str() {
        "json" | "glsl" if file_name == "scene.json" || file_name == "main.glsl" => match parent {
            Some(dir) => Dropped::Scene(dir.to_path_buf()),
            None if extension == "json" => Dropped::Unknown(format!(
                "{} has no main.glsl beside it, so it is not a scene.",
                name_of(path)
            )),
            None => unknown(path),
        },
        "json" => Dropped::Shadertoy,
        "toml" => Dropped::Playlist,
        "png" | "jpg" | "jpeg" => Dropped::Media,
        _ => unknown(path),
    }
}

/// What dropping a path of this kind will do, for the banner shown while
/// it hovers over a window.
pub fn hint(kind: &Dropped) -> String {
    match kind {
        Dropped::Shadertoy => "Drop to import this Shadertoy export.".to_string(),
        Dropped::Playlist => "Drop to load this playlist.".to_string(),
        Dropped::Media => "Drop to show this behind syn_Media.".to_string(),
        Dropped::Scene(_) => {
            "Drop to play this scene (one from outside the library is copied in first).".to_string()
        }
        Dropped::Unknown(why) => why.clone(),
    }
}

/// Whether `dir` is a scene folder.
fn is_scene(dir: &Path) -> bool {
    dir.join("main.glsl").is_file()
}

fn unknown(path: &Path) -> Dropped {
    Dropped::Unknown(format!(
        "{} is not something Strobe opens. Drop a scene folder, a playlist (.toml), \
         a picture or a folder of pictures (for syn_Media), or a Shadertoy export (.json).",
        name_of(path)
    ))
}

fn name_of(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

/// Copies the scene folder `from` into `library`, under its own name, and
/// returns where it landed. Refuses a name already in the library, a folder
/// past [`MAX_SCENE_FILES`] or [`MAX_SCENE_BYTES`], and a symbolic link
/// inside it; a copy that fails part-way is removed.
pub fn copy_scene(from: &Path, library: &Path) -> Result<PathBuf, String> {
    let name = from
        .file_name()
        .ok_or_else(|| format!("{} has no folder name", from.display()))?;
    let to = library.join(name);
    if to.exists() {
        return Err(format!(
            "the library already has a scene called {}",
            name.to_string_lossy()
        ));
    }
    let mut files = Vec::new();
    measure(from, from, &mut files)?;
    let bytes: u64 = files.iter().map(|(_, len)| *len).sum();
    if files.len() > MAX_SCENE_FILES || bytes > MAX_SCENE_BYTES {
        return Err(format!(
            "{} holds {} files, {} MB: more than a scene should ({MAX_SCENE_FILES} files, {} MB)",
            from.display(),
            files.len(),
            bytes / (1024 * 1024),
            MAX_SCENE_BYTES / (1024 * 1024)
        ));
    }
    let copied = files.iter().try_for_each(|(relative, _)| {
        let target = to.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("{}: {e}", parent.display()))?;
        }
        std::fs::copy(from.join(relative), &target)
            .map(|_| ())
            .map_err(|e| format!("{}: {e}", target.display()))
    });
    match copied {
        Ok(()) => Ok(to),
        Err(error) => {
            if let Err(cleanup) = std::fs::remove_dir_all(&to) {
                log::warn!(
                    "could not remove the partial copy {}: {cleanup}",
                    to.display()
                );
            }
            Err(error)
        }
    }
}

/// Lists every file under `dir` relative to `root`, with its length, and
/// stops as soon as the bounds are passed (so a huge folder is not walked
/// to the end).
fn measure(root: &Path, dir: &Path, files: &mut Vec<(PathBuf, u64)>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = entry.path();
        let kind = entry
            .file_type()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        if kind.is_symlink() {
            return Err(format!(
                "{} is a link; copy the scene by hand",
                path.display()
            ));
        }
        if kind.is_dir() {
            measure(root, &path, files)?;
        } else {
            let len = entry
                .metadata()
                .map_err(|e| format!("{}: {e}", path.display()))?
                .len();
            let relative = path.strip_prefix(root).map_err(|e| e.to_string())?;
            files.push((relative.to_path_buf(), len));
        }
        if files.len() > MAX_SCENE_FILES {
            return Ok(());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    /// A scratch folder removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Result<Self> {
            let dir =
                std::env::temp_dir().join(format!("strobe-intake-{}-{tag}", std::process::id()));
            if dir.exists() {
                std::fs::remove_dir_all(&dir)?;
            }
            std::fs::create_dir_all(&dir)?;
            Ok(Self(dir))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn scene(dir: &Path) -> Result<()> {
        std::fs::create_dir_all(dir.join("images"))?;
        std::fs::write(
            dir.join("main.glsl"),
            "vec4 renderMain() { return vec4(1.0); }",
        )?;
        std::fs::write(dir.join("scene.json"), "{}")?;
        std::fs::write(dir.join("images/a.png"), b"not really")?;
        Ok(())
    }

    #[test]
    fn each_kind_of_drop_is_told_apart() -> Result<()> {
        let scratch = Scratch::new("classify")?;
        let root = &scratch.0;
        let ink = root.join("ink.synScene");
        scene(&ink)?;
        let pictures = root.join("loop");
        std::fs::create_dir_all(&pictures)?;
        std::fs::write(pictures.join("f1.png"), b"x")?;
        let empty = root.join("empty");
        std::fs::create_dir_all(&empty)?;
        std::fs::write(root.join("loose-scene.json"), "{}")?;

        assert_eq!(
            classify(&ink),
            Dropped::Scene(ink.clone()),
            "a scene folder"
        );
        assert_eq!(
            classify(&ink.join("scene.json")),
            Dropped::Scene(ink.clone()),
            "a scene's scene.json stands for its folder"
        );
        assert_eq!(
            classify(&ink.join("main.glsl")),
            Dropped::Scene(ink.clone()),
            "so does its main.glsl"
        );
        assert_eq!(classify(&pictures), Dropped::Media, "a folder of pictures");
        assert_eq!(classify(&root.join("x.JPG")), Dropped::Media, "a picture");
        assert_eq!(
            classify(&root.join("show.toml")),
            Dropped::Playlist,
            "a playlist"
        );
        assert_eq!(
            classify(&root.join("loose-scene.json")),
            Dropped::Shadertoy,
            "any other .json is a Shadertoy export"
        );
        assert!(
            matches!(classify(&empty), Dropped::Unknown(why) if why.contains("no scene")),
            "an empty folder says why"
        );
        assert!(
            matches!(classify(&root.join("notes.txt")), Dropped::Unknown(why) if why.contains("Drop a scene folder")),
            "anything else says what can be dropped"
        );
        Ok(())
    }

    #[test]
    fn a_scene_is_copied_whole_and_never_over_another() -> Result<()> {
        let scratch = Scratch::new("copy")?;
        let from = scratch.0.join("outside/ink.synScene");
        scene(&from)?;
        let library = scratch.0.join("library");
        std::fs::create_dir_all(&library)?;

        let landed = copy_scene(&from, &library).map_err(anyhow::Error::msg)?;
        assert_eq!(landed, library.join("ink.synScene"), "under its own name");
        assert!(landed.join("main.glsl").is_file(), "the shader came");
        assert!(landed.join("images/a.png").is_file(), "and its subfolders");
        assert!(
            from.join("main.glsl").is_file(),
            "the original is left where it was"
        );

        let again = copy_scene(&from, &library);
        assert!(
            again.as_ref().is_err_and(|why| why.contains("already has")),
            "a second copy is refused, not merged: {again:?}"
        );
        Ok(())
    }
}
