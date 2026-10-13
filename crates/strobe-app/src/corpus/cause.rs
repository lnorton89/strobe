// ============================================================================
// Module:       strobe_app::corpus::cause
// Description:  Turns a loader, script or GLSL compiler message into a cause
//               category (undeclared:<ident>, no_function:<name>, ...) and
//               picks the first error line out of a long log.
//
// Dependencies: none
// ============================================================================

//! Cause categories.
//!
//! The report groups failures by *why*, not by the driver's wording, so
//! that "31 scenes fail on ANGLE" becomes "27 of them are the implicit
//! conversion family". The input is the message text only, because that is
//! all a record keeps; the one refinement that needs the shader,
//! [`categorise_in`], is optional.
//!
//! | Label | Meaning |
//! |---|---|
//! | `undeclared:<ident>` | the shader uses a name nothing declares |
//! | `no_function:<name>` | a function the shader or script calls does not exist |
//! | `unknown_control_type:<type>` | `scene.json` has a control `TYPE` Strobe does not define |
//! | `implicit_conversion` | the dialect will not convert `int` to `float` (or similar) |
//! | `json` | `scene.json` is not valid or not the shape SSF describes |
//! | `image_decode` | a picture the scene names would not decode |
//! | `preprocessor` | `#include`, `#if` on an unknown macro, `#extension`... |
//! | `other` | none of the above |
//!
//! ## Known limit
//!
//! GLSL ES and desktop drivers word a call that does not type-check the
//! same way whether the function is missing or only called with an `int`
//! where it wants a `float` ("no matching overloaded function found").
//! A built-in such as `clamp` or `mix` is always the conversion; a name the
//! shader does not define is a missing function; and a name the shader
//! *does* define is the conversion again, which only [`categorise_in`],
//! given the source, can tell.

/// The GLSL built-ins whose "no matching function" error means the argument
/// types did not convert, not that the function is absent.
const BUILTINS: &[&str] = &[
    "abs",
    "acos",
    "asin",
    "atan",
    "ceil",
    "clamp",
    "cos",
    "cross",
    "degrees",
    "distance",
    "dot",
    "exp",
    "exp2",
    "faceforward",
    "floor",
    "fract",
    "inversesqrt",
    "length",
    "log",
    "log2",
    "max",
    "min",
    "mix",
    "mod",
    "normalize",
    "pow",
    "radians",
    "reflect",
    "refract",
    "round",
    "sign",
    "sin",
    "smoothstep",
    "sqrt",
    "step",
    "tan",
    "trunc",
    "texture",
    "texelFetch",
    "textureLod",
    "textureSize",
    "transpose",
    "determinant",
    "inverse",
    "outerProduct",
    "matrixCompMult",
    "any",
    "all",
    "not",
    "lessThan",
    "greaterThan",
    "equal",
    "notEqual",
    "vec2",
    "vec3",
    "vec4",
    "mat2",
    "mat3",
    "mat4",
    "float",
    "int",
    "bool",
];

/// The longest error line a record keeps.
const MAX_LINE: usize = 240;

/// The label for `message`: one of the forms in the module docs.
pub fn categorise(message: &str) -> String {
    // The first error only: a driver log lists candidates and follow-on
    // errors after it, and a name quoted in a later line is not this
    // error's subject.
    let message = first_error_text(message);
    let lower = message.to_lowercase();

    if let Some(ty) = between(message, "unknown type `", "`") {
        return format!("unknown_control_type:{ty}");
    }
    if lower.contains("scene.json")
        || lower.contains("expected value at line")
        || lower.contains("missing field")
        || lower.contains("unknown field")
        || lower.contains("invalid type:")
        || lower.contains("key must be a string")
        || lower.contains("trailing characters")
        || lower.contains("eof while parsing")
    {
        return "json".to_string();
    }
    if lower.contains("decod")
        || lower.contains("image format")
        || lower.contains(".png")
        || lower.contains(".jpg")
        || lower.contains(".jpeg")
    {
        return "image_decode".to_string();
    }
    // A script's own failures: a callback that is not there, or a name the
    // engine cannot resolve.
    if let Some(name) = between(message, "there is no function named `", "`") {
        return format!("no_function:{name}");
    }
    for needle in [" is not a function", " is not defined"] {
        if let Some(at) = message.find(needle)
            && let Some(name) = identifier_ending_at(message, at)
        {
            return format!("no_function:{name}");
        }
    }
    if mentions_preprocessor(&lower) {
        return "preprocessor".to_string();
    }
    if mentions_conversion(&lower) {
        return "implicit_conversion".to_string();
    }
    if let Some(label) = missing_function(message, &lower) {
        return label;
    }
    if lower.contains("undeclared")
        || lower.contains("undefined variable")
        || lower.contains("undefined identifier")
        || lower.contains("not declared")
        || lower.contains("use of undeclared")
    {
        return match first_quoted_identifier(message) {
            Some(name) => format!("undeclared:{name}"),
            None => "undeclared:?".to_string(),
        };
    }
    "other".to_string()
}

/// [`categorise`], then refined with the shader's source: a
/// `no_function:<name>` for a function the source itself defines is the
/// conversion family, because the function exists and the call did not
/// match its types.
pub fn categorise_in(message: &str, source: &str) -> String {
    let label = categorise(message);
    match label.strip_prefix("no_function:") {
        Some(name) if defines_function(source, name) => "implicit_conversion".to_string(),
        Some(_) | None => label,
    }
}

/// The part of `label` before the `:`, the category without its subject.
pub fn kind(label: &str) -> &str {
    label.split_once(':').map_or(label, |(kind, _)| kind)
}

/// The first line of `message` that reports an error, trimmed and capped.
/// Falls back to the first non-empty line.
pub fn first_error_line(message: &str) -> String {
    let line = first_error_text(message);
    if line.chars().count() > MAX_LINE {
        let mut cut: String = line.chars().take(MAX_LINE).collect();
        cut.push('…');
        cut
    } else {
        line.to_string()
    }
}

/// The first line of `message` that reports an error, trimmed; the first
/// non-empty line when none says "error".
fn first_error_text(message: &str) -> &str {
    let lines = || {
        message
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
    };
    lines()
        .find(|line| line.to_lowercase().contains("error"))
        .or_else(|| lines().next())
        .unwrap_or("(no message)")
}

/// Whether the message is about the preprocessor.
fn mentions_preprocessor(lower: &str) -> bool {
    [
        "#include",
        "#if",
        "#ifdef",
        "#elif",
        "#error",
        "#extension",
        "#version",
        "#define",
        "#undef",
        "#line",
        "#pragma",
        "macro",
        "preprocessor",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

/// Whether the message is a type conversion refusal.
fn mentions_conversion(lower: &str) -> bool {
    [
        "implicit",
        "cannot convert",
        "can't convert",
        "wrong operand types",
        "no operation",
        "type mismatch",
        "cannot be assigned",
        "incompatible types",
        "mismatched types",
        "must have the same type",
        "could not implicitly",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

/// A call that matched no function: a built-in means the arguments did not
/// convert, anything else is a function that is not there.
fn missing_function(message: &str, lower: &str) -> Option<String> {
    let phrases = [
        "no matching overloaded function",
        "no matching function",
        "unable to find compatible overloaded function",
        "undeclared function",
        "undefined function",
        "function not found",
        "no function with name",
        "is not a function",
    ];
    if !phrases.iter().any(|phrase| lower.contains(phrase)) {
        return None;
    }
    match first_quoted_identifier(message) {
        Some(name) if BUILTINS.contains(&name.as_str()) => Some("implicit_conversion".to_string()),
        Some(name) => Some(format!("no_function:{name}")),
        None => Some("no_function:?".to_string()),
    }
}

/// The text of `text` between the first `open` and the next `close`.
fn between(text: &str, open: &str, close: &str) -> Option<String> {
    let start = text.find(open)? + open.len();
    let rest = text.get(start..)?;
    let end = rest.find(close)?;
    let found = rest.get(..end)?.trim();
    (!found.is_empty()).then(|| found.to_string())
}

/// The identifier that ends right before byte `end` of `text`.
fn identifier_ending_at(text: &str, end: usize) -> Option<String> {
    let head = text.get(..end)?;
    let name: String = head
        .chars()
        .rev()
        .take_while(|c| c.is_alphanumeric() || matches!(c, '_' | '.' | '$'))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    (!name.is_empty()).then_some(name)
}

/// The first identifier written in quotes in `text`: `'foo'` (ANGLE, Mesa),
/// `` `foo(float)' `` (Mesa) or `"foo"` (NVIDIA). Prose apostrophes do not
/// match, because an identifier must follow the quote and be closed by a
/// quote or a `(`.
fn first_quoted_identifier(text: &str) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if matches!(chars.get(i), Some('\'' | '`' | '"')) {
            let start = i + 1;
            let mut end = start;
            while chars
                .get(end)
                .is_some_and(|c| c.is_alphanumeric() || *c == '_')
            {
                end += 1;
            }
            let closes = matches!(chars.get(end), Some('\'' | '`' | '"' | '('));
            let starts_ok = chars
                .get(start)
                .is_some_and(|c| c.is_alphabetic() || *c == '_');
            if end > start && closes && starts_ok {
                return Some(chars.get(start..end)?.iter().collect());
            }
        }
        i += 1;
    }
    None
}

/// Whether `source` defines a function called `name`: the name, an opening
/// parenthesis, and a type word before it on the same line.
fn defines_function(source: &str, name: &str) -> bool {
    source.lines().any(|line| {
        let line = line.trim_start();
        let Some(at) = line.find(&format!("{name}(")) else {
            return false;
        };
        let head = line.get(..at).unwrap_or_default();
        // `float f(` or `vec3 f (` defines it; `x = f(` and `return f(` do
        // not, because their head does not end in a type word.
        let word = head.split_whitespace().last().unwrap_or_default();
        let before_is_boundary = head.is_empty() || head.ends_with(char::is_whitespace);
        before_is_boundary
            && !head.contains('=')
            && !matches!(word, "" | "return" | "else")
            && [
                "void", "float", "int", "uint", "bool", "vec2", "vec3", "vec4", "mat2", "mat3",
                "mat4", "ivec2", "ivec3", "ivec4", "bvec2", "bvec3", "bvec4",
            ]
            .contains(&word)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Message shapes written from the three drivers' documented formats,
    // not copied from any scene.

    #[test]
    fn mesa_undeclared_names_the_identifier() {
        assert_eq!(
            categorise("pass 0: 0:12(5): error: 'glowAmount' undeclared"),
            "undeclared:glowAmount",
            "Mesa GLSL"
        );
    }

    #[test]
    fn nvidia_undefined_variable_names_the_identifier() {
        assert_eq!(
            categorise("pass 1: 0(41) : error C1008: undefined variable \"tint\""),
            "undeclared:tint",
            "NVIDIA"
        );
    }

    #[test]
    fn angle_undeclared_identifier_names_the_identifier() {
        assert_eq!(
            categorise("pass 0: ERROR: 0:88: 'bassLevel' : undeclared identifier"),
            "undeclared:bassLevel",
            "ANGLE"
        );
    }

    #[test]
    fn a_builtin_that_matched_nothing_is_a_conversion_problem() {
        assert_eq!(
            categorise("ERROR: 0:30: 'clamp' : no matching overloaded function found"),
            "implicit_conversion",
            "ANGLE, clamp(x, 0, 1)"
        );
        assert_eq!(
            categorise(
                "0:30(12): error: no matching function for call to `mix(float, int, float)'"
            ),
            "implicit_conversion",
            "Mesa, mix with an int"
        );
        assert_eq!(
            categorise(
                "0(30) : error C1115: unable to find compatible overloaded function \"pow(float, int)\""
            ),
            "implicit_conversion",
            "NVIDIA, pow with an int"
        );
    }

    #[test]
    fn a_function_nothing_defines_is_a_missing_function() {
        assert_eq!(
            categorise("ERROR: 0:19: 'warpCoords' : no matching overloaded function found"),
            "no_function:warpCoords",
            "ANGLE"
        );
        assert_eq!(
            categorise("0:19(9): error: no matching function for call to `warpCoords(vec2)'"),
            "no_function:warpCoords",
            "Mesa"
        );
        assert_eq!(
            categorise(
                "0(19) : error C1115: unable to find compatible overloaded function \"warpCoords(float2)\""
            ),
            "no_function:warpCoords",
            "NVIDIA"
        );
    }

    #[test]
    fn a_function_the_shader_defines_is_a_conversion_problem() {
        let message = "ERROR: 0:19: 'warp' : no matching overloaded function found";
        assert_eq!(categorise(message), "no_function:warp", "message alone");
        let source = "float other(float x) { return x; }\nvec2 warp(vec2 p, float k) { return p * k; }\nvec4 renderMain() { return vec4(warp(_uv, 2)); }";
        assert_eq!(
            categorise_in(message, source),
            "implicit_conversion",
            "the source defines warp, so the call's types were wrong"
        );
        assert_eq!(
            categorise_in(message, "vec4 renderMain() { return vec4(warp(_uv, 2)); }"),
            "no_function:warp",
            "a call is not a definition"
        );
    }

    #[test]
    fn only_the_first_error_decides_the_subject() {
        let log = "pass 0: main.glsl:3: 1:3(1): warning: 'time' is shadowed\n\
                   main.glsl:20: 1:20(7): error: no matching function for call to \
                   `mixColours(float, int)'; candidates are:\n\
                   main.glsl:2: 1:2(1): vec3 mixColours(float, float)";
        assert_eq!(
            categorise(log),
            "no_function:mixColours",
            "not the `time` in an earlier warning"
        );
        assert_eq!(
            categorise_in(
                log,
                "vec3 mixColours(float a, float b) { return vec3(a, b, 0.0); }"
            ),
            "implicit_conversion",
            "the shader defines it, so the call's argument types were wrong"
        );
        assert_eq!(
            categorise("main.glsl:9: 1:9(4): error: no function with name 'palette'"),
            "no_function:palette",
            "Mesa's other wording"
        );
    }

    #[test]
    fn mesa_wordings_for_a_conversion_and_a_missing_include() {
        assert_eq!(
            categorise("main.glsl:77: 1:77(6): error: operands of `==' must have the same type"),
            "implicit_conversion",
            "float == int"
        );
        assert_eq!(
            categorise(
                "main.glsl:5: 1:5(7): error: initializer of type int cannot be assigned to variable of type float"
            ),
            "implicit_conversion",
            "float x = 1"
        );
        assert_eq!(
            categorise("main.glsl:4: 1:4(1): preprocessor error: include \"lib.glsl\" not found"),
            "preprocessor",
            "an #include the engine does not provide"
        );
    }

    #[test]
    fn implicit_conversions_in_operators_and_assignments() {
        for message in [
            "ERROR: 0:44: '=' : cannot convert from 'const int' to 'highp float'",
            "ERROR: 0:51: '*' : wrong operand types - no operation '*' exists that takes a left-hand operand of type 'highp float' and a right operand of type 'const int'",
            "0:44(17): error: could not implicitly convert operands to arithmetic operator",
            "0(44) : error C7011: implicit cast from \"int\" to \"float\"",
        ] {
            assert_eq!(categorise(message), "implicit_conversion", "{message}");
        }
    }

    #[test]
    fn preprocessor_problems_come_before_undeclared_names() {
        assert_eq!(
            categorise(
                "ERROR: 0:15: 'LOW_QUALITY' : undefined macro in expression not allowed in es 300"
            ),
            "preprocessor",
            "ANGLE, #if on a name that is not a macro"
        );
        assert_eq!(
            categorise("0:3(1): error: #include is not supported"),
            "preprocessor",
            "Mesa"
        );
    }

    #[test]
    fn loader_messages_have_their_own_categories() {
        assert_eq!(
            categorise("control `speed`: unknown type `traveler slider`"),
            "unknown_control_type:traveler slider",
            "the whole TYPE as written"
        );
        assert_eq!(
            categorise("scene.json: expected value at line 3 column 5"),
            "json",
            "serde"
        );
        assert_eq!(
            categorise("/x/a.synScene/img.png: Format error decoding Png: invalid signature"),
            "image_decode",
            "the image crate"
        );
        assert_eq!(
            categorise("script.js failed to load: SyntaxError: unexpected token"),
            "other",
            "a script that does not parse has no finer category"
        );
    }

    #[test]
    fn script_messages_name_the_missing_function() {
        assert_eq!(
            categorise("onChange(`speed`): there is no function named `doesNotExist`"),
            "no_function:doesNotExist",
            "a missing callback"
        );
        assert_eq!(
            categorise("update() failed: TypeError: syn_helper is not a function"),
            "no_function:syn_helper",
            "QuickJS"
        );
        assert_eq!(
            categorise("update() failed: ReferenceError: lerp is not defined"),
            "no_function:lerp",
            "QuickJS, an unresolved name"
        );
    }

    #[test]
    fn unrecognised_messages_are_other() {
        assert_eq!(categorise("GL error 0x502"), "other", "a GL error code");
        assert_eq!(categorise(""), "other", "nothing");
    }

    #[test]
    fn the_first_error_line_skips_the_noise_and_is_capped() {
        let log = "pass 0: shader log\n\n  0:1(1): warning: something\n  0:12(5): error: 'x' undeclared\n  0:13(5): error: 'y' undeclared";
        assert_eq!(
            first_error_line(log),
            "0:12(5): error: 'x' undeclared",
            "the first line that says error"
        );
        assert_eq!(first_error_line("only this"), "only this", "fallback");
        assert_eq!(first_error_line("  \n "), "(no message)", "blank");
        let long = format!("error: {}", "x".repeat(500));
        assert_eq!(
            first_error_line(&long).chars().count(),
            MAX_LINE + 1,
            "capped, plus the ellipsis"
        );
    }

    #[test]
    fn kind_drops_the_subject() {
        assert_eq!(kind("undeclared:foo"), "undeclared", "split");
        assert_eq!(kind("json"), "json", "whole");
    }
}
