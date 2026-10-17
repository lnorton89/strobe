// ============================================================================
// Module:       strobe_gfx::caps
// Description:  What the context can do, decided once: the GLSL dialect to
//               assemble for, and the storage format float passes get.
//
// Dependencies: strobe-scene (Dialect); crate::gl (Info, Format)
// ============================================================================

//! Context capabilities.
//!
//! Decided once per context from what GL reports, then passed down, so no
//! per-frame path asks the driver anything.
//!
//! ## `#version 330 core` on every desktop context
//!
//! A 4.x core context — including macOS's 4.1 — accepts `#version 330
//! core`, and SSF code needs nothing newer. One dialect for every desktop
//! machine means one set of compile results to reason about.
//!
//! ## Float passes and old GPUs
//!
//! SSF says `FLOAT` passes are RGBA32F. On an integrated GPU that is 16
//! bytes a texel of shared-memory bandwidth for every read and write;
//! RGBA16F halves it and is plenty for almost every feedback effect.
//! [`FloatPrecision`] is the switch, and the benchmark measures both.

use crate::gl::{Format, Info};
use strobe_scene::assemble::Dialect;

/// Storage chosen for passes marked `FLOAT`.
///
/// The default is [`FloatPrecision::Half`], decided on measurements from the
/// owner's ThinkPad T480 (Intel UHD 620, 2026-10-09, D13). RGBA16F made the
/// bandwidth-bound reaction-diffusion scene 2.8x faster (4.47 to 1.59 ms at
/// 720p), and no scene was meaningfully slower. `Full` stays available for
/// a scene that integrates values large enough to lose precision in 16 bits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FloatPrecision {
    /// RGBA32F, as SSF specifies.
    Full,
    /// RGBA16F: half the bandwidth.
    #[default]
    Half,
}

/// The facts the renderer needs about its context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Caps {
    /// What GL reported.
    pub info: Info,
    /// The dialect every program is assembled in.
    pub dialect: Dialect,
    /// Whether `sampler1D` exists (desktop GL only).
    pub has_1d_textures: bool,
    /// Whether float color attachments are renderable.
    pub float_render: bool,
}

impl Caps {
    /// Decides capabilities from what the context reported.
    pub fn from_info(info: Info) -> Self {
        let (_, _, es) = info.parsed;
        Self {
            dialect: if es { Dialect::Es300 } else { Dialect::Glsl330 },
            has_1d_textures: !es,
            // Desktop GL 3.0+ requires float color attachments; ES 3.0
            // needs EXT_color_buffer_float (ANGLE on D3D11 exposes it).
            float_render: !es || info.es_float_render,
            info,
        }
    }

    /// The storage format for a pass.
    pub fn pass_format(&self, float: bool, precision: FloatPrecision) -> Format {
        match (float && self.float_render, precision) {
            (false, _) => Format::Rgba8,
            (true, FloatPrecision::Full) => Format::Rgba32f,
            (true, FloatPrecision::Half) => Format::Rgba16f,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(parsed: (u32, u32, bool), es_float_render: bool) -> Info {
        Info {
            version: String::new(),
            renderer: String::new(),
            vendor: String::new(),
            parsed,
            es_float_render,
            max_texture_size: 4096,
            max_texture_units: 16,
            gpu_timer: true,
        }
    }

    #[test]
    fn desktop_contexts_assemble_glsl_330() {
        for parsed in [(3, 3, false), (4, 1, false), (4, 6, false)] {
            assert_eq!(
                Caps::from_info(info(parsed, false)).dialect,
                Dialect::Glsl330,
                "{parsed:?}"
            );
        }
    }

    #[test]
    fn es_contexts_assemble_es_300_without_1d_textures() {
        let caps = Caps::from_info(info((3, 0, true), true));
        assert_eq!(caps.dialect, Dialect::Es300, "ES dialect");
        assert!(!caps.has_1d_textures, "GLSL ES has no sampler1D");
    }

    #[test]
    fn float_passes_fall_back_to_8_bit_without_float_render() {
        let caps = Caps::from_info(info((3, 0, true), false));
        assert_eq!(
            caps.pass_format(true, FloatPrecision::Full),
            Format::Rgba8,
            "no float render"
        );
        let desktop = Caps::from_info(info((3, 3, false), false));
        assert_eq!(
            desktop.pass_format(true, FloatPrecision::Half),
            Format::Rgba16f,
            "half"
        );
        assert_eq!(
            desktop.pass_format(false, FloatPrecision::Full),
            Format::Rgba8,
            "not float"
        );
    }
}
