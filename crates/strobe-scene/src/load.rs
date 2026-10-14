// ============================================================================
// Module:       strobe_scene::load
// Description:  Reads a .synScene folder from disk into a validated scene plus
//               its shader and script source.
//
// Dependencies: std::fs; crate::scene, crate::error, crate::include
// ============================================================================

//! Loading a scene folder.
//!
//! The file access in this crate, beside `include` and `synesthesia`. A
//! folder needs `main.glsl`; `scene.json` is optional (a bare shader is a
//! valid scene with no controls), and so is `script.js`. [`scan_deep`]
//! searches a folder recursively, as Synesthesia searches the scene
//! folders in its settings (D271).
//!
//! A scene that sets `PASS_SOURCES` also has one shader file per pass,
//! read here too. `main.glsl` stays mandatory even then (the final pass's
//! file is normally `main.glsl` itself): it is what [`scan`] looks for to
//! tell a scene folder from any other folder.

use crate::error::SceneError;
use crate::scene::SceneDef;
use std::fs;
use std::path::{Path, PathBuf};

/// The file name errors in a pass are reported against when the scene has
/// no `PASS_SOURCES`.
pub const MAIN_SHADER: &str = "main.glsl";

/// A scene folder, read and validated.
#[derive(Debug, Clone)]
pub struct SceneFolder {
    /// The folder.
    pub dir: PathBuf,
    /// The validated `scene.json`.
    pub def: SceneDef,
    /// `main.glsl`.
    pub shader: String,
    /// The source of each pass, in pass order, when the scene sets
    /// `PASS_SOURCES`; empty otherwise.
    pub pass_shaders: Vec<String>,
    /// `script.js`, if present.
    pub script: Option<String>,
}

impl SceneFolder {
    /// The GLSL that pass `index` runs: its own file under `PASS_SOURCES`,
    /// `main.glsl` for every pass otherwise.
    pub fn shader_for(&self, index: usize) -> &str {
        self.pass_shaders
            .get(index)
            .map_or(self.shader.as_str(), String::as_str)
    }

    /// The file name pass `index`'s code lives in, for error messages.
    pub fn file_for(&self, index: usize) -> &str {
        self.def
            .pass_sources
            .as_ref()
            .and_then(|files| files.get(index))
            .map_or(MAIN_SHADER, String::as_str)
    }

    /// Reads and validates the scene at `dir`.
    pub fn load(dir: &Path) -> Result<Self, SceneError> {
        let read = |name: &str| -> Result<Option<String>, SceneError> {
            let path = dir.join(name);
            match fs::read_to_string(&path) {
                // Windows line endings, as 50 of 64 real Synesthesia shaders
                // have, break `\` continuations on ANGLE and double its
                // error line numbers (TORNADO T2, D193). One newline form
                // everywhere costs nothing on the desktop path.
                Ok(text) if text.contains('\r') => Ok(Some(newlines(&text))),
                Ok(text) => Ok(Some(text)),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
                Err(source) => Err(SceneError::Io { path, source }),
            }
        };
        let shader =
            read("main.glsl")?.ok_or_else(|| SceneError::MissingShader(dir.to_path_buf()))?;
        let json = read("scene.json")?.unwrap_or_else(|| "{}".to_string());
        let def = SceneDef::parse(&json, &folder_title(dir))?;
        let pass_shaders = def
            .pass_sources
            .iter()
            .flatten()
            .map(|file| read(file)?.ok_or_else(|| SceneError::missing_file(dir, file)))
            .collect::<Result<Vec<_>, _>>()?;
        let script = read("script.js")?;
        let search = crate::include::search_path();
        let shader = crate::include::expand(shader, dir, &search);
        let pass_shaders = pass_shaders
            .into_iter()
            .map(|text| crate::include::expand(text, dir, &search))
            .collect();
        Ok(Self {
            dir: dir.to_path_buf(),
            def,
            shader,
            pass_shaders,
            script,
        })
    }
}

/// `text` with every line ending as one `\n`.
///
/// `\r\n` is Windows. A run of `\r` before a `\n` is one line ending too:
/// 38 of the owner's 65 Synesthesia scenes end their lines `\r\r\n`
/// (TORNADO T6). Drivers count those by the `\n`, so a `\` continuation
/// before one breaks and editors disagree with the driver about line
/// numbers. A `\r` with no `\n` after it (classic Mac) is a line ending of
/// its own.
pub fn newlines(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\r' {
            out.push(c);
            continue;
        }
        let mut run = 1;
        while chars.next_if_eq(&'\r').is_some() {
            run += 1;
        }
        if chars.next_if_eq(&'\n').is_some() {
            out.push('\n');
        } else {
            out.extend(std::iter::repeat_n('\n', run));
        }
    }
    out
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

/// The most folders [`scan_deep`] looks into, so a scene folder setting that
/// names a whole drive cannot stall the caller.
const MAX_VISITS: usize = 20_000;

/// Every scene folder under `root` down to `depth` levels (1 is [`scan`]'s
/// children only), sorted by path: how Synesthesia searches a scene folder
/// recursively (D271). A scene folder is not looked inside, folders behind
/// links are not descended into, a subfolder that cannot be read is
/// skipped, and the walk stops after [`MAX_VISITS`] folders.
pub fn scan_deep(root: &Path, depth: usize) -> Result<Vec<PathBuf>, SceneError> {
    let entries = fs::read_dir(root).map_err(|source| SceneError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    let mut found = Vec::new();
    let mut visits = 0;
    walk(entries, depth, &mut found, &mut visits);
    found.sort();
    Ok(found)
}

/// [`scan_deep`]'s walk over one folder's `entries`.
fn walk(entries: fs::ReadDir, depth: usize, found: &mut Vec<PathBuf>, visits: &mut usize) {
    for entry in entries.filter_map(Result::ok) {
        if *visits >= MAX_VISITS {
            return;
        }
        *visits += 1;
        let path = entry.path();
        if path.join("main.glsl").is_file() {
            found.push(path);
            continue;
        }
        // `file_type` does not follow links, so a link is never descended.
        let folder = entry.file_type().is_ok_and(|kind| kind.is_dir());
        if depth > 1
            && folder
            && let Ok(inner) = fs::read_dir(&path)
        {
            walk(inner, depth - 1, found, visits);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deep_scan_finds_scenes_to_its_depth_and_not_inside_scenes() -> anyhow::Result<()> {
        let root = std::env::temp_dir().join(format!("strobe-scan-deep-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for dir in [
            "top.synScene",
            "a/b.synScene",
            "a/c/d.synScene",
            "top.synScene/inner",
        ] {
            fs::create_dir_all(root.join(dir))?;
            fs::write(
                root.join(dir).join("main.glsl"),
                "vec4 renderMain(void) { return vec4(1.0); }",
            )?;
        }
        let names = |found: Vec<PathBuf>| -> Vec<String> {
            found
                .iter()
                .filter_map(|path| path.strip_prefix(&root).ok())
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .collect()
        };
        assert_eq!(
            names(scan_deep(&root, 1)?),
            ["top.synScene"],
            "depth 1 is the children, as scan"
        );
        assert_eq!(
            names(scan_deep(&root, 2)?),
            ["a/b.synScene", "top.synScene"],
            "one level more"
        );
        assert_eq!(
            names(scan_deep(&root, 10)?),
            ["a/b.synScene", "a/c/d.synScene", "top.synScene"],
            "all of them, and nothing inside a scene"
        );
        fs::remove_dir_all(&root)?;
        Ok(())
    }

    #[test]
    fn every_line_ending_becomes_one_newline() {
        assert_eq!(
            newlines("a\r\nb\r\r\nc\nd\re\r\rf"),
            "a\nb\nc\nd\ne\n\nf",
            "CRLF, CRCRLF, LF, CR, CR CR"
        );
        assert_eq!(
            newlines("x \\\r\r\ny").lines().count(),
            2,
            "a continuation before CRCRLF stays one continuation"
        );
    }

    fn folder(json: &str, pass_shaders: &[&str]) -> Result<SceneFolder, SceneError> {
        Ok(SceneFolder {
            dir: PathBuf::from("."),
            def: SceneDef::parse(json, "t")?,
            shader: "main".to_string(),
            pass_shaders: pass_shaders.iter().map(|s| (*s).to_string()).collect(),
            script: None,
        })
    }

    #[test]
    fn each_pass_gets_its_own_file_or_falls_back_to_main() -> Result<(), SceneError> {
        let per_pass = folder(
            r#"{ "PASSES": [ {"TARGET": "a"} ], "PASS_SOURCES": ["passes/a.glsl", "main.glsl"] }"#,
            &["code a", "code main"],
        )?;
        assert_eq!(
            per_pass.shader_for(0),
            "code a",
            "pass 0 has its own source"
        );
        assert_eq!(
            per_pass.file_for(0),
            "passes/a.glsl",
            "and its own file name"
        );
        assert_eq!(
            per_pass.shader_for(1),
            "code main",
            "so does the final pass"
        );

        let plain = folder(r#"{ "PASSES": [ {"TARGET": "a"} ] }"#, &[])?;
        assert_eq!(plain.shader_for(1), "main", "no PASS_SOURCES: main.glsl");
        assert_eq!(plain.file_for(0), "main.glsl", "for every pass");
        Ok(())
    }

    #[test]
    fn windows_line_endings_are_normalised_at_load() -> anyhow::Result<()> {
        // D193: 50 of 64 real Synesthesia shaders are CRLF, which breaks `\`
        // continuations on ANGLE and doubles its error line numbers.
        let dir = std::env::temp_dir().join(format!("strobe-crlf-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        std::fs::write(
            dir.join("main.glsl"),
            "#define TWO(x) \\\r\n  (x * 2.0)\r\nvec4 renderMain() { return vec4(TWO(0.25)); }\r\n",
        )?;
        let loaded = SceneFolder::load(&dir);
        std::fs::remove_dir_all(&dir)?;
        let scene = loaded?;
        assert!(!scene.shader.contains('\r'), "no carriage returns left");
        assert_eq!(scene.shader.lines().count(), 3, "same lines");
        Ok(())
    }

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
