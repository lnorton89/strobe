// ============================================================================
// Module:       strobe_shadertoy::convert
// Description:  Layer (c): converts a Shadertoy shader into the files of a
//               Strobe scene folder, deterministically and without a GPU.
//
// Dependencies: crate::model, crate::dialect, crate::glsl, strobe-scene
// ============================================================================

//! The converter.
//!
//! [`convert`] takes a parsed [`Shader`] and [`Options`] and returns
//! [`Converted`]: the exact bytes of every file in the scene folder, a
//! list of things that were converted with a loss ([`Warning`]), and the
//! assets still to be fetched. It reads no clock, no network and no
//! environment, so the same input gives the same bytes; [`write`] puts the
//! result on disk atomically.
//!
//! ## The scene it builds
//!
//! ```text
//! <name>-<id>.synScene/
//!   scene.json              PASSES, PASS_SOURCES, IMAGES, CONTROLS, SOURCE
//!   main.glsl               the Image pass (also the last PASS_SOURCES entry)
//!   passes/st_clock.glsl    the time pass, only when a pass reads time
//!   passes/buffer_a.glsl    ... one file per Shadertoy buffer, A to D
//!   images/                 textures, a 1x1 black PNG, pending-assets.json
//!   shadertoy.json          the original export, verbatim, when supplied
//! ```
//!
//! Each pass file is complete and independent: shim, then `Common`, then
//! that pass's own code, then the entry point (see [`shim`]). That is what
//! lets two buffers define the same helper function (D90).
//!
//! ## Pass order and the read rule
//!
//! Shadertoy runs Buffer A, B, C, D and then Image. A pass reading a
//! buffer that has not run yet this frame sees that buffer's previous
//! frame. Strobe's player does exactly that for pass `j >= i`, so the
//! converter only has to list the passes in Shadertoy's order and mark
//! every buffer `DOUBLE_BUFFER` so a buffer can read itself. The tests
//! confirm the rule numerically rather than assuming it.
//!
//! ## What is rejected
//!
//! `sound` and `cubemap` passes, and `cubemap` and `volume` inputs,
//! cannot be expressed (no audio-synthesis pass, no `samplerCube` or 3D
//! binding behind `iChannelN`), so they are errors naming the pass and
//! channel. `video`, `webcam` and `keyboard` inputs are bound to black
//! with a [`Warning`], because the shader still runs sensibly without
//! them.

mod names;
mod plan;
mod scene_json;
mod shim;
mod write;

use crate::dialect::Note;
use crate::model::Shader;
use std::collections::BTreeMap;
use std::fmt;

pub use write::{WriteError, write};

/// A 1x1 black PNG, bound to channels with nothing to show.
const BLACK_PNG: [u8; 70] = [
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0x60, 0x60, 0x60, 0xf8,
    0x0f, 0x00, 0x01, 0x04, 0x01, 0x00, 0x80, 0xbb, 0xd1, 0x5b, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

/// Where Shadertoy's media paths (`/media/a/<hash>.jpg`) are served from.
const MEDIA_HOST: &str = "https://www.shadertoy.com";

/// What to convert and how.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// The scene's title; the shader's own name when absent.
    pub name: Option<String>,
    /// Asset files already fetched, by file name (`<hash>.jpg`). A texture
    /// input whose file is here is copied into `images/`; one whose file
    /// is not here is bound to black and listed as pending.
    pub assets: BTreeMap<String, Vec<u8>>,
    /// Treat a missing asset as an error instead of a warning.
    pub strict_assets: bool,
    /// The JSON the shader was parsed from, kept verbatim as
    /// `shadertoy.json` so the import can be redone offline.
    pub original_json: Option<String>,
}

/// The result of a conversion: everything needed to write the folder.
#[derive(Debug, Clone, PartialEq)]
pub struct Converted {
    /// The scene folder's name, ending in `.synScene`.
    pub folder_name: String,
    /// The scene's title.
    pub title: String,
    /// Every file, by path relative to the folder, in sorted order.
    pub files: BTreeMap<String, Vec<u8>>,
    /// Things converted with a loss.
    pub warnings: Vec<Warning>,
    /// Texture assets the scene wants that were not supplied.
    pub pending_assets: Vec<PendingAsset>,
    /// Dialect rewrites applied to a pass's code, by pass name.
    pub rewrites: Vec<(String, Note)>,
}

/// A texture the scene wants but the caller did not have.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAsset {
    /// The pass that reads it.
    pub pass: String,
    /// The channel it is bound to.
    pub channel: u32,
    /// The file name it will have under `images/`.
    pub file: String,
    /// Where to download it.
    pub url: String,
}

/// Something that was converted with a loss.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// A texture asset was not supplied; the channel reads black.
    MissingAsset(PendingAsset),
    /// An input kind with no equivalent yet; the channel reads black.
    InputBoundToBlack {
        /// The pass.
        pass: String,
        /// The channel.
        channel: u32,
        /// The input kind as Shadertoy spells it.
        kind: String,
        /// Why, and when it will be supported.
        reason: &'static str,
    },
    /// The code reads a channel nothing is connected to.
    UnboundChannelRead {
        /// The pass.
        pass: String,
        /// The channel.
        channel: u32,
    },
    /// Two passes read one buffer with different sampler settings; the
    /// first reader's settings are used for everyone.
    SamplerConflict {
        /// The buffer's name.
        buffer: String,
        /// The pass whose settings were not used.
        pass: String,
        /// Its channel.
        channel: u32,
    },
    /// A sampler setting with a value Shadertoy does not define; the
    /// default is used.
    UnknownSamplerValue {
        /// The pass.
        pass: String,
        /// The channel.
        channel: u32,
        /// The setting.
        field: &'static str,
        /// The value as written.
        value: String,
    },
    /// `srgb: true` on a texture: the texture is sampled as plain data,
    /// not decoded from sRGB, so it will look brighter than on Shadertoy.
    SrgbNotDecoded {
        /// The pass.
        pass: String,
        /// The channel.
        channel: u32,
    },
}

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingAsset(asset) => write!(
                f,
                "{} channel {}: texture {} was not available, so the channel reads black \
                 (download {})",
                asset.pass, asset.channel, asset.file, asset.url
            ),
            Self::InputBoundToBlack {
                pass,
                channel,
                kind,
                reason,
            } => write!(
                f,
                "{pass} channel {channel}: `{kind}` input reads black ({reason})"
            ),
            Self::UnboundChannelRead { pass, channel } => write!(
                f,
                "{pass}: the code reads iChannel{channel} but nothing is connected to it; \
                 it reads black"
            ),
            Self::SamplerConflict {
                buffer,
                pass,
                channel,
            } => write!(
                f,
                "{pass} channel {channel}: reads {buffer} with different filter/wrap \
                 settings than another pass; the first reader's settings apply to both"
            ),
            Self::UnknownSamplerValue {
                pass,
                channel,
                field,
                value,
            } => write!(
                f,
                "{pass} channel {channel}: unknown sampler {field} `{value}`, using the default"
            ),
            Self::SrgbNotDecoded { pass, channel } => write!(
                f,
                "{pass} channel {channel}: the texture is marked sRGB but is not decoded \
                 from sRGB when sampled, so it will look brighter than on Shadertoy"
            ),
        }
    }
}

/// Why a shader could not be converted.
#[derive(Debug, thiserror::Error)]
pub enum ConvertError {
    /// There is no `image` pass.
    #[error("the shader has no Image pass")]
    NoImagePass,
    /// There is more than one `image` pass.
    #[error("the shader has {0} Image passes; there must be exactly one")]
    SeveralImagePasses(usize),
    /// Something that cannot be converted.
    #[error("{pass}: {what} is not supported: {reason}")]
    Unsupported {
        /// The pass.
        pass: String,
        /// What was found, e.g. `cubemap input on channel 1`.
        what: String,
        /// Why not.
        reason: &'static str,
    },
    /// A pass has no `mainImage`.
    #[error("{0}: no `mainImage` function found")]
    NoMainImage(String),
    /// More than four buffers, or two with the same letter.
    #[error("buffers: {0}")]
    Buffers(String),
    /// An input's channel is not 0 to 3.
    #[error("{pass}: input on channel {channel}, but Shadertoy has channels 0 to 3")]
    BadChannel {
        /// The pass.
        pass: String,
        /// The channel as written.
        channel: u32,
    },
    /// Two inputs on one channel.
    #[error("{pass}: two inputs on channel {channel}")]
    DuplicateChannel {
        /// The pass.
        pass: String,
        /// The channel.
        channel: u32,
    },
    /// A buffer input that names no buffer of this shader.
    #[error("{pass} channel {channel}: buffer input `{id}` matches no buffer pass")]
    UnresolvedBuffer {
        /// The pass.
        pass: String,
        /// The channel.
        channel: u32,
        /// The input's id (or `src`) as written.
        id: String,
    },
    /// A texture input with no asset path.
    #[error("{pass} channel {channel}: a texture input with no `src` or `filepath`")]
    MissingSrc {
        /// The pass.
        pass: String,
        /// The channel.
        channel: u32,
    },
    /// A texture asset was needed, none was supplied, and the options
    /// asked for that to be an error.
    #[error("{} channel {}: texture {} is missing (download {})", .0.pass, .0.channel, .0.file, .0.url)]
    MissingAsset(PendingAsset),
    /// The shader's own code uses a name the shim generates.
    #[error(
        "{pass}: the code uses the identifier `{name}`, which the importer reserves; \
         rename it in the shader and import again"
    )]
    NameClash {
        /// The pass.
        pass: String,
        /// The identifier.
        name: String,
    },
    /// The generated `scene.json` did not validate. This is a converter
    /// bug (or a title/author the scene format rejects), never the shader.
    #[error("the generated scene did not validate: {0}")]
    InvalidScene(#[from] strobe_scene::SceneError),
}

/// Converts `shader`. See the module docs for what comes out.
pub fn convert(shader: &Shader, options: &Options) -> Result<Converted, ConvertError> {
    let plan = plan::build(shader, options)?;
    let title = options
        .name
        .clone()
        .filter(|n| !n.trim().is_empty())
        .or_else(|| shader.info.name.clone().filter(|n| !n.trim().is_empty()))
        .unwrap_or_else(|| "Shadertoy import".to_string());

    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut rewrites = Vec::new();
    let mut layouts = Vec::new();
    for planned in &plan.passes {
        let generated = shim::pass_file(&plan, planned, shader);
        layouts.push(generated.layout);
        files.insert(planned.file.clone(), generated.text.into_bytes());
        for note in &planned.notes {
            rewrites.push((planned.name.clone(), note.clone()));
        }
    }
    if plan.clock {
        files.insert(
            names::CLOCK_FILE.to_string(),
            shim::clock_file().into_bytes(),
        );
    }
    for (file, bytes) in &plan.assets {
        files.insert(format!("images/{file}"), bytes.clone());
    }
    if plan.black_used {
        files.insert(names::BLACK_FILE.to_string(), BLACK_PNG.to_vec());
    }
    if !plan.pending.is_empty() {
        files.insert(
            "images/pending-assets.json".to_string(),
            scene_json::pending_assets(&plan.pending).into_bytes(),
        );
    }
    if let Some(original) = &options.original_json {
        files.insert("shadertoy.json".to_string(), original.clone().into_bytes());
    }

    let json = scene_json::scene(&plan, shader, &title, &layouts);
    // Validate what was generated against the loader's own rules.
    strobe_scene::SceneDef::parse(&json, &title)?;
    files.insert("scene.json".to_string(), json.into_bytes());

    Ok(Converted {
        folder_name: names::folder_name(&title, shader.info.id.as_deref()),
        title,
        files,
        warnings: plan.warnings,
        pending_assets: plan.pending,
        rewrites,
    })
}
