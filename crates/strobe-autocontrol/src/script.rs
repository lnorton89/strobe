// ============================================================================
// Module:       strobe_autocontrol::script
// Description:  The uniforms a scene's script.js creates with setUniform,
//               found by a static scan, so a scripted scene compiles here
//               without running the script.
//
// Dependencies: strobe-scene (SceneFolder, names, ExtraType), crate::source
// ============================================================================

//! Script uniforms, found statically.
//!
//! A scripted scene's GLSL reads uniforms the script creates at load with
//! `setUniform("name", ...)` (or the older `uniforms.name = ...`). The app
//! learns them by running the script's first frame and passes them to
//! `ScenePlayer::with_extras`. This crate does not run scripts: it would
//! need `strobe-script` and QuickJS, and scoring renders fixed frames
//! anyway. Instead it reads `script.js` for those calls and declares what
//! it finds, so the scene compiles. Every such uniform stays at zero while
//! the engine measures, in the original and in the rewrite alike, so the
//! comparison is fair; the written scene keeps its script and runs it in
//! the app as before.
//!
//! **This is a static approximation**, documented as such (D259):
//!
//! - The type comes from the call's shape: two to four loose numbers, an
//!   array or object literal of that many components, or a
//!   `Float32Array([...])` give `vec2`..`vec4`; anything else (one number,
//!   a variable, a boolean) gives `float`. The first call for a name wins,
//!   as in the script runtime.
//! - A name built at run time (`"spectrum_" + i`, or a template string)
//!   gives a prefix; every identifier in the scene's GLSL that starts with
//!   it is declared, with that call's type.
//! - A name the scene already declares (a control, a pass target, an
//!   image, a global in its GLSL) is left alone, as the app does.

use crate::source::{Scan, Sources};
use std::collections::BTreeSet;
use strobe_scene::SceneFolder;
use strobe_scene::assemble::ExtraType;
use strobe_scene::names::{is_glsl_identifier, is_reserved};

/// One JavaScript token, as much as the scan needs.
#[derive(Debug, Clone, PartialEq)]
enum Js<'a> {
    /// An identifier or keyword.
    Ident(&'a str),
    /// A string literal's contents; `true` when it is a template with a
    /// `${...}` in it (its text is then the part before the first one).
    Str(&'a str, bool),
    /// A number.
    Number,
    /// One punctuation character.
    Punct(char),
}

/// Tokenizes enough JavaScript to find calls: comments are skipped,
/// strings are kept whole, everything else is an identifier, a number or
/// one character of punctuation. A regular expression literal is read as
/// punctuation, which is harmless for this purpose.
fn lex(source: &str) -> Vec<Js<'_>> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        if byte.is_ascii_whitespace() {
            at += 1;
        } else if byte == b'/' && bytes.get(at + 1) == Some(&b'/') {
            while at < bytes.len() && bytes[at] != b'\n' {
                at += 1;
            }
        } else if byte == b'/' && bytes.get(at + 1) == Some(&b'*') {
            at += 2;
            while at < bytes.len() && !(bytes[at] == b'*' && bytes.get(at + 1) == Some(&b'/')) {
                at += 1;
            }
            at += 2;
        } else if matches!(byte, b'"' | b'\'' | b'`') {
            let start = at + 1;
            let mut end = start;
            let mut template = false;
            let mut cut = None;
            while end < bytes.len() && bytes[end] != byte {
                if bytes[end] == b'\\' {
                    end += 1;
                } else if byte == b'`' && bytes[end] == b'$' && bytes.get(end + 1) == Some(&b'{') {
                    template = true;
                    cut.get_or_insert(end);
                }
                end += 1;
            }
            let text = source.get(start..cut.unwrap_or(end)).unwrap_or("");
            out.push(Js::Str(text, template));
            at = end + 1;
        } else if byte.is_ascii_alphabetic() || byte == b'_' || byte == b'$' {
            let start = at;
            while at < bytes.len()
                && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_' || bytes[at] == b'$')
            {
                at += 1;
            }
            out.push(Js::Ident(source.get(start..at).unwrap_or("")));
        } else if byte.is_ascii_digit()
            || (byte == b'.' && bytes.get(at + 1).is_some_and(u8::is_ascii_digit))
        {
            while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'.') {
                at += 1;
            }
            out.push(Js::Number);
        } else {
            let c = source
                .get(at..)
                .and_then(|s| s.chars().next())
                .unwrap_or('?');
            out.push(Js::Punct(c));
            at += c.len_utf8();
        }
    }
    out
}

/// What a `setUniform` names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UniformName {
    /// A literal name.
    Exact(String),
    /// The fixed start of a name built at run time.
    Prefix(String),
}

/// One uniform a script creates, as the scan reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// Its name, or the prefix of the names it stands for.
    pub name: UniformName,
    /// Its GLSL type, from the call's shape.
    pub ty: ExtraType,
}

/// The index of the bracket that closes the one at `open`.
fn closing(tokens: &[Js<'_>], open: usize) -> Option<usize> {
    let mut depth = 0_usize;
    for (index, token) in tokens.iter().enumerate().skip(open) {
        match token {
            Js::Punct('(' | '[' | '{') => depth += 1,
            Js::Punct(')' | ']' | '}') => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(index);
                }
            }
            Js::Punct(_) | Js::Ident(_) | Js::Str(..) | Js::Number => {}
        }
    }
    None
}

/// The top-level comma-separated items in `tokens[from..to]`.
fn items(tokens: &[Js<'_>], from: usize, to: usize) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut depth = 0_usize;
    let mut start = from;
    for index in from..to {
        match tokens.get(index) {
            Some(Js::Punct('(' | '[' | '{')) => depth += 1,
            Some(Js::Punct(')' | ']' | '}')) => depth = depth.saturating_sub(1),
            Some(Js::Punct(',')) if depth == 0 => {
                out.push((start, index));
                start = index + 1;
            }
            Some(Js::Punct(_) | Js::Ident(_) | Js::Str(..) | Js::Number) | None => {}
        }
    }
    if start < to {
        out.push((start, to));
    }
    out
}

/// How many components one value expression spans: an array or object
/// literal, or a typed array of one, counts its items; anything else is
/// one.
fn components(tokens: &[Js<'_>], from: usize, to: usize) -> usize {
    let mut at = from;
    // `new Float32Array([...])`
    if matches!(tokens.get(at), Some(Js::Ident("new")))
        && matches!(tokens.get(at + 1), Some(Js::Ident(_)))
        && matches!(tokens.get(at + 2), Some(Js::Punct('(')))
    {
        at += 3;
    }
    match tokens.get(at) {
        Some(Js::Punct('[' | '{')) => {
            let Some(close) = closing(tokens, at) else {
                return 1;
            };
            // The literal must be the whole value (`[1, 2].map(f)` is not).
            let whole = (close + 1..to).all(|j| tokens.get(j) == Some(&Js::Punct(')')));
            if whole {
                items(tokens, at + 1, close).len()
            } else {
                1
            }
        }
        Some(Js::Punct(_) | Js::Ident(_) | Js::Str(..) | Js::Number) | None => 1,
    }
}

/// The end of the expression starting at `from`: the first `;`, `,` or
/// unmatched closing bracket at its own depth.
fn expression_end(tokens: &[Js<'_>], from: usize) -> usize {
    let mut depth = 0_usize;
    for (index, token) in tokens.iter().enumerate().skip(from) {
        match token {
            Js::Punct('(' | '[' | '{') => depth += 1,
            Js::Punct(')' | ']' | '}') if depth == 0 => return index,
            Js::Punct(')' | ']' | '}') => depth -= 1,
            Js::Punct(';' | ',') if depth == 0 => return index,
            Js::Punct(_) | Js::Ident(_) | Js::Str(..) | Js::Number => {}
        }
    }
    tokens.len()
}

/// The type for a count of components, if one exists.
fn type_of(count: usize) -> Option<ExtraType> {
    match count {
        1 => Some(ExtraType::Float),
        2 => Some(ExtraType::Vec2),
        3 => Some(ExtraType::Vec3),
        4 => Some(ExtraType::Vec4),
        _ => None,
    }
}

/// A name argument starting at `at`: a plain string, or a string followed
/// by `+` (or a template string) for a name built at run time.
fn name_at(tokens: &[Js<'_>], at: usize, end: usize) -> Option<UniformName> {
    let Some(Js::Str(text, template)) = tokens.get(at) else {
        return None;
    };
    let text = (*text).to_string();
    if *template || at + 1 < end {
        return (!text.is_empty()).then_some(UniformName::Prefix(text));
    }
    Some(UniformName::Exact(text))
}

/// Every uniform `script` creates, in source order, first call per name.
pub fn scan(script: &str) -> Vec<Found> {
    let tokens = lex(script);
    let mut found: Vec<Found> = Vec::new();
    let push = |found: &mut Vec<Found>, item: Found| {
        if !found.iter().any(|f| f.name == item.name) {
            found.push(item);
        }
    };
    for (index, token) in tokens.iter().enumerate() {
        match token {
            // `setUniform("name", ...)`
            Js::Ident("setUniform") if matches!(tokens.get(index + 1), Some(Js::Punct('('))) => {
                let Some(close) = closing(&tokens, index + 1) else {
                    continue;
                };
                let args = items(&tokens, index + 2, close);
                let Some(((name_from, name_to), values)) = args.split_first() else {
                    continue;
                };
                let Some(name) = name_at(&tokens, *name_from, *name_to) else {
                    continue;
                };
                let count = match values {
                    [] => continue,
                    [(from, to)] => components(&tokens, *from, *to),
                    more => more.len(),
                };
                if let Some(ty) = type_of(count) {
                    push(&mut found, Found { name, ty });
                }
            }
            // `uniforms.name = ...` and `uniforms["name"] = ...`
            Js::Ident("uniforms")
                if index == 0 || tokens.get(index - 1) != Some(&Js::Punct('.')) =>
            {
                let (name, eq) = match (tokens.get(index + 1), tokens.get(index + 2)) {
                    (Some(Js::Punct('.')), Some(Js::Ident(field))) => {
                        (UniformName::Exact((*field).to_string()), index + 3)
                    }
                    (Some(Js::Punct('[')), Some(Js::Str(text, false)))
                        if tokens.get(index + 3) == Some(&Js::Punct(']')) =>
                    {
                        (UniformName::Exact((*text).to_string()), index + 4)
                    }
                    _ => continue,
                };
                let assigns = tokens.get(eq) == Some(&Js::Punct('='))
                    && tokens.get(eq + 1) != Some(&Js::Punct('='));
                if !assigns {
                    continue;
                }
                let end = expression_end(&tokens, eq + 1);
                if let Some(ty) = type_of(components(&tokens, eq + 1, end)) {
                    push(&mut found, Found { name, ty });
                }
            }
            Js::Ident(_) | Js::Str(..) | Js::Number | Js::Punct(_) => {}
        }
    }
    found
}

/// GLSL type words that, written before an identifier at global scope,
/// declare it.
const DECLARING: [&str; 16] = [
    "float",
    "int",
    "uint",
    "bool",
    "vec2",
    "vec3",
    "vec4",
    "ivec2",
    "ivec3",
    "ivec4",
    "mat2",
    "mat3",
    "mat4",
    "sampler2D",
    "void",
    "struct",
];

/// The identifiers `sources` already declares at global scope (variables,
/// uniforms and functions), and every identifier they mention.
fn glsl_names(sources: &Sources) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut declared = BTreeSet::new();
    let mut mentioned = BTreeSet::new();
    for (path, text) in sources.iter() {
        let scan = Scan::new(path, text);
        for i in 0..scan.len() {
            if !scan.is_any_ident(i) {
                continue;
            }
            let name = scan.t(i);
            mentioned.insert(name.to_string());
            if i > 0 && scan.brace_depth(i) == 0 && DECLARING.contains(&scan.t(i - 1)) {
                declared.insert(name.to_string());
            }
        }
    }
    (declared, mentioned)
}

/// The uniforms to declare for `folder`'s script, with their types, in
/// the order the script creates them. Empty without a script.
pub fn uniforms(folder: &SceneFolder) -> Vec<(String, ExtraType)> {
    let Some(script) = folder.script.as_deref() else {
        return Vec::new();
    };
    let found = scan(script);
    if found.is_empty() {
        return Vec::new();
    }
    let def = &folder.def;
    let mut taken: BTreeSet<String> = BTreeSet::new();
    taken.extend(def.controls.iter().map(|c| c.name.clone()));
    taken.extend(def.passes.iter().filter_map(|p| p.target.clone()));
    taken.extend(def.images.iter().map(|i| i.name.clone()));
    taken.extend(def.cubemaps.iter().map(|c| c.name.clone()));
    taken.extend(def.transition_uniforms.iter().map(|t| t.name.clone()));
    let (declared, mentioned) = glsl_names(&Sources::from_folder(folder));
    taken.extend(declared);

    let mut out: Vec<(String, ExtraType)> = Vec::new();
    let add = |name: &str, ty: ExtraType, out: &mut Vec<(String, ExtraType)>| {
        let usable = is_glsl_identifier(name)
            && !is_reserved(name)
            && !taken.contains(name)
            && !out.iter().any(|(n, _)| n == name);
        if usable {
            out.push((name.to_string(), ty));
        }
    };
    for item in found {
        match &item.name {
            UniformName::Exact(name) => add(name, item.ty, &mut out),
            UniformName::Prefix(prefix) => {
                for name in mentioned
                    .iter()
                    .filter(|n| n.len() > prefix.len() && n.starts_with(prefix.as_str()))
                {
                    add(name, item.ty, &mut out);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use strobe_scene::scene::SceneDef;

    fn types(script: &str) -> Vec<(UniformName, ExtraType)> {
        scan(script).into_iter().map(|f| (f.name, f.ty)).collect()
    }

    fn exact(name: &str, ty: ExtraType) -> (UniformName, ExtraType) {
        (UniformName::Exact(name.to_string()), ty)
    }

    #[test]
    fn the_type_follows_the_shape_of_the_call() {
        let got = types(
            "function update(dt) {\n\
             setUniform('script_time', TIME * 0.5);\n\
             setUniform(\"itScript\", 0.1, 0.2);\n\
             setUniform('rgb', [0.1, 0.2, 0.3]);\n\
             setUniform('quad', {x: 1, y: 2, z: 3, w: 4});\n\
             setUniform('typed', new Float32Array([0.5, 0.25]));\n\
             setUniform('flag', true);\n\
             }",
        );
        assert_eq!(
            got,
            vec![
                exact("script_time", ExtraType::Float),
                exact("itScript", ExtraType::Vec2),
                exact("rgb", ExtraType::Vec3),
                exact("quad", ExtraType::Vec4),
                exact("typed", ExtraType::Vec2),
                exact("flag", ExtraType::Float),
            ],
            "numbers, arrays, objects and typed arrays"
        );
    }

    #[test]
    fn computed_names_give_a_prefix_and_the_old_uniforms_object_counts() {
        let got = types(
            "for (var i = 0; i < 4; i++) { setUniform('spectrum_' + i, s[i]); }\n\
             setUniform(`band${i}`, 1, 2, 3);\n\
             uniforms.beat_time = t;\n\
             uniforms['pulse'] = [1, 2];\n\
             if (uniforms.x == 2) {}\n",
        );
        assert_eq!(
            got,
            vec![
                (
                    UniformName::Prefix("spectrum_".to_string()),
                    ExtraType::Float
                ),
                (UniformName::Prefix("band".to_string()), ExtraType::Vec3),
                exact("beat_time", ExtraType::Float),
                exact("pulse", ExtraType::Vec2),
            ],
            "a comparison is not an assignment"
        );
    }

    #[test]
    fn comments_strings_and_bad_calls_are_ignored() {
        let got = types(
            "// setUniform('commented', 1);\n\
             /* setUniform('blocked', 1); */\n\
             print(\"setUniform('quoted', 1)\");\n\
             setUniform('none');\n\
             setUniform('five', 1, 2, 3, 4, 5);\n\
             setUniform(name, 1);\n\
             setUniform('kept', 1); setUniform('kept', 1, 2);\n",
        );
        assert_eq!(
            got,
            vec![exact("kept", ExtraType::Float)],
            "only real calls, and the first type for a name"
        );
    }

    #[test]
    fn declared_names_are_left_to_the_scene() -> Result<()> {
        let folder = SceneFolder {
            dir: std::path::PathBuf::from("."),
            def: SceneDef::parse(
                r#"{"CONTROLS": [{"NAME": "amount", "TYPE": "slider"}]}"#,
                "t",
            )?,
            shader: "uniform float mine;\nfloat rot(float a) { return a; }\n\
                     vec4 renderMain() { return vec4(spectrum_0 + spectrum_1 + script_time + mine + rot(1.0)); }\n"
                .to_string(),
            pass_shaders: Vec::new(),
            script: Some(
                "setUniform('script_time', 1); setUniform('amount', 1);\n\
                 setUniform('mine', 1); setUniform('rot', 1); setUniform('TIME', 1);\n\
                 for (var i = 0; i < 2; i++) setUniform('spectrum_' + i, 0);\n"
                    .to_string(),
            ),
        };
        let names: Vec<String> = uniforms(&folder).into_iter().map(|(n, _)| n).collect();
        assert_eq!(
            names,
            vec!["script_time", "spectrum_0", "spectrum_1"],
            "a control, a uniform and a function of the scene, and a reserved name, are skipped"
        );
        Ok(())
    }
}
