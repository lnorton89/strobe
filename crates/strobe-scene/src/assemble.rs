// ============================================================================
// Module:       strobe_scene::assemble
// Description:  Builds the GLSL the driver compiles: version header for the
//               context's dialect, SSF uniforms, function library, user code.
//
// Dependencies: crate::scene, crate::audio, strobe-ssf-lib
// ============================================================================

//! Shader assembly.
//!
//! A scene's `main.glsl` is not a complete shader: it uses uniforms it
//! never declares, functions it never defines, and has `renderMain`
//! instead of `main`. This module wraps it into one that compiles:
//!
//! ```text
//! #version …                  dialect header
//! uniform …                   standard, audio, control, pass, image uniforms
//! <SSF function library>
//! #line 1 1                   user code is "source string 1", from line 1
//! <main.glsl>
//! #line 1 2
//! void main() { … renderMain() … }
//! ```
//!
//! The `#line` directives give the user's file its own source-string
//! number, so a driver error such as `1:14(3): error …` points at line 14
//! of `main.glsl`, and [`map_error_line`] can tell an error in the user's
//! code from one in the preamble.
//!
//! ## One program per pass, with `PASSINDEX` a constant
//!
//! Synesthesia compiles `main.glsl` once and branches on a `PASSINDEX`
//! uniform. That costs more than it looks on old GPUs: a fragment
//! program's register allocation is set by its *heaviest* path, so a cheap
//! blur pass sharing a program with a raymarcher runs at the raymarcher's
//! occupancy. With [`PassIndex::Constant`] each pass gets its own program
//! in which `PASSINDEX` is a compile-time constant, and the driver deletes
//! every branch that pass never takes. [`PassIndex::Uniform`] is the
//! faithful fallback.
//!
//! ## One source file per pass
//!
//! A scene that sets `PASS_SOURCES` gives each pass its own file, and the
//! assembler is simply called once per pass with that pass's text as
//! `user_code`. Nothing here changes: the file is still source string 1,
//! so a driver error still maps to [`ErrorSite::User`] with a line of *that
//! pass's file*, and the caller (which knows the file name) reports it as
//! `passes/buffer_b.glsl:14`. Helper functions with the same name in two
//! passes cannot collide, because no two passes share a program.
//!
//! ## Dialects
//!
//! SSF code is desktop GLSL, and real scenes lean on it: `PASSINDEX ==
//! 0.0` compares an `int` with a `float`, which desktop GLSL converts
//! implicitly and GLSL ES rejects. Every dialect gets the conversions
//! written out by [`crate::promote`] and `#if` on a non-macro read as 0 by
//! [`crate::directives`] (D199). [`Dialect::Es300`] (the ANGLE path) also
//! gets [`crate::fixups`]: global initializers that are not constant
//! expressions move into `main` (D192), an initialised uniform becomes
//! a global, and `pow(x, y)` is written as `exp2(log2(x) * (y))`, so a
//! negative base is NaN as on the native drivers rather than ANGLE's
//! `|x|^y` (D301).

use crate::audio::SCALARS;
use crate::directives::{self, Branches};
use crate::fixups;
use crate::promote;
use crate::scene::SceneDef;
use std::borrow::Cow;
use std::fmt::Write as _;

/// The GLSL dialect a context compiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Dialect {
    /// Desktop GLSL 3.30 core (OpenGL 3.3+).
    Glsl330,
    /// Desktop GLSL 4.10 core (macOS's ceiling).
    Glsl410,
    /// GLSL ES 3.00 (OpenGL ES 3.0, and ANGLE on Direct3D 11).
    Es300,
}

impl Dialect {
    /// The `#version` line.
    pub fn version_line(self) -> &'static str {
        match self {
            Self::Glsl330 => "#version 330 core",
            Self::Glsl410 => "#version 410 core",
            Self::Es300 => "#version 300 es",
        }
    }

    /// Whether this is GLSL ES.
    pub fn is_es(self) -> bool {
        match self {
            Self::Es300 => true,
            Self::Glsl330 | Self::Glsl410 => false,
        }
    }

    /// Default precision statements, needed in GLSL ES and harmless (but
    /// omitted) elsewhere.
    pub fn precision(self) -> &'static str {
        if self.is_es() {
            "precision highp float;\nprecision highp int;\nprecision highp sampler2D;\nprecision highp samplerCube;\n"
        } else {
            ""
        }
    }
}

/// How `PASSINDEX` reaches the shader.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PassIndex {
    /// A compile-time constant: one specialised program per pass.
    Constant(usize),
    /// A `uniform int`: one program shared by every pass.
    Uniform,
}

/// The GLSL type of a uniform added from outside the scene definition,
/// such as one a script creates with `setUniform`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ExtraType {
    /// `float`.
    Float,
    /// `vec2`.
    Vec2,
    /// `vec3`.
    Vec3,
    /// `vec4`.
    Vec4,
}

impl ExtraType {
    /// The GLSL spelling.
    pub fn glsl(self) -> &'static str {
        match self {
            Self::Float => "float",
            Self::Vec2 => "vec2",
            Self::Vec3 => "vec3",
            Self::Vec4 => "vec4",
        }
    }
}

/// Source string numbers used in `#line` directives.
pub mod source {
    /// The generated preamble and library.
    pub const PREAMBLE: u32 = 0;
    /// The user's `main.glsl`.
    pub const USER: u32 = 1;
    /// The generated `main`.
    pub const EPILOGUE: u32 = 2;
    /// Files `#include`d by the user's code (D198).
    pub const INCLUDED: u32 = 3;
}

/// Everything the assembler needs for one program.
#[derive(Debug, Clone, Copy)]
pub struct Request<'a> {
    /// The validated scene.
    pub scene: &'a SceneDef,
    /// The user's `main.glsl`.
    pub user_code: &'a str,
    /// The context's dialect.
    pub dialect: Dialect,
    /// How `PASSINDEX` is provided.
    pub pass_index: PassIndex,
    /// Uniforms created by a script, by name.
    pub extra: &'a [(String, ExtraType)],
}

/// Assembles one fragment shader.
pub fn fragment(request: &Request<'_>) -> String {
    let Request {
        scene,
        user_code,
        dialect,
        pass_index,
        extra,
    } = *request;
    let mut out = String::with_capacity(user_code.len() + strobe_ssf_lib::LIBRARY.len() + 4096);
    let es = dialect.is_es();

    // `writeln!` into a String cannot fail; the results are discarded.
    let _ = writeln!(out, "{}", dialect.version_line());
    out.push_str(dialect.precision());
    out.push_str("// ---- strobe preamble (generated) ----\n");
    out.push_str("uniform vec2 RENDERSIZE;\nuniform float TIME;\nuniform float FRAMECOUNT;\n");
    match pass_index {
        PassIndex::Constant(index) => {
            let _ = writeln!(out, "const int PASSINDEX = {index};");
        }
        PassIndex::Uniform => out.push_str("uniform int PASSINDEX;\n"),
    }
    out.push_str(
        "uniform vec4 _mouse;\nuniform vec3 _click;\nuniform vec2 _muv;\nuniform vec2 _muvc;\n\
         uniform sampler2D syn_FinalPass;\nuniform sampler2D syn_Media;\nuniform float syn_MediaType;\n",
    );
    // Strobe's own additions, behind the reserved `strobe_` prefix: the
    // Shadertoy audio texture (512x2, spectrum then waveform) and `iDate`.
    out.push_str("uniform sampler2D strobe_ShadertoyAudio;\nuniform vec4 strobe_Date;\n");
    for name in SCALARS {
        let _ = writeln!(out, "uniform float {name};");
    }
    // The documentation calls the audio data textures 1D samplers. GLSL ES
    // has no sampler1D, so there they are 2D textures one texel tall.
    let sampler1d = if es { "sampler2D" } else { "sampler1D" };
    let _ = writeln!(
        out,
        "uniform {sampler1d} syn_Spectrum;\nuniform {sampler1d} syn_LevelTrail;"
    );

    for control in &scene.controls {
        let _ = writeln!(
            out,
            "uniform {} {};",
            control.kind.glsl_type(),
            control.name
        );
    }
    for pass in &scene.passes {
        if let Some(target) = &pass.target {
            let _ = writeln!(out, "uniform sampler2D {target};");
        }
    }
    for image in &scene.images {
        let _ = writeln!(out, "uniform sampler2D {};", image.name);
    }
    for cube in &scene.cubemaps {
        let _ = writeln!(out, "uniform samplerCube {};", cube.name);
    }
    for transition in &scene.transition_uniforms {
        let _ = writeln!(out, "uniform float {};", transition.name);
    }
    for (name, ty) in extra {
        let _ = writeln!(out, "uniform {} {name};", ty.glsl());
    }

    out.push_str(
        "const float PI = 3.14159265359;\n\
         vec2 _xy = vec2(0.0);\nvec2 _uv = vec2(0.0);\nvec2 _uvc = vec2(0.0);\n\
         out vec4 strobe_FragColor;\n\
         #define texture2D texture\n\
         #define texture2DLod textureLod\n\
         #define IMG_NORM_PIXEL(img, uv) texture(img, uv)\n\
         #define IMG_PIXEL(img, xy) texture(img, (xy) / vec2(textureSize(img, 0)))\n\
         #define IMG_SIZE(img) vec2(textureSize(img, 0))\n",
    );
    out.push_str("// ---- SSF function library ----\n");
    out.push_str(strobe_ssf_lib::LIBRARY);
    if !strobe_ssf_lib::LIBRARY.ends_with('\n') {
        out.push('\n');
    }

    // `#if` on a name that is not a macro reads as 0 on every driver, as on
    // NVIDIA. Then the desktop-only idioms, made explicit for ES, with
    // everything above as the declarations the rewrite types against
    // (D199). Line count is preserved throughout.
    let user_code = directives::undefined_names_to_zero(user_code, &out);
    let (user_code, hoisted) = if es {
        let compares = fixups::passindex_float_compares(&user_code);
        let uniforms = fixups::uniform_initializers(&compares, Branches::for_es(&out));
        let promoted = promote::rewrite(&uniforms, &out, true);
        // A sampler chosen with `?:` that the rewrite could not move into
        // a call fails on every ES driver; say so in Strobe's words, at the
        // user's line, rather than leave the driver's message alone (D261).
        for line in &promoted.unsupported {
            let _ = writeln!(out, "#line {line} {}", source::USER);
            out.push_str(UNSUPPORTED_SELECT);
        }
        // `pow` of a negative base is undefined: NaN on the native drivers,
        // `|x|^y` under ANGLE. Spell out the native sequence (D301).
        let guarded = fixups::pow_as_exp2_log2(&promoted.code);
        let (code, hoisted) = fixups::hoist_global_initializers(&guarded, Branches::for_es(&out));
        (Cow::Owned(code.into_owned()), hoisted)
    } else {
        // Strict desktop compilers (Mesa, Apple's) reject some of the same
        // conversions NVIDIA accepts, such as an int `return` from a float
        // function. The explicit form is accepted by every driver.
        let promoted = promote::int_to_float(&user_code, &out, false);
        // Desktop drivers accept a non-constant global initializer, but run
        // it before `main`, when `_xy`/`_uv`/`_uvc` are still zero: a scene's
        // `vec2 uv = _uv;` was (0, 0) on every pixel, and sprinkles rendered
        // black. Hoisted, it runs after them, as on ES (D192, D293).
        let (code, hoisted) =
            fixups::hoist_global_initializers(&promoted, Branches::for_dialect(false, &out));
        (Cow::Owned(code.into_owned()), hoisted)
    };

    let _ = writeln!(out, "#line 1 {}", source::USER);
    out.push_str(&user_code);
    if !user_code.ends_with('\n') {
        out.push('\n');
    }

    let _ = writeln!(out, "#line 1 {}", source::EPILOGUE);
    out.push_str(
        "void main() {\n\
         \x20   _xy = gl_FragCoord.xy;\n\
         \x20   _uv = _xy / RENDERSIZE;\n\
         \x20   _uvc = (_xy - 0.5 * RENDERSIZE) / RENDERSIZE.y;\n",
    );
    // Global initializers, run here in source order after the coordinates
    // are set, each under its own line of main.glsl so an error still
    // points there (D192 for ES, D293 for desktop).
    for hoist in &hoisted {
        let _ = writeln!(out, "#line {} {}", hoist.line, source::USER);
        let _ = writeln!(out, "    {}", hoist.assignment);
    }
    if !hoisted.is_empty() {
        let _ = writeln!(out, "#line 5 {}", source::EPILOGUE);
    }
    out.push_str("    strobe_FragColor = renderMain();\n");
    // The output is opaque, as Synesthesia's and Shadertoy's are: a final
    // pass that returns alpha 0 still shows its colour (D192). Buffer passes
    // keep their alpha, which scenes use as a fourth data channel.
    let last = scene.final_pass();
    match pass_index {
        PassIndex::Constant(index) if index == last => {
            out.push_str("    strobe_FragColor.a = 1.0;\n");
        }
        PassIndex::Constant(_) => {}
        PassIndex::Uniform => {
            let _ = writeln!(
                out,
                "    if (PASSINDEX == {last}) {{ strobe_FragColor.a = 1.0; }}"
            );
        }
    }
    out.push_str("}\n");
    out
}

/// The `#error` an ES program gets for each sampler `?:` that
/// [`promote::rewrite`] could not move into a call. It holds no quotes, which
/// some preprocessors read as an unterminated literal.
pub const UNSUPPORTED_SELECT: &str = "#error strobe: a sampler is chosen with ?: here, which GLSL ES does not allow. Strobe rewrites it only as the one such argument of a function call, as in texture(c ? a : b, uv)\n";

/// The full-screen-triangle vertex shader every pass shares.
///
/// One triangle covering the viewport, generated from `gl_VertexID`, so
/// there is no vertex buffer to bind — only an empty vertex array, which
/// core profiles require to exist.
pub fn vertex(dialect: Dialect) -> String {
    format!(
        "{}\n{}\
         void main() {{\n\
         \x20   vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));\n\
         \x20   gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);\n\
         }}\n",
        dialect.version_line(),
        dialect.precision()
    )
}

/// Where a driver error line points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorSite {
    /// Line `n` of the user's `main.glsl`.
    User(u32),
    /// Generated code — a Strobe bug, or a name the user redeclared.
    Generated,
}

/// Finds the source string and line in one line of a driver's info log.
///
/// Drivers disagree on format: Mesa writes `1:14(3): error: …`, NVIDIA
/// `1(14) : error C0000: …`, and ANGLE / AMD / Intel `ERROR: 1:14: …`.
/// All three put the source string number first, then the line.
pub fn map_error_line(log_line: &str) -> Option<ErrorSite> {
    let trimmed = log_line.trim_start();
    let rest = trimmed
        .strip_prefix("ERROR:")
        .or_else(|| trimmed.strip_prefix("WARNING:"))
        .unwrap_or(trimmed)
        .trim_start();
    let digits = |s: &str| -> Option<(u32, usize)> {
        let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
        if end == 0 {
            return None;
        }
        s.get(..end)?.parse().ok().map(|n| (n, end))
    };
    let (string, used) = digits(rest)?;
    let after = rest.get(used..)?;
    let after = after
        .strip_prefix(':')
        .or_else(|| after.strip_prefix('('))?;
    let (line, _) = digits(after)?;
    Some(if string == source::USER {
        ErrorSite::User(line)
    } else {
        ErrorSite::Generated
    })
}

/// The 1-based column a driver log line gives, when it gives one.
///
/// Only Mesa does: `1:14(3): error: …` is source string 1, line 14, column
/// 3, counted from 1. NVIDIA's `1(14) : …` and the `ERROR: 1:14: …` of
/// ANGLE, AMD and Intel name a line only. Mesa counts the line as its
/// preprocessor printed it, not as it was given ([`compiled_column`]), and
/// the text it was given is the assembler's, which may have rewritten the
/// user's ([`original_column`]).
pub fn driver_column(log_line: &str) -> Option<u32> {
    let trimmed = log_line.trim_start();
    let rest = trimmed
        .strip_prefix("ERROR:")
        .or_else(|| trimmed.strip_prefix("WARNING:"))
        .unwrap_or(trimmed)
        .trim_start();
    let digits = |s: &str| -> Option<(u32, usize)> {
        let end = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
        if end == 0 {
            return None;
        }
        s.get(..end)?.parse().ok().map(|n| (n, end))
    };
    let (_, used) = digits(rest)?;
    let rest = rest.get(used..)?.strip_prefix(':')?;
    let (_, used) = digits(rest)?;
    let rest = rest.get(used..)?.strip_prefix('(')?;
    let (column, used) = digits(rest)?;
    let closed = rest.get(used..)?.starts_with(')');
    (closed && column > 0).then_some(column)
}

/// The text of line `line` of source string `string` in `text`, following
/// its `#line` directives as a driver does, starting at line 1 of
/// `first_string`. The first such line wins.
///
/// The assembled shader starts in [`source::PREAMBLE`], and the user's code
/// is introduced by `#line 1 1`; a scene's own shader text (with its
/// `#include`s expanded) starts in [`source::USER`].
pub fn source_line(text: &str, first_string: u32, string: u32, line: u32) -> Option<&str> {
    let mut at = (first_string, 1_u32);
    for text_line in text.split('\n') {
        if let Some((number, named)) = line_directive(text_line) {
            at = (named.unwrap_or(at.0), number);
            continue;
        }
        if at == (string, line) {
            return Some(text_line.strip_suffix('\r').unwrap_or(text_line));
        }
        at.1 = at.1.saturating_add(1);
    }
    None
}

/// `#line N` or `#line N S`: the next line's number and, when given, its
/// source string.
fn line_directive(text_line: &str) -> Option<(u32, Option<u32>)> {
    let rest = text_line.trim_start().strip_prefix('#')?.trim_start();
    let rest = rest.strip_prefix("line")?;
    if !rest.starts_with(|c: char| c.is_ascii_whitespace()) {
        return None;
    }
    let mut words = rest.split_whitespace();
    let number = words.next()?.parse().ok()?;
    let string = match words.next() {
        Some(word) => Some(word.parse().ok()?),
        None => None,
    };
    Some((number, string))
}

/// Where Mesa's `column` falls on `compiled`, the line Mesa was given, as a
/// 1-based byte column.
///
/// Mesa's compiler reads its preprocessor's output, in which every run of
/// spaces, tabs and comments within a line is one space, leading ones
/// included, so `    return x;` has `return` at column 2. Checked on
/// llvmpipe (`render.rs`, the column test). Macro expansion moves columns
/// too, and is not undone.
pub fn compiled_column(compiled: &str, column: u32) -> u32 {
    let bytes = compiled.as_bytes();
    let wanted = (column.max(1) - 1) as usize;
    let mut printed = 0_usize;
    let mut at = 0_usize;
    while at < bytes.len() {
        if printed == wanted {
            break;
        }
        let run = blank_run(bytes, at);
        at += run.max(1);
        printed += 1;
    }
    u32::try_from(at.min(bytes.len()) + 1).unwrap_or(u32::MAX)
}

/// How many bytes from `at` are blank to Mesa's preprocessor: spaces,
/// tabs, `/* */` comments and a `//` comment to the end of the line.
fn blank_run(bytes: &[u8], from: usize) -> usize {
    let mut at = from;
    loop {
        match (bytes.get(at), bytes.get(at + 1)) {
            (Some(b' ' | b'\t' | b'\r' | b'\x0b' | b'\x0c'), _) => at += 1,
            (Some(b'/'), Some(b'/')) => at = bytes.len(),
            (Some(b'/'), Some(b'*')) => {
                at = bytes
                    .get(at + 2..)
                    .and_then(|rest| rest.windows(2).position(|w| w == b"*/"))
                    .map_or(bytes.len(), |end| at + 2 + end + 2);
            }
            _ => return at - from,
        }
    }
}

/// Where `column` (1-based, in bytes) of `compiled`, a line as the driver
/// compiled it, falls on `original`, the same line as the user wrote it.
///
/// The assembler's rewrites ([`crate::promote`], [`crate::fixups`],
/// [`crate::directives`]) keep lines but not columns: `float x = 1;` is
/// compiled as `float x = 1.0;`. The text both lines share at their start
/// keeps its columns, and so does the text they share at their end, counted
/// from the end; a column inside the stretch the rewrite changed is the
/// start of that stretch on the user's line. The result is at most one past
/// the end of `original`.
pub fn original_column(original: &str, compiled: &str, column: u32) -> u32 {
    let (a, b) = (original.as_bytes(), compiled.as_bytes());
    let prefix = a.iter().zip(b).take_while(|(x, y)| x == y).count();
    let room = a.len().min(b.len()) - prefix;
    let suffix = a
        .iter()
        .rev()
        .zip(b.iter().rev())
        .take(room)
        .take_while(|(x, y)| x == y)
        .count();
    let at = (column.max(1) - 1) as usize;
    let mapped = if at < prefix {
        at
    } else if at >= b.len() - suffix {
        // Counted back from the end, which the two lines share.
        a.len().saturating_sub(b.len().saturating_sub(at))
    } else {
        prefix
    };
    u32::try_from(mapped.min(a.len()) + 1).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    fn scene() -> Result<SceneDef> {
        Ok(SceneDef::parse(
            r#"{ "CONTROLS": [ {"NAME": "amount", "TYPE": "slider"},
                               {"NAME": "pos", "TYPE": "xy"},
                               {"NAME": "tint", "TYPE": "color"} ],
                 "PASSES": [ {"TARGET": "buf", "DOUBLE_BUFFER": true} ],
                 "IMAGES": [ {"NAME": "photo", "PATH": "images/a.png"} ] }"#,
            "s",
        )?)
    }

    #[test]
    fn every_scene_name_is_declared_with_its_type() -> Result<()> {
        let scene = scene()?;
        let extra = vec![("fromScript".to_string(), ExtraType::Vec4)];
        let src = fragment(&Request {
            scene: &scene,
            user_code: "vec4 renderMain() { return vec4(amount); }",
            dialect: Dialect::Glsl330,
            pass_index: PassIndex::Uniform,
            extra: &extra,
        });
        for decl in [
            "uniform float amount;",
            "uniform vec2 pos;",
            "uniform vec3 tint;",
            "uniform sampler2D buf;",
            "uniform sampler2D photo;",
            "uniform vec4 fromScript;",
            "uniform int PASSINDEX;",
            "uniform float syn_BassLevel;",
            "uniform sampler1D syn_Spectrum;",
            "uniform sampler2D strobe_ShadertoyAudio;",
            "uniform vec4 strobe_Date;",
        ] {
            assert!(src.contains(decl), "missing `{decl}`");
        }
        assert!(
            src.starts_with("#version 330 core\n"),
            "the version comes first"
        );
        Ok(())
    }

    #[test]
    fn a_constant_pass_index_is_baked_in() -> Result<()> {
        let scene = scene()?;
        let src = fragment(&Request {
            scene: &scene,
            user_code: "",
            dialect: Dialect::Glsl330,
            pass_index: PassIndex::Constant(1),
            extra: &[],
        });
        assert!(
            src.contains("const int PASSINDEX = 1;"),
            "constant declared"
        );
        assert!(
            !src.contains("uniform int PASSINDEX"),
            "and not as a uniform"
        );
        Ok(())
    }

    #[test]
    fn a_global_initializer_runs_after_the_coordinates_in_every_dialect() -> Result<()> {
        // D293: run before main, `vec2 uv = _uv;` read (0, 0) on desktop.
        let scene = scene()?;
        for dialect in [Dialect::Glsl330, Dialect::Glsl410, Dialect::Es300] {
            let src = fragment(&Request {
                scene: &scene,
                user_code: "vec2 uv = _uv;\nvec4 renderMain() { return vec4(uv, 0.0, 1.0); }\n",
                dialect,
                pass_index: PassIndex::Constant(1),
                extra: &[],
            });
            let set = src.find("_uv = _xy / RENDERSIZE;");
            let read = src.find("    uv = _uv;");
            assert!(
                matches!((set, read), (Some(set), Some(read)) if set < read),
                "{dialect:?}: the assignment follows `_uv` in main"
            );
            assert!(
                !src.contains("vec2 uv = _uv;"),
                "{dialect:?}: and the global keeps no initializer"
            );
        }
        Ok(())
    }

    #[test]
    fn only_es_spells_out_pow() -> Result<()> {
        // D301: ES gets the native drivers' sequence; desktop keeps its pow.
        let scene = scene()?;
        let user = "float g = pow(TIME - 9.0, 0.5);\nvec4 renderMain() { return vec4(g); }\n";
        let source = |dialect| {
            fragment(&Request {
                scene: &scene,
                user_code: user,
                dialect,
                pass_index: PassIndex::Constant(1),
                extra: &[],
            })
        };
        assert!(
            source(Dialect::Es300).contains("g = exp2(log2(TIME - 9.0) * ( 0.5));"),
            "ES: the hoisted initializer is spelled out"
        );
        for dialect in [Dialect::Glsl330, Dialect::Glsl410] {
            let src = source(dialect);
            assert!(
                src.contains("g = pow(TIME - 9.0, 0.5);") && !src.contains("log2(TIME"),
                "{dialect:?}: desktop source unchanged"
            );
        }
        Ok(())
    }

    #[test]
    fn the_es_dialect_has_precision_and_no_sampler1d() -> Result<()> {
        let scene = scene()?;
        let src = fragment(&Request {
            scene: &scene,
            user_code: "",
            dialect: Dialect::Es300,
            pass_index: PassIndex::Uniform,
            extra: &[],
        });
        assert!(
            src.starts_with("#version 300 es\nprecision highp float;"),
            "ES header"
        );
        assert!(!src.contains("sampler1D"), "GLSL ES has no sampler1D");
        Ok(())
    }

    #[test]
    fn every_dialect_gets_the_passindex_cast() -> Result<()> {
        let scene = scene()?;
        let user = "vec4 renderMain() {\n  if (PASSINDEX == 0.0) { return vec4(1.0); }\n  return vec4(0.0);\n}";
        let source = |dialect| {
            fragment(&Request {
                scene: &scene,
                user_code: user,
                dialect,
                pass_index: PassIndex::Constant(0),
                extra: &[],
            })
        };
        assert!(
            source(Dialect::Es300).contains("if (float(PASSINDEX) == 0.0)"),
            "ES: cast"
        );
        for dialect in [Dialect::Glsl330, Dialect::Glsl410] {
            assert!(
                source(dialect).contains("if (float(PASSINDEX) == 0.0)"),
                "{dialect:?}: the explicit form on desktop too, for strict drivers (D199)"
            );
        }
        Ok(())
    }

    #[test]
    fn the_user_file_is_its_own_source_string() -> Result<()> {
        let scene = scene()?;
        let src = fragment(&Request {
            scene: &scene,
            user_code: "// first line",
            dialect: Dialect::Glsl330,
            pass_index: PassIndex::Uniform,
            extra: &[],
        });
        assert!(
            src.contains("#line 1 1\n// first line\n"),
            "user code follows #line 1 1"
        );
        Ok(())
    }

    #[test]
    fn driver_error_formats_map_to_the_user_file() {
        let cases = [
            (
                "1:14(3): error: `foo' undeclared",
                Some(ErrorSite::User(14)),
            ),
            (
                "1(14) : error C1008: undefined variable",
                Some(ErrorSite::User(14)),
            ),
            (
                "ERROR: 1:14: 'foo' : undeclared identifier",
                Some(ErrorSite::User(14)),
            ),
            ("0:220(5): error: redefinition", Some(ErrorSite::Generated)),
            ("some other text", None),
        ];
        for (line, expected) in cases {
            assert_eq!(map_error_line(line), expected, "`{line}`");
        }
    }

    #[test]
    fn only_mesas_format_gives_a_column() {
        let cases = [
            ("1:14(3): error: `foo' undeclared", Some(3)),
            ("WARNING: 1:2(19): warning: unused", Some(19)),
            ("1(14) : error C1008: undefined variable", None),
            ("ERROR: 1:14: 'foo' : undeclared identifier", None),
            ("1:14(0): error: no column", None),
            ("1:14(x): error", None),
            ("some other text", None),
        ];
        for (line, expected) in cases {
            assert_eq!(driver_column(line), expected, "`{line}`");
        }
    }

    #[test]
    fn a_source_line_is_found_through_line_directives() {
        let assembled = "#version 330 core\nuniform float a;\n#line 1 1\nfirst\nsecond\n\
                         #line 1 2\nvoid main() {\n#line 2 1\n    hoisted\n}\n";
        assert_eq!(
            source_line(assembled, source::PREAMBLE, source::USER, 2),
            Some("second"),
            "the user's line 2, not the hoisted copy after it"
        );
        assert_eq!(
            source_line(assembled, source::PREAMBLE, source::PREAMBLE, 2),
            Some("uniform float a;"),
            "the preamble counts from 1"
        );
        assert_eq!(
            source_line(assembled, source::PREAMBLE, source::EPILOGUE, 1),
            Some("void main() {"),
            "the epilogue"
        );
        let user = "one\n#line 1 3\nincluded\n#line 2 1\ntwo\r\n";
        assert_eq!(
            source_line(user, source::USER, source::USER, 2),
            Some("two"),
            "an expanded include keeps the user's numbering; a CR is not text"
        );
        assert_eq!(
            source_line(user, source::USER, source::USER, 9),
            None,
            "past the end"
        );
    }

    #[test]
    fn mesas_column_counts_runs_of_blanks_as_one() {
        let line = "    float x = 1.0;  /* note */ return  y;";
        let at = |needle: &str| line.find(needle).map_or(0, |i| i as u32 + 1);
        assert_eq!(
            compiled_column(line, 2),
            at("float"),
            "the leading spaces print as one"
        );
        assert_eq!(
            compiled_column(line, 17),
            at("return"),
            "a comment and the spaces around it print as one"
        );
        assert_eq!(compiled_column(line, 1), 1, "the first column");
        assert_eq!(
            compiled_column(line, 99),
            line.len() as u32 + 1,
            "past the end is the end"
        );
        assert_eq!(
            compiled_column("a // rest", 3),
            "a // rest".len() as u32 + 1,
            "a line comment runs to the end"
        );
    }

    #[test]
    fn a_rewritten_lines_column_goes_back_to_the_users_text() {
        let original = "    float x = 1; vec4 c = vec4(nope);";
        let compiled = "    float x = 1.0; vec4 c = vec4(nope);";
        let nope = |line: &str| line.find("nope").map_or(0, |at| at as u32 + 1);
        assert_eq!(
            original_column(original, compiled, nope(compiled)),
            nope(original),
            "after the rewrite: counted from the shared end"
        );
        assert_eq!(
            original_column(original, compiled, 5),
            5,
            "before the rewrite: unchanged"
        );
        assert_eq!(
            original_column(original, compiled, 17),
            16,
            "inside the rewrite: where it starts on the user's line"
        );
        assert_eq!(
            original_column("abc", "abc", 2),
            2,
            "an unchanged line keeps its columns"
        );
        assert_eq!(
            original_column("ab", "abcdef", 6),
            3,
            "never past the end of the user's line"
        );
        assert_eq!(original_column("", "", 4), 1, "an empty line");
    }
}
