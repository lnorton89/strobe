// ============================================================================
// Module:       strobe_scene::promote::tests
// Description:  Unit tests for the int→float rewrite: each conversion site,
//               the cases it must leave alone, and line preservation.
//
// Dependencies: super::int_to_float
// ============================================================================

//! The rewrite, case by case. The GL tests in `assemble` and the corpus
//! harness compile its output; these pin down the text.

use super::int_to_float;

/// A stand-in for the generated preamble: a few uniforms and library
/// overloads, as the real context provides them.
const CONTEXT: &str = "\
uniform vec2 RENDERSIZE;
uniform float TIME;
const int PASSINDEX = 0;
uniform sampler2D syn_Media;
uniform sampler2D syn_Spectrum;
#define texture2D texture
float _scale(float v, float lo, float hi) { return lo + v * (hi - lo); }
vec3 _normalizeRGB(float r, float g, float b) { return vec3(r, g, b) / 255.0; }
float pick(int i) { return 1.0; }
float pick(float x) { return x; }
";

fn rewrite(user: &str) -> String {
    int_to_float(user, CONTEXT, true).into_owned()
}

#[test]
fn literals_beside_floats_become_float_literals() {
    assert_eq!(
        rewrite("float f(float x) { return x * 2 + 1; }"),
        "float f(float x) { return x * 2.0 + 1.0; }",
        "both operands of a float sum"
    );
    assert_eq!(
        rewrite("float f(float x) { return x > 0 ? x : -1; }"),
        "float f(float x) { return x > 0.0 ? x : -1.0; }",
        "a comparison and a ternary branch"
    );
}

#[test]
fn initializers_assignments_and_returns_convert() {
    assert_eq!(
        rewrite("float g = 1;\nvoid f() { float a = 0, b; b = 3; a += 2; }"),
        "float g = 1.0;\nvoid f() { float a = 0.0, b; b = 3.0; a += 2.0; }",
        "global and local initializers, plain and compound assignment"
    );
    assert_eq!(
        rewrite("float sgn(float x) { return (x < 0) ? -1 : 1; }"),
        "float sgn(float x) { return (x < 0.0) ? -1.0 : 1.0; }",
        "the hg_sdf sgn idiom"
    );
}

#[test]
fn integer_variables_are_wrapped() {
    assert_eq!(
        rewrite(
            "vec4 renderMain() { float s = 0.0; for (int i = 0; i < 4; i++) { s += i * 0.25; } return vec4(s); }"
        ),
        "vec4 renderMain() { float s = 0.0; for (int i = 0; i < 4; i++) { s += float(i) * 0.25; } return vec4(s); }",
        "the loop stays int, the use beside a float converts"
    );
    assert_eq!(
        rewrite("void f() { ivec2 p = ivec2(1); vec2 q = p * 0.5; }"),
        "void f() { ivec2 p = ivec2(1); vec2 q = vec2(p) * 0.5; }",
        "an int vector meets a float"
    );
}

#[test]
fn integer_division_is_wrapped_not_rewritten() {
    assert_eq!(
        rewrite("float f(float x) { return x * (1 / 2); }"),
        "float f(float x) { return x * float((1 / 2)); }",
        "1/2 is 0 on desktop and must stay 0"
    );
}

#[test]
fn builtins_convert_their_float_arguments() {
    assert_eq!(
        rewrite("float f(float x) { return pow(x, 2) + mod(x, 3) + clamp(x, 0, 1) + max(0, x); }"),
        "float f(float x) { return pow(x, 2.0) + mod(x, 3.0) + clamp(x, 0.0, 1.0) + max(0.0, x); }",
        "float-only and mixed built-ins"
    );
    assert_eq!(
        rewrite("int f(int a) { return max(a, 1) + abs(-2); }"),
        "int f(int a) { return max(a, 1) + abs(-2); }",
        "integer overloads stay integer"
    );
    assert_eq!(
        rewrite(
            "vec4 f(vec2 uv) { return textureLod(syn_Media, uv, 0) + texture2D(syn_Media, uv); }"
        ),
        "vec4 f(vec2 uv) { return textureLod(syn_Media, uv, 0.0) + texture2D(syn_Media, uv); }",
        "the LOD converts, and a macro alias still types as a texture call"
    );
    assert_eq!(
        rewrite("vec4 f(ivec2 p) { return texelFetch(syn_Media, p, 0); }"),
        "vec4 f(ivec2 p) { return texelFetch(syn_Media, p, 0); }",
        "texelFetch takes integers"
    );
}

#[test]
fn user_functions_resolve_overloads_first() {
    assert_eq!(
        rewrite("vec3 f(float x) { return _normalizeRGB(252, 255, 245) * _scale(x, -1, 1); }"),
        "vec3 f(float x) { return _normalizeRGB(252.0, 255.0, 245.0) * _scale(x, -1.0, 1.0); }",
        "a library function whose only overload takes floats"
    );
    assert_eq!(
        rewrite("float f() { return pick(1); }"),
        "float f() { return pick(1); }",
        "an exact int overload wins, as on desktop"
    );
}

#[test]
fn macros_are_typed_from_their_replacement() {
    assert_eq!(
        rewrite(
            "#define STEPS 64\n#define EPS 0.001\nfloat f(float x) { return x / STEPS + EPS; }"
        ),
        "#define STEPS 64\n#define EPS 0.001\nfloat f(float x) { return x / float(STEPS) + EPS; }",
        "an int macro is wrapped where it meets a float"
    );
    assert_eq!(
        rewrite(
            "#define S(a, b, t) smoothstep(a, b, t)\nfloat f(float x) { return S(0, 1, x) * 2; }"
        ),
        "#define S(a, b, t) smoothstep(a, b, t)\nfloat f(float x) { return S(0, 1, x) * 2; }",
        "a function-like macro's result is unknown, so nothing beside it changes"
    );
}

#[test]
fn unknown_names_and_unparsable_code_are_left_alone() {
    assert_eq!(
        rewrite("float f() { return mystery * 2; }"),
        "float f() { return mystery * 2; }",
        "an unknown name makes the whole expression unknown"
    );
    assert_eq!(
        rewrite("float f(float x) { return mystery(x) * 2; }"),
        "float f(float x) { return mystery(x) * 2; }",
        "an unknown function's result is unknown"
    );
    assert_eq!(
        rewrite("float f(float x) { x = x * 2; @@@ }\nfloat g(float y) { return y * 2; }"),
        "float f(float x) { x = x * 2; @@@ }\nfloat g(float y) { return y * 2.0; }",
        "a body that does not parse keeps its text; the next one still converts"
    );
}

#[test]
fn structs_swizzles_and_arrays_are_followed() {
    let source = "struct Hit { float d; int id; };\n\
                  void f(vec3 p) { Hit h = Hit(1, 2); float a[3] = float[3](1, 2, 3); h.d = p.x * 2; a[0] = h.id; }";
    let expected = "struct Hit { float d; int id; };\n\
                    void f(vec3 p) { Hit h = Hit(1.0, 2); float a[3] = float[3](1.0, 2.0, 3.0); h.d = p.x * 2.0; a[0] = float(h.id); }";
    assert_eq!(
        rewrite(source),
        expected,
        "fields, swizzles and array elements"
    );
}

#[test]
fn passindex_against_a_float_is_wrapped() {
    assert_eq!(
        rewrite("vec4 renderMain() { if (PASSINDEX == 0.0) { return vec4(1); } return vec4(0); }"),
        "vec4 renderMain() { if (float(PASSINDEX) == 0.0) { return vec4(1); } return vec4(0); }",
        "the D-era fix-up falls out of the general rule"
    );
}

#[test]
fn odd_literals_are_wrapped_and_lines_never_move() {
    let source = "float f(float x) {\n  return x * 010 + x * 0x10 + x * 3u;\n}\n";
    let out = rewrite(source);
    assert_eq!(
        out, "float f(float x) {\n  return x * float(010) + x * float(0x10) + x * float(3u);\n}\n",
        "octal, hex and unsigned literals keep their value"
    );
    assert_eq!(
        out.lines().count(),
        source.lines().count(),
        "insertions only"
    );
}

#[test]
fn nothing_to_do_borrows() {
    let source = "float f(float x) { return x * 2.0; }";
    assert!(
        matches!(
            int_to_float(source, CONTEXT, true),
            std::borrow::Cow::Borrowed(_)
        ),
        "an already explicit source is returned as is"
    );
}

#[test]
fn a_scalar_coordinate_into_an_audio_texture_gains_its_row() {
    assert_eq!(
        rewrite(
            "float f(float x) { return texture(syn_Spectrum, x).r + textureLod(syn_Spectrum, x, 0).g; }"
        ),
        "float f(float x) { return texture(syn_Spectrum, vec2(x, 0.5)).r + textureLod(syn_Spectrum, vec2(x, 0.5), 0.0).g; }",
        "1D on desktop, one texel tall on ES"
    );
    assert_eq!(
        rewrite(
            "float f(vec2 uv) { return texture(syn_Spectrum, uv).r + texture(syn_Media, uv.x).r; }"
        ),
        "float f(vec2 uv) { return texture(syn_Spectrum, uv).r + texture(syn_Media, uv.x).r; }",
        "a 2D coordinate, or another texture, is left alone"
    );
}

#[test]
fn built_in_variables_and_preprocessor_branches_are_followed() {
    assert_eq!(
        rewrite("vec2 f() { return vec2(-1 + 2 * gl_FragCoord.x, 0.0); }"),
        "vec2 f() { return vec2(-1.0 + 2.0 * gl_FragCoord.x, 0.0); }",
        "gl_FragCoord is a vec4"
    );
    assert_eq!(
        rewrite(
            "float f(float x) {\n const float\n#ifdef KODELIFE\n off = .4\n#else\n off = .2\n#endif\n ;\n return x * 13 - off;\n}\n"
        ),
        "float f(float x) {\n const float\n#ifdef KODELIFE\n off = .4\n#else\n off = .2\n#endif\n ;\n return x * 13.0 - off;\n}\n",
        "only the branch the preprocessor keeps is parsed"
    );
}
