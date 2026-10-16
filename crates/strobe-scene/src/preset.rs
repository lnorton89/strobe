// ============================================================================
// Module:       strobe_scene::preset
// Description:  Presets (a scene's control values under a name, kept in the
//               scene folder) and fav slots (eight scene + preset pairs for
//               quick recall).
//
// Dependencies: serde, serde_json, thiserror; crate::control, crate::names
// ============================================================================

//! Presets and fav slots (PLAN section 10, D170).
//!
//! A [`Preset`] is the values of a scene's controls, and optionally of the
//! meta controls, under a name. It lives in the scene folder as
//! `presets/<name>.json`:
//!
//! ```json
//! { "name": "slow", "channels": { "scene": true, "meta": true },
//!   "scene": { "speed": 0.2, "tint": [0.9, 0.4, 0.1] },
//!   "meta":  { "brightness": 0.1 } }
//! ```
//!
//! Capture and apply go through [`ControlBank`] (`target` and `set`), so a
//! recalled preset is a write of targets exactly like the inspector's: a
//! smoothed control eases to it, a toggle snaps to either end, a dropdown
//! takes an index, and nothing about those rules is repeated here. Bangs hold
//! no value and are never saved. The meta `master` is never saved either
//! (PLAN section 9: it is the operator's fader, not part of a look).
//!
//! A third channel, `media` (D223), is on only in a preset saved while
//! `syn_Media` showed something other than the scene's own `MEDIA`; it
//! then names that picture or folder. The host applies it, since a preset
//! knows nothing of the renderer.
//!
//! **Applying is by name** (the same normalised match the script API uses),
//! so a preset keeps working when a scene gains or reorders controls. A name
//! the scene no longer has is *reported and skipped*, never an error: the
//! rest of the preset still applies. A control the preset does not mention is
//! left alone, which is what lets a hand-written preset hold two values.
//!
//! **Names are file names, so they are checked.** A preset name arrives from
//! a text field, a playlist file and an OSC address, and becomes a path.
//! [`valid_name`] allows letters, digits, space, `-`, `_` and `.` (not first),
//! at most [`MAX_NAME`] characters, so no separator or `..` can get through,
//! and [`load`] finds a preset by *listing the folder* and comparing stems,
//! never by joining the name onto a path.
//!
//! [`FavSlots`] are global to the show, not to a scene: slot 3 is "this scene
//! with that preset", recalled by a pad, a key or an OSC message. The app
//! stores them in its show-control file.

use crate::control::ControlBank;
use crate::control::ControlKind;
use crate::names::normalize;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The folder inside a scene folder that holds presets.
pub const DIR: &str = "presets";

/// The preset a scene may name `default` to be applied when it is loaded.
pub const STARTUP: &str = "default";

/// The longest preset name.
pub const MAX_NAME: usize = 48;

/// How many fav slots there are.
pub const FAV_SLOTS: usize = 8;

/// The meta control that presets never save.
const UNSAVED_META: &str = "master";

/// Why a preset could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum PresetError {
    /// The name cannot be a file name.
    #[error("\"{0}\" is not a usable preset name: use up to 48 letters, digits, spaces, - _ and .")]
    BadName(String),
    /// No preset has this name in the scene.
    #[error("the scene has no preset called \"{0}\"")]
    Missing(String),
    /// A rename would replace another preset.
    #[error("the scene already has a preset called \"{0}\"")]
    Exists(String),
    /// A file could not be read, written or listed.
    #[error("{path}: {source}")]
    Io {
        /// The file or folder.
        path: PathBuf,
        /// The cause.
        source: std::io::Error,
    },
    /// The file is not a preset.
    #[error("{path}: {source}")]
    Json {
        /// The file.
        path: PathBuf,
        /// The cause.
        source: serde_json::Error,
    },
}

/// Which banks a preset covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Channels {
    /// The scene's own controls.
    #[serde(default = "yes")]
    pub scene: bool,
    /// The meta controls.
    #[serde(default = "yes")]
    pub meta: bool,
    /// What `syn_Media` shows (D223). Off unless the preset was saved
    /// with media chosen, so a preset from before it never touches media.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub media: bool,
}

fn yes() -> bool {
    true
}

impl Default for Channels {
    fn default() -> Self {
        Self {
            scene: true,
            meta: true,
            media: false,
        }
    }
}

/// One control's saved value: a number, or one per component.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Value {
    /// A single number (every one-component control; a dropdown's index).
    One(f32),
    /// One number per component (an XY pad, a colour).
    Many(Vec<f32>),
}

/// A named set of control values.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Preset {
    /// The preset's name.
    #[serde(default)]
    pub name: String,
    /// Which banks it covers.
    #[serde(default)]
    pub channels: Channels,
    /// Scene controls by declared name.
    #[serde(default)]
    pub scene: BTreeMap<String, Value>,
    /// Meta controls by name.
    #[serde(default)]
    pub meta: BTreeMap<String, Value>,
    /// The picture or folder behind `syn_Media` when the media channel is
    /// on: relative to the scene folder when it is inside it, else as it
    /// was chosen (D223).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub media: Option<String>,
}

impl Preset {
    /// The current targets of `scene` and `meta`, for the channels asked for.
    pub fn capture(
        name: &str,
        scene: &ControlBank,
        meta: &ControlBank,
        channels: Channels,
    ) -> Self {
        Self {
            name: name.to_string(),
            channels,
            scene: if channels.scene {
                snapshot(scene, None)
            } else {
                BTreeMap::new()
            },
            meta: if channels.meta {
                snapshot(meta, Some(UNSAVED_META))
            } else {
                BTreeMap::new()
            },
            media: None,
        }
    }

    /// This preset with `media` (what `syn_Media` showed when it was saved)
    /// on its media channel, or the channel off for `None` (D223). A path
    /// inside `scene_dir` is kept relative, so the scene folder can move.
    #[must_use]
    pub fn with_media(mut self, scene_dir: &Path, media: Option<&Path>) -> Self {
        self.channels.media = media.is_some();
        self.media = media.map(|path| {
            path.strip_prefix(scene_dir)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/")
        });
        self
    }

    /// The media this preset shows, resolved against `scene_dir`, when its
    /// media channel is on.
    pub fn media_path(&self, scene_dir: &Path) -> Option<PathBuf> {
        if !self.channels.media {
            return None;
        }
        self.media.as_deref().map(|path| scene_dir.join(path))
    }

    /// Writes the preset's values into `scene` and `meta` as targets.
    /// Returns what could not be applied, one line each; the rest was.
    pub fn apply(&self, scene: &mut ControlBank, meta: &mut ControlBank) -> Vec<String> {
        let mut problems = Vec::new();
        if self.channels.scene {
            apply_values(&self.scene, scene, "", &mut problems);
        }
        if self.channels.meta {
            apply_values(&self.meta, meta, "meta ", &mut problems);
        }
        problems
    }

    /// The preset as pretty JSON.
    pub fn to_text(&self) -> Result<String, serde_json::Error> {
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        Ok(text)
    }

    /// Reads a preset from JSON. A leading byte-order mark, as Windows
    /// editors write into a hand-edited preset, is skipped.
    pub fn from_text(text: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(
            text.strip_prefix(crate::load::BYTE_ORDER_MARK)
                .unwrap_or(text),
        )
    }
}

fn snapshot(bank: &ControlBank, skip: Option<&str>) -> BTreeMap<String, Value> {
    let mut values = BTreeMap::new();
    for (index, def) in bank.defs().iter().enumerate() {
        if matches!(def.kind, ControlKind::Bang(_)) {
            continue;
        }
        if skip.is_some_and(|skip| normalize(&def.name) == skip) {
            continue;
        }
        let Some(target) = bank.target(index) else {
            continue;
        };
        let dims = def.dims();
        let value = if dims <= 1 {
            Value::One(target[0])
        } else {
            Value::Many(target[..dims].to_vec())
        };
        values.insert(def.name.clone(), value);
    }
    values
}

fn apply_values(
    values: &BTreeMap<String, Value>,
    bank: &mut ControlBank,
    what: &str,
    problems: &mut Vec<String>,
) {
    for (name, value) in values {
        let Some(index) = bank.find(name) else {
            problems.push(format!("{what}control \"{name}\" is not in this scene"));
            continue;
        };
        let Some(def) = bank.defs().get(index) else {
            continue;
        };
        let dims = def.dims();
        let numbers: Vec<f32> = match value {
            Value::One(x) => vec![*x; dims.max(1)],
            Value::Many(xs) if xs.len() == dims => xs.clone(),
            Value::Many(xs) => {
                problems.push(format!(
                    "{what}control \"{name}\" has {dims} component(s), the preset gives {}",
                    xs.len()
                ));
                continue;
            }
        };
        if numbers.iter().any(|x| !x.is_finite()) {
            problems.push(format!(
                "{what}control \"{name}\" has a value that is not a number"
            ));
            continue;
        }
        if matches!(def.kind, ControlKind::Bang(_)) {
            problems.push(format!(
                "{what}control \"{name}\" is a bang and has no value"
            ));
            continue;
        }
        bank.set(index, None, &numbers);
    }
}

/// Whether `name` can be a preset name (and so a file name).
pub fn valid_name(name: &str) -> bool {
    let trimmed = name.trim();
    !trimmed.is_empty()
        && trimmed == name
        && name.chars().count() <= MAX_NAME
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.'))
}

/// The folder a scene's presets are in.
pub fn dir_of(scene_dir: &Path) -> PathBuf {
    scene_dir.join(DIR)
}

/// The names of a scene's presets, sorted, from the file stems. A scene with
/// no presets folder has none.
pub fn list(scene_dir: &Path) -> Vec<String> {
    let dir = dir_of(scene_dir);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| {
            let path = entry.path();
            let is_json = path
                .extension()
                .is_some_and(|ext| ext.eq_ignore_ascii_case("json"));
            let stem = path.file_stem()?.to_str()?.to_string();
            (is_json && path.is_file() && valid_name(&stem)).then_some(stem)
        })
        .collect();
    names.sort_by_key(|name| name.to_lowercase());
    names
}

/// The entry of `names` that `wanted` means: the same name ignoring case, or
/// the same with each `_` read as a space (an OSC address cannot hold a
/// space, so `/strobe/preset/Slow_Burn` reaches "Slow Burn"). An exact match
/// wins over the underscore reading.
pub fn find_name<'a>(names: &'a [String], wanted: &str) -> Option<&'a String> {
    names
        .iter()
        .find(|candidate| candidate.eq_ignore_ascii_case(wanted))
        .or_else(|| {
            let spaced = wanted.replace('_', " ");
            names
                .iter()
                .find(|candidate| candidate.eq_ignore_ascii_case(&spaced))
        })
}

/// Reads the preset called `name` (case-insensitive) from `scene_dir`.
pub fn load(scene_dir: &Path, name: &str) -> Result<Preset, PresetError> {
    if !valid_name(name) {
        return Err(PresetError::BadName(name.to_string()));
    }
    let names = list(scene_dir);
    let stem = find_name(&names, name).ok_or_else(|| PresetError::Missing(name.to_string()))?;
    let path = dir_of(scene_dir).join(format!("{stem}.json"));
    let text = std::fs::read_to_string(&path).map_err(|source| PresetError::Io {
        path: path.clone(),
        source,
    })?;
    Preset::from_text(&text).map_err(|source| PresetError::Json { path, source })
}

/// Every preset of a scene, read once (when the scene is loaded) so that
/// recalling one during a show touches no disk. Returns the presets by name,
/// and one line for each file that could not be read; the rest are kept.
pub fn load_all(scene_dir: &Path) -> (Vec<(String, Preset)>, Vec<String>) {
    let mut loaded = Vec::new();
    let mut problems = Vec::new();
    for name in list(scene_dir) {
        match load(scene_dir, &name) {
            Ok(preset) => loaded.push((name, preset)),
            Err(error) => problems.push(error.to_string()),
        }
    }
    (loaded, problems)
}

/// Writes `preset` as `presets/<name>.json`, replacing a preset of that
/// name. The file appears whole or not at all (written aside, then renamed).
pub fn save(scene_dir: &Path, preset: &Preset) -> Result<PathBuf, PresetError> {
    if !valid_name(&preset.name) {
        return Err(PresetError::BadName(preset.name.clone()));
    }
    let dir = dir_of(scene_dir);
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source| PresetError::Io { path, source }
    };
    std::fs::create_dir_all(&dir).map_err(io(&dir))?;
    // Replace an existing preset whose name differs only in case, instead of
    // leaving two files that case-folding filesystems would call one.
    let stem = list(scene_dir)
        .into_iter()
        .find(|candidate| candidate.eq_ignore_ascii_case(&preset.name))
        .unwrap_or_else(|| preset.name.clone());
    let path = dir.join(format!("{stem}.json"));
    let temp = dir.join(format!("{stem}.json.tmp"));
    let text = preset.to_text().map_err(|source| PresetError::Json {
        path: path.clone(),
        source,
    })?;
    std::fs::write(&temp, text).map_err(io(&temp))?;
    std::fs::rename(&temp, &path).map_err(io(&path))?;
    Ok(path)
}

/// Deletes the preset called `name`.
pub fn delete(scene_dir: &Path, name: &str) -> Result<(), PresetError> {
    if !valid_name(name) {
        return Err(PresetError::BadName(name.to_string()));
    }
    let stem = list(scene_dir)
        .into_iter()
        .find(|candidate| candidate.eq_ignore_ascii_case(name))
        .ok_or_else(|| PresetError::Missing(name.to_string()))?;
    let path = dir_of(scene_dir).join(format!("{stem}.json"));
    std::fs::remove_file(&path).map_err(|source| PresetError::Io { path, source })
}

/// Renames preset `from` to `to` (P-07). The new file is written whole
/// before the old one is removed, so a crash leaves at least one copy.
/// Refused when another preset already has the new name (a change of case
/// only is allowed): a rename never replaces anything.
pub fn rename(scene_dir: &Path, from: &str, to: &str) -> Result<PathBuf, PresetError> {
    let to = to.trim();
    if !valid_name(to) {
        return Err(PresetError::BadName(to.to_string()));
    }
    let names = list(scene_dir);
    let old = find_name(&names, from)
        .cloned()
        .ok_or_else(|| PresetError::Missing(from.to_string()))?;
    if !old.eq_ignore_ascii_case(to) && find_name(&names, to).is_some() {
        return Err(PresetError::Exists(to.to_string()));
    }
    let mut preset = load(scene_dir, &old)?;
    preset.name = to.to_string();
    let dir = dir_of(scene_dir);
    let path = dir.join(format!("{to}.json"));
    let temp = dir.join(format!("{to}.json.tmp"));
    let text = preset.to_text().map_err(|source| PresetError::Json {
        path: path.clone(),
        source,
    })?;
    let io = |path: &Path| {
        let path = path.to_path_buf();
        move |source| PresetError::Io { path, source }
    };
    std::fs::write(&temp, text).map_err(io(&temp))?;
    let old_path = dir.join(format!("{old}.json"));
    if old.eq_ignore_ascii_case(to) {
        // A change of case: on a case-insensitive filesystem the old file
        // *is* the new one, so it is replaced in place, never removed.
        std::fs::rename(&temp, &old_path).map_err(io(&old_path))?;
        if old != to {
            std::fs::rename(&old_path, &path).map_err(io(&path))?;
        }
    } else {
        std::fs::rename(&temp, &path).map_err(io(&path))?;
        std::fs::remove_file(&old_path).map_err(io(&old_path))?;
    }
    Ok(path)
}

/// One fav slot: a scene, and which of its presets to recall with it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Fav {
    /// The scene's library name.
    pub scene: String,
    /// The preset to apply after loading it, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preset: Option<String>,
}

/// The show's fav slots.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct FavSlots {
    slots: Vec<Option<Fav>>,
}

impl Default for FavSlots {
    fn default() -> Self {
        Self {
            slots: vec![None; FAV_SLOTS],
        }
    }
}

impl FavSlots {
    /// Slot `slot` (0 to 7), if it exists and is filled.
    pub fn get(&self, slot: usize) -> Option<&Fav> {
        self.slots.get(slot).and_then(Option::as_ref)
    }

    /// Fills slot `slot`. Returns false for a slot that does not exist.
    pub fn set(&mut self, slot: usize, fav: Fav) -> bool {
        self.pad();
        match self.slots.get_mut(slot) {
            Some(entry) => {
                *entry = Some(fav);
                true
            }
            None => false,
        }
    }

    /// Empties slot `slot`.
    pub fn clear(&mut self, slot: usize) {
        if let Some(entry) = self.slots.get_mut(slot) {
            *entry = None;
        }
    }

    /// Every slot in order, filled or not.
    pub fn iter(&self) -> impl Iterator<Item = Option<&Fav>> {
        (0..FAV_SLOTS).map(|slot| self.get(slot))
    }

    /// Points every slot that recalls `scene` with preset `from` at `to`
    /// instead, after a rename. Returns how many changed.
    pub fn rename_preset(&mut self, scene: &str, from: &str, to: &str) -> usize {
        let mut changed = 0;
        for fav in self.slots.iter_mut().flatten() {
            if fav.scene == scene
                && fav
                    .preset
                    .as_deref()
                    .is_some_and(|p| p.eq_ignore_ascii_case(from))
            {
                fav.preset = Some(to.to_string());
                changed += 1;
            }
        }
        changed
    }

    /// Makes the list exactly [`FAV_SLOTS`] long, whatever a file held.
    pub fn pad(&mut self) {
        self.slots.resize(FAV_SLOTS, None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::ControlDef;
    use crate::json::RawControl;
    use anyhow::{Result, anyhow};

    fn bank(controls: &[&str]) -> Result<ControlBank> {
        let mut defs = Vec::new();
        for json in controls {
            let raw: RawControl = serde_json::from_str(json)?;
            defs.push(ControlDef::from_raw(&raw)?);
        }
        Ok(ControlBank::new(defs)?)
    }

    fn scene_bank() -> Result<ControlBank> {
        bank(&[
            r#"{ "NAME": "speed", "TYPE": "slider", "MIN": 0, "MAX": 10, "DEFAULT": 1 }"#,
            r#"{ "NAME": "mode", "TYPE": "dropdown", "LABELS": ["a","b","c"], "VALUES": [0,1,2] }"#,
            r#"{ "NAME": "tint", "TYPE": "color", "DEFAULT": [1,0,0] }"#,
            r#"{ "NAME": "flash", "TYPE": "bang" }"#,
            r#"{ "NAME": "on", "TYPE": "toggle" }"#,
        ])
    }

    fn value(bank: &ControlBank, name: &str) -> Result<Vec<f32>> {
        let index = bank.find(name).ok_or_else(|| anyhow!("no {name}"))?;
        let def = bank.defs().get(index).ok_or_else(|| anyhow!("def"))?;
        let target = bank.target(index).ok_or_else(|| anyhow!("target"))?;
        Ok(target[..def.dims()].to_vec())
    }

    #[test]
    fn capture_then_apply_restores_every_value_and_skips_bangs_and_master() -> Result<()> {
        let mut scene = scene_bank()?;
        let mut meta = crate::meta::bank()?;
        scene.set(0, None, &[7.5]);
        scene.set(1, None, &[2.0]);
        scene.set(2, None, &[0.1, 0.2, 0.3]);
        scene.press(4);
        let hue = meta.find("hue").ok_or_else(|| anyhow!("hue"))?;
        meta.set_normalized(hue, None, &[0.5]);
        let master = meta.find("master").ok_or_else(|| anyhow!("master"))?;
        meta.set(master, None, &[0.25]);

        let preset = Preset::capture("look", &scene, &meta, Channels::default());
        assert!(!preset.scene.contains_key("flash"), "a bang has no value");
        assert!(!preset.meta.contains_key("master"), "master is not saved");

        let mut other = scene_bank()?;
        let mut other_meta = crate::meta::bank()?;
        let problems = preset.apply(&mut other, &mut other_meta);
        assert!(problems.is_empty(), "no problems: {problems:?}");
        assert_eq!(value(&other, "speed")?, vec![7.5], "slider");
        assert_eq!(value(&other, "mode")?, vec![2.0], "dropdown index");
        assert_eq!(value(&other, "tint")?, vec![0.1, 0.2, 0.3], "colour");
        assert_eq!(value(&other, "on")?, vec![1.0], "toggle");
        assert_eq!(value(&other_meta, "hue")?, vec![0.5], "meta");
        assert_eq!(value(&other_meta, "master")?, vec![1.0], "master untouched");
        Ok(())
    }

    #[test]
    fn a_hand_written_preset_saved_with_a_byte_order_mark_reads() -> Result<()> {
        let preset = Preset::from_text("\u{feff}{ \"scene\": { \"speed\": 2 } }")?;
        assert_eq!(
            preset.scene.get("speed"),
            Some(&Value::One(2.0)),
            "read past the mark"
        );
        Ok(())
    }

    #[test]
    fn apply_goes_through_the_banks_rules() -> Result<()> {
        let mut scene = scene_bank()?;
        let mut meta = crate::meta::bank()?;
        let mut preset = Preset::default();
        preset.scene.insert("SPEED".into(), Value::One(99.0));
        preset.scene.insert("On".into(), Value::One(0.9));
        preset.scene.insert("mode".into(), Value::One(7.0));
        let problems = preset.apply(&mut scene, &mut meta);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(value(&scene, "speed")?, vec![10.0], "clamped to MAX");
        assert_eq!(value(&scene, "on")?, vec![1.0], "toggle snaps to an end");
        assert_eq!(
            value(&scene, "mode")?,
            vec![2.0],
            "dropdown clamped to its last"
        );
        Ok(())
    }

    #[test]
    fn what_cannot_apply_is_reported_and_the_rest_still_applies() -> Result<()> {
        let mut scene = scene_bank()?;
        let mut meta = crate::meta::bank()?;
        let mut preset = Preset::default();
        preset.scene.insert("gone".into(), Value::One(1.0));
        preset
            .scene
            .insert("tint".into(), Value::Many(vec![1.0, 2.0]));
        preset.scene.insert("flash".into(), Value::One(1.0));
        preset.scene.insert("speed".into(), Value::One(f32::NAN));
        preset.scene.insert("mode".into(), Value::One(1.0));
        let problems = preset.apply(&mut scene, &mut meta);
        assert_eq!(problems.len(), 4, "four problems: {problems:?}");
        assert_eq!(value(&scene, "mode")?, vec![1.0], "the good one applied");
        assert_eq!(value(&scene, "speed")?, vec![1.0], "NaN left the default");
        Ok(())
    }

    #[test]
    fn a_channel_that_is_off_is_neither_saved_nor_applied() -> Result<()> {
        let scene = scene_bank()?;
        let mut meta = crate::meta::bank()?;
        let only_scene = Channels {
            scene: true,
            meta: false,
            media: false,
        };
        let preset = Preset::capture("x", &scene, &meta, only_scene);
        assert!(preset.meta.is_empty(), "meta not captured");
        let mut with_meta = Preset::capture("y", &scene, &meta, Channels::default());
        with_meta.channels.meta = false;
        with_meta.meta.insert("hue".into(), Value::One(0.9));
        let mut scene = scene_bank()?;
        with_meta.apply(&mut scene, &mut meta);
        assert_eq!(value(&meta, "hue")?, vec![0.0], "meta channel off");
        Ok(())
    }

    #[test]
    fn media_is_kept_relative_inside_the_scene_and_off_in_old_files() -> Result<()> {
        let scene = scene_bank()?;
        let meta = crate::meta::bank()?;
        let dir = Path::new("/lib/glow.synScene");
        let inside = Preset::capture("a", &scene, &meta, Channels::default())
            .with_media(dir, Some(&dir.join("loops/rain")));
        assert!(inside.channels.media, "the channel is on");
        assert_eq!(inside.media.as_deref(), Some("loops/rain"), "relative");
        assert_eq!(
            inside.media_path(Path::new("/moved/glow.synScene")),
            Some(PathBuf::from("/moved/glow.synScene/loops/rain")),
            "so the folder can move"
        );
        let elsewhere = Preset::capture("b", &scene, &meta, Channels::default())
            .with_media(dir, Some(Path::new("/media/sky.png")));
        assert_eq!(
            elsewhere.media_path(dir),
            Some(PathBuf::from("/media/sky.png")),
            "a path outside the scene stays as chosen"
        );
        let round = Preset::from_text(&inside.to_text()?)?;
        assert_eq!(round, inside, "round trip");

        let old = Preset::from_text(r#"{ "name": "old", "scene": { "speed": 0.2 } }"#)?;
        assert!(!old.channels.media, "an old preset has no media channel");
        assert_eq!(old.media_path(dir), None, "and shows no media");
        let none = Preset::capture("c", &scene, &meta, Channels::default()).with_media(dir, None);
        assert!(
            !none.to_text()?.contains("media"),
            "nothing about media is written without it"
        );
        Ok(())
    }

    #[test]
    fn a_single_number_for_a_colour_fills_every_component() -> Result<()> {
        let mut scene = scene_bank()?;
        let mut meta = crate::meta::bank()?;
        let mut preset = Preset::default();
        preset.scene.insert("tint".into(), Value::One(0.5));
        preset.apply(&mut scene, &mut meta);
        assert_eq!(value(&scene, "tint")?, vec![0.5, 0.5, 0.5], "all three");
        Ok(())
    }

    #[test]
    fn names_that_could_escape_the_folder_are_refused() {
        for bad in [
            "",
            " ",
            " lead",
            "trail ",
            "..",
            ".hidden",
            "a/b",
            "a\\b",
            "../x",
            "x/../y",
            "c:evil",
            "nul\0",
            &"x".repeat(MAX_NAME + 1),
        ] {
            assert!(!valid_name(bad), "{bad:?} must be refused");
        }
        for good in [
            "slow",
            "Slow Burn",
            "a-b_c.d",
            "v2",
            "x".repeat(MAX_NAME).as_str(),
        ] {
            assert!(valid_name(good), "{good:?} must be allowed");
        }
    }

    fn temp_scene(tag: &str) -> Result<PathBuf> {
        let dir = std::env::temp_dir().join(format!("strobe-preset-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    #[test]
    fn a_rename_moves_the_file_never_replaces_and_follows_the_favs() -> Result<()> {
        let dir = temp_scene("rename")?;
        let scene = scene_bank()?;
        let meta = crate::meta::bank()?;
        for name in ["Slow Burn", "Peak"] {
            save(
                &dir,
                &Preset::capture(name, &scene, &meta, Channels::default()),
            )?;
        }
        rename(&dir, "slow burn", "Warm Up")?;
        assert_eq!(list(&dir), vec!["Peak", "Warm Up"], "moved");
        assert_eq!(
            load(&dir, "warm up")?.name,
            "Warm Up",
            "the file says its new name"
        );
        assert!(
            matches!(rename(&dir, "Warm Up", "peak"), Err(PresetError::Exists(_))),
            "a rename never replaces another preset"
        );
        rename(&dir, "Warm Up", "WARM UP")?;
        assert_eq!(
            list(&dir),
            vec!["Peak", "WARM UP"],
            "a change of case is allowed"
        );
        assert!(
            matches!(rename(&dir, "nope", "x"), Err(PresetError::Missing(_))),
            "missing"
        );

        let mut favs = FavSlots::default();
        favs.set(
            0,
            Fav {
                scene: "05".into(),
                preset: Some("Slow Burn".into()),
            },
        );
        favs.set(
            1,
            Fav {
                scene: "06".into(),
                preset: Some("Slow Burn".into()),
            },
        );
        assert_eq!(
            favs.rename_preset("05", "slow burn", "Warm Up"),
            1,
            "one slot"
        );
        assert_eq!(
            favs.get(0).and_then(|f| f.preset.clone()).as_deref(),
            Some("Warm Up"),
            "this scene's slot follows"
        );
        assert_eq!(
            favs.get(1).and_then(|f| f.preset.clone()).as_deref(),
            Some("Slow Burn"),
            "another scene's slot does not"
        );
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn save_list_load_and_delete_round_trip_in_the_scene_folder() -> Result<()> {
        let dir = temp_scene("roundtrip")?;
        let scene = scene_bank()?;
        let meta = crate::meta::bank()?;
        assert!(list(&dir).is_empty(), "no folder, no presets");
        let preset = Preset::capture("Slow Burn", &scene, &meta, Channels::default());
        let path = save(&dir, &preset)?;
        assert!(
            path.ends_with("presets/Slow Burn.json"),
            "{}",
            path.display()
        );
        save(
            &dir,
            &Preset::capture("b", &scene, &meta, Channels::default()),
        )?;
        assert_eq!(list(&dir), vec!["b", "Slow Burn"], "sorted without case");
        let loaded = load(&dir, "slow burn")?;
        assert_eq!(
            loaded, preset,
            "case-insensitive load returns what was saved"
        );
        // Saving over a name that differs only in case replaces that file.
        save(
            &dir,
            &Preset::capture("SLOW BURN", &scene, &meta, Channels::default()),
        )?;
        assert_eq!(list(&dir).len(), 2, "still two files");
        assert!(
            matches!(load(&dir, "nope"), Err(PresetError::Missing(_))),
            "missing"
        );
        assert!(
            matches!(load(&dir, "../x"), Err(PresetError::BadName(_))),
            "bad name"
        );
        delete(&dir, "b")?;
        assert_eq!(list(&dir), vec!["Slow Burn"], "deleted");
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn load_all_keeps_the_good_presets_and_reports_the_bad_one() -> Result<()> {
        let dir = temp_scene("loadall")?;
        let scene = scene_bank()?;
        let meta = crate::meta::bank()?;
        save(
            &dir,
            &Preset::capture("good", &scene, &meta, Channels::default()),
        )?;
        std::fs::write(dir_of(&dir).join("bad.json"), "{ nope")?;
        let (loaded, problems) = load_all(&dir);
        assert_eq!(loaded.len(), 1, "one good");
        assert_eq!(loaded[0].0, "good", "by name");
        assert_eq!(problems.len(), 1, "one bad: {problems:?}");
        assert!(problems[0].contains("bad.json"), "{}", problems[0]);
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn a_damaged_preset_file_is_an_error_naming_the_file() -> Result<()> {
        let dir = temp_scene("damaged")?;
        std::fs::create_dir_all(dir_of(&dir))?;
        std::fs::write(dir_of(&dir).join("bad.json"), "{ nope")?;
        let error = load(&dir, "bad")
            .err()
            .ok_or_else(|| anyhow!("should fail"))?;
        assert!(error.to_string().contains("bad.json"), "{error}");
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }

    #[test]
    fn an_underscore_reads_as_a_space_but_an_exact_name_wins() {
        let names = vec![
            "Slow Burn".to_string(),
            "a_b".to_string(),
            "a b".to_string(),
        ];
        assert_eq!(
            find_name(&names, "slow_burn"),
            Some(&names[0]),
            "underscore"
        );
        assert_eq!(find_name(&names, "SLOW BURN"), Some(&names[0]), "case");
        assert_eq!(find_name(&names, "a_b"), Some(&names[1]), "exact first");
        assert_eq!(find_name(&names, "nope"), None, "none");
    }

    #[test]
    fn a_hand_written_preset_needs_only_the_values() -> Result<()> {
        let preset = Preset::from_text(r#"{ "scene": { "speed": 3 } }"#)?;
        assert_eq!(
            preset.channels,
            Channels::default(),
            "both channels by default"
        );
        assert_eq!(preset.scene.get("speed"), Some(&Value::One(3.0)), "value");
        Ok(())
    }

    #[test]
    fn fav_slots_are_eight_and_a_short_file_is_padded() -> Result<()> {
        let mut favs: FavSlots = serde_json::from_str(r#"{ "slots": [ { "scene": "a" } ] }"#)?;
        favs.pad();
        assert_eq!(favs.iter().count(), FAV_SLOTS, "eight");
        assert_eq!(favs.get(0).map(|f| f.scene.as_str()), Some("a"), "slot 0");
        assert!(favs.get(1).is_none(), "slot 1 empty");
        assert!(
            favs.set(
                7,
                Fav {
                    scene: "z".into(),
                    preset: Some("p".into())
                }
            ),
            "last slot"
        );
        assert!(
            !favs.set(
                8,
                Fav {
                    scene: "z".into(),
                    preset: None
                }
            ),
            "no ninth"
        );
        favs.clear(0);
        assert!(favs.get(0).is_none(), "cleared");
        let text = serde_json::to_string(&favs)?;
        assert_eq!(serde_json::from_str::<FavSlots>(&text)?, favs, "round trip");
        Ok(())
    }
}
