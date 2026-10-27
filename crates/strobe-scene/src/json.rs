// ============================================================================
// Module:       strobe_scene::json
// Description:  scene.json exactly as written: the raw serde shape, tolerant of
//               what real scenes contain, before any validation.
//
// Dependencies: serde, serde_json
// ============================================================================

//! The raw `scene.json`.
//!
//! These types mirror the file, not the meaning: every field is optional
//! and every number that SSF allows to be an array is a [`Components`].
//! Validation — is this control type real, does the default have the
//! right arity, is the name a GLSL identifier — happens in
//! [`crate::scene`], where an error can say what is wrong in terms of the
//! scene rather than in terms of serde.
//!
//! Unknown keys are ignored rather than rejected. Real scenes carry keys
//! Strobe does not use (an `X_STROBE` block among them: Strobe's own
//! extensions are root keys instead, D90), and a scene that loads in
//! Synesthesia should not fail here because it carries one more key than
//! this file lists.

use serde::Deserialize;

/// One number or several: SSF writes `"DEFAULT": 0.5` for a slider and
/// `"DEFAULT": [0.5, 0.5]` for an xy pad, under the same key.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum Components {
    /// A single number.
    One(f32),
    /// An array of numbers.
    Many(Vec<f32>),
}

impl Components {
    /// The components as a slice-like vector.
    pub fn to_vec(&self) -> Vec<f32> {
        match self {
            Self::One(value) => vec![*value],
            Self::Many(values) => values.clone(),
        }
    }
}

/// A pass dimension: pixels as a number, or a string such as `"$WIDTH/2"`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum RawSize {
    /// Absolute pixels at the scene's design resolution.
    Pixels(f32),
    /// An expression over `$WIDTH` / `$HEIGHT`.
    Expr(String),
}

/// Reads `null` as the field's default. Real scenes write
/// `"HARD_TRANSITIONS": null` where they mean "none" (D196).
fn null_as_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Option::unwrap_or_default)
}

/// The whole file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE", default)]
pub struct RawScene {
    /// Display name.
    pub title: Option<String>,
    /// Author credit.
    pub credit: Option<String>,
    /// Free text.
    pub description: Option<String>,
    /// Thumbnail path, relative to the folder.
    pub image_path: Option<String>,
    /// Design width in pixels.
    pub width: Option<f32>,
    /// Design height in pixels.
    pub height: Option<f32>,
    /// The controls, in UI order.
    #[serde(deserialize_with = "null_as_default")]
    pub controls: Vec<RawControl>,
    /// Render passes, in execution order. Empty means one implicit pass.
    #[serde(deserialize_with = "null_as_default")]
    pub passes: Vec<RawPass>,
    /// Still images bound as samplers.
    #[serde(deserialize_with = "null_as_default")]
    pub images: Vec<RawImage>,
    /// Cube maps bound as samplers.
    #[serde(deserialize_with = "null_as_default")]
    pub cubemaps: Vec<RawCubemap>,
    /// Uniforms that jump to a new listed value on a song change.
    #[serde(deserialize_with = "null_as_default")]
    pub hard_transitions: Vec<RawHardTransition>,
    /// One-hot uniform families that crossfade on a song change.
    #[serde(deserialize_with = "null_as_default")]
    pub smooth_transitions: Vec<RawSmoothTransition>,
    /// The picture behind `syn_Media`: a still image, or a folder of images
    /// played as a sequence (a Strobe extension, D186).
    pub media: Option<RawMedia>,
    /// One GLSL file per pass, final pass included, instead of one
    /// `main.glsl` for all of them (a Strobe extension).
    pub pass_sources: Option<Vec<String>>,
    /// Where an imported scene came from (a Strobe extension).
    pub source: Option<RawSource>,
    /// Labels for finding and generating (`["dark", "tunnel"]`). Kept as
    /// raw JSON: [`crate::tags::from_value`] reads any shape without
    /// failing the scene.
    pub tags: Option<serde_json::Value>,
}

/// The `SOURCE` block: provenance for a scene converted from elsewhere.
///
/// Every field is free text for people to read; nothing here changes how
/// the scene renders. Unknown keys inside it are ignored like everywhere
/// else, so an importer can record more than this lists.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "snake_case", default)]
pub struct RawSource {
    /// What it was converted from, e.g. `shadertoy`.
    pub kind: Option<String>,
    /// The identifier at the origin (a Shadertoy shader id).
    pub id: Option<String>,
    /// Where to find the original.
    pub url: Option<String>,
    /// The original author.
    pub author: Option<String>,
    /// The original title.
    pub name: Option<String>,
    /// What is known (and not known) about the licence.
    pub license_note: Option<String>,
}

/// One `HARD_TRANSITIONS` entry.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE", default)]
pub struct RawHardTransition {
    /// The uniform it drives.
    pub uniform: String,
    /// The values it chooses between.
    pub values: Vec<f32>,
}

/// One `SMOOTH_TRANSITIONS` entry.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE", default)]
pub struct RawSmoothTransition {
    /// The family of uniforms, exactly one of which is "on".
    pub uniforms: Vec<String>,
    /// Crossfade length in seconds.
    pub duration: Option<f32>,
}

/// One cube map.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE", default)]
pub struct RawCubemap {
    /// The sampler name.
    pub name: String,
    /// Six face paths: +X, -X, +Y, -Y, +Z, -Z.
    pub paths: Vec<String>,
}

/// The `MEDIA` key: a bare path, or an object with options.
#[derive(Debug, Clone, Deserialize)]
#[serde(untagged)]
pub enum RawMedia {
    /// `"MEDIA": "media/backdrop.png"`.
    Path(String),
    /// `"MEDIA": { "PATH": "frames", "FPS": 12 }`.
    Full(RawMediaOptions),
    /// Synesthesia's form, `"MEDIA": [ { "FILTER": "linear", "WRAP": "clamp" } ]`:
    /// input slots with sampler settings and no file (D193).
    Slots(Vec<RawMediaOptions>),
}

/// The object form of `MEDIA`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE", default)]
pub struct RawMediaOptions {
    /// A file (a still image) or a folder (a sequence), relative to the
    /// scene folder.
    pub path: String,
    /// Sequence rate in frames per second. Absent means 24.
    pub fps: Option<f32>,
    /// Whether a sequence repeats. Absent means `true`.
    #[serde(rename = "LOOP")]
    pub looping: Option<bool>,
    /// `repeat`, `clamp` or `mirror`. Absent means `clamp`.
    pub wrap: Option<String>,
    /// `linear`, `nearest` or `mipmap`. Absent means `linear`.
    pub filter: Option<String>,
    /// Flip on load so `_uv` reads it upright. Absent means `true`.
    pub vflip: Option<bool>,
}

/// One control.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE", default)]
pub struct RawControl {
    /// Becomes the uniform name.
    pub name: String,
    /// Base type plus modifiers, e.g. `"slider smooth"`.
    #[serde(rename = "TYPE", deserialize_with = "null_as_default")]
    pub ty: String,
    /// Initial value.
    pub default: Option<Components>,
    /// Lower bound(s).
    pub min: Option<Components>,
    /// Upper bound(s).
    pub max: Option<Components>,
    /// Smoothing amount: roughly `1 / PARAMS` frames to converge.
    pub params: Option<f32>,
    /// Group in the UI, and the unit `randomizeGroup` acts on.
    pub ui_group: Option<String>,
    /// Tooltip text.
    pub description: Option<String>,
    /// Dropdown labels.
    pub labels: Option<Vec<String>>,
    /// Dropdown values.
    pub values: Option<Vec<f32>>,
    /// Color default, an alternative spelling to `DEFAULT`.
    pub def_color: Option<Components>,
}

/// One render pass.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE", default)]
pub struct RawPass {
    /// The sampler name later passes (and next frame) read this pass by.
    pub target: Option<String>,
    /// Width override.
    pub width: Option<RawSize>,
    /// Height override.
    pub height: Option<RawSize>,
    /// Float storage instead of 8-bit. SSF's default is float.
    pub float: Option<bool>,
    /// Keep last frame readable from this pass itself.
    pub double_buffer: Option<bool>,
    /// `repeat`, `clamp` or `mirror`.
    pub wrap: Option<String>,
    /// `linear`, `nearest` or `mipmap`.
    pub filter: Option<String>,
    /// Store this pass in 32-bit float whatever precision the player
    /// defaults to, for a pass that integrates values 16 bits cannot hold
    /// (a Strobe extension).
    pub full_precision: Option<bool>,
    /// Keep `WIDTH` x `HEIGHT` as absolute pixels, whatever the output
    /// size or render scale (a Strobe extension, D191). For simulations,
    /// whose look depends on their grid.
    pub fixed_size: Option<bool>,
    /// Keep what this pass holds when a resize restarts the passes that
    /// read it, as long as its own size does not change (a Strobe
    /// extension, D495). For a pass that carries state over time: a
    /// Shadertoy import's clock.
    pub keep_on_resize: Option<bool>,
}

/// One image.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE", default)]
pub struct RawImage {
    /// The sampler name.
    pub name: String,
    /// Path relative to the scene folder.
    pub path: String,
    /// `repeat`, `clamp` or `mirror`. Absent means `repeat`.
    pub wrap: Option<String>,
    /// `linear`, `nearest` or `mipmap`. Absent means `mipmap`.
    pub filter: Option<String>,
    /// Flip the file vertically on load so `_uv` reads it upright. Absent
    /// means `true`.
    pub vflip: Option<bool>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn a_scalar_and_an_array_both_read_as_components() -> Result<()> {
        let one: Components = serde_json::from_str("0.5")?;
        let many: Components = serde_json::from_str("[0.5, 1]")?;
        assert_eq!(one.to_vec(), vec![0.5], "a bare number is one component");
        assert_eq!(
            many.to_vec(),
            vec![0.5, 1.0],
            "an array keeps its components"
        );
        Ok(())
    }

    #[test]
    fn unknown_keys_are_ignored() -> Result<()> {
        let scene: RawScene = serde_json::from_str(
            r#"{ "TITLE": "t", "SOMETHING_NEW": 1, "X_STROBE": { "TAGS": [] } }"#,
        )?;
        assert_eq!(scene.title.as_deref(), Some("t"), "known keys still read");
        Ok(())
    }

    #[test]
    fn a_pass_size_reads_as_pixels_or_an_expression() -> Result<()> {
        let pass: RawPass =
            serde_json::from_str(r#"{ "TARGET": "a", "WIDTH": 512, "HEIGHT": "$HEIGHT/2" }"#)?;
        assert_eq!(pass.width, Some(RawSize::Pixels(512.0)), "numeric width");
        assert_eq!(
            pass.height,
            Some(RawSize::Expr("$HEIGHT/2".to_string())),
            "expression height"
        );
        Ok(())
    }
}
