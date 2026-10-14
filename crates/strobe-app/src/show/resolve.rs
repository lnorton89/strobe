// ============================================================================
// Module:       strobe_app::show::resolve
// Description:  Turns a playlist's scene references into scene folders, and
//               a cue's control overrides into control-bank writes.
//
// Dependencies: strobe-playlist (SceneRef, ControlValue); strobe-scene
//               (ControlBank, load); crate::library
// ============================================================================

//! The two places a playlist meets the rest of the app.
//!
//! ## Scene references
//!
//! A cue names its scene either as a library name (`04-voronoi-glass`) or as
//! a path. [`scene_dir`] finds the folder. A library name must match an entry
//! *exactly* (ignoring case): a playlist is a script for a show, and a
//! prefix match that quietly picked a different scene when a new folder was
//! added would be worse than an error. Synesthesia's own scenes are library
//! entries too where they are listed (D271); they come after the library
//! folder's, so a name both have plays the person's own. A path is tried
//! relative to the playlist file first and then to the working directory,
//! and `~/` is the home folder.
//!
//! ## Control overrides
//!
//! A cue's `overrides` are `name = value` pairs in the playlist's own
//! [`ControlValue`] type. [`apply_overrides`] writes them into a scene's
//! [`ControlBank`] as targets, the same write the inspector makes, so the
//! scene's smoothing eases the controls to them. A value of the wrong shape
//! for its control (a colour for a slider) or a name the scene does not have
//! is *skipped with a reason*, never a crash and never a half-applied
//! control: the rest of the overrides still apply.

use crate::library::Library;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use strobe_playlist::{ControlValue, SceneRef};
use strobe_scene::control::def::ControlDef;
use strobe_scene::control::{ControlBank, ControlKind};

/// A scene reference found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved {
    /// The name to show and log: the folder's title.
    pub name: String,
    /// The scene folder.
    pub dir: PathBuf,
    /// Its entry in the library, when it is one.
    pub library_index: Option<usize>,
}

/// Finds the folder `scene` names. `base` is the folder holding the
/// playlist file, for relative paths.
pub fn scene_dir(library: &Library, base: &Path, scene: &SceneRef) -> Result<Resolved, String> {
    match scene {
        SceneRef::Library(name) => {
            let found = library
                .entries
                .iter()
                .position(|entry| entry.name == *name)
                .or_else(|| {
                    let lower = name.to_lowercase();
                    library.entries.iter().position(|e| e.lower == lower)
                });
            match found.and_then(|index| library.entries.get(index).map(|e| (index, e))) {
                Some((index, entry)) => Ok(Resolved {
                    name: entry.name.clone(),
                    dir: entry.dir.clone(),
                    library_index: Some(index),
                }),
                None => Err(format!(
                    "no scene called \"{name}\" in the library {}",
                    library.dir.display()
                )),
            }
        }
        SceneRef::Path(path) => {
            let expanded = expand_home(path);
            let candidates = if expanded.is_absolute() {
                vec![expanded]
            } else {
                vec![base.join(&expanded), expanded]
            };
            let Some(dir) = candidates.iter().find(|candidate| candidate.is_dir()) else {
                return Err(format!(
                    "no scene folder at {} (tried {})",
                    path.display(),
                    candidates
                        .iter()
                        .map(|c| c.display().to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            };
            let library_index = library.entries.iter().position(|entry| entry.dir == *dir);
            Ok(Resolved {
                name: strobe_scene::load::folder_title(dir),
                dir: dir.clone(),
                library_index,
            })
        }
    }
}

/// `~/x` as the home folder joined with `x`; any other path unchanged.
fn expand_home(path: &Path) -> PathBuf {
    let Ok(rest) = path.strip_prefix("~") else {
        return path.to_path_buf();
    };
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"));
    match home {
        Some(home) => PathBuf::from(home).join(rest),
        None => path.to_path_buf(),
    }
}

/// The values to write for `value` into control `def`, or why it does not
/// fit.
pub fn values_for(def: &ControlDef, value: &ControlValue) -> Result<Vec<f32>, String> {
    let wrong = |what: &str| {
        Err(format!(
            "`{}` is {} and cannot be set to {what}",
            def.name,
            describe(&def.kind)
        ))
    };
    match (&def.kind, value) {
        (ControlKind::Slider(_) | ControlKind::Knob(_), ControlValue::Float(x)) => Ok(vec![*x]),
        (ControlKind::Slider(_) | ControlKind::Knob(_), ControlValue::Int(i)) => {
            Ok(vec![*i as f32])
        }
        (ControlKind::Toggle { .. }, ControlValue::Bool(on)) => {
            Ok(vec![if *on { def.max[0] } else { def.min[0] }])
        }
        (ControlKind::Toggle { .. }, ControlValue::Float(x)) => Ok(vec![*x]),
        (ControlKind::Toggle { .. }, ControlValue::Int(i)) => Ok(vec![*i as f32]),
        (ControlKind::Dropdown { .. }, ControlValue::Int(i)) => Ok(vec![*i as f32]),
        (ControlKind::Dropdown { .. }, ControlValue::Float(x)) => Ok(vec![x.round()]),
        (ControlKind::Dropdown { .. }, ControlValue::Text(label)) => def
            .labels
            .iter()
            .position(|candidate| candidate.eq_ignore_ascii_case(label.trim()))
            .map(|index| vec![index as f32])
            .ok_or_else(|| {
                format!(
                    "`{}` has no option \"{label}\" (it has: {})",
                    def.name,
                    def.labels.join(", ")
                )
            }),
        (ControlKind::Xy(_), ControlValue::Vec2(v)) => Ok(v.to_vec()),
        (ControlKind::Color { .. }, ControlValue::Color(c)) => Ok(c.to_vec()),
        (ControlKind::Bang(_), _) => Err(format!(
            "`{}` is a bang, which is pressed rather than set",
            def.name
        )),
        (ControlKind::Xy(_) | ControlKind::Color { .. }, ControlValue::Float(_)) => {
            wrong("a number")
        }
        (_, ControlValue::Int(_)) => wrong("a whole number"),
        (_, ControlValue::Bool(_)) => wrong("on or off"),
        (_, ControlValue::Vec2(_)) => wrong("a pair of numbers"),
        (_, ControlValue::Color(_)) => wrong("a colour"),
        (_, ControlValue::Text(label)) => wrong(&format!("\"{label}\"")),
    }
}

/// What a control is, in words.
fn describe(kind: &ControlKind) -> &'static str {
    match kind {
        ControlKind::Slider(_) => "a slider",
        ControlKind::Knob(_) => "a knob",
        ControlKind::Toggle { .. } => "a toggle",
        ControlKind::Bang(_) => "a bang",
        ControlKind::Xy(_) => "an xy pad",
        ControlKind::Color { .. } => "a colour",
        ControlKind::Dropdown { .. } => "a dropdown",
    }
}

/// Returns every control to its default, at once (output and target), so a
/// scene that plays a second time does not carry the last cue's settings.
pub fn reset_controls(controls: &mut ControlBank) {
    for index in 0..controls.defs().len() {
        controls.reset(index);
    }
}

/// Writes a cue's overrides into `controls` and returns one line per
/// override that could not be applied.
pub fn apply_overrides(
    controls: &mut ControlBank,
    overrides: &BTreeMap<String, ControlValue>,
) -> Vec<String> {
    let mut problems = Vec::new();
    for (name, value) in overrides {
        let Some(index) = controls.find(name) else {
            problems.push(format!("the scene has no control called `{name}`"));
            continue;
        };
        let Some(def) = controls.defs().get(index) else {
            continue;
        };
        match values_for(def, value) {
            Ok(values) => controls.set(index, None, &values),
            Err(problem) => problems.push(problem),
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Entry;
    use anyhow::{Result, anyhow};
    use strobe_scene::scene::SceneDef;

    fn bank() -> Result<ControlBank> {
        let json = r#"{ "CONTROLS": [
            {"NAME": "amount", "TYPE": "slider", "MIN": 0.0, "MAX": 2.0, "DEFAULT": 0.5},
            {"NAME": "invert", "TYPE": "toggle", "DEFAULT": 0},
            {"NAME": "mode", "TYPE": "dropdown", "LABELS": ["calm", "wild"], "VALUES": [0, 1]},
            {"NAME": "centre", "TYPE": "xy", "MIN": [0.0, 0.0], "MAX": [1.0, 1.0], "DEFAULT": [0.5, 0.5]},
            {"NAME": "tint", "TYPE": "color", "DEFAULT": [1.0, 1.0, 1.0]},
            {"NAME": "flash", "TYPE": "bang"}
        ] }"#;
        Ok(ControlBank::new(SceneDef::parse(json, "t")?.controls)?)
    }

    fn target(controls: &ControlBank, name: &str) -> Result<Vec<f32>> {
        let index = controls
            .find(name)
            .ok_or_else(|| anyhow!("no control {name}"))?;
        let dims = controls.defs().get(index).map_or(0, ControlDef::dims);
        let values = controls
            .target(index)
            .ok_or_else(|| anyhow!("no target for {name}"))?;
        Ok(values[..dims].to_vec())
    }

    fn overrides(pairs: &[(&str, ControlValue)]) -> BTreeMap<String, ControlValue> {
        pairs
            .iter()
            .map(|(name, value)| ((*name).to_string(), value.clone()))
            .collect()
    }

    #[test]
    fn every_shape_of_override_lands_on_the_control_it_fits() -> Result<()> {
        let mut controls = bank()?;
        let problems = apply_overrides(
            &mut controls,
            &overrides(&[
                ("amount", ControlValue::Float(1.5)),
                ("invert", ControlValue::Bool(true)),
                ("mode", ControlValue::Text("Wild".into())),
                ("centre", ControlValue::Vec2([0.25, 0.75])),
                ("tint", ControlValue::Color([1.0, 0.45, 0.1])),
            ]),
        );
        assert!(
            problems.is_empty(),
            "nothing should be refused: {problems:?}"
        );
        assert_eq!(target(&controls, "amount")?, vec![1.5], "slider");
        assert_eq!(target(&controls, "invert")?, vec![1.0], "toggle");
        assert_eq!(target(&controls, "mode")?, vec![1.0], "dropdown by label");
        assert_eq!(target(&controls, "centre")?, vec![0.25, 0.75], "xy");
        assert_eq!(target(&controls, "tint")?, vec![1.0, 0.45, 0.1], "colour");
        Ok(())
    }

    #[test]
    fn a_bad_override_is_reported_and_the_rest_still_apply() -> Result<()> {
        let mut controls = bank()?;
        let problems = apply_overrides(
            &mut controls,
            &overrides(&[
                ("amount", ControlValue::Color([1.0, 0.0, 0.0])),
                ("ghost", ControlValue::Float(1.0)),
                ("mode", ControlValue::Text("sleepy".into())),
                ("flash", ControlValue::Bool(true)),
                ("invert", ControlValue::Bool(true)),
            ]),
        );
        assert_eq!(problems.len(), 4, "four refusals: {problems:?}");
        let joined = problems.join("\n");
        assert!(
            joined.contains("`amount` is a slider and cannot be set to a colour"),
            "{joined}"
        );
        assert!(joined.contains("no control called `ghost`"), "{joined}");
        assert!(joined.contains("no option \"sleepy\""), "{joined}");
        assert!(joined.contains("`flash` is a bang"), "{joined}");
        assert_eq!(
            target(&controls, "amount")?,
            vec![0.5],
            "the slider is untouched"
        );
        assert_eq!(
            target(&controls, "invert")?,
            vec![1.0],
            "the good one landed"
        );
        Ok(())
    }

    #[test]
    fn resetting_returns_every_control_to_its_default() -> Result<()> {
        let mut controls = bank()?;
        let _ = apply_overrides(
            &mut controls,
            &overrides(&[("amount", ControlValue::Float(2.0))]),
        );
        reset_controls(&mut controls);
        assert_eq!(
            target(&controls, "amount")?,
            vec![0.5],
            "back to the default"
        );
        Ok(())
    }

    fn library() -> Library {
        Library {
            dir: PathBuf::from("scenes"),
            entries: vec![
                Entry::new("01-aurora".to_string(), PathBuf::from("scenes/01-aurora")),
                Entry::new("02-plasma".to_string(), PathBuf::from("scenes/02-plasma")),
            ],
        }
    }

    #[test]
    fn a_library_name_must_match_exactly_but_not_in_case() -> Result<()> {
        let lib = library();
        let base = Path::new(".");
        let found = scene_dir(&lib, base, &SceneRef::Library("02-plasma".into()))
            .map_err(|e| anyhow!(e))?;
        assert_eq!(found.library_index, Some(1), "exact");
        let found = scene_dir(&lib, base, &SceneRef::Library("02-PLASMA".into()))
            .map_err(|e| anyhow!(e))?;
        assert_eq!(found.library_index, Some(1), "case-insensitive");
        let prefix = scene_dir(&lib, base, &SceneRef::Library("02".into()));
        assert!(
            prefix.is_err(),
            "a prefix must not quietly pick a scene: {prefix:?}"
        );
        let error = prefix.err().unwrap_or_default();
        assert!(
            error.contains("\"02\"") && error.contains("scenes"),
            "the error names the scene and the library: {error}"
        );
        Ok(())
    }

    #[test]
    fn a_cue_finds_synesthesias_scenes_and_the_libraries_own_first_on_a_clash() -> Result<()> {
        use crate::library::Source;
        let mut lib = library();
        for name in ["02-plasma", "ink"] {
            lib.entries.push(
                Entry::new(
                    name.to_string(),
                    PathBuf::from(format!("synesthesia/release/{name}.synScene")),
                )
                .from_source(Source::Synesthesia),
            );
        }
        let base = Path::new(".");
        let ink =
            scene_dir(&lib, base, &SceneRef::Library("ink".into())).map_err(|e| anyhow!(e))?;
        assert_eq!(
            (ink.library_index, ink.dir),
            (Some(3), PathBuf::from("synesthesia/release/ink.synScene")),
            "a scene only Synesthesia has is found"
        );
        let plasma = scene_dir(&lib, base, &SceneRef::Library("02-plasma".into()))
            .map_err(|e| anyhow!(e))?;
        assert_eq!(
            plasma.dir,
            PathBuf::from("scenes/02-plasma"),
            "a name in both is the library's own"
        );
        Ok(())
    }

    #[test]
    fn a_path_is_found_beside_the_playlist_then_in_the_working_folder() -> Result<()> {
        let root = std::env::temp_dir().join(format!("strobe-resolve-{}", std::process::id()));
        let scene = root.join("shows").join("mine.synScene");
        std::fs::create_dir_all(&scene)?;
        let lib = library();
        let by_relative = scene_dir(
            &lib,
            &root.join("shows"),
            &SceneRef::Path(PathBuf::from("mine.synScene")),
        )
        .map_err(|e| anyhow!(e))?;
        assert_eq!(by_relative.dir, scene, "relative to the playlist");
        assert_eq!(by_relative.name, "mine", "the title drops the suffix");
        let missing = scene_dir(
            &lib,
            &root,
            &SceneRef::Path(PathBuf::from("nowhere.synScene")),
        );
        let _ = std::fs::remove_dir_all(&root);
        let error = missing.err().unwrap_or_default();
        assert!(error.contains("nowhere.synScene"), "{error}");
        Ok(())
    }
}
