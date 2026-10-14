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
//!
//! Literal sizes are the exception: they are absolute pixels (D194). When a
//! literal pass is exactly the scene's root `WIDTH` x `HEIGHT`, the final
//! pass is pinned to that size too (D280, [`SceneDef::pinned_size`]), so a
//! shader reading it with `texelFetch(pass, ivec2(_xy), 0)` reads the
//! whole frame; the output stage scales the result to the window.

use crate::control::ControlDef;
use crate::error::SceneError;
use crate::json::{RawImage, RawMedia, RawMediaOptions, RawPass, RawScene, RawSize};
use crate::names::{is_glsl_identifier, is_reserved};
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
    /// Store in 32-bit float even when the player defaults to 16-bit.
    pub full_precision: bool,
    /// An absolute size in pixels that ignores the output size and render
    /// scale (`FIXED_SIZE`, D191), or `None` to follow [`Self::scale`].
    pub fixed: Option<[i32; 2]>,
}

/// An image file bound as a `sampler2D`.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageDef {
    /// The sampler name.
    pub name: String,
    /// Path relative to the scene folder.
    pub path: String,
    /// Sampler wrap mode. SSF scenes get `Repeat`.
    pub wrap: Wrap,
    /// Sampler filter mode. SSF scenes get `Mipmap`.
    pub filter: Filter,
    /// Whether the file is flipped vertically on load, so `_uv` (origin at
    /// the bottom left) reads it upright. SSF scenes get `true`.
    pub vflip: bool,
}

/// Where a converted scene came from: the `SOURCE` block, as text for
/// people. Nothing here affects rendering.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SceneSource {
    /// What it was converted from, e.g. `shadertoy`.
    pub kind: Option<String>,
    /// The identifier at the origin.
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

/// A cube map bound as a `samplerCube`.
#[derive(Debug, Clone, PartialEq)]
pub struct CubemapDef {
    /// The sampler name.
    pub name: String,
    /// Face paths +X, -X, +Y, -Y, +Z, -Z, relative to the scene folder.
    pub paths: Vec<String>,
}

/// The default sequence rate, frames per second.
pub const DEFAULT_MEDIA_FPS: f32 = 24.0;

/// What `syn_Media` shows when the scene brings its own picture: a still
/// image, or a folder of images played in file-name order (D186).
#[derive(Debug, Clone, PartialEq)]
pub struct MediaDef {
    /// A file or a folder, relative to the scene folder.
    pub path: String,
    /// Sequence rate in frames per second (ignored for a still).
    pub fps: f32,
    /// Whether a sequence repeats.
    pub looping: bool,
    /// Sampler wrap mode. Media gets `Clamp`, so `_uv` offsets do not tile.
    pub wrap: Wrap,
    /// Sampler filter mode. Media gets `Linear`.
    pub filter: Filter,
    /// Whether the files are flipped vertically on load.
    pub vflip: bool,
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
    /// The scene's own `syn_Media` picture, if it names one.
    pub media: Option<MediaDef>,
    /// How media the operator chooses is sampled: the `MEDIA` slot's
    /// `WRAP` and `FILTER`, which default to repeat and linear as SSF
    /// documents (M-01). A scene with media of its own samples the chosen
    /// media as it does its own.
    pub media_sampling: (Wrap, Filter),
    /// One shader file per pass (the final pass last), relative to the
    /// scene folder, when the scene sets `PASS_SOURCES`. `None` means every
    /// pass runs `main.glsl`, as in SSF. When present it has exactly
    /// `passes.len()` entries.
    pub pass_sources: Option<Vec<String>>,
    /// Provenance, when the scene says where it came from.
    pub source: Option<SceneSource>,
    /// `TAGS`, normalised (D205); malformed tags are no tags, never an error.
    pub tags: Vec<String>,
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
        let main = main_size(raw, &passes);
        passes.push(PassDef {
            target: None,
            scale: [1.0, 1.0],
            float: false,
            double_buffer: false,
            wrap: Wrap::Clamp,
            filter: Filter::Linear,
            full_precision: false,
            fixed: main,
        });

        let images = raw
            .images
            .iter()
            .map(image_def)
            .collect::<Result<Vec<_>, _>>()?;
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

        let media = raw.media.as_ref().map(media_def).transpose()?.flatten();
        let media_sampling = media_sampling(raw.media.as_ref())?;
        let pass_sources = match &raw.pass_sources {
            None => None,
            Some(files) => {
                if files.len() != passes.len() {
                    return Err(SceneError::invalid(format!(
                        "PASS_SOURCES lists {} file(s) but the scene has {} pass(es) \
                         (the implicit final pass counts, and comes last)",
                        files.len(),
                        passes.len()
                    )));
                }
                if let Some(bad) = files.iter().find(|file| !is_folder_relative(file)) {
                    return Err(SceneError::invalid(format!(
                        "PASS_SOURCES entry `{bad}` must be a relative path inside the scene folder"
                    )));
                }
                Some(files.clone())
            }
        };
        let source = raw.source.as_ref().map(|s| SceneSource {
            kind: s.kind.clone(),
            id: s.id.clone(),
            url: s.url.clone(),
            author: s.author.clone(),
            name: s.name.clone(),
            license_note: s.license_note.clone(),
        });

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
            media,
            media_sampling,
            pass_sources,
            source,
            tags: raw
                .tags
                .as_ref()
                .map(crate::tags::from_value)
                .unwrap_or_default(),
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

    /// The size the final pass is pinned to, in pixels, or `None` when it
    /// follows the output and the render scale (D280): the scene's root
    /// `WIDTH` x `HEIGHT`, when a buffer pass has exactly that absolute
    /// size, so `ivec2(_xy)` addresses that pass texel for texel.
    pub fn pinned_size(&self) -> Option<[i32; 2]> {
        self.passes.last().and_then(|pass| pass.fixed)
    }

    /// Rejects names that are not identifiers, are reserved, or collide.
    fn check_names(&self) -> Result<(), SceneError> {
        // Exact duplicates collide in GLSL. Names equal only once normalised
        // (`in_or_out`, `inOrOut`) are legal there and in real scenes (D196);
        // the script API then prefers the exact name.
        let mut seen: HashMap<&str, ()> = HashMap::new();
        for name in self.declared_names() {
            if !is_glsl_identifier(name) {
                return Err(SceneError::InvalidName(name.to_string()));
            }
            if is_reserved(name) {
                return Err(SceneError::ReservedName(name.to_string()));
            }
            if seen.insert(name, ()).is_some() {
                return Err(SceneError::NameCollision {
                    first: name.to_string(),
                    second: name.to_string(),
                });
            }
        }
        Ok(())
    }
}

/// A wrap mode from its `scene.json` spelling; `None` means `default`.
fn parse_wrap(text: Option<&str>, default: Wrap) -> Result<Wrap, String> {
    match text.map(str::to_ascii_lowercase).as_deref() {
        None => Ok(default),
        Some("repeat") => Ok(Wrap::Repeat),
        Some("clamp") => Ok(Wrap::Clamp),
        Some("mirror") => Ok(Wrap::Mirror),
        Some(other) => Err(other.to_string()),
    }
}

/// A filter mode from its `scene.json` spelling; `None` means `default`.
fn parse_filter(text: Option<&str>, default: Filter) -> Result<Filter, String> {
    match text.map(str::to_ascii_lowercase).as_deref() {
        None => Ok(default),
        Some("linear") => Ok(Filter::Linear),
        Some("nearest") => Ok(Filter::Nearest),
        Some("mipmap") => Ok(Filter::Mipmap),
        Some(other) => Err(other.to_string()),
    }
}

/// Whether `path` stays inside the scene folder: relative, with no `..`,
/// no drive or root, and no backslash (which means different things on
/// different systems).
pub fn is_folder_relative(path: &str) -> bool {
    use std::path::{Component, Path};
    !path.is_empty()
        && !path.contains(['\\', ':', '\0'])
        && Path::new(path)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

/// Validates one image.
fn image_def(raw: &RawImage) -> Result<ImageDef, SceneError> {
    let name = raw.name.trim().to_string();
    let bad = |key: &str, value: String| {
        SceneError::invalid(format!("image `{name}`: unknown {key} `{value}`"))
    };
    let wrap = parse_wrap(raw.wrap.as_deref(), Wrap::Repeat).map_err(|v| bad("WRAP", v))?;
    let filter =
        parse_filter(raw.filter.as_deref(), Filter::Mipmap).map_err(|v| bad("FILTER", v))?;
    Ok(ImageDef {
        name: name.clone(),
        path: raw.path.clone(),
        wrap,
        filter,
        vflip: raw.vflip.unwrap_or(true),
    })
}

/// Validates the `MEDIA` key. A path that leaves the scene folder is an
/// error here, because unlike a missing file it can never become right.
/// `None` when the key names no file: Synesthesia's own form is a list of
/// input slots with sampler settings only (`[{ "FILTER": ..., "WRAP": ... }]`),
/// filled at run time from the user's media, so the scene loads with no
/// picture until one is given (`--media`, D193).
fn media_def(raw: &RawMedia) -> Result<Option<MediaDef>, SceneError> {
    let options = match raw {
        RawMedia::Path(path) => RawMediaOptions {
            path: path.clone(),
            ..RawMediaOptions::default()
        },
        RawMedia::Full(options) => options.clone(),
        RawMedia::Slots(slots) => slots.first().cloned().unwrap_or_default(),
    };
    if options.path.trim().is_empty() {
        return Ok(None);
    }
    let bad =
        |key: &str, value: String| SceneError::invalid(format!("MEDIA: unknown {key} `{value}`"));
    if !is_folder_relative(&options.path) {
        return Err(SceneError::invalid(format!(
            "MEDIA path `{}` must be a relative path inside the scene folder",
            options.path
        )));
    }
    let fps = options.fps.unwrap_or(DEFAULT_MEDIA_FPS);
    if !fps.is_finite() || fps <= 0.0 {
        return Err(SceneError::invalid(format!(
            "MEDIA FPS must be above zero, got {fps}"
        )));
    }
    Ok(Some(MediaDef {
        path: options.path.clone(),
        fps,
        looping: options.looping.unwrap_or(true),
        wrap: parse_wrap(options.wrap.as_deref(), Wrap::Clamp).map_err(|v| bad("WRAP", v))?,
        filter: parse_filter(options.filter.as_deref(), Filter::Linear)
            .map_err(|v| bad("FILTER", v))?,
        vflip: options.vflip.unwrap_or(true),
    }))
}

/// How the operator's media is sampled in a scene whose `MEDIA` key is
/// `raw` (see [`SceneDef::media_sampling`]).
fn media_sampling(raw: Option<&RawMedia>) -> Result<(Wrap, Filter), SceneError> {
    let (options, wrap) = match raw {
        None => return Ok((Wrap::Repeat, Filter::Linear)),
        Some(RawMedia::Path(_)) => (RawMediaOptions::default(), Wrap::Clamp),
        Some(RawMedia::Full(options)) => (options.clone(), Wrap::Clamp),
        Some(RawMedia::Slots(slots)) => (slots.first().cloned().unwrap_or_default(), Wrap::Repeat),
    };
    let bad =
        |key: &str, value: String| SceneError::invalid(format!("MEDIA: unknown {key} `{value}`"));
    Ok((
        parse_wrap(options.wrap.as_deref(), wrap).map_err(|v| bad("WRAP", v))?,
        parse_filter(options.filter.as_deref(), Filter::Linear).map_err(|v| bad("FILTER", v))?,
    ))
}

/// Validates one listed pass.
fn pass_def(index: usize, raw: &RawPass, design: [f32; 2]) -> Result<PassDef, SceneError> {
    let scale = [
        size_fraction(index, raw.width.as_ref(), design[0], design)?,
        size_fraction(index, raw.height.as_ref(), design[1], design)?,
    ];
    let option = |key: &'static str, value: String| SceneError::PassOption { index, key, value };
    let wrap = parse_wrap(raw.wrap.as_deref(), Wrap::Repeat).map_err(|v| option("WRAP", v))?;
    let filter =
        parse_filter(raw.filter.as_deref(), Filter::Linear).map_err(|v| option("FILTER", v))?;
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
        full_precision: raw.full_precision.unwrap_or(false),
        // Literal pixel sizes are absolute, as in Synesthesia (D194); `$WIDTH`
        // and `$HEIGHT` expressions follow the output and the render scale
        // (D11). FIXED_SIZE overrides either way (D191).
        fixed: raw
            .fixed_size
            .unwrap_or_else(|| is_literal(raw.width.as_ref()) && is_literal(raw.height.as_ref()))
            .then(|| {
                [
                    fixed_pixels(scale[0] * design[0]),
                    fixed_pixels(scale[1] * design[1]),
                ]
            }),
    })
}

/// The final pass's pinned size (D280), from the validated buffer passes.
///
/// A buffer pass with an absolute size equal to the scene's own root
/// `WIDTH` x `HEIGHT` is a full frame at the size the scene was written
/// for, and real scenes read it with `texelFetch(pass, ivec2(_xy), 0)`
/// from the final pass. That reads the right texel only when the final
/// pass is the same size, so it is pinned there, as Synesthesia renders it
/// at its output size. Both root dimensions must be written: the 1920x1080
/// default is a guess, not something the scene said.
fn main_size(raw: &RawScene, passes: &[PassDef]) -> Option<[i32; 2]> {
    let declared = |value: Option<f32>| value.filter(|v| v.is_finite() && *v >= 1.0);
    let root = [
        fixed_pixels(declared(raw.width)?),
        fixed_pixels(declared(raw.height)?),
    ];
    passes
        .iter()
        .any(|pass| pass.fixed == Some(root))
        .then_some(root)
}

/// Whether a pass dimension is a literal number of pixels (not absent, not
/// 0, which means the output size, and not a `$WIDTH` expression).
fn is_literal(raw: Option<&RawSize>) -> bool {
    match raw {
        Some(RawSize::Pixels(px)) => *px > 0.0,
        Some(RawSize::Expr(expr)) => expr.trim().parse::<f32>().is_ok_and(|px| px > 0.0),
        None => false,
    }
}

/// A `FIXED_SIZE` dimension in whole pixels. [`size_fraction`] has already
/// refused anything below one pixel or not finite.
#[expect(
    clippy::cast_possible_truncation,
    reason = "a pass dimension is a few thousand pixels at most"
)]
fn fixed_pixels(pixels: f32) -> i32 {
    (pixels.round() as i32).max(1)
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
    // 0 means "the output size": real scenes write it (D193).
    if pixels == 0.0 {
        return Ok(1.0);
    }
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
    fn literal_pass_sizes_are_absolute_and_expressions_follow_the_output() -> Result<()> {
        // D194 (Q24): real scenes declare data passes in pixels, e.g. 10x10;
        // `$WIDTH` expressions stay relative so render scale still works.
        let def = SceneDef::parse(
            r#"{ "WIDTH": 1920, "HEIGHT": 1080,
                 "PASSES": [ {"TARGET": "data", "WIDTH": 10, "HEIGHT": 10},
                             {"TARGET": "half", "WIDTH": "$WIDTH/2", "HEIGHT": "$HEIGHT/2"},
                             {"TARGET": "free", "WIDTH": 960, "HEIGHT": 540, "FIXED_SIZE": false},
                             {"TARGET": "mixed", "WIDTH": 960, "HEIGHT": "$HEIGHT/2"},
                             {"TARGET": "full"} ] }"#,
            "s",
        )?;
        let fixed: Vec<Option<[i32; 2]>> = def.passes.iter().map(|p| p.fixed).collect();
        assert_eq!(
            fixed,
            vec![Some([10, 10]), None, None, None, None, None],
            "literal fixed; expressions, FIXED_SIZE false, mixed, absent and the final pass follow the output"
        );
        Ok(())
    }

    #[test]
    fn real_synesthesia_shapes_load() -> Result<()> {
        // D193, from 65 real scenes on TORNADO: a control with no TYPE, a
        // pass sized 0x0, and MEDIA as a list of input slots with no file.
        let def = SceneDef::parse(
            r#"{ "WIDTH": 1920, "HEIGHT": 1080,
                 "CONTROLS": [ {"NAME": "amount", "MIN": 0, "MAX": 2, "DEFAULT": 1},
                               {"NAME": "spot", "DEFAULT": [0.5, 0.5]} ],
                 "PASSES": [ {"TARGET": "full", "WIDTH": 0, "HEIGHT": 0} ],
                 "MEDIA": [ {"FILTER": "linear", "WRAP": "clamp"} ] }"#,
            "s",
        )?;
        let kinds: Vec<&str> = def.controls.iter().map(|c| c.kind.glsl_type()).collect();
        assert_eq!(
            kinds,
            vec!["float", "vec2"],
            "a slider, and an xy pad from two defaults"
        );
        assert_eq!(def.passes[0].scale, [1.0, 1.0], "0 means the output size");
        assert!(
            def.media.is_none(),
            "slots with no file load with no picture"
        );
        Ok(())
    }

    #[test]
    fn a_fixed_size_pass_keeps_its_design_pixels() -> Result<()> {
        let def = SceneDef::parse(
            r#"{ "WIDTH": 1920, "HEIGHT": 1080,
                 "PASSES": [ {"TARGET": "a", "WIDTH": 960, "HEIGHT": 540, "FIXED_SIZE": true},
                             {"TARGET": "b", "WIDTH": "$WIDTH/3"} ] }"#,
            "s",
        )?;
        assert_eq!(def.passes[0].fixed, Some([960, 540]), "absolute pixels");
        assert_eq!(def.passes[1].fixed, None, "FIXED_SIZE defaults to off");
        Ok(())
    }

    #[test]
    fn a_literal_pass_at_the_root_size_pins_the_final_pass() -> Result<()> {
        // D280: `texelFetch(pass, ivec2(_xy), 0)` from the final pass reads
        // the whole of a root-sized literal pass only at that size.
        let pinned = |json: &str| SceneDef::parse(json, "s").map(|def| def.pinned_size());
        assert_eq!(
            pinned(
                r#"{ "WIDTH": 1920, "HEIGHT": 1080,
                     "PASSES": [ {"TARGET": "a", "WIDTH": 1920, "HEIGHT": 1080},
                                 {"TARGET": "b", "WIDTH": 10, "HEIGHT": 10} ] }"#
            )?,
            Some([1920, 1080]),
            "a literal pass equal to the root pins the final pass"
        );
        assert_eq!(
            pinned(
                r#"{ "WIDTH": 1280, "HEIGHT": 720,
                     "PASSES": [ {"TARGET": "a", "WIDTH": "1280", "HEIGHT": "720"} ] }"#
            )?,
            Some([1280, 720]),
            "a literal written as a string counts"
        );
        assert_eq!(
            pinned(r#"{ "PASSES": [ {"TARGET": "a", "WIDTH": 1920, "HEIGHT": 1080} ] }"#)?,
            None,
            "no root size declared: the default is not the scene's word"
        );
        assert_eq!(
            pinned(
                r#"{ "WIDTH": 1920, "HEIGHT": 1080,
                     "PASSES": [ {"TARGET": "a", "WIDTH": 1280, "HEIGHT": 720},
                                 {"TARGET": "b", "WIDTH": 10, "HEIGHT": 10} ] }"#
            )?,
            None,
            "literal passes of other sizes leave the final pass free"
        );
        assert_eq!(
            pinned(
                r#"{ "WIDTH": 1920, "HEIGHT": 1080,
                     "PASSES": [ {"TARGET": "a", "WIDTH": "$WIDTH", "HEIGHT": "$HEIGHT"},
                                 {"TARGET": "b"} ] }"#
            )?,
            None,
            "expressions and absent sizes follow the output"
        );
        assert_eq!(
            pinned(
                r#"{ "WIDTH": 1920, "HEIGHT": 1080,
                     "PASSES": [ {"TARGET": "a", "WIDTH": 1920, "HEIGHT": 1080, "FIXED_SIZE": false} ] }"#
            )?,
            None,
            "FIXED_SIZE false opts the pass, and so the final pass, out"
        );
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
    fn pass_sources_list_one_file_per_pass_final_included() -> Result<()> {
        let def = SceneDef::parse(
            r#"{ "PASSES": [ {"TARGET": "a"} ],
                 "PASS_SOURCES": ["passes/a.glsl", "main.glsl"] }"#,
            "s",
        )?;
        assert_eq!(
            def.pass_sources,
            Some(vec!["passes/a.glsl".to_string(), "main.glsl".to_string()]),
            "one file for the buffer and one for the implicit final pass"
        );
        let plain = SceneDef::parse(r#"{ "PASSES": [ {"TARGET": "a"} ] }"#, "s")?;
        assert_eq!(plain.pass_sources, None, "absent means main.glsl for all");
        Ok(())
    }

    #[test]
    fn pass_sources_with_the_wrong_count_are_rejected() {
        let result = SceneDef::parse(
            r#"{ "PASSES": [ {"TARGET": "a"} ], "PASS_SOURCES": ["only.glsl"] }"#,
            "s",
        );
        let message = result.err().map(|e| e.to_string()).unwrap_or_default();
        assert!(
            message.contains("1 file") && message.contains("2 pass"),
            "the count mismatch should say both numbers, got: {message}"
        );
    }

    #[test]
    fn pass_sources_cannot_leave_the_scene_folder() {
        for bad in ["../x.glsl", "/etc/x.glsl", "a\\\\b.glsl", "C:/x.glsl", ""] {
            let json = format!(r#"{{ "PASS_SOURCES": ["{bad}"] }}"#);
            assert!(
                SceneDef::parse(&json, "s").is_err(),
                "`{bad}` must be rejected"
            );
        }
    }

    #[test]
    fn images_default_to_ssf_sampling_and_honour_overrides() -> Result<()> {
        let def = SceneDef::parse(
            r#"{ "IMAGES": [ {"NAME": "a", "PATH": "a.png"},
                             {"NAME": "b", "PATH": "b.png", "WRAP": "clamp",
                              "FILTER": "nearest", "VFLIP": false} ] }"#,
            "s",
        )?;
        let [a, b] = [&def.images[0], &def.images[1]];
        assert_eq!(
            (a.wrap, a.filter, a.vflip),
            (Wrap::Repeat, Filter::Mipmap, true),
            "SSF's behaviour is the default"
        );
        assert_eq!(
            (b.wrap, b.filter, b.vflip),
            (Wrap::Clamp, Filter::Nearest, false),
            "explicit options win"
        );
        let bad = SceneDef::parse(
            r#"{ "IMAGES": [ {"NAME": "a", "PATH": "a.png", "WRAP": "spiral"} ] }"#,
            "s",
        );
        assert!(bad.is_err(), "an unknown image wrap is an error");
        Ok(())
    }

    #[test]
    fn chosen_media_is_sampled_as_the_media_slot_says() -> Result<()> {
        let sampling = |json: &str| SceneDef::parse(json, "s").map(|def| def.media_sampling);
        assert_eq!(
            sampling("{}")?,
            (Wrap::Repeat, Filter::Linear),
            "SSF's defaults with no MEDIA key"
        );
        assert_eq!(
            sampling(r#"{ "MEDIA": [ { "WRAP": "mirror", "FILTER": "mipmap" } ] }"#)?,
            (Wrap::Mirror, Filter::Mipmap),
            "Synesthesia's slot form"
        );
        assert_eq!(
            sampling(r#"{ "MEDIA": [ {} ] }"#)?,
            (Wrap::Repeat, Filter::Linear),
            "an empty slot"
        );
        assert_eq!(
            sampling(r#"{ "MEDIA": "pics/a.png" }"#)?,
            (Wrap::Clamp, Filter::Linear),
            "as the scene's own picture"
        );
        assert!(
            sampling(r#"{ "MEDIA": [ { "WRAP": "spiral" } ] }"#).is_err(),
            "an unknown wrap in a slot is an error"
        );
        Ok(())
    }

    #[test]
    fn media_is_a_bare_path_or_an_object_and_stays_in_the_folder() -> Result<()> {
        let bare = SceneDef::parse(r#"{ "MEDIA": "pics/a.png" }"#, "s")?;
        let media = bare.media.ok_or_else(|| anyhow::anyhow!("no media"))?;
        assert_eq!(media.path, "pics/a.png", "path");
        assert_eq!(
            (
                media.fps,
                media.looping,
                media.wrap,
                media.filter,
                media.vflip
            ),
            (24.0, true, Wrap::Clamp, Filter::Linear, true),
            "defaults"
        );
        let full = SceneDef::parse(
            r#"{ "MEDIA": { "PATH": "frames", "FPS": 8, "LOOP": false, "FILTER": "nearest" } }"#,
            "s",
        )?;
        let media = full.media.ok_or_else(|| anyhow::anyhow!("no media"))?;
        assert_eq!(
            (media.fps, media.looping, media.filter),
            (8.0, false, Filter::Nearest),
            "options"
        );
        assert!(
            SceneDef::parse("{}", "s")?.media.is_none(),
            "absent means none"
        );
        for bad in [
            r#"{ "MEDIA": "../x.png" }"#,
            r#"{ "MEDIA": { "PATH": "a", "FPS": 0 } }"#,
            r#"{ "MEDIA": { "PATH": "a", "WRAP": "spiral" } }"#,
        ] {
            assert!(SceneDef::parse(bad, "s").is_err(), "{bad} must be rejected");
        }
        Ok(())
    }

    #[test]
    fn the_source_block_and_full_precision_are_read() -> Result<()> {
        let def = SceneDef::parse(
            r#"{ "SOURCE": { "kind": "shadertoy", "id": "XsXXDn", "author": "iq",
                             "license_note": "check it", "extra": 1 },
                 "PASSES": [ {"TARGET": "a", "FULL_PRECISION": true}, {"TARGET": "b"} ] }"#,
            "s",
        )?;
        let source = def.source.unwrap_or_default();
        assert_eq!(source.kind.as_deref(), Some("shadertoy"), "kind");
        assert_eq!(source.id.as_deref(), Some("XsXXDn"), "id");
        assert_eq!(source.license_note.as_deref(), Some("check it"), "note");
        assert!(def.passes[0].full_precision, "explicit full precision");
        assert!(!def.passes[1].full_precision, "off by default");
        Ok(())
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
