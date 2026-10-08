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
//! Strobe does not use (and Strobe's own `X_STROBE` block is read by
//! later phases), and a scene that loads in Synesthesia should not fail
//! here because it carries one more key than this file lists.

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
    pub controls: Vec<RawControl>,
    /// Render passes, in execution order. Empty means one implicit pass.
    pub passes: Vec<RawPass>,
    /// Still images bound as samplers.
    pub images: Vec<RawImage>,
    /// Cube maps bound as samplers.
    pub cubemaps: Vec<RawCubemap>,
    /// Uniforms that jump to a new listed value on a song change.
    pub hard_transitions: Vec<RawHardTransition>,
    /// One-hot uniform families that crossfade on a song change.
    pub smooth_transitions: Vec<RawSmoothTransition>,
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

/// One control.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE", default)]
pub struct RawControl {
    /// Becomes the uniform name.
    pub name: String,
    /// Base type plus modifiers, e.g. `"slider smooth"`.
    #[serde(rename = "TYPE")]
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
}

/// One image.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE", default)]
pub struct RawImage {
    /// The sampler name.
    pub name: String,
    /// Path relative to the scene folder.
    pub path: String,
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
