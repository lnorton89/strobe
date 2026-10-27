// ============================================================================
// Module:       strobe_shadertoy::convert::scene_json
// Description:  Serialises the plan to scene.json with a fixed key order so
//               the bytes are the same every time.
//
// Dependencies: serde, serde_json, super::plan, super::names
// ============================================================================

//! `scene.json` and the pending-assets list.
//!
//! Built from `#[derive(Serialize)]` structs rather than a
//! `serde_json::Value`: a `Value`'s key order depends on a cargo feature
//! (`preserve_order`) that any crate in the build can switch on, and the
//! converter's output must not change because of what else was compiled
//! alongside it.

use super::PendingAsset;
use super::names;
use super::plan::{Plan, Role};
use super::shim::Layout;
use crate::model::Shader;
use serde::Serialize;
use strobe_scene::scene::{Filter, Wrap};

/// What is known about the licence of a Shadertoy shader. Written into
/// every import; the importer cannot know more than this.
const LICENSE_NOTE: &str = "Shadertoy shaders are published under CC BY-NC-SA 3.0 unless the \
author says otherwise (check the shader page and the comments at the top of the code). That \
licence forbids commercial use without the author's permission; get it before showing this \
in a paid setting.";

#[derive(Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
struct Scene<'a> {
    title: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    credit: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<&'a str>,
    passes: Vec<Pass>,
    pass_sources: Vec<String>,
    images: Vec<Image>,
    controls: Vec<Control>,
    source: Source<'a>,
}

#[derive(Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
struct Pass {
    target: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    height: Option<u32>,
    float: bool,
    double_buffer: bool,
    wrap: &'static str,
    filter: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    full_precision: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    keep_on_resize: Option<bool>,
}

#[derive(Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
struct Image {
    name: String,
    path: String,
    wrap: &'static str,
    filter: &'static str,
    vflip: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
struct Control {
    name: &'static str,
    #[serde(rename = "TYPE")]
    ty: &'static str,
    min: f32,
    max: f32,
    default: f32,
    ui_group: &'static str,
    description: &'static str,
}

#[derive(Serialize)]
struct Source<'a> {
    kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    author: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    license_note: &'static str,
    tags: &'a [String],
    passes: Vec<SourcePass<'a>>,
}

/// Where one pass's pieces start in its file, for mapping errors back to
/// the Shadertoy tabs.
#[derive(Serialize)]
struct SourcePass<'a> {
    name: &'a str,
    file: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    common_line: Option<u32>,
    code_line: u32,
}

fn wrap_name(wrap: Wrap) -> &'static str {
    match wrap {
        Wrap::Repeat => "repeat",
        Wrap::Clamp => "clamp",
        Wrap::Mirror => "mirror",
    }
}

fn filter_name(filter: Filter) -> &'static str {
    match filter {
        Filter::Linear => "linear",
        Filter::Nearest => "nearest",
        Filter::Mipmap => "mipmap",
    }
}

/// The five controls every import gets.
fn controls() -> Vec<Control> {
    let control = |name, min, max, default, description| Control {
        name,
        ty: "slider",
        min,
        max,
        default,
        ui_group: names::CONTROL_GROUP,
        description,
    };
    vec![
        control(
            names::SPEED,
            0.0,
            3.0,
            1.0,
            "How fast iTime runs. 1 is real time.",
        ),
        control(
            names::AUDIO_TIME,
            0.0,
            1.0,
            0.0,
            "Blend iTime's pace toward the bass: slow when quiet, surging on loud bass.",
        ),
        control(
            names::BASS_FLASH,
            0.0,
            1.0,
            0.0,
            "Brightness kick on bass hits.",
        ),
        control(
            names::ZOOM,
            0.5,
            2.0,
            1.0,
            "Zoom of the picture around its centre.",
        ),
        control(
            names::HUE,
            -180.0,
            180.0,
            0.0,
            "Hue rotation of the picture, degrees.",
        ),
    ]
}

/// The `scene.json` text for a plan.
pub(super) fn scene(plan: &Plan, shader: &Shader, title: &str, layouts: &[Layout]) -> String {
    let mut passes = Vec::new();
    let mut pass_sources = Vec::new();
    if plan.clock {
        // The clock runs on through a resize; the frame pass restarts with
        // the buffers that read it, so `iFrame` does too (D495).
        for (target, keep) in [(names::CLOCK_TARGET, true), (names::FRAME_TARGET, false)] {
            passes.push(Pass {
                target: target.to_string(),
                width: Some(1),
                height: Some(1),
                float: true,
                double_buffer: true,
                wrap: "clamp",
                filter: "nearest",
                full_precision: Some(true),
                keep_on_resize: keep.then_some(true),
            });
            pass_sources.push(names::CLOCK_FILE.to_string());
        }
    }
    let mut buffer = 0;
    for planned in &plan.passes {
        pass_sources.push(planned.file.clone());
        if let (Role::Buffer(_), Some(target)) = (planned.role, &planned.target) {
            let sampling = plan.sampling.get(buffer).copied();
            buffer += 1;
            passes.push(Pass {
                target: target.clone(),
                width: None,
                height: None,
                float: true,
                double_buffer: true,
                wrap: sampling.map_or("clamp", |s| wrap_name(s.wrap)),
                filter: sampling.map_or("linear", |s| filter_name(s.filter)),
                full_precision: None,
                keep_on_resize: None,
            });
        }
    }

    let mut images: Vec<Image> = plan
        .images
        .iter()
        .map(|i| Image {
            name: i.name.clone(),
            path: format!("images/{}", i.file),
            wrap: wrap_name(i.wrap),
            filter: filter_name(i.filter),
            vflip: i.vflip,
        })
        .collect();
    if plan.black_used {
        images.push(Image {
            name: names::BLACK_IMAGE.to_string(),
            path: names::BLACK_FILE.to_string(),
            wrap: "clamp",
            filter: "nearest",
            vflip: false,
        });
    }

    let info = &shader.info;
    let out = Scene {
        title,
        credit: info
            .username
            .as_ref()
            .filter(|u| !u.trim().is_empty())
            .map(|u| format!("{u} (Shadertoy)")),
        description: info.description.as_deref().filter(|d| !d.trim().is_empty()),
        passes,
        pass_sources,
        images,
        controls: controls(),
        source: Source {
            kind: "shadertoy",
            id: info.id.as_deref(),
            url: info
                .id
                .as_deref()
                .map(|id| format!("https://www.shadertoy.com/view/{id}")),
            author: info.username.as_deref(),
            name: info.name.as_deref(),
            license_note: LICENSE_NOTE,
            tags: &info.tags,
            passes: plan
                .passes
                .iter()
                .zip(layouts)
                .map(|(p, l)| SourcePass {
                    name: &p.name,
                    file: &p.file,
                    common_line: l.common_line,
                    code_line: l.code_line,
                })
                .collect(),
        },
    };
    let mut text = serde_json::to_string_pretty(&out).unwrap_or_default();
    text.push('\n');
    text
}

#[derive(Serialize)]
struct PendingOut<'a> {
    file: String,
    url: &'a str,
    pass: &'a str,
    channel: u32,
}

/// `images/pending-assets.json`: what to download, and where it goes.
pub(super) fn pending_assets(pending: &[PendingAsset]) -> String {
    let list: Vec<PendingOut<'_>> = pending
        .iter()
        .map(|p| PendingOut {
            file: format!("images/{}", p.file),
            url: &p.url,
            pass: &p.pass,
            channel: p.channel,
        })
        .collect();
    let mut text = serde_json::to_string_pretty(&list).unwrap_or_default();
    text.push('\n');
    text
}
