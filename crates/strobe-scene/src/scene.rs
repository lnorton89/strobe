// ============================================================================
// Module:       strobe_scene::scene
// Description:  The validated scene: metadata, controls, passes (with the
//               implicit final pass appended), images and transition uniforms.
//
// Dependencies: crate::json (raw input), crate::control, crate::error,
//               crate::names
// ============================================================================

//! The validated scene definition.
//!
//! ## The final pass is implicit
//!
//! `PASSES` lists the *buffer* passes. After them, `renderMain` runs once
//! more with `PASSINDEX` equal to the number of listed passes, and that
//! run is what reaches the screen. Real scenes are written this way — two
//! listed targets and a `PASSINDEX == 2.0` branch for the output — so
//! [`SceneDef::passes`] always ends with that final pass, untargeted, at
//! full resolution.
//!
//! ## Pass sizes are fractions of the design resolution
//!
//! A pass written as `"WIDTH": 960` in a 1920-wide scene is stored as
//! `0.5`. The renderer multiplies by the size it is actually rendering at,
//! so a scene designed at 1080p and played at 720p on an old laptop keeps
//! its passes in proportion instead of rendering a 1080p buffer to show a
//! 720p frame.

use crate::control::ControlDef;
use crate::error::SceneError;
use crate::json::{RawPass, RawScene, RawSize};
use crate::names::{is_glsl_identifier, is_reserved, normalize};
use std::collections::HashMap;

/// The design resolution when `WIDTH` / `HEIGHT` are absent.
pub const DEFAULT_SIZE: [f32; 2] = [1920.0, 1080.0];

/// How a sampler treats coordinates outside `0..1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Wrap {
    /// Tile.
    #[default]
    Repeat,
    /// Hold the edge texel.
    Clamp,
    /// Tile, reflecting every other copy.
    Mirror,
}

/// How a sampler filters between texels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Filter {
    /// Bilinear.
    #[default]
    Linear,
    /// Nearest texel.
    Nearest,
    /// Trilinear over a mip chain, regenerated after each render.
    Mipmap,
}

/// One render pass.
#[derive(Debug, Clone, PartialEq)]
pub struct PassDef {
    /// The sampler name it is read by; `None` for the final pass.
    pub target: Option<String>,
    /// Width and height as fractions of the render resolution.
    pub scale: [f32; 2],
    /// Float storage rather than 8-bit.
    pub float: bool,
    /// Keep the previous frame readable while this pass renders.
    pub double_buffer: bool,
    /// Sampler wrap mode.
    pub wrap: Wrap,
    /// Sampler filter mode.
    pub filter: Filter,
}

/// An image file bound as a `sampler2D`.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageDef {
    /// The sampler name.
    pub name: String,
    /// Path relative to the scene folder.
    pub path: String,
}

/// A cube map bound as a `samplerCube`.
#[derive(Debug, Clone, PartialEq)]
pub struct CubemapDef {
    /// The sampler name.
    pub name: String,
    /// Face paths +X, -X, +Y, -Y, +Z, -Z, relative to the scene folder.
    pub paths: Vec<String>,
}

/// A float uniform driven by song-change transitions rather than a control.
#[derive(Debug, Clone, PartialEq)]
pub struct TransitionUniform {
    /// The uniform name.
    pub name: String,
    /// Its value before any transition fires.
    pub initial: f32,
}

/// A validated scene.
#[derive(Debug, Clone, PartialEq)]
pub struct SceneDef {
    /// Display name.
    pub title: String,
    /// Author credit.
    pub credit: Option<String>,
    /// Free text.
    pub description: Option<String>,
    /// Thumbnail path, relative to the folder.
    pub image_path: Option<String>,
    /// Design resolution.
    pub size: [f32; 2],
    /// Controls, in UI order.
    pub controls: Vec<ControlDef>,
    /// Passes in execution order; the last is the implicit final pass.
    pub passes: Vec<PassDef>,
    /// Image samplers.
    pub images: Vec<ImageDef>,
    /// Cube map samplers.
    pub cubemaps: Vec<CubemapDef>,
    /// Uniforms declared by `HARD_TRANSITIONS` / `SMOOTH_TRANSITIONS`.
    pub transition_uniforms: Vec<TransitionUniform>,
}

impl SceneDef {
    /// Parses and validates `scene.json` text. `fallback_title` names the
    /// scene when `TITLE` is absent (normally the folder name).
    pub fn parse(json: &str, fallback_title: &str) -> Result<Self, SceneError> {
        let raw: RawScene = serde_json::from_str(json)?;
        Self::from_raw(&raw, fallback_title)
    }

    /// Validates a parsed `scene.json`.
    pub fn from_raw(raw: &RawScene, fallback_title: &str) -> Result<Self, SceneError> {
        let size = [
            raw.width.filter(|w| *w >= 1.0).unwrap_or(DEFAULT_SIZE[0]),
            raw.height.filter(|h| *h >= 1.0).unwrap_or(DEFAULT_SIZE[1]),
        ];

        let controls = raw
            .controls
            .iter()
            .map(ControlDef::from_raw)
            .collect::<Result<Vec<_>, _>>()?;

        let mut passes = raw
            .passes
            .iter()
            .enumerate()
            .map(|(index, pass)| pass_def(index, pass, size))
            .collect::<Result<Vec<_>, _>>()?;
        passes.push(PassDef {
            target: None,
            scale: [1.0, 1.0],
            float: false,
            double_buffer: false,
            wrap: Wrap::Clamp,
            filter: Filter::Linear,
        });

        let images = raw
            .images
            .iter()
            .map(|image| ImageDef {
                name: image.name.trim().to_string(),
                path: image.path.clone(),
            })
            .collect::<Vec<_>>();
        let cubemaps = raw
            .cubemaps
            .iter()
            .map(|cube| CubemapDef {
                name: cube.name.trim().to_string(),
                paths: cube.paths.clone(),
            })
            .collect::<Vec<_>>();

        let mut transition_uniforms = Vec::new();
        for hard in &raw.hard_transitions {
            transition_uniforms.push(TransitionUniform {
                name: hard.uniform.trim().to_string(),
                initial: hard.values.first().copied().unwrap_or(0.0),
            });
        }
        for smooth in &raw.smooth_transitions {
            for (i, name) in smooth.uniforms.iter().enumerate() {
                transition_uniforms.push(TransitionUniform {
                    name: name.trim().to_string(),
                    initial: if i == 0 { 1.0 } else { 0.0 },
                });
            }
        }

        let def = Self {
            title: raw
                .title
                .clone()
                .filter(|t| !t.trim().is_empty())
                .unwrap_or_else(|| fallback_title.to_string()),
            credit: raw.credit.clone(),
            description: raw.description.clone(),
            image_path: raw.image_path.clone(),
            size,
            controls,
            passes,
            images,
            cubemaps,
            transition_uniforms,
        };
        def.check_names()?;
        Ok(def)
    }

    /// Every name this scene declares as a uniform, with what declares it.
    pub fn declared_names(&self) -> impl Iterator<Item = &str> {
        self.controls
            .iter()
            .map(|c| c.name.as_str())
            .chain(self.passes.iter().filter_map(|p| p.target.as_deref()))
            .chain(self.images.iter().map(|i| i.name.as_str()))
            .chain(self.cubemaps.iter().map(|c| c.name.as_str()))
            .chain(self.transition_uniforms.iter().map(|t| t.name.as_str()))
    }

    /// The pass index of the final (on-screen) pass.
    pub fn final_pass(&self) -> usize {
        self.passes.len().saturating_sub(1)
    }

    /// Rejects names that are not identifiers, are reserved, or collide.
    fn check_names(&self) -> Result<(), SceneError> {
        let mut seen: HashMap<String, &str> = HashMap::new();
        for name in self.declared_names() {
            if !is_glsl_identifier(name) {
                return Err(SceneError::InvalidName(name.to_string()));
            }
            if is_reserved(name) {
                return Err(SceneError::ReservedName(name.to_string()));
            }
            if let Some(first) = seen.insert(normalize(name), name) {
                return Err(SceneError::NameCollision {
                    first: first.to_string(),
                    second: name.to_string(),
                });
            }
        }
        Ok(())
    }
}

/// Validates one listed pass.
fn pass_def(index: usize, raw: &RawPass, design: [f32; 2]) -> Result<PassDef, SceneError> {
    let scale = [
        size_fraction(index, raw.width.as_ref(), design[0], design)?,
        size_fraction(index, raw.height.as_ref(), design[1], design)?,
    ];
    let wrap = match raw.wrap.as_deref().map(str::to_ascii_lowercase).as_deref() {
        None | Some("repeat") => Wrap::Repeat,
        Some("clamp") => Wrap::Clamp,
        Some("mirror") => Wrap::Mirror,
        Some(other) => {
            return Err(SceneError::PassOption {
                index,
                key: "WRAP",
                value: other.to_string(),
            });
        }
    };
    let filter = match raw
        .filter
        .as_deref()
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        None | Some("linear") => Filter::Linear,
        Some("nearest") => Filter::Nearest,
        Some("mipmap") => Filter::Mipmap,
        Some(other) => {
            return Err(SceneError::PassOption {
                index,
                key: "FILTER",
                value: other.to_string(),
            });
        }
    };
    Ok(PassDef {
        target: raw
            .target
            .as_ref()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty()),
        scale,
        float: raw.float.unwrap_or(true),
        double_buffer: raw.double_buffer.unwrap_or(false),
        wrap,
        filter,
    })
}

/// A pass dimension as a fraction of the design dimension `full`.
///
/// Numbers are pixels at the design resolution. Strings are a small
/// expression language from ISF: `$WIDTH` or `$HEIGHT`, optionally `/ n`
/// or `* n`, or a bare number.
fn size_fraction(
    index: usize,
    raw: Option<&RawSize>,
    full: f32,
    design: [f32; 2],
) -> Result<f32, SceneError> {
    let bad = |value: String| SceneError::PassSize { index, value };
    let pixels = match raw {
        None => return Ok(1.0),
        Some(RawSize::Pixels(px)) => *px,
        Some(RawSize::Expr(expr)) => {
            let compact: String = expr.chars().filter(|c| !c.is_whitespace()).collect();
            let upper = compact.to_ascii_uppercase();
            let (base, rest) = if let Some(rest) = upper.strip_prefix("$WIDTH") {
                (design[0], rest)
            } else if let Some(rest) = upper.strip_prefix("$HEIGHT") {
                (design[1], rest)
            } else {
                (1.0, upper.as_str())
            };
            let number = |s: &str| s.parse::<f32>().map_err(|_| bad(expr.clone()));
            if rest.is_empty() {
                base
            } else if let Some(n) = rest.strip_prefix('/') {
                base / number(n)?
            } else if let Some(n) = rest.strip_prefix('*') {
                base * number(n)?
            } else if base == 1.0 {
                number(rest)?
            } else {
                return Err(bad(expr.clone()));
            }
        }
    };
    if !pixels.is_finite() || pixels < 1.0 {
        return Err(bad(pixels.to_string()));
    }
    Ok(pixels / full)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn an_empty_scene_has_one_final_pass() -> Result<()> {
        let def = SceneDef::parse("{}", "folder")?;
        assert_eq!(def.title, "folder", "title falls back to the folder name");
        assert_eq!(def.passes.len(), 1, "just the final pass");
        assert_eq!(def.final_pass(), 0, "and it is pass 0");
        Ok(())
    }

    #[test]
    fn listed_passes_are_followed_by_the_final_pass() -> Result<()> {
        let def = SceneDef::parse(
            r#"{ "PASSES": [ {"TARGET": "firstBuffer"}, {"TARGET": "secondBuffer"} ] }"#,
            "s",
        )?;
        assert_eq!(def.passes.len(), 3, "two buffers plus the output");
        assert_eq!(def.final_pass(), 2, "PASSINDEX 2 reaches the screen");
        assert!(
            def.passes[2].target.is_none(),
            "the final pass is untargeted"
        );
        Ok(())
    }

    #[test]
    fn pass_defaults_follow_the_documentation() -> Result<()> {
        let def = SceneDef::parse(r#"{ "PASSES": [ {"TARGET": "a"} ] }"#, "s")?;
        let pass = &def.passes[0];
        assert!(pass.float, "FLOAT defaults to true");
        assert!(!pass.double_buffer, "DOUBLE_BUFFER defaults to false");
        assert_eq!(pass.wrap, Wrap::Repeat, "WRAP defaults to repeat");
        assert_eq!(pass.filter, Filter::Linear, "FILTER defaults to linear");
        Ok(())
    }

    #[test]
    fn pass_sizes_become_fractions_of_the_design_size() -> Result<()> {
        let def = SceneDef::parse(
            r#"{ "WIDTH": 1920, "HEIGHT": 1080,
                 "PASSES": [ {"TARGET": "a", "WIDTH": 960, "HEIGHT": "$HEIGHT/4"} ] }"#,
            "s",
        )?;
        assert_eq!(def.passes[0].scale, [0.5, 0.25], "960/1920 and 1080/4/1080");
        Ok(())
    }

    #[test]
    fn transition_uniforms_are_declared() -> Result<()> {
        let def = SceneDef::parse(
            r#"{ "HARD_TRANSITIONS": [ {"UNIFORM": "direction", "VALUES": [-1, 1]} ],
                 "SMOOTH_TRANSITIONS": [ {"UNIFORMS": ["c1", "c2"], "DURATION": 3} ] }"#,
            "s",
        )?;
        let names: Vec<(&str, f32)> = def
            .transition_uniforms
            .iter()
            .map(|t| (t.name.as_str(), t.initial))
            .collect();
        assert_eq!(
            names,
            vec![("direction", -1.0), ("c1", 1.0), ("c2", 0.0)],
            "initial values"
        );
        Ok(())
    }

    #[test]
    fn a_target_colliding_with_a_control_is_rejected() {
        let result = SceneDef::parse(
            r#"{ "CONTROLS": [ {"NAME": "buf", "TYPE": "slider"} ],
                 "PASSES": [ {"TARGET": "buf"} ] }"#,
            "s",
        );
        assert!(
            result.is_err(),
            "a control and a pass cannot share a uniform name"
        );
    }

    #[test]
    fn an_unknown_wrap_names_the_pass() {
        let result = SceneDef::parse(
            r#"{ "PASSES": [ {"TARGET": "a", "WRAP": "spiral"} ] }"#,
            "s",
        );
        let message = result.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(
            message.contains("pass 0") && message.contains("spiral"),
            "got {message}"
        );
    }
}
