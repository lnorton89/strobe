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
//! implicitly and GLSL ES rejects. [`Dialect::Es300`] (the ANGLE path)
//! therefore declares what it can (`texture2D`) and rewrites what is
//! unambiguous without types ([`crate::fixups`]: `PASSINDEX` compared with
//! a float literal, and global initializers that are not constant
//! expressions, D192) to keep such code compiling, but desktop GL is the
//! primary target and the rest of the implicit-conversion family is still
//! open (Q8).

use crate::audio::SCALARS;
use crate::fixups;
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
    // Desktop-only idioms, made explicit for ES. Line count is preserved.
    let (user_code, hoisted) = if dialect.is_es() {
        let compares = fixups::passindex_float_compares(user_code);
        let (code, hoisted) = fixups::hoist_global_initializers(&compares);
        (Cow::Owned(code.into_owned()), hoisted)
    } else {
        (Cow::Borrowed(user_code), Vec::new())
    };
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
    // Global initializers ES will not take, run here in source order, each
    // under its own line of main.glsl so an error still points there (D192).
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
    fn es_gets_the_passindex_cast_and_desktop_gets_the_code_as_written() -> Result<()> {
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
                source(dialect).contains("if (PASSINDEX == 0.0)"),
                "{dialect:?}: desktop compiles it as written"
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
}
