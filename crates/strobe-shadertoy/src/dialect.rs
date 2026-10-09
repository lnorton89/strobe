// ============================================================================
// Module:       strobe_shadertoy::dialect
// Description:  Layer (d): deterministic, token-level rewrites that make
//               Shadertoy source acceptable to both Strobe dialects.
//
// Dependencies: crate::glsl (the lexer)
// ============================================================================

//! Dialect normalisation.
//!
//! Shadertoy compiles as GLSL ES 3.00 with a prelude of its own. Strobe
//! compiles `#version 330 core` on desktop and `#version 300 es` under
//! ANGLE, with its own preamble. This stage rewrites what differs and can
//! be rewritten *safely*, and nothing else:
//!
//! | Found | Becomes | Why |
//! |---|---|---|
//! | `texture2D`, `textureCube`, `texture2DLod`, `texture3D`... | `texture`, `textureLod`... | WebGL1 names; core profiles do not have them |
//! | `precision highp float;` | removed | the assembler sets precision; a stray `mediump` would lower it |
//! | `#version ...` | removed | a second `#version` is an error |
//! | `#extension ...` | removed | the built-ins they enabled are core in ES 3.00, and a late `#extension` is an error in ES |
//! | `uniform vec3 iResolution;` (any Shadertoy built-in) | removed | the shim provides it; redeclaring it would collide |
//! | non-ASCII text in comments | `?` | some drivers reject it, even in comments |
//! | `\r\n`, lone `\r`, a BOM | `\n`, nothing | one line ending; error lines count the same everywhere |
//!
//! Every removal keeps its newlines, so line numbers in the output are the
//! line numbers of the input: a driver error at line 40 is line 40 of the
//! Shadertoy tab (plus the converter's known header).
//!
//! ## Why the output is dialect-neutral
//!
//! Shadertoy accepts only what ES 3.00 accepts, so valid Shadertoy code is
//! already inside the intersection of the two dialects, and one rewritten
//! file serves both. The assembler picks the `#version` line when it
//! assembles for the context in hand.
//!
//! ## What it does not do
//!
//! Implicit `int` to `float` conversion, the construct desktop GLSL
//! accepts and ES rejects (OPEN_QUESTIONS Q8), is not rewritten: finding
//! every place needs types, and a half-working inserter is worse than
//! none. Shadertoy source never needs it; Synesthesia scenes do, and they
//! are not this stage's input. The packing built-ins (`packHalf2x16` and
//! friends) are core in ES 3.00 but need GLSL 4.00 on desktop; they are
//! left alone and recorded in Q8 too.
//!
//! The Strobe semantic rewrites (`gl_FragColor`, names that collide with
//! the assembler's) are decisions about the *shader*, not the dialect,
//! and live in the converter.

use crate::glsl::{self, Kind, Token};
use std::collections::BTreeSet;

/// What the rewriting did, for the import report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rewrite {
    /// A legacy built-in was renamed.
    LegacyCall {
        /// The name found.
        from: String,
        /// The name it became.
        to: &'static str,
    },
    /// A `precision` statement was removed.
    PrecisionStatement,
    /// A `#version` line was removed.
    Version,
    /// A `#extension` line was removed; holds the line's text.
    Extension(String),
    /// A redeclaration of a Shadertoy built-in uniform was removed.
    UniformDeclaration(String),
    /// Non-ASCII characters in a comment were replaced.
    NonAsciiComment,
}

/// One rewrite and where it happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Note {
    /// 1-based line in the input (and the output).
    pub line: u32,
    /// What changed.
    pub rewrite: Rewrite,
}

/// Source after normalisation, and what is known about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Normalised {
    /// The rewritten code.
    pub code: String,
    /// Every rewrite, in source order.
    pub notes: Vec<Note>,
    /// Names the code defines as macros (`#define NAME`).
    pub macros: BTreeSet<String>,
    /// Every identifier in the rewritten code outside comments.
    pub identifiers: BTreeSet<String>,
    /// Built-in functions the code defines itself (see
    /// [`shadowed_builtins`]).
    pub shadowed_builtins: BTreeSet<String>,
}

/// Legacy built-in names and what they become.
const LEGACY_CALLS: &[(&str, &str)] = &[
    ("texture2D", "texture"),
    ("texture2DLod", "textureLod"),
    ("texture2DProj", "textureProj"),
    ("texture2DProjLod", "textureProjLod"),
    ("texture2DGrad", "textureGrad"),
    ("texture2DLodEXT", "textureLod"),
    ("texture2DGradEXT", "textureGrad"),
    ("textureCube", "texture"),
    ("textureCubeLod", "textureLod"),
    ("textureCubeGrad", "textureGrad"),
    ("textureCubeLodEXT", "textureLod"),
    ("textureCubeGradEXT", "textureGrad"),
    ("texture3D", "texture"),
    ("texture3DLod", "textureLod"),
    ("textureLodEXT", "textureLod"),
    ("textureGradEXT", "textureGrad"),
];

/// The uniforms Shadertoy declares for every shader.
pub const SHADERTOY_UNIFORMS: &[&str] = &[
    "iResolution",
    "iTime",
    "iTimeDelta",
    "iFrame",
    "iFrameRate",
    "iMouse",
    "iDate",
    "iSampleRate",
    "iChannelTime",
    "iChannelResolution",
    "iChannel0",
    "iChannel1",
    "iChannel2",
    "iChannel3",
];

/// Rewrites `source`. Pure and deterministic: the same text always gives
/// the same output and notes.
pub fn normalise(source: &str) -> Normalised {
    let text = clean_line_endings(source);
    let tokens = glsl::lex(&text);
    let mut out = String::with_capacity(text.len());
    let mut notes = Vec::new();
    let mut macros = BTreeSet::new();
    let mut line = 1_u32;
    let mut i = 0;

    while let Some(token) = tokens.get(i) {
        let here = line;
        let mut note = |rewrite: Rewrite| {
            notes.push(Note {
                line: here,
                rewrite,
            })
        };
        match token.kind {
            Kind::Ident => {
                if token.text == "precision"
                    && let Some(end) = precision_statement_end(&tokens, i)
                {
                    emit_removed(&mut out, &tokens, i, end);
                    note(Rewrite::PrecisionStatement);
                    line += newlines(&tokens, i, end);
                    i = end;
                    continue;
                }
                if token.text == "uniform"
                    && let Some((end, name)) = shadertoy_uniform_end(&tokens, i)
                {
                    emit_removed(&mut out, &tokens, i, end);
                    note(Rewrite::UniformDeclaration(name));
                    line += newlines(&tokens, i, end);
                    i = end;
                    continue;
                }
                match legacy_name(token.text) {
                    Some(to) => {
                        out.push_str(to);
                        note(Rewrite::LegacyCall {
                            from: token.text.to_string(),
                            to,
                        });
                    }
                    None => out.push_str(token.text),
                }
            }
            Kind::Directive => match glsl::split_directive(token.text) {
                Some(("version", _)) => {
                    out.push_str(&blank_lines(token.text));
                    note(Rewrite::Version);
                }
                Some(("extension", body)) => {
                    out.push_str(&blank_lines(token.text));
                    note(Rewrite::Extension(body.trim().to_string()));
                }
                Some((name, body)) => {
                    let prefix = token
                        .text
                        .get(..token.text.len() - body.len())
                        .unwrap_or("");
                    out.push_str(prefix);
                    let body_tokens = glsl::lex_directive_body(body);
                    if name == "define"
                        && let Some(defined) = body_tokens.iter().find(|t| t.kind == Kind::Ident)
                    {
                        macros.insert(defined.text.to_string());
                    }
                    for inner in &body_tokens {
                        match (inner.kind, legacy_name(inner.text)) {
                            (Kind::Ident, Some(to)) => {
                                out.push_str(to);
                                note(Rewrite::LegacyCall {
                                    from: inner.text.to_string(),
                                    to,
                                });
                            }
                            _ => out.push_str(&ascii_only(inner, &mut note)),
                        }
                    }
                }
                None => out.push_str(token.text),
            },
            Kind::LineComment | Kind::BlockComment => {
                out.push_str(&ascii_only(token, &mut note));
            }
            Kind::Number | Kind::Punct | Kind::Space => out.push_str(token.text),
        }
        line += token.text.matches('\n').count() as u32;
        i += 1;
    }

    let identifiers = glsl::identifiers(&out);
    let shadowed_builtins = shadowed_builtins(&out);
    Normalised {
        code: out,
        notes,
        macros,
        identifiers,
        shadowed_builtins,
    }
}

/// Built-in functions that WebGL1-era shaders commonly define themselves
/// because GLSL ES 1.00 lacked them.
const REDEFINABLE_BUILTINS: &[&str] = &[
    "tanh",
    "sinh",
    "cosh",
    "asinh",
    "acosh",
    "atanh",
    "round",
    "roundEven",
    "trunc",
    "inverse",
    "transpose",
    "determinant",
    "outerProduct",
    "isnan",
    "isinf",
];

/// Return types a function definition can start with.
const TYPE_NAMES: &[&str] = &[
    "void", "float", "int", "uint", "bool", "vec2", "vec3", "vec4", "ivec2", "ivec3", "ivec4",
    "uvec2", "uvec3", "uvec4", "bvec2", "bvec3", "bvec4", "mat2", "mat3", "mat4",
];

/// The built-in functions `code` defines itself, e.g. `float tanh(float x)`.
///
/// GLSL ES 3.00 forbids that (Mesa rejects it), though desktop GLSL 3.30
/// lets a definition hide the built-in. The converter aliases each name
/// away under `#ifdef GL_ES`, so the ES build calls the shader's own
/// version under a private name and desktop compiles the code as written.
/// Detection is by shape (`<type> <name> (`), so a *call* does not count.
pub fn shadowed_builtins(code: &str) -> BTreeSet<String> {
    let tokens: Vec<Token<'_>> = glsl::lex(code)
        .into_iter()
        .filter(|t| !matches!(t.kind, Kind::Space | Kind::LineComment | Kind::BlockComment))
        .collect();
    tokens
        .windows(3)
        .filter_map(|window| match window {
            [ty, name, paren]
                if ty.kind == Kind::Ident
                    && TYPE_NAMES.contains(&ty.text)
                    && name.kind == Kind::Ident
                    && REDEFINABLE_BUILTINS.contains(&name.text)
                    && paren.text == "(" =>
            {
                Some(name.text.to_string())
            }
            _ => None,
        })
        .collect()
}

/// A comment's text with non-ASCII characters replaced by `?`.
fn ascii_only(token: &Token<'_>, note: &mut impl FnMut(Rewrite)) -> String {
    let comment = matches!(token.kind, Kind::LineComment | Kind::BlockComment);
    if comment && !token.text.is_ascii() {
        note(Rewrite::NonAsciiComment);
        token
            .text
            .chars()
            .map(|c| if c.is_ascii() { c } else { '?' })
            .collect()
    } else {
        token.text.to_string()
    }
}

fn legacy_name(name: &str) -> Option<&'static str> {
    LEGACY_CALLS
        .iter()
        .find(|(from, _)| *from == name)
        .map(|(_, to)| *to)
}

/// Strips a BOM and makes every line ending `\n`.
fn clean_line_endings(source: &str) -> String {
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);
    source.replace("\r\n", "\n").replace('\r', "\n")
}

/// Newlines in tokens `from..to`.
fn newlines(tokens: &[Token<'_>], from: usize, to: usize) -> u32 {
    tokens
        .get(from..to)
        .unwrap_or_default()
        .iter()
        .map(|t| t.text.matches('\n').count() as u32)
        .sum()
}

/// A run of newlines standing in for removed text.
fn blank_lines(text: &str) -> String {
    "\n".repeat(text.matches('\n').count())
}

fn emit_removed(out: &mut String, tokens: &[Token<'_>], from: usize, to: usize) {
    out.push_str(&"\n".repeat(newlines(tokens, from, to) as usize));
}

/// The significant (non-space, non-comment) token at or after `from`.
fn next_significant(tokens: &[Token<'_>], from: usize) -> Option<usize> {
    (from..tokens.len()).find(|&j| {
        tokens.get(j).is_some_and(|t| {
            !matches!(t.kind, Kind::Space | Kind::LineComment | Kind::BlockComment)
        })
    })
}

/// If `tokens[start]` begins `precision <lowp|mediump|highp> <type> ;`,
/// the index just past the `;`.
fn precision_statement_end(tokens: &[Token<'_>], start: usize) -> Option<usize> {
    let qualifier = next_significant(tokens, start + 1)?;
    if !matches!(tokens.get(qualifier)?.text, "lowp" | "mediump" | "highp") {
        return None;
    }
    let ty = next_significant(tokens, qualifier + 1)?;
    if tokens.get(ty)?.kind != Kind::Ident {
        return None;
    }
    let semicolon = next_significant(tokens, ty + 1)?;
    (tokens.get(semicolon)?.text == ";").then_some(semicolon + 1)
}

/// If `tokens[start]` begins `uniform [qualifier] type NAME [ [N] ] ;` for
/// a Shadertoy built-in NAME, the index past the `;` and the name.
fn shadertoy_uniform_end(tokens: &[Token<'_>], start: usize) -> Option<(usize, String)> {
    let mut idents = Vec::new();
    let mut j = start + 1;
    let mut brackets = 0;
    // A declaration is a few tokens; give up rather than scan far.
    for _ in 0..16 {
        let at = next_significant(tokens, j)?;
        let token = tokens.get(at)?;
        j = at + 1;
        match (token.kind, token.text) {
            (Kind::Punct, ";") => {
                let name = idents.last().copied()?;
                return (idents.len() >= 2 && SHADERTOY_UNIFORMS.contains(&name))
                    .then(|| (j, name.to_string()));
            }
            (Kind::Ident, text) if brackets == 0 => idents.push(text),
            (Kind::Punct, "[") => brackets += 1,
            (Kind::Punct, "]") => brackets -= 1,
            (Kind::Number | Kind::Ident, _) if brackets > 0 => {}
            // A comma (several declarators), an initialiser, anything else:
            // not the simple redeclaration this removes.
            _ => return None,
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn code(source: &str) -> String {
        normalise(source).code
    }

    #[test]
    fn legacy_texture_calls_are_renamed() {
        let got = code(
            "vec4 a = texture2D(s, uv); vec4 b = texture2DLod(s, uv, 0.0);\nvec4 c = textureCube(k, d);",
        );
        assert_eq!(
            got,
            "vec4 a = texture(s, uv); vec4 b = textureLod(s, uv, 0.0);\nvec4 c = texture(k, d);",
            "renamed in place"
        );
    }

    #[test]
    fn names_inside_comments_and_longer_names_are_untouched() {
        let source =
            "// texture2D is old\nfloat mytexture2D = 1.0; /* texture2D */ float texture2Dx;";
        assert_eq!(
            code(source),
            source,
            "only whole identifiers outside comments"
        );
    }

    #[test]
    fn renames_reach_macro_bodies() {
        let got = code("#define TEX(s, p) texture2D(s, p)\nvec4 c = TEX(a, b);");
        assert_eq!(
            got, "#define TEX(s, p) texture(s, p)\nvec4 c = TEX(a, b);",
            "macro body"
        );
    }

    #[test]
    fn precision_statements_go_but_precision_qualifiers_stay() {
        let got = code(
            "precision highp float;\nprecision mediump int; precision lowp sampler2D;\nhighp vec3 x = vec3(0.0);\nmediump float f;",
        );
        assert_eq!(
            got, "\n \nhighp vec3 x = vec3(0.0);\nmediump float f;",
            "statements removed with their line breaks, declarations kept"
        );
    }

    #[test]
    fn version_and_extension_lines_go_and_line_numbers_hold() {
        let n = normalise(
            "#version 300 es\n#extension GL_OES_standard_derivatives : enable\nfloat x;\n",
        );
        assert_eq!(n.code, "\n\nfloat x;\n", "blanked, not deleted");
        assert_eq!(
            n.notes,
            vec![
                Note {
                    line: 1,
                    rewrite: Rewrite::Version
                },
                Note {
                    line: 2,
                    rewrite: Rewrite::Extension("GL_OES_standard_derivatives : enable".to_string())
                },
            ],
            "what was removed, and where"
        );
    }

    #[test]
    fn redeclared_shadertoy_uniforms_are_removed() {
        let source = "uniform vec3 iResolution;\nuniform highp float iTime;\nuniform sampler2D iChannel0;\nuniform float iChannelTime[4];\nuniform vec3 iChannelResolution[4];\nuniform float mine;\nuniform vec3 iResolution, other;\nfloat k;";
        let n = normalise(source);
        assert_eq!(
            n.code, "\n\n\n\n\nuniform float mine;\nuniform vec3 iResolution, other;\nfloat k;",
            "the five plain redeclarations go; a user uniform and a multi-declarator stay"
        );
        let names: Vec<String> = n
            .notes
            .iter()
            .filter_map(|note| match &note.rewrite {
                Rewrite::UniformDeclaration(name) => Some(name.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            names,
            vec![
                "iResolution",
                "iTime",
                "iChannel0",
                "iChannelTime",
                "iChannelResolution"
            ],
            "reported by name"
        );
    }

    #[test]
    fn line_endings_and_bom_are_normalised() {
        assert_eq!(code("\u{feff}a\r\nb\rc\n"), "a\nb\nc\n", "BOM, CRLF and CR");
    }

    #[test]
    fn non_ascii_comment_text_is_replaced_and_code_is_kept() {
        let n = normalise("// 45\u{b0} turn \u{2014} ok\nfloat x; /* \u{3c0} */");
        assert_eq!(
            n.code, "// 45? turn ? ok\nfloat x; /* ? */",
            "comments sanitised"
        );
        assert_eq!(
            n.notes
                .iter()
                .filter(|x| x.rewrite == Rewrite::NonAsciiComment)
                .count(),
            2,
            "one note per comment"
        );
    }

    #[test]
    fn defined_macros_and_identifiers_are_reported() {
        let n = normalise(
            "#define PI 3.14159\n#define SQ(x) ((x)*(x))\nfloat f(float a) { return SQ(a) * PI; }",
        );
        let macros: Vec<&str> = n.macros.iter().map(String::as_str).collect();
        assert_eq!(macros, vec!["PI", "SQ"], "macro names");
        assert!(
            n.identifiers.contains("f") && n.identifiers.contains("a"),
            "idents"
        );
        assert!(
            !n.identifiers.contains("texture2D"),
            "renamed names are gone"
        );
    }

    #[test]
    fn user_defined_builtins_are_found_but_calls_are_not() {
        let n = normalise(
            "float tanh(float x) { return x; }\n\
             vec3 f(vec3 v) { return v * tanh(2.0) + round(v); }\n\
             mat2 inverse(mat2 m) { return m; }",
        );
        let found: Vec<&str> = n.shadowed_builtins.iter().map(String::as_str).collect();
        assert_eq!(
            found,
            vec!["inverse", "tanh"],
            "definitions only: {found:?}"
        );
    }

    #[test]
    fn normalising_twice_changes_nothing() {
        let source = "#version 300 es\nprecision highp float;\nuniform vec3 iResolution;\nvec4 c = texture2D(s, uv); // \u{b0}\r\n";
        let once = code(source);
        assert_eq!(code(&once), once, "the stage is idempotent");
    }

    #[test]
    fn a_real_looking_legacy_header_comes_out_clean() {
        let source = "#ifdef GL_ES\nprecision mediump float;\n#endif\nvoid mainImage(out vec4 fragColor, in vec2 fragCoord) {\n    fragColor = texture2D(iChannel0, fragCoord / iResolution.xy);\n}\n";
        let n = normalise(source);
        assert_eq!(n.notes.len(), 2, "precision and texture2D: {:?}", n.notes);
        assert!(
            n.code.contains("#ifdef GL_ES\n\n#endif"),
            "the guard stays, empty: {}",
            n.code
        );
        assert!(n.code.contains("texture(iChannel0"), "renamed: {}", n.code);
    }
}
