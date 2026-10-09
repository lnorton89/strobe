// ============================================================================
// Module:       strobe_shadertoy::convert::shim
// Description:  Generates each pass's GLSL file: the uniform shim, the
//               Common and pass code, and the entry point.
//
// Dependencies: super::plan, super::names, crate::model
// ============================================================================

//! The uniform shim.
//!
//! A Shadertoy pass expects uniforms (`iTime`, `iChannel0`...) and an
//! entry point (`mainImage`) that Strobe does not have. Rather than edit
//! the author's code, each pass file wraps it:
//!
//! ```text
//! // header (3 lines): what this is, where things start
//! globals and strobe_begin()      set each i* value from Strobe's uniforms
//! #define iTime strobe_iTime ...  the author's names now mean those values
//! // Common tab (verbatim, dialect-normalised)
//! // this pass's tab (verbatim, dialect-normalised)
//! vec4 renderMain()               strobe_begin(); mainImage(...); strobe_end()
//! ```
//!
//! ## Why globals and `#define`s
//!
//! `iResolution`, `iMouse` and friends are `#define`d to **globals the
//! shim sets at the top of `renderMain`**, not to expressions over Strobe's
//! uniforms, because the shim's definitions must keep working even if the
//! author's code redefines a name the shim's expressions use (`TIME`,
//! `RENDERSIZE`). All shim code that touches Strobe's names is *above*
//! the `#define`s; the entry point below the author's code uses only
//! `strobe_` names. A name the author's code declares at global scope that
//! the assembler also declares (`PI` is the common one) is aliased away
//! with a `#define PI strobe_user_PI`.
//!
//! Only the names a pass actually mentions are emitted, so a pass that
//! never reads time never fetches the clock, and one that never reads
//! `iChannelResolution` never calls `textureSize` on a sampler it does
//! not otherwise use.
//!
//! ## Time
//!
//! `iTime`, `iTimeDelta`, `iFrame` and `iFrameRate` come from the clock
//! pass (see [`clock_file`]), a one-pixel float buffer that accumulates
//! `dt * rate` where `rate` is the `st_speed` control blended with the
//! bass by `st_audio_time`. Doing this in a pass keeps the scene format
//! unchanged: no host-side state, nothing for the app to wire (D78).

use super::plan::{Binding, Plan, PlannedPass, Role};
use crate::model::Shader;
use std::fmt::Write as _;

/// Where a pass's pieces start, for `SOURCE.passes` and error mapping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Layout {
    /// The file line at which the Common tab's line 1 sits.
    pub common_line: Option<u32>,
    /// The file line at which the pass tab's line 1 sits.
    pub code_line: u32,
}

/// A generated pass file and where its pieces are.
pub(super) struct Generated {
    pub text: String,
    pub layout: Layout,
}

/// Names the assembler declares globally that author code may also
/// declare; each is aliased away when the author uses it.
const ASSEMBLER_NAMES: &[&str] = &[
    "PI",
    "TIME",
    "RENDERSIZE",
    "FRAMECOUNT",
    "PASSINDEX",
    "_xy",
    "_uv",
    "_uvc",
    "_mouse",
    "_click",
    "_muv",
    "_muvc",
];

/// The pieces of the shim, filled as the pass's names are found.
#[derive(Default)]
struct Parts {
    /// Global declarations.
    decls: String,
    /// Statements for `strobe_begin`.
    init: String,
    /// `#define`s placed after the shim, before the author's code.
    macros: String,
}

impl Parts {
    /// A global `strobe_<name>` of type `glsl`, set to `value` in
    /// `strobe_begin`, which the author's `<name>` is defined to mean.
    fn define(&mut self, name: &str, glsl: &str, value: &str) {
        let _ = writeln!(self.decls, "{glsl} strobe_{name};");
        let _ = writeln!(self.init, "    strobe_{name} = {value};");
        let _ = writeln!(self.macros, "#define {name} strobe_{name}");
    }
}

/// Lines in the file header.
const HEADER_LINES: u32 = 3;

/// `text` ending in exactly one newline.
fn with_newline(text: &str) -> String {
    if text.ends_with('\n') {
        text.to_string()
    } else {
        format!("{text}\n")
    }
}

fn lines(text: &str) -> u32 {
    text.matches('\n').count() as u32
}

/// The GLSL file for one pass.
pub(super) fn pass_file(plan: &Plan, pass: &PlannedPass, shader: &Shader) -> Generated {
    let used = |name: &str| pass.idents.contains(name);
    let image = pass.role == Role::Image;
    let needs_clock = super::plan::CLOCK_NAMES.iter().any(|n| used(n));

    let mut parts = Parts::default();

    if needs_clock {
        parts
            .init
            .push_str("    vec4 strobe_clock = texelFetch(st_clock, ivec2(0, 0), 0);\n");
    }
    if used("iResolution") {
        parts.define("iResolution", "vec3", "vec3(RENDERSIZE, 1.0)");
    }
    if used("iTime") {
        parts.define("iTime", "float", "strobe_clock.x");
    }
    if used("iTimeDelta") {
        parts.define("iTimeDelta", "float", "strobe_clock.z * strobe_rate()");
    }
    if used("iFrame") {
        parts.define(
            "iFrame",
            "int",
            "int(max(FRAMECOUNT - (strobe_clock.w - 1.0), 0.0) + 0.5)",
        );
    }
    if used("iFrameRate") {
        parts.define(
            "iFrameRate",
            "float",
            "(strobe_clock.z > 0.0 ? 1.0 / strobe_clock.z : 60.0)",
        );
    }
    if used("iMouse") {
        parts.define("iMouse", "vec4", "_mouse");
    }
    if used("iDate") {
        parts.define("iDate", "vec4", "strobe_Date");
    }
    if used("iSampleRate") {
        parts.macros.push_str("#define iSampleRate 44100.0\n");
    }
    if used("iChannelTime") {
        parts
            .decls
            .push_str("float strobe_iChannelTime[4] = float[4](0.0, 0.0, 0.0, 0.0);\n");
        parts
            .macros
            .push_str("#define iChannelTime strobe_iChannelTime\n");
        for (slot, binding) in pass.channels.iter().enumerate() {
            if *binding == Some(Binding::Audio) {
                let _ = writeln!(
                    parts.init,
                    "    strobe_iChannelTime[{slot}] = strobe_clock.x;"
                );
            }
        }
    }
    if used("iChannelResolution") {
        parts.decls.push_str(
            "vec3 strobe_iChannelResolution[4] = vec3[4](vec3(0.0), vec3(0.0), vec3(0.0), vec3(0.0));\n",
        );
        parts
            .macros
            .push_str("#define iChannelResolution strobe_iChannelResolution\n");
        for (slot, binding) in pass.channels.iter().enumerate() {
            if let Some(binding) = binding {
                let _ = writeln!(
                    parts.init,
                    "    strobe_iChannelResolution[{slot}] = vec3(vec2(textureSize({}, 0)), 1.0);",
                    plan.sampler_name(*binding)
                );
            }
        }
    }
    for (slot, binding) in pass.channels.iter().enumerate() {
        if used(&format!("iChannel{slot}"))
            && let Some(binding) = binding
        {
            let _ = writeln!(
                parts.macros,
                "#define iChannel{slot} {}",
                plan.sampler_name(*binding)
            );
        }
    }
    if pass.legacy_frag_color {
        parts
            .decls
            .push_str("vec4 strobe_gl_FragColor = vec4(0.0, 0.0, 0.0, 1.0);\n");
    }

    // Names the author declares that the assembler already did.
    let mut aliases = String::new();
    for name in ASSEMBLER_NAMES {
        if used(name) && !pass.macros.contains(*name) {
            let _ = writeln!(aliases, "#define {name} strobe_user_{name}");
        }
    }
    // GLSL ES 3.00 forbids redefining a built-in; desktop GLSL lets it hide.
    for name in &pass.shadowed {
        let _ = writeln!(
            aliases,
            "#ifdef GL_ES\n#define {name} strobe_user_{name}\n#endif"
        );
    }

    let mut body = String::new();
    body.push_str("vec2 strobe_fragCoord = vec2(0.0);\n");
    body.push_str(&parts.decls);
    if used("iTimeDelta") {
        body.push_str(
            "float strobe_rate() {\n    return st_speed * mix(1.0, 0.3 + 2.0 * syn_BassLevel, st_audio_time);\n}\n",
        );
    }
    body.push_str("void strobe_begin() {\n");
    body.push_str(&parts.init);
    if image {
        body.push_str(
            "    strobe_fragCoord = (gl_FragCoord.xy - 0.5 * RENDERSIZE) / st_zoom + 0.5 * RENDERSIZE;\n",
        );
    } else {
        body.push_str("    strobe_fragCoord = gl_FragCoord.xy;\n");
    }
    body.push_str("}\n");
    if image {
        body.push_str(
            "vec4 strobe_end(vec4 c) {\n\
             \x20   c.rgb *= 1.0 + st_bass_flash * syn_BassHits;\n\
             \x20   if (st_hue != 0.0) { c.rgb = _hueRotate(c.rgb, st_hue); }\n\
             \x20   c.a = 1.0;\n\
             \x20   return c;\n}\n",
        );
    } else {
        body.push_str("vec4 strobe_end(vec4 c) { return c; }\n");
    }
    body.push_str(&parts.macros);
    body.push_str(&aliases);

    let common = pass.common.as_ref().map(|c| with_newline(&c.code));
    let code = with_newline(&pass.code.code);
    let first = HEADER_LINES + lines(&body);
    // Line numbers of the first line of each tab, 1-based.
    let (common_line, code_line) = match &common {
        Some(text) => {
            let common_line = first + 2;
            (Some(common_line), common_line + lines(text) + 1)
        }
        None => (None, first + 2),
    };

    let info = &shader.info;
    let mut out = String::new();
    let _ = writeln!(
        out,
        "// ---- Strobe import of Shadertoy {} \"{}\" by {} (generated) ----",
        info.id.as_deref().unwrap_or("?"),
        info.name.as_deref().unwrap_or("?").replace('\n', " "),
        info.username.as_deref().unwrap_or("?").replace('\n', " "),
    );
    let _ = writeln!(
        out,
        "// Pass: {}. Shim above the first marker; Shadertoy code below, unedited.",
        pass.name
    );
    let _ = writeln!(
        out,
        "// Tab line 1 is file line {} for {}{}.",
        code_line,
        pass.name,
        common_line.map_or_else(String::new, |l| format!(", {l} for Common"))
    );
    out.push_str(&body);
    if let Some(text) = &common {
        out.push_str("// ---- Shadertoy tab: Common ----\n");
        out.push_str(text);
    }
    let _ = writeln!(out, "// ---- Shadertoy tab: {} ----", pass.name);
    out.push_str(&code);
    out.push_str("// ---- Strobe entry (generated) ----\n");
    let result = if pass.legacy_frag_color {
        "strobe_gl_FragColor"
    } else {
        "strobe_color"
    };
    let _ = write!(
        out,
        "vec4 renderMain() {{\n\
         \x20   vec4 strobe_color = vec4(0.0, 0.0, 0.0, 1.0);\n\
         \x20   strobe_begin();\n\
         \x20   mainImage(strobe_color, strobe_fragCoord);\n\
         \x20   return strobe_end({result});\n}}\n"
    );
    Generated {
        text: out,
        layout: Layout {
            common_line,
            code_line,
        },
    }
}

/// The clock pass: a one-texel float buffer holding
/// `(clock, TIME, dt, start frame + 1)`.
///
/// - `x` is the scaled clock, `iTime`. It advances by `dt * rate` each
///   frame, with `rate` from `st_speed` and `st_audio_time`.
/// - `y` is the scene `TIME` last frame, so `dt` needs no host input.
/// - `z` is the real `dt`, for `iTimeDelta` and `iFrameRate`.
/// - `w` is the frame the clock started on, plus one so that zero means
///   "never ran". A cleared target (first frame, or after a resize wipes
///   feedback) restarts the clock at `TIME` and `iFrame` at 0, which is
///   what a Shadertoy feedback shader that initialises on `iFrame == 0`
///   needs after its buffers were wiped.
pub(super) fn clock_file() -> String {
    "// ---- Strobe clock pass (generated) ----\n\
     // One float texel: x = iTime, y = last TIME, z = real dt, w = start frame + 1.\n\
     float strobe_rate() {\n\
     \x20   return st_speed * mix(1.0, 0.3 + 2.0 * syn_BassLevel, st_audio_time);\n\
     }\n\
     vec4 renderMain() {\n\
     \x20   vec4 prev = texelFetch(st_clock, ivec2(0, 0), 0);\n\
     \x20   bool fresh = prev.w == 0.0 || TIME < prev.y;\n\
     \x20   float dt = fresh ? 0.0 : TIME - prev.y;\n\
     \x20   float clock = fresh ? TIME : prev.x + dt * strobe_rate();\n\
     \x20   float start = fresh ? FRAMECOUNT + 1.0 : prev.w;\n\
     \x20   return vec4(clock, TIME, dt, start);\n\
     }\n"
    .to_string()
}
