// ============================================================================
// Module:       strobe_scene::include
// Description:  `#include "file.glsl"` in scene shaders: resolution against the
//               including file, the scene folder and the library folders.
//
// Dependencies: std::fs, std::env; crate::assemble::source,
//               crate::synesthesia
// ============================================================================

//! `#include` (D198).
//!
//! Synesthesia scenes include the libraries the engine ships (`lygia`,
//! `hg_sdf`) and the user's own `.glsl` files, e.g. `#include "hg_sdf.glsl"`
//! or `#include "lygia/math/rotate3d.glsl"`. Neither is compiled in unless a
//! scene asks for it. GLSL has no `#include` of its own, so this expands
//! them when the scene loads.
//!
//! A path is looked up, in order:
//! 1. relative to the file doing the including (lygia's own files include
//!    with `../`);
//! 2. relative to the scene folder;
//! 3. in each folder of [`search_path`]: `STROBE_GLSL_PATH` (a path list),
//!    then, where they exist, Synesthesia's own `lib` folder and its user
//!    `glsl` data folder, so a machine with Synesthesia installed just works.
//!    The install is found by [`crate::synesthesia`], the same detection the
//!    app's library uses for Synesthesia's scenes (D271). Strobe ships none of these libraries: `lygia`'s licence does not allow
//!    it.
//!
//! A scene with no folder behind it, the browser sandbox's (D524), has only
//! its own files: [`expand_files`] resolves against those, relative to the
//! including file and then to the scene, with no search path (D554).
//!
//! Each file is included once, whatever the number of `#include`s naming it,
//! which is what lygia's include guards would do anyway and what stops a
//! cycle. A file that cannot be found is left as the `#include` line, so the
//! driver's error names it.
//!
//! **Line numbers.** The included text is compiled under source string
//! [`source::INCLUDED`] and the `#include` line is replaced by
//! `#line 1 3` … `#line n+1 1`, so every line of `main.glsl` keeps its own
//! number and a driver error in it still maps (`assemble::map_error_line`).

use crate::assemble::source;
use crate::synesthesia;
use std::collections::HashSet;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};

/// How deep includes may nest before the rest are left alone.
const MAX_DEPTH: usize = 16;

/// The library folders `#include` searches after the scene's own, in order,
/// keeping only those that exist.
pub fn search_path() -> Vec<PathBuf> {
    let env = synesthesia::Env::process();
    let install = synesthesia::detect_with(&env);
    search_path_with(&env, install.as_ref())
}

/// [`search_path`] for this environment and this Synesthesia install, as
/// [`synesthesia::detect_with`] found it: the same detection the app's
/// library lists Synesthesia's scenes from (D271). `STROBE_GLSL_PATH`, then
/// the install's `lib`, then the person's own Synesthesia `glsl` folder.
pub fn search_path_with(
    env: &synesthesia::Env<'_>,
    install: Option<&synesthesia::Install>,
) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = (env.var)("STROBE_GLSL_PATH")
        .map(|list| std::env::split_paths(&list).collect())
        .unwrap_or_default();
    dirs.extend(install.map(synesthesia::Install::lib_dir));
    dirs.extend(synesthesia::user_glsl_dir(env));
    dirs.retain(|dir| dir.is_dir());
    dirs
}

/// Expands every `#include` in `text`, the user's file in `scene_dir`.
/// Returns `text` unchanged (no copy) when there is none.
pub fn expand(text: String, scene_dir: &Path, search: &[PathBuf]) -> String {
    expand_from(text, scene_dir, search, &Files::Disk)
}

/// Expands every `#include` in `text` against a scene's own files, with no
/// folder on disk: `file` answers a scene-relative name such as
/// `helpers.glsl` or `lib/noise.glsl` (D554). The lookup order is
/// [`expand`]'s without the search path, and a name that leaves the scene
/// (`../x.glsl` from the scene's top) finds nothing.
pub fn expand_files(text: String, file: &dyn Fn(&str) -> Option<String>) -> String {
    expand_from(text, Path::new(""), &[], &Files::Memory(file))
}

fn expand_from(text: String, scene_dir: &Path, search: &[PathBuf], files: &Files<'_>) -> String {
    if !text.contains("#include") {
        return text;
    }
    let mut seen = HashSet::new();
    let mut out = String::with_capacity(text.len());
    expand_into(
        &mut out,
        &text,
        scene_dir,
        &Context {
            scene_dir,
            search,
            files,
            source: source::USER,
        },
        &mut seen,
        0,
    );
    out
}

/// Where included files are read from.
enum Files<'a> {
    /// The file system: a scene folder and the library folders.
    Disk,
    /// A scene's own files by scene-relative name, as the browser holds them.
    Memory(&'a dyn Fn(&str) -> Option<String>),
}

impl Files<'_> {
    /// The text of the file at `path`, if there is one.
    fn read(&self, path: &Path) -> Option<String> {
        match self {
            Self::Disk => path
                .is_file()
                .then(|| crate::load::read_text(path).ok())
                .flatten(),
            Self::Memory(file) => file(&scene_name(path)?),
        }
    }

    /// What makes two paths the same file, for including each once.
    fn key(&self, path: &Path) -> PathBuf {
        match self {
            Self::Disk => fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf()),
            Self::Memory(_) => scene_name(path).map_or_else(|| path.to_path_buf(), PathBuf::from),
        }
    }
}

/// `path` as a scene-relative name with `/` separators and no `.` or `..`
/// left in it, or `None` when it climbs out of the scene or is absolute.
fn scene_name(path: &Path) -> Option<String> {
    use std::path::Component;
    let mut parts: Vec<String> = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(part) => parts.push(part.to_string_lossy().into_owned()),
            Component::CurDir => {}
            Component::ParentDir => {
                parts.pop()?;
            }
            Component::RootDir | Component::Prefix(_) => return None,
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

/// What stays the same while expanding one file.
struct Context<'a> {
    scene_dir: &'a Path,
    search: &'a [PathBuf],
    files: &'a Files<'a>,
    /// The source-string number the current file compiles under.
    source: u32,
}

fn expand_into(
    out: &mut String,
    text: &str,
    dir: &Path,
    context: &Context<'_>,
    seen: &mut HashSet<PathBuf>,
    depth: usize,
) {
    for (index, line) in text.lines().enumerate() {
        let Some(name) = include_target(line) else {
            out.push_str(line);
            out.push('\n');
            continue;
        };
        let found = (depth < MAX_DEPTH)
            .then(|| resolve(name, dir, context))
            .flatten();
        let Some((path, included)) = found else {
            out.push_str(line);
            out.push('\n');
            continue;
        };
        if !seen.insert(context.files.key(&path)) {
            out.push('\n');
            continue;
        }
        let included = crate::load::clean(included);
        let _ = writeln!(out, "#line 1 {}", source::INCLUDED);
        let parent = path
            .parent()
            .map_or_else(|| dir.to_path_buf(), Path::to_path_buf);
        let nested = Context {
            scene_dir: context.scene_dir,
            search: context.search,
            files: context.files,
            source: source::INCLUDED,
        };
        expand_into(out, &included, &parent, &nested, seen, depth + 1);
        // The next line of this file is line index + 2 (lines count from 1).
        let _ = writeln!(out, "#line {} {}", index + 2, context.source);
    }
}

/// The quoted path of an `#include` line, or `None` for any other line.
fn include_target(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix('#')?.trim_start();
    let rest = rest.strip_prefix("include")?.trim_start();
    let (open, close) = match rest.chars().next()? {
        '"' => ('"', '"'),
        '<' => ('<', '>'),
        _ => return None,
    };
    let inner = rest.strip_prefix(open)?;
    let end = inner.find(close)?;
    inner.get(..end).filter(|name| !name.is_empty())
}

/// The first readable file `name` names, by the lookup order above, and its
/// text.
fn resolve(name: &str, dir: &Path, context: &Context<'_>) -> Option<(PathBuf, String)> {
    std::iter::once(dir)
        .chain(std::iter::once(context.scene_dir))
        .chain(context.search.iter().map(PathBuf::as_path))
        .map(|base| base.join(name))
        .find_map(|candidate| {
            let text = context.files.read(&candidate)?;
            Some((candidate, text))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    /// A scratch folder that removes itself.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Result<Self> {
            let dir =
                std::env::temp_dir().join(format!("strobe-include-{}-{name}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
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
    fn includes_resolve_relative_then_on_the_search_path_once_each() -> Result<()> {
        let scene = Scratch::new("scene")?;
        let library = Scratch::new("library")?;
        fs::create_dir_all(library.0.join("lib/math"))?;
        fs::write(library.0.join("lib/const.glsl"), "#define K 2.0\n")?;
        // A library file including its sibling folder with `../`, as lygia does.
        fs::write(
            library.0.join("lib/math/twice.glsl"),
            "#include \"../const.glsl\"\nfloat twice(float x) { return x * K; }\n",
        )?;
        let main = "#include \"lib/math/twice.glsl\"\n#include \"lib/const.glsl\"\nvec4 renderMain() { return vec4(twice(0.25)); }\n";
        let out = expand(main.to_string(), &scene.0, std::slice::from_ref(&library.0));
        assert!(out.contains("float twice"), "the library file is in: {out}");
        assert_eq!(
            out.matches("#define K").count(),
            1,
            "const.glsl once: {out}"
        );
        // `#line 2 1`, then the second (already included) include as a blank
        // line 2, so `renderMain` is line 3, as in main.glsl.
        assert!(
            out.contains("#line 2 1\n\nvec4 renderMain()"),
            "main.glsl's lines keep their numbers: {out}"
        );
        Ok(())
    }

    #[test]
    fn the_include_path_finds_synesthesia_by_the_libraries_detection() -> Result<()> {
        let install = Scratch::new("install")?;
        let extra = Scratch::new("extra")?;
        fs::create_dir_all(install.0.join("lib"))?;
        fs::write(
            install.0.join("lib/hg_like.glsl"),
            "float fromTheInstall() { return 1.0; }\n",
        )?;
        let vars = std::collections::HashMap::from([
            (synesthesia::DIR_VAR, install.0.clone().into_os_string()),
            ("STROBE_GLSL_PATH", extra.0.clone().into_os_string()),
        ]);
        let lookup = |name: &str| vars.get(name).cloned();
        let env = synesthesia::Env {
            var: &lookup,
            windows: false,
        };
        // The app's library asks the same function where Synesthesia is.
        let found = synesthesia::detect_with(&env);
        let search = search_path_with(&env, found.as_ref());
        assert_eq!(
            search,
            vec![extra.0.clone(), install.0.join("lib")],
            "STROBE_GLSL_PATH first, then the detected install's lib"
        );
        let main =
            "#include \"hg_like.glsl\"\nvec4 renderMain() { return vec4(fromTheInstall()); }\n";
        let scene = Scratch::new("synesthesia-scene")?;
        let out = expand(main.to_string(), &scene.0, &search);
        assert!(
            out.contains("float fromTheInstall()"),
            "resolved from the install: {out}"
        );
        let none = search_path_with(&env, None);
        assert_eq!(none, vec![extra.0.clone()], "no install, no lib");
        Ok(())
    }

    #[test]
    fn an_included_files_byte_order_mark_is_dropped() -> Result<()> {
        let scene = Scratch::new("bom")?;
        fs::write(
            scene.0.join("lib.glsl"),
            "\u{feff}float lib() { return 1.0; }\r\n",
        )?;
        let main = "#include \"lib.glsl\"\nvec4 renderMain() { return vec4(lib()); }\n";
        let out = expand(main.to_string(), &scene.0, &[]);
        assert!(!out.contains('\u{feff}'), "no mark: {out:?}");
        assert!(out.contains("float lib()"), "the file is in: {out}");
        Ok(())
    }

    #[test]
    fn a_missing_include_is_left_for_the_driver_to_name() -> Result<()> {
        let scene = Scratch::new("missing")?;
        let main = "#include \"nowhere.glsl\"\nvec4 renderMain() { return vec4(1.0); }\n";
        let out = expand(main.to_string(), &scene.0, &[]);
        assert_eq!(out, main, "unchanged");
        Ok(())
    }

    #[test]
    fn a_scenes_own_files_resolve_with_no_folder_behind_them() {
        let files = [
            ("main.glsl", "unused"),
            (
                "helpers.glsl",
                "#include \"lib/k.glsl\"\nfloat twice(float x) { return x * K; }\n",
            ),
            // Relative to the including file, as lygia includes its siblings.
            ("lib/k.glsl", "#include \"../lib/k.glsl\"\n#define K 2.0\n"),
        ];
        let file = |name: &str| {
            files
                .iter()
                .find(|(n, _)| *n == name)
                .map(|(_, text)| (*text).to_string())
        };
        let main = "#include \"helpers.glsl\"\n#include \"./lib/k.glsl\"\nvec4 renderMain() { return vec4(twice(0.25)); }\n";
        let out = expand_files(main.to_string(), &file);
        assert!(out.contains("float twice"), "helpers.glsl is in: {out}");
        assert_eq!(
            out.matches("#define K").count(),
            1,
            "lib/k.glsl once, by any spelling: {out}"
        );
        assert!(
            out.contains("#line 2 1\n\nvec4 renderMain()"),
            "main.glsl's lines keep their numbers: {out}"
        );
        let outside = "#include \"../secret.glsl\"\nvec4 renderMain() { return vec4(1.0); }\n";
        let none = |_: &str| Some("float leaked;".to_string());
        assert_eq!(
            expand_files(outside.to_string(), &none),
            outside,
            "a name that leaves the scene finds nothing"
        );
    }

    #[test]
    fn only_include_lines_are_includes() {
        assert_eq!(
            include_target("  #  include \"a.glsl\""),
            Some("a.glsl"),
            "spaced"
        );
        assert_eq!(
            include_target("#include <b.glsl>"),
            Some("b.glsl"),
            "angle brackets"
        );
        assert_eq!(
            include_target("// #include \"c.glsl\""),
            None,
            "commented out"
        );
        assert_eq!(include_target("#define include 1"), None, "not an include");
    }
}
