// ============================================================================
// Module:       strobe_scene::promote::tests
// Description:  Unit tests for the int→float rewrite: each conversion site,
//               the cases it must leave alone, and line preservation.
//
// Dependencies: super::int_to_float
// ============================================================================

//! The rewrite, case by case. The GL tests in `assemble` and the corpus
//! harness compile its output; these pin down the text.

use super::{int_to_float, rewrite as report};

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

fn desktop(user: &str) -> String {
    int_to_float(user, CONTEXT, false).into_owned()
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
    let source =
        "#define S(a, b, t) smoothstep(a, b, t)\nfloat f(float x) { return S(0, 1, x) * 2; }";
    assert_eq!(
        desktop(source),
        source,
        "on desktop a function-like macro's result is unknown, so nothing beside it changes"
    );
    assert_eq!(
        rewrite(source),
        "#define S(a, b, t) smoothstep(a, b, t)\nfloat f(float x) { return S(0.0, 1.0, x) * 2.0; }",
        "on ES the call is typed through its expansion (D261)"
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

#[test]
fn a_function_like_macro_converts_its_arguments_at_the_call_on_es() {
    let cases = [
        (
            "#define S(a, b, t) smoothstep(a, b, t)\nfloat f(float x) { return S(0, 1., x); }",
            "#define S(a, b, t) smoothstep(a, b, t)\nfloat f(float x) { return S(0.0, 1., x); }",
            "a literal passed straight to a float built-in",
        ),
        (
            "#define ROT(a) mat2(cos(a), -sin(a), sin(a), cos(a))\nvec2 f(vec2 p) { return ROT(1) * p; }",
            "#define ROT(a) mat2(cos(a), -sin(a), sin(a), cos(a))\nvec2 f(vec2 p) { return ROT(1.0) * p; }",
            "four uses of one parameter that agree, and the result typed as a mat2",
        ),
        (
            "#define S(a, b, t) smoothstep(a, b, t)\n#define H(n) fract(sin(n) * 43758.5453)\nfloat f() { return S(0, 1, H(3)); }",
            "#define S(a, b, t) smoothstep(a, b, t)\n#define H(n) fract(sin(n) * 43758.5453)\nfloat f() { return S(0.0, 1.0, H(3.0)); }",
            "a macro call inside another's argument",
        ),
    ];
    for (source, expected, what) in cases {
        assert_eq!(rewrite(source), expected, "ES: {what}");
        assert_eq!(desktop(source), source, "desktop is untouched: {what}");
    }
}

#[test]
fn a_call_whose_macro_body_needs_a_conversion_is_inlined_on_its_lines() {
    let source = "#define sat(x) clamp(x, 0, 1)\n\
                  float f(float v, int i) { int k = sat(i); return sat(v); }\n\
                  float g(float v) {\n  return sat(\n    v);\n}\n";
    let out = rewrite(source);
    assert_eq!(
        out,
        "#define sat(x) clamp(x, 0, 1)\n\
         float f(float v, int i) { int k = sat(i); return  clamp ( v , 0.0 , 1.0 ) ; }\n\
         float g(float v) {\n  return  clamp ( v , 0.0 , 1.0 ) \n;\n}\n",
        "the int call is left alone, each float call becomes its expansion, and the body is never edited"
    );
    assert_eq!(
        out.lines().count(),
        source.lines().count(),
        "a call over two lines still covers two"
    );
    assert_eq!(desktop(source), source, "desktop is untouched");
}

#[test]
fn a_parameter_used_as_an_int_and_a_float_is_inlined() {
    assert_eq!(
        rewrite("float w[4];\n#define TAP(i) (w[i] * i)\nfloat f() { return TAP(2); }"),
        "float w[4];\n#define TAP(i) (w[i] * i)\nfloat f() { return  ( w [ 2 ] * 2.0 ) ; }",
        "the index stays 2, the factor becomes 2.0"
    );
}

#[test]
fn macros_the_walker_cannot_follow_are_left_alone() {
    for (source, what) in [
        (
            "#define S(a) sin(a)\n#undef S\n#define S(a) cos(a)\nfloat f() { return S(1) * 2; }",
            "a macro undefined and defined again",
        ),
        (
            "#define C(a, b) a ## b\nfloat f() { return sin(C(1, 2)) * 2; }",
            "token pasting",
        ),
        (
            "#define abs(x) abs((x) * 1)\nfloat f(float v) { return abs(v) * 2; }",
            "a body that needs editing and names its own macro",
        ),
        (
            "#define S(a, b, t) smoothstep(a, b, t)\nfloat f(float x) { return S(0, x) * 2; }",
            "the wrong number of arguments",
        ),
    ] {
        assert_eq!(rewrite(source), source, "{what}");
    }
}

#[test]
fn a_sampler_chosen_with_a_ternary_moves_the_call_inside() {
    let cases = [
        (
            "vec4 f(int i, vec2 uv) { return texture(i == 0 ? syn_Media : syn_Spectrum, uv); }",
            "vec4 f(int i, vec2 uv) { return (i == 0 ? texture ( syn_Media , uv ) : texture ( syn_Spectrum , uv )); }",
            "the plain shape",
        ),
        (
            "vec4 f(float x, vec2 uv) { return textureLod(x > 0 ? syn_Media : syn_Spectrum, uv, 0); }",
            "vec4 f(float x, vec2 uv) { return (x > 0.0 ? textureLod ( syn_Media , uv , 0.0 ) : textureLod ( syn_Spectrum , uv , 0.0 )); }",
            "conversions in the condition and the other arguments are kept",
        ),
        (
            "vec4 f(int i, vec2 uv) { return texture((i == 0) ? syn_Media : (i == 1 ? syn_Spectrum : syn_Media), uv); }",
            "vec4 f(int i, vec2 uv) { return (( i == 0 ) ? texture ( syn_Media , uv ) : (i == 1 ? texture ( syn_Spectrum , uv ) : texture ( syn_Media , uv ))); }",
            "nested and parenthesised choices",
        ),
        (
            "vec4 tap(sampler2D s, vec2 p) { return texture(s, p); }\nvec4 f(bool b, vec2 uv) { return tap(b ? syn_Media : syn_Spectrum, uv * 2); }",
            "vec4 tap(sampler2D s, vec2 p) { return texture(s, p); }\nvec4 f(bool b, vec2 uv) { return (b ? tap ( syn_Media , uv * 2.0 ) : tap ( syn_Spectrum , uv * 2.0 )); }",
            "a user function",
        ),
    ];
    for (source, expected, what) in cases {
        assert_eq!(rewrite(source), expected, "ES: {what}");
        assert_eq!(
            desktop(source),
            expected,
            "desktop, where Mesa rejects it too: {what}"
        );
        assert!(
            report(source, CONTEXT, true).unsupported.is_empty(),
            "nothing left to report: {what}"
        );
    }
    let source = "vec4 f(int i, vec2 uv) {\n    return texture(\n        i == 0 ? syn_Media : syn_Spectrum,\n        uv).r * vec4(1);\n}\n";
    let out = rewrite(source);
    assert_eq!(
        out,
        "vec4 f(int i, vec2 uv) {\n    return (i == 0 ? texture ( syn_Media , uv ) : texture ( syn_Spectrum , uv ))\n\n.r * vec4(1);\n}\n",
        "a call over three lines"
    );
    assert_eq!(out.lines().count(), source.lines().count(), "no line moves");
}

#[test]
fn a_sampler_choice_inside_a_macro_is_rewritten_on_es_and_reported_on_desktop() {
    let source = "#define TAP(s, p) texture(s, p)\nvec4 f(bool c, vec2 uv) { return TAP(c ? syn_Media : syn_Spectrum, uv); }";
    let es = report(source, CONTEXT, true);
    assert_eq!(
        es.code,
        "#define TAP(s, p) texture(s, p)\nvec4 f(bool c, vec2 uv) { return  (c ? texture ( syn_Media , uv ) : texture ( syn_Spectrum , uv )) ; }",
        "ES inlines the macro and moves the call inside"
    );
    assert!(es.unsupported.is_empty(), "ES: nothing to report");
    let desktop = report(source, CONTEXT, false);
    assert_eq!(desktop.code, source, "desktop leaves the macro alone");
    assert_eq!(desktop.unsupported, vec![2], "and reports the line");
}

#[test]
fn two_sampler_choices_in_one_call_are_reported_not_rewritten() {
    let source = "vec4 two(sampler2D a, sampler2D b) { return texture(a, vec2(0.5)) + texture(b, vec2(0.5)); }\n\
                  vec4 f(bool c) {\n\
                    return two(c ? syn_Media : syn_Spectrum, c ? syn_Spectrum : syn_Media);\n\
                  }\n";
    let out = report(source, CONTEXT, true);
    assert_eq!(out.code, source, "left as written");
    assert_eq!(out.unsupported, vec![3], "one line, reported once");
}

#[test]
fn deeply_nested_macro_calls_stay_cheap() {
    let mut call = "x".to_owned();
    for _ in 0..14 {
        call = format!("cube({call})");
    }
    let source =
        format!("#define cube(v) ((v) * (v) * (v))\nfloat f(float x) {{ return {call} * 2; }}\n");
    let started = std::time::Instant::now();
    let out = rewrite(&source);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "3^14 typings are cut off by the expansion budget"
    );
    assert_eq!(out.lines().count(), source.lines().count(), "no line moves");
}
