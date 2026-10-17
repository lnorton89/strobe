// ============================================================================
// Module:       strobe_shadertoy::convert::plan
// Description:  Reads the shader model into a plan: pass order, what each
//               channel is bound to, which images and assets are needed.
//
// Dependencies: crate::model, crate::dialect, crate::glsl, super::names
// ============================================================================

//! Planning.
//!
//! Everything that needs a decision happens here, so that the shim and the
//! JSON writer only format the plan. In order: classify the passes and
//! reject the ones that cannot be converted; put the buffers in
//! Shadertoy's order (A to D); normalise each pass's code; bind every
//! input channel to a pass target, an image, the audio texture, or black;
//! and decide whether the clock pass is needed (only when some pass reads
//! time).

use super::names;
use super::{ConvertError, MEDIA_HOST, Options, PendingAsset, Warning};
use crate::dialect::{self, Normalised, Note};
use crate::glsl;
use crate::model::{Input, InputKind, PassKind, RenderPass, Sampler, Shader};
use std::collections::{BTreeMap, BTreeSet};
use strobe_scene::scene::{Filter, Wrap};

/// Identifiers whose use means the pass needs the clock.
pub(super) const CLOCK_NAMES: &[&str] = &[
    "iTime",
    "iTimeDelta",
    "iFrame",
    "iFrameRate",
    "iChannelTime",
];

/// What a pass is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Role {
    /// Shadertoy buffer `n` (0 is A).
    Buffer(usize),
    /// The Image pass.
    Image,
}

/// What an input channel reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Binding {
    /// The target of `Plan::passes[n]`.
    Buffer(usize),
    /// `Plan::images[n]`.
    Image(usize),
    /// The Shadertoy audio texture.
    Audio,
    /// The black image.
    Black,
}

/// A texture file bound as a scene image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct PlannedImage {
    /// Sampler name.
    pub name: String,
    /// File name under `images/`.
    pub file: String,
    pub wrap: Wrap,
    pub filter: Filter,
    pub vflip: bool,
}

/// One pass to emit.
#[derive(Debug, Clone)]
pub(super) struct PlannedPass {
    pub role: Role,
    /// The tab's name for messages: `Buffer A`, `Image`.
    pub name: String,
    /// The file in the scene folder.
    pub file: String,
    /// The pass target (buffers only).
    pub target: Option<String>,
    /// The Common tab, normalised, when there is one.
    pub common: Option<Normalised>,
    /// The pass's own code, normalised.
    pub code: Normalised,
    /// Dialect rewrites applied to either.
    pub notes: Vec<Note>,
    /// Identifiers in common plus code.
    pub idents: BTreeSet<String>,
    /// Macros defined in common plus code.
    pub macros: BTreeSet<String>,
    /// Built-in functions the code defines itself.
    pub shadowed: BTreeSet<String>,
    pub channels: [Option<Binding>; 4],
    /// Whether the code writes `gl_FragColor`.
    pub legacy_frag_color: bool,
}

/// Filter and wrap chosen for a buffer's target.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct TargetSampling {
    pub filter: Filter,
    pub wrap: Wrap,
}

/// The analysed shader.
#[derive(Debug)]
pub(super) struct Plan {
    /// Whether the clock pass exists.
    pub clock: bool,
    /// Buffers in order, then the Image pass.
    pub passes: Vec<PlannedPass>,
    /// Sampling for each buffer's target, parallel to the buffers in `passes`.
    pub sampling: Vec<TargetSampling>,
    pub images: Vec<PlannedImage>,
    /// Asset files to copy into `images/`.
    pub assets: BTreeMap<String, Vec<u8>>,
    pub black_used: bool,
    pub warnings: Vec<Warning>,
    pub pending: Vec<PendingAsset>,
    /// The file name under `images/` each texture URL was given, so a URL
    /// read by several channels keeps one file and two URLs never share
    /// one ([`asset_file`]).
    pub asset_files: BTreeMap<String, String>,
}

impl Plan {
    /// The sampler name a binding is read through.
    pub(super) fn sampler_name(&self, binding: Binding) -> String {
        match binding {
            Binding::Buffer(n) => self
                .passes
                .get(n)
                .and_then(|p| p.target.clone())
                .unwrap_or_else(|| names::BLACK_IMAGE.to_string()),
            Binding::Image(n) => self
                .images
                .get(n)
                .map_or_else(|| names::BLACK_IMAGE.to_string(), |i| i.name.clone()),
            Binding::Audio => names::AUDIO_SAMPLER.to_string(),
            Binding::Black => names::BLACK_IMAGE.to_string(),
        }
    }
}

/// A pass's display name.
fn display_name(pass: &RenderPass, fallback: &str) -> String {
    if pass.name.trim().is_empty() {
        fallback.to_string()
    } else {
        pass.name.trim().to_string()
    }
}

/// Orders buffers A to D. Uses the letters in their names (`Buffer A`)
/// or their output ids (257 to 260) when every buffer has a distinct one,
/// otherwise the order they were listed in.
fn buffer_order(buffers: &[&RenderPass]) -> Result<Vec<usize>, ConvertError> {
    if buffers.len() > 4 {
        return Err(ConvertError::Buffers(format!(
            "{} buffer passes, but Shadertoy has only A to D",
            buffers.len()
        )));
    }
    let from_name = |p: &RenderPass| {
        p.name
            .trim()
            .chars()
            .last()
            .filter(|c| ('A'..='D').contains(c))
            .map(|c| usize::from(c as u8 - b'A'))
    };
    let from_id = |p: &RenderPass| {
        p.outputs
            .first()
            .and_then(|o| o.id.as_deref())
            .and_then(|id| id.parse::<usize>().ok())
            .filter(|n| (257..=260).contains(n))
            .map(|n| n - 257)
    };
    for source in [
        &from_name as &dyn Fn(&RenderPass) -> Option<usize>,
        &from_id,
    ] {
        let letters: Vec<Option<usize>> = buffers.iter().map(|p| source(p)).collect();
        if letters.iter().all(Option::is_some) {
            let unique: BTreeSet<usize> = letters.iter().flatten().copied().collect();
            if unique.len() == letters.len() {
                return Ok(letters.into_iter().flatten().collect());
            }
        }
    }
    Ok((0..buffers.len()).collect())
}

/// Reads `buffer00.png`-style preview names as a buffer letter.
fn letter_from_src(src: &str) -> Option<usize> {
    let after = src.rsplit('/').next()?.strip_prefix("buffer0")?;
    let digit = after.chars().next()?.to_digit(10)?;
    (digit < 4).then_some(digit as usize)
}

fn parse_filter(
    text: Option<&str>,
    default: Filter,
    report: &mut impl FnMut(&'static str, &str),
) -> Filter {
    match text.map(str::to_ascii_lowercase).as_deref() {
        None | Some("") => default,
        Some("nearest") => Filter::Nearest,
        Some("linear") => Filter::Linear,
        Some("mipmap") => Filter::Mipmap,
        Some(other) => {
            report("filter", other);
            default
        }
    }
}

fn parse_wrap(
    text: Option<&str>,
    default: Wrap,
    report: &mut impl FnMut(&'static str, &str),
) -> Wrap {
    match text.map(str::to_ascii_lowercase).as_deref() {
        None | Some("") => default,
        Some("clamp") => Wrap::Clamp,
        Some("repeat") => Wrap::Repeat,
        Some("mirror") => Wrap::Mirror,
        Some(other) => {
            report("wrap", other);
            default
        }
    }
}

/// Analyses `shader`.
pub(super) fn build(shader: &Shader, options: &Options) -> Result<Plan, ConvertError> {
    let mut commons = Vec::new();
    let mut buffers = Vec::new();
    let mut images = Vec::new();
    for pass in &shader.renderpass {
        match &pass.kind {
            PassKind::Image => images.push(pass),
            PassKind::Buffer => buffers.push(pass),
            PassKind::Common => commons.push(pass),
            PassKind::Sound => {
                return Err(ConvertError::Unsupported {
                    pass: display_name(pass, "Sound"),
                    what: "a Sound pass".to_string(),
                    reason: "audio-synthesis shaders (mainSound) have no equivalent in Strobe",
                });
            }
            PassKind::Cubemap => {
                return Err(ConvertError::Unsupported {
                    pass: display_name(pass, "Cubemap"),
                    what: "a Cubemap pass".to_string(),
                    reason: "cube-map buffers (mainCubemap) are not supported yet",
                });
            }
            PassKind::Other(kind) => {
                return Err(ConvertError::Unsupported {
                    pass: display_name(pass, kind),
                    what: format!("a pass of type `{kind}`"),
                    reason: "this importer does not know that pass type",
                });
            }
        }
    }
    let image = match images.as_slice() {
        [] => return Err(ConvertError::NoImagePass),
        [only] => *only,
        several => return Err(ConvertError::SeveralImagePasses(several.len())),
    };

    let common = if commons.is_empty() {
        None
    } else {
        let joined: Vec<&str> = commons.iter().map(|p| p.code.as_str()).collect();
        Some(dialect::normalise(&joined.join("\n")))
    };

    // Buffers in A..D order. `ordered[k]` is (letter, source pass).
    let letters = buffer_order(&buffers)?;
    let mut ordered: Vec<(usize, &RenderPass)> = letters
        .iter()
        .copied()
        .zip(buffers.iter().copied())
        .collect();
    ordered.sort_by_key(|(letter, _)| *letter);
    let mut seen = BTreeSet::new();
    for (letter, _) in &ordered {
        if !seen.insert(*letter) {
            return Err(ConvertError::Buffers(format!(
                "two passes are both Buffer {}",
                names::letter_name(*letter)
            )));
        }
    }
    let mut sources: Vec<(Role, &RenderPass)> = ordered
        .iter()
        .map(|(letter, p)| (Role::Buffer(*letter), *p))
        .collect();
    sources.push((Role::Image, image));

    // How other passes refer to each buffer.
    let mut by_id: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_letter: BTreeMap<usize, usize> = BTreeMap::new();
    for (position, (role, pass)) in sources.iter().enumerate() {
        if let Role::Buffer(letter) = role {
            by_letter.insert(*letter, position);
            for output in &pass.outputs {
                if let Some(id) = &output.id {
                    by_id.insert(id.clone(), position);
                }
            }
        }
    }

    let mut plan = Plan {
        clock: false,
        passes: Vec::new(),
        sampling: Vec::new(),
        images: Vec::new(),
        assets: BTreeMap::new(),
        black_used: false,
        asset_files: BTreeMap::new(),
        warnings: Vec::new(),
        pending: Vec::new(),
    };
    let mut target_sampling: BTreeMap<usize, TargetSampling> = BTreeMap::new();
    let reserved = names::reserved();

    let display_names: Vec<String> = sources
        .iter()
        .map(|(role, pass)| match role {
            Role::Buffer(letter) => {
                display_name(pass, &format!("Buffer {}", names::letter_name(*letter)))
            }
            Role::Image => display_name(pass, "Image"),
        })
        .collect();

    for (role, pass) in &sources {
        let name = display_names
            .get(plan.passes.len())
            .cloned()
            .unwrap_or_default();
        let mut code = dialect::normalise(&pass.code);
        let mut common = common.clone();
        let mut notes: Vec<Note> = Vec::new();
        if let Some(c) = &common {
            notes.extend(c.notes.iter().cloned());
        }
        notes.extend(code.notes.iter().cloned());

        let mut idents: BTreeSet<String> = code.identifiers.clone();
        let mut macros: BTreeSet<String> = code.macros.clone();
        let mut shadowed: BTreeSet<String> = code.shadowed_builtins.clone();
        if let Some(c) = &common {
            idents.extend(c.identifiers.iter().cloned());
            macros.extend(c.macros.iter().cloned());
            shadowed.extend(c.shadowed_builtins.iter().cloned());
        }
        if !code.identifiers.contains("mainImage") {
            return Err(ConvertError::NoMainImage(name));
        }
        if let Some(clash) = idents
            .iter()
            .find(|i| i.starts_with("strobe_") || reserved.contains(i))
        {
            return Err(ConvertError::NameClash {
                pass: name,
                name: clash.clone(),
            });
        }

        let legacy_frag_color = idents.contains("gl_FragColor");
        if legacy_frag_color {
            code.code = glsl::rename_ident(&code.code, "gl_FragColor", "strobe_gl_FragColor");
            if let Some(c) = common.as_mut() {
                c.code = glsl::rename_ident(&c.code, "gl_FragColor", "strobe_gl_FragColor");
            }
        }

        // Channels.
        let mut channels: [Option<Binding>; 4] = [None; 4];
        for input in &pass.inputs {
            let slot = usize::try_from(input.channel).ok().filter(|c| *c < 4);
            let Some(slot) = slot else {
                return Err(ConvertError::BadChannel {
                    pass: name,
                    channel: input.channel,
                });
            };
            if channels[slot].is_some() {
                return Err(ConvertError::DuplicateChannel {
                    pass: name,
                    channel: input.channel,
                });
            }
            let binding = bind(
                &mut plan,
                &mut target_sampling,
                options,
                &BindContext {
                    pass: &name,
                    by_id: &by_id,
                    by_letter: &by_letter,
                    names: &display_names,
                },
                input,
            )?;
            channels[slot] = Some(binding);
        }
        for (slot, bound) in channels.iter_mut().enumerate() {
            if bound.is_none() && idents.contains(&format!("iChannel{slot}")) {
                plan.warnings.push(Warning::UnboundChannelRead {
                    pass: name.clone(),
                    channel: slot as u32,
                });
                plan.black_used = true;
                *bound = Some(Binding::Black);
            }
        }

        let (target, file) = match role {
            Role::Buffer(letter) => (
                Some(names::buffer_target(*letter)),
                names::buffer_file(*letter),
            ),
            Role::Image => (None, names::IMAGE_FILE.to_string()),
        };
        plan.passes.push(PlannedPass {
            role: *role,
            name,
            file,
            target,
            common,
            code,
            notes,
            idents,
            macros,
            shadowed,
            channels,
            legacy_frag_color,
        });
    }

    plan.clock = plan
        .passes
        .iter()
        .any(|p| CLOCK_NAMES.iter().any(|n| p.idents.contains(*n)));
    plan.black_used |= plan
        .passes
        .iter()
        .any(|p| p.channels.iter().flatten().any(|b| *b == Binding::Black));
    // Sampling for every buffer, in pass order.
    plan.sampling = plan
        .passes
        .iter()
        .enumerate()
        .filter(|(_, p)| matches!(p.role, Role::Buffer(_)))
        .map(|(position, _)| {
            target_sampling
                .get(&position)
                .copied()
                .unwrap_or(TargetSampling {
                    filter: Filter::Linear,
                    wrap: Wrap::Clamp,
                })
        })
        .collect();
    Ok(plan)
}

/// What `bind` needs to know about the pass it is binding for.
struct BindContext<'a> {
    pass: &'a str,
    by_id: &'a BTreeMap<String, usize>,
    by_letter: &'a BTreeMap<usize, usize>,
    /// Display names of the passes, by position in the plan.
    names: &'a [String],
}

/// Binds one input.
fn bind(
    plan: &mut Plan,
    target_sampling: &mut BTreeMap<usize, TargetSampling>,
    options: &Options,
    cx: &BindContext<'_>,
    input: &Input,
) -> Result<Binding, ConvertError> {
    let pass = cx.pass.to_string();
    let channel = input.channel;
    let black = |plan: &mut Plan, reason: &'static str| {
        plan.warnings.push(Warning::InputBoundToBlack {
            pass: pass.clone(),
            channel,
            kind: input.kind.name().to_string(),
            reason,
        });
        plan.black_used = true;
        Binding::Black
    };
    let unsupported = |what: &str, reason: &'static str| ConvertError::Unsupported {
        pass: pass.clone(),
        what: format!("{what} on channel {channel}"),
        reason,
    };
    match &input.kind {
        InputKind::Music | InputKind::MusicStream | InputKind::Mic => Ok(Binding::Audio),
        InputKind::Keyboard => Ok(black(plan, "keyboard input arrives in a later phase")),
        InputKind::Video => Ok(black(plan, "video input arrives in a later phase")),
        InputKind::Webcam => Ok(black(plan, "webcam input arrives in a later phase")),
        InputKind::Cubemap => Err(unsupported(
            "a cubemap input",
            "cube-map inputs need a samplerCube behind iChannelN, which is not built yet",
        )),
        InputKind::Volume => Err(unsupported(
            "a volume input",
            "3D textures are not supported",
        )),
        InputKind::Other(kind) => Err(unsupported(
            &format!("an input of type `{kind}`"),
            "this importer does not know that input type",
        )),
        InputKind::Buffer => {
            let position = input
                .id
                .as_deref()
                .and_then(|id| cx.by_id.get(id).copied())
                .or_else(|| {
                    input
                        .src
                        .as_deref()
                        .and_then(letter_from_src)
                        .and_then(|l| cx.by_letter.get(&l).copied())
                })
                .ok_or_else(|| ConvertError::UnresolvedBuffer {
                    pass: pass.clone(),
                    channel,
                    id: input
                        .id
                        .clone()
                        .or_else(|| input.src.clone())
                        .unwrap_or_default(),
                })?;
            let wanted = {
                let mut warn = |field: &'static str, value: &str| {
                    plan.warnings.push(Warning::UnknownSamplerValue {
                        pass: pass.clone(),
                        channel,
                        field,
                        value: value.to_string(),
                    });
                };
                TargetSampling {
                    filter: parse_filter(
                        input.sampler.filter.as_deref(),
                        Filter::Linear,
                        &mut warn,
                    ),
                    wrap: parse_wrap(input.sampler.wrap.as_deref(), Wrap::Clamp, &mut warn),
                }
            };
            match target_sampling.get(&position) {
                None => {
                    target_sampling.insert(position, wanted);
                }
                Some(first) if *first != wanted => {
                    plan.warnings.push(Warning::SamplerConflict {
                        buffer: cx.names.get(position).cloned().unwrap_or_default(),
                        pass: pass.clone(),
                        channel,
                    });
                }
                Some(_) => {}
            }
            Ok(Binding::Buffer(position))
        }
        InputKind::Texture => {
            let src = input
                .src
                .as_deref()
                .filter(|s| !s.trim().is_empty())
                .ok_or(ConvertError::MissingSrc {
                    pass: pass.clone(),
                    channel,
                })?;
            let url = if src.starts_with('/') {
                format!("{MEDIA_HOST}{src}")
            } else {
                src.to_string()
            };
            let file = asset_file(plan, &url, src);
            let Some(bytes) = options.assets.get(&file) else {
                let asset = PendingAsset {
                    pass: pass.clone(),
                    channel,
                    file: file.clone(),
                    url,
                };
                if options.strict_assets {
                    return Err(ConvertError::MissingAsset(asset));
                }
                if !plan.pending.iter().any(|p| p.file == file) {
                    plan.pending.push(asset.clone());
                }
                plan.warnings.push(Warning::MissingAsset(asset));
                plan.black_used = true;
                return Ok(Binding::Black);
            };
            let sampling = sampler_for_texture(plan, &input.sampler, &pass, channel);
            let (filter, wrap, vflip) = sampling;
            let existing = plan.images.iter().position(|i| {
                i.file == file && i.filter == filter && i.wrap == wrap && i.vflip == vflip
            });
            let index = existing.unwrap_or_else(|| {
                plan.images.push(PlannedImage {
                    name: names::image_name(plan.images.len()),
                    file: file.clone(),
                    wrap,
                    filter,
                    vflip,
                });
                plan.images.len() - 1
            });
            plan.assets.insert(file, bytes.clone());
            Ok(Binding::Image(index))
        }
    }
}

/// The file name under `images/` for the texture at `url` (`src` as the
/// JSON wrote it): the URL's own base name, made safe
/// ([`names::asset_file_name`]), so a Shadertoy media file keeps the name
/// its pending list and a person's asset folder use (D417). It can never be
/// a file the converter writes there itself (a generated name gets a
/// `tex-` prefix, [`names::is_generated_image`]), nor, ignoring case as
/// Windows does, the file of another URL (a `-2`, `-3`... before the
/// extension). The same URL always gets the same file. Names are given in
/// the order the passes and channels are read, so a second conversion of
/// the same shader gives the same ones.
fn asset_file(plan: &mut Plan, url: &str, src: &str) -> String {
    if let Some(file) = plan.asset_files.get(url) {
        return file.clone();
    }
    let mut base = names::asset_file_name(src);
    if names::is_generated_image(&base) {
        base = format!("tex-{base}");
    }
    let taken = |plan: &Plan, name: &str| {
        plan.asset_files
            .values()
            .any(|file| file.eq_ignore_ascii_case(name))
    };
    let mut file = base.clone();
    let mut number = 2_usize;
    while taken(plan, &file) {
        file = names::numbered_file(&base, number);
        number += 1;
    }
    plan.asset_files.insert(url.to_string(), file.clone());
    file
}

/// Filter, wrap and vflip for a texture, with Shadertoy's defaults
/// (mipmap, repeat, flipped), warning about values it does not define.
fn sampler_for_texture(
    plan: &mut Plan,
    sampler: &Sampler,
    pass: &str,
    channel: u32,
) -> (Filter, Wrap, bool) {
    let mut warnings = Vec::new();
    let mut warn = |field: &'static str, value: &str| {
        warnings.push(Warning::UnknownSamplerValue {
            pass: pass.to_string(),
            channel,
            field,
            value: value.to_string(),
        });
    };
    let filter = parse_filter(sampler.filter.as_deref(), Filter::Mipmap, &mut warn);
    let wrap = parse_wrap(sampler.wrap.as_deref(), Wrap::Repeat, &mut warn);
    plan.warnings.extend(warnings);
    if sampler.srgb == Some(true) {
        plan.warnings.push(Warning::SrgbNotDecoded {
            pass: pass.to_string(),
            channel,
        });
    }
    (filter, wrap, sampler.vflip.unwrap_or(true))
}
