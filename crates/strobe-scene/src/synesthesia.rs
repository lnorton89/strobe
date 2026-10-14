// ============================================================================
// Module:       strobe_scene::synesthesia
// Description:  Where Synesthesia is installed on this machine, where its
//               settings live, and which folders hold its scenes and GLSL
//               libraries.
//
// Dependencies: serde_json (its settings files); std::env, std::fs
// ============================================================================

//! Finding Synesthesia (D198, D271).
//!
//! Strobe ships none of Synesthesia's files. Where the person has
//! Synesthesia installed, two things read from that install, and both ask
//! this module where it is, so there is one answer:
//!
//! - `#include` searches its `lib` folder ([`crate::include::search_path`]);
//! - the app's library lists its scenes after the person's own.
//!
//! **The install** is, in order:
//! 1. `STROBE_SYNESTHESIA_DIR`, when it is set and not empty. It replaces
//!    the usual places, so a test (on any platform) or a machine with
//!    Synesthesia somewhere unusual can point at it. A value that is not a
//!    folder means no install: an override is never second-guessed.
//! 2. On Windows, `Synesthesia` under `%ProgramFiles(x86)%` (where 1.25.4
//!    installs) and then `%ProgramFiles%`, falling back to `C:\Program Files
//!    (x86)` and `C:\Program Files` when the variables are unset.
//!
//! **Its data folder** (settings, and the person's own `glsl`) is
//! `STROBE_SYNESTHESIA_DATA` when set, else `%LOCALAPPDATA%\Synesthesia\Data`
//! on Windows.
//!
//! **Its scene folders**, as a survey of a real 1.25.4.189 install found
//! them:
//! - the scenes it ships, in the install's [`BUILT_IN`] folders (`release`,
//!   87 scenes, and `default`, the welcome scene). These are under Program
//!   Files and read-only;
//! - the folders the person added in Synesthesia's Settings > Custom Scene
//!   Folder: `vs.json`'s `sceneFolders` and `primarySceneFolder`, and
//!   `preferences.json`'s `PRIMARY_SCENE_FOLDER` (where Synesthesia writes
//!   new scenes). They are the person's own and writable, and are searched
//!   as Synesthesia searches them: recursively, [`DEFAULT_DEPTH`] levels
//!   deep, unless `RECURSIVE_SCENE_SEARCH` is `false` or
//!   `RECURSIVE_SCENE_SEARCH_DEPTH` says otherwise. There is no default
//!   user folder: `Documents\Synesthesia` does not exist.
//!
//! A folder is read-only exactly when it is inside the install. The
//! settings files are read leniently: missing, unreadable or malformed is
//! "no folders of the person's", with a note saying why, never an error.
//!
//! Everything here reads the environment through [`Env`], so the rules are
//! tested on any machine without changing the process's environment.

use serde_json::Value;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The environment variable naming the install folder, over the usual
/// places.
pub const DIR_VAR: &str = "STROBE_SYNESTHESIA_DIR";

/// The environment variable naming Synesthesia's data folder (where
/// `vs.json` and `preferences.json` are), over `%LOCALAPPDATA%`.
pub const DATA_VAR: &str = "STROBE_SYNESTHESIA_DATA";

/// The folders under the install that hold the scenes Synesthesia ships,
/// in the order the library lists them. Add a row if a version adds one.
pub const BUILT_IN: &[&str] = &["release", "default"];

/// How deep Synesthesia searches a scene folder of the person's when its
/// preferences do not say (its own default).
pub const DEFAULT_DEPTH: usize = 10;

/// The deepest search taken from the preferences file, whatever it says.
const MAX_DEPTH: usize = 32;

/// The folder Synesthesia installs into under each Program Files folder,
/// and names its data folder after.
const NAME: &str = "Synesthesia";

/// The Program Files folders an install is looked for under, in order: the
/// environment variable that names each, and where it is when unset.
const PROGRAM_FILES: [(&str, &str); 2] = [
    ("ProgramFiles(x86)", r"C:\Program Files (x86)"),
    ("ProgramFiles", r"C:\Program Files"),
];

/// The settings file listing the person's scene folders.
const SCENE_LIST: &str = "vs.json";

/// The settings file with the primary folder and the search depth.
const PREFERENCES: &str = "preferences.json";

/// What detection reads from the environment.
pub struct Env<'a> {
    /// An environment variable's value.
    pub var: &'a dyn Fn(&str) -> Option<OsString>,
    /// Whether the usual Windows places apply.
    pub windows: bool,
}

impl Env<'_> {
    /// This process's environment, on this platform.
    pub fn process() -> Env<'static> {
        Env {
            var: &|name| std::env::var_os(name),
            windows: cfg!(windows),
        }
    }

    /// A variable's value, when it is set and not empty.
    fn value(&self, name: &str) -> Option<OsString> {
        (self.var)(name).filter(|value| !value.is_empty())
    }
}

/// Where the install was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Found {
    /// `STROBE_SYNESTHESIA_DIR` named it.
    Override,
    /// One of the usual places.
    Standard,
}

/// A Synesthesia install on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Install {
    /// The install folder: the one holding `lib`, `release` and the
    /// executable.
    pub dir: PathBuf,
    /// How it was found.
    pub found: Found,
    /// Its data folder, where the environment names one.
    pub data: Option<PathBuf>,
}

/// One folder Synesthesia lists scenes from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SceneFolder {
    /// The folder.
    pub path: PathBuf,
    /// Inside the install, so never written to.
    pub read_only: bool,
    /// How many levels below `path` a scene folder is looked for: 1 is its
    /// children only.
    pub depth: usize,
}

/// Synesthesia's scene folders on this machine, and why any the settings
/// name are not among them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Folders {
    /// The built-in folders that exist, then the person's, without repeats.
    pub folders: Vec<SceneFolder>,
    /// One sentence for each thing that could not be read or used, for the
    /// log.
    pub notes: Vec<String>,
}

impl Install {
    /// The GLSL libraries Synesthesia ships (`lygia`, `hg_sdf`).
    pub fn lib_dir(&self) -> PathBuf {
        self.dir.join("lib")
    }

    /// Every folder Synesthesia lists scenes from: the [`BUILT_IN`] ones
    /// that exist (read-only), then the person's from its settings files.
    /// Reads those files.
    pub fn scene_folders(&self) -> Folders {
        let mut found = Folders::default();
        for name in BUILT_IN {
            let path = self.dir.join(name);
            if path.is_dir() {
                found.folders.push(SceneFolder {
                    path,
                    read_only: true,
                    depth: 1,
                });
            }
        }
        match &self.data {
            Some(data) => self.add_settings_folders(data, &mut found),
            None => found
                .notes
                .push("no Synesthesia data folder is named here".to_string()),
        }
        found
    }

    /// Adds the folders `vs.json` and `preferences.json` in `data` name.
    fn add_settings_folders(&self, data: &Path, found: &mut Folders) {
        let list = read_json(&data.join(SCENE_LIST), &mut found.notes);
        let preferences = read_json(&data.join(PREFERENCES), &mut found.notes);
        let text = |value: Option<&Value>| value.and_then(Value::as_str).map(str::to_owned);
        let mut paths: Vec<String> = list
            .as_ref()
            .and_then(|list| list.get("sceneFolders"))
            .and_then(Value::as_array)
            .map(|folders| folders.iter().filter_map(|v| text(Some(v))).collect())
            .unwrap_or_default();
        paths.extend(text(
            list.as_ref().and_then(|l| l.get("primarySceneFolder")),
        ));
        paths.extend(text(
            preferences
                .as_ref()
                .and_then(|p| p.get("PRIMARY_SCENE_FOLDER")),
        ));
        let depth = search_depth(preferences.as_ref());
        for path in paths {
            let trimmed = path.trim();
            if trimmed.is_empty() {
                continue;
            }
            let path = PathBuf::from(trimmed);
            if found.folders.iter().any(|known| known.path == path) {
                continue;
            }
            if !path.is_dir() {
                found.notes.push(format!(
                    "the scene folder {} in Synesthesia's settings is not a folder here",
                    path.display()
                ));
                continue;
            }
            let read_only = path.starts_with(&self.dir);
            found.folders.push(SceneFolder {
                path,
                read_only,
                depth,
            });
        }
    }
}

/// How deep the person's folders are searched, from `preferences.json`:
/// Synesthesia's own default where a key is missing or not understood.
fn search_depth(preferences: Option<&Value>) -> usize {
    let recursive = preferences
        .and_then(|p| p.get("RECURSIVE_SCENE_SEARCH"))
        .and_then(Value::as_bool)
        .unwrap_or(true);
    if !recursive {
        return 1;
    }
    preferences
        .and_then(|p| p.get("RECURSIVE_SCENE_SEARCH_DEPTH"))
        .and_then(Value::as_u64)
        .and_then(|depth| usize::try_from(depth).ok())
        .map_or(DEFAULT_DEPTH, |depth| depth.clamp(1, MAX_DEPTH))
}

/// A settings file as JSON, or `None` with a note saying why not.
fn read_json(path: &Path, notes: &mut Vec<String>) -> Option<Value> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            notes.push(format!("{} does not exist", path.display()));
            return None;
        }
        Err(error) => {
            notes.push(format!("{} could not be read: {error}", path.display()));
            return None;
        }
    };
    match serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) {
        Ok(value) if value.is_object() => Some(value),
        Ok(_) => {
            notes.push(format!("{} is not a JSON object", path.display()));
            None
        }
        Err(error) => {
            notes.push(format!(
                "{} is not JSON Strobe can read: {error}",
                path.display()
            ));
            None
        }
    }
}

/// Synesthesia's install on this machine, if there is one.
pub fn detect() -> Option<Install> {
    detect_with(&Env::process())
}

/// [`detect`], reading `env`.
pub fn detect_with(env: &Env<'_>) -> Option<Install> {
    let install = |dir: PathBuf, found: Found| Install {
        dir,
        found,
        data: data_dir(env),
    };
    if let Some(dir) = env.value(DIR_VAR) {
        let dir = PathBuf::from(dir);
        return dir.is_dir().then(|| install(dir, Found::Override));
    }
    if !env.windows {
        return None;
    }
    PROGRAM_FILES
        .iter()
        .map(|(var, fallback)| {
            env.value(var)
                .map_or_else(|| PathBuf::from(fallback), PathBuf::from)
                .join(NAME)
        })
        .find(|dir| dir.is_dir())
        .map(|dir| install(dir, Found::Standard))
}

/// Synesthesia's data folder (`STROBE_SYNESTHESIA_DATA`, else
/// `%LOCALAPPDATA%\Synesthesia\Data` on Windows), whether or not it exists.
pub fn data_dir(env: &Env<'_>) -> Option<PathBuf> {
    if let Some(data) = env.value(DATA_VAR) {
        return Some(PathBuf::from(data));
    }
    if !env.windows {
        return None;
    }
    env.value("LOCALAPPDATA")
        .map(|local| PathBuf::from(local).join(NAME).join("Data"))
}

/// Synesthesia's per-user GLSL folder (`glsl` in its data folder, D198),
/// whether or not it exists. Not part of the install: it belongs to the
/// person.
pub fn user_glsl_dir(env: &Env<'_>) -> Option<PathBuf> {
    data_dir(env).map(|data| data.join("glsl"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::collections::HashMap;

    /// A scratch folder that removes itself.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(name: &str) -> Result<Self> {
            let dir = std::env::temp_dir()
                .join(format!("strobe-synesthesia-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir)?;
            Ok(Self(dir))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn detect_in(vars: &HashMap<&str, OsString>, windows: bool) -> Option<Install> {
        let lookup = |name: &str| vars.get(name).cloned();
        detect_with(&Env {
            var: &lookup,
            windows,
        })
    }

    /// The folders' paths, and whether each is read-only and how deep.
    fn listed(folders: &Folders) -> Vec<(PathBuf, bool, usize)> {
        folders
            .folders
            .iter()
            .map(|folder| (folder.path.clone(), folder.read_only, folder.depth))
            .collect()
    }

    /// A JSON string holding `path`.
    fn json_path(path: &Path) -> Result<String> {
        Ok(serde_json::to_string(&path.display().to_string())?)
    }

    #[test]
    fn the_override_names_the_install_on_any_platform() -> Result<()> {
        let scratch = Scratch::new("override")?;
        std::fs::create_dir_all(scratch.0.join("release"))?;
        let vars = HashMap::from([(DIR_VAR, scratch.0.clone().into_os_string())]);
        let install =
            detect_in(&vars, false).ok_or_else(|| anyhow::anyhow!("the override was not taken"))?;
        assert_eq!(install.dir, scratch.0, "the folder named");
        assert_eq!(install.found, Found::Override, "and says so");
        assert_eq!(install.lib_dir(), scratch.0.join("lib"), "lib inside it");
        let folders = install.scene_folders();
        assert_eq!(
            listed(&folders),
            vec![(scratch.0.join("release"), true, 1)],
            "only the built-in folders that exist, read-only, their children only"
        );
        assert!(
            !folders.notes.is_empty(),
            "no data folder is named off Windows without the override, and that is noted"
        );
        Ok(())
    }

    #[test]
    fn an_override_that_is_not_a_folder_is_no_install() {
        let vars = HashMap::from([(DIR_VAR, OsString::from("/nowhere/at/all/synesthesia"))]);
        assert_eq!(detect_in(&vars, true), None, "not second-guessed");
        let empty = HashMap::from([(DIR_VAR, OsString::new())]);
        assert_eq!(
            detect_in(&empty, false),
            None,
            "empty is unset, and off Windows there is nowhere else to look"
        );
    }

    #[test]
    fn program_files_x86_comes_first_then_program_files() -> Result<()> {
        let scratch = Scratch::new("program-files")?;
        let x86 = scratch.0.join("x86");
        let plain = scratch.0.join("plain");
        std::fs::create_dir_all(plain.join(NAME))?;
        std::fs::create_dir_all(&x86)?;
        let vars = HashMap::from([
            ("ProgramFiles(x86)", x86.clone().into_os_string()),
            ("ProgramFiles", plain.clone().into_os_string()),
        ]);
        let found = detect_in(&vars, true).map(|install| install.dir);
        assert_eq!(
            found,
            Some(plain.join(NAME)),
            "nothing under x86, so Program Files"
        );
        std::fs::create_dir_all(x86.join(NAME))?;
        let found = detect_in(&vars, true).map(|install| install.dir);
        assert_eq!(found, Some(x86.join(NAME)), "x86 first");
        assert_eq!(
            detect_in(&vars, false),
            None,
            "the Windows places are not looked at elsewhere"
        );
        Ok(())
    }

    #[test]
    fn the_persons_scene_folders_come_from_synesthesias_settings() -> Result<()> {
        let scratch = Scratch::new("settings")?;
        let install = scratch.0.join("install");
        let data = scratch.0.join("data");
        let mine = scratch.0.join("my scenes");
        let primary = scratch.0.join("primary");
        for dir in [
            install.join("release"),
            install.join("default"),
            data.clone(),
            mine.clone(),
            primary.clone(),
        ] {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(
            data.join(SCENE_LIST),
            format!(
                "\u{feff}{{ \"primarySceneFolder\": \"\", \"sceneFolders\": [{}, {}, 7, {}], \
                 \"sceneSources\": {{ \"Welcome\": \"built-in\" }}, \"scenes\": {{}} }}",
                json_path(&mine)?,
                json_path(&scratch.0.join("gone"))?,
                json_path(&mine)?,
            ),
        )?;
        std::fs::write(
            data.join(PREFERENCES),
            format!(
                "{{ \"PRIMARY_SCENE_FOLDER\": {}, \"RECURSIVE_SCENE_SEARCH\": true, \
                 \"RECURSIVE_SCENE_SEARCH_DEPTH\": 4 }}",
                json_path(&primary)?
            ),
        )?;
        let vars = HashMap::from([
            (DIR_VAR, install.clone().into_os_string()),
            (DATA_VAR, data.clone().into_os_string()),
        ]);
        let found = detect_in(&vars, false)
            .ok_or_else(|| anyhow::anyhow!("the override was not taken"))?
            .scene_folders();
        assert_eq!(
            listed(&found),
            vec![
                (install.join("release"), true, 1),
                (install.join("default"), true, 1),
                (mine.clone(), false, 4),
                (primary.clone(), false, 4),
            ],
            "built-ins read-only; the person's own writable, searched as deep as the \
             preferences say, once each; a number in the list is skipped"
        );
        assert!(
            found.notes.iter().any(|note| note.contains("gone")),
            "a listed folder that is missing is noted: {:?}",
            found.notes
        );

        std::fs::write(
            data.join(PREFERENCES),
            "{ \"RECURSIVE_SCENE_SEARCH\": false }",
        )?;
        let depths: Vec<usize> = detect_in(&vars, false)
            .ok_or_else(|| anyhow::anyhow!("the override was not taken"))?
            .scene_folders()
            .folders
            .iter()
            .map(|folder| folder.depth)
            .collect();
        assert_eq!(depths, [1, 1, 1], "not recursive: children only");

        std::fs::remove_file(data.join(PREFERENCES))?;
        let depths: Vec<usize> = detect_in(&vars, false)
            .ok_or_else(|| anyhow::anyhow!("the override was not taken"))?
            .scene_folders()
            .folders
            .iter()
            .map(|folder| folder.depth)
            .collect();
        assert_eq!(
            depths,
            [1, 1, DEFAULT_DEPTH],
            "no preferences: Synesthesia's default depth"
        );
        Ok(())
    }

    #[test]
    fn broken_or_missing_settings_mean_no_folders_of_the_persons_and_say_why() -> Result<()> {
        let scratch = Scratch::new("broken")?;
        let install = scratch.0.join("install");
        let data = scratch.0.join("data");
        std::fs::create_dir_all(install.join("release"))?;
        std::fs::create_dir_all(&data)?;
        std::fs::write(data.join(SCENE_LIST), "{ \"sceneFolders\": [ broken")?;
        std::fs::write(data.join(PREFERENCES), "[1, 2]")?;
        let vars = HashMap::from([
            (DIR_VAR, install.clone().into_os_string()),
            (DATA_VAR, data.clone().into_os_string()),
        ]);
        let found = detect_in(&vars, false)
            .ok_or_else(|| anyhow::anyhow!("the override was not taken"))?
            .scene_folders();
        assert_eq!(
            listed(&found),
            vec![(install.join("release"), true, 1)],
            "the built-ins still list"
        );
        assert_eq!(found.notes.len(), 2, "one note per file: {:?}", found.notes);
        std::fs::remove_file(data.join(SCENE_LIST))?;
        std::fs::remove_file(data.join(PREFERENCES))?;
        let found = detect_in(&vars, false)
            .ok_or_else(|| anyhow::anyhow!("the override was not taken"))?
            .scene_folders();
        assert_eq!(found.folders.len(), 1, "missing files: built-ins only");
        Ok(())
    }

    #[test]
    fn a_folder_of_the_persons_inside_the_install_is_read_only() -> Result<()> {
        let scratch = Scratch::new("inside")?;
        let install = scratch.0.join("install");
        let data = scratch.0.join("data");
        std::fs::create_dir_all(install.join("extra"))?;
        std::fs::create_dir_all(&data)?;
        std::fs::write(
            data.join(SCENE_LIST),
            format!(
                "{{ \"sceneFolders\": [{}] }}",
                json_path(&install.join("extra"))?
            ),
        )?;
        let vars = HashMap::from([
            (DIR_VAR, install.clone().into_os_string()),
            (DATA_VAR, data.clone().into_os_string()),
        ]);
        let found = detect_in(&vars, false)
            .ok_or_else(|| anyhow::anyhow!("the override was not taken"))?
            .scene_folders();
        assert_eq!(
            listed(&found),
            vec![(install.join("extra"), true, DEFAULT_DEPTH)],
            "under Program Files, so never written"
        );
        Ok(())
    }

    #[test]
    fn the_built_in_table_is_what_a_real_install_has() {
        assert_eq!(BUILT_IN, ["release", "default"], "1.25.4's folders");
    }

    #[test]
    fn the_data_folder_is_named_by_the_override_or_localappdata_on_windows() {
        let vars = HashMap::from([("LOCALAPPDATA", OsString::from("/data"))]);
        let lookup = |name: &str| vars.get(name).cloned();
        let windows = Env {
            var: &lookup,
            windows: true,
        };
        assert_eq!(
            user_glsl_dir(&windows),
            Some(PathBuf::from("/data").join(NAME).join("Data").join("glsl")),
            "under LOCALAPPDATA"
        );
        let elsewhere = Env {
            var: &lookup,
            windows: false,
        };
        assert_eq!(user_glsl_dir(&elsewhere), None, "nowhere else");
        let named = HashMap::from([(DATA_VAR, OsString::from("/fake/data"))]);
        let lookup = |name: &str| named.get(name).cloned();
        let overridden = Env {
            var: &lookup,
            windows: false,
        };
        assert_eq!(
            data_dir(&overridden),
            Some(PathBuf::from("/fake/data")),
            "the override, on any platform"
        );
    }
}
