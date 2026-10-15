// ============================================================================
// Module:       strobe_scene::directives
// Description:  Preprocessor conditionals on names that are not macros: the
//               name reads as 0, as NVIDIA's desktop compiler and the C
//               preprocessor read it, on every driver.
//
// Dependencies: std only
// ============================================================================

//! `#if NAME == 0` where `NAME` is not a macro.
//!
//! Real scenes write `#if TUBE_SHAPE == 0` where `TUBE_SHAPE` is a plain
//! `float` global (TORNADO T5: two scenes, five conditionals). The
//! preprocessor runs before the compiler knows about globals, so the name
//! is simply undefined there. NVIDIA's compiler reads an undefined name as
//! `0`, as the C preprocessor does, so on Synesthesia's usual machines the
//! first branch is always taken. GLSL ES (ANGLE) and strict drivers reject
//! the directive instead. [`undefined_names_to_zero`] writes the `0` out, so
//! every driver takes the branch NVIDIA takes.
//!
//! Only `#if` and `#elif` are touched. A name counts as defined when a
//! `#define` for it comes earlier, in the preamble or the user's code, and
//! no `#undef` since; the operand of `defined` is never replaced. Names that
//! start with `GL_` or `__` are the driver's own and are left for it. Lines
//! never move.

use std::borrow::Cow;
use std::collections::HashSet;

/// Rewrites, in every `#if`/`#elif` of `source`, each identifier that is
/// not a macro at that point as `0`. `preamble` is the text compiled before
/// `source`; its `#define`s count as defined.
pub fn undefined_names_to_zero<'a>(source: &'a str, preamble: &str) -> Cow<'a, str> {
    let mut defined: HashSet<&str> = HashSet::new();
    for line in preamble.lines() {
        track(line, &mut defined);
    }
    let mut out: Option<String> = None;
    let mut offset = 0;
    for line in source.split_inclusive('\n') {
        let rewritten = condition(line, &defined);
        track(line, &mut defined);
        match (rewritten, &mut out) {
            (Some(new), Some(text)) => text.push_str(&new),
            (Some(new), None) => {
                let mut text = String::with_capacity(source.len() + 16);
                text.push_str(source.get(..offset).unwrap_or_default());
                text.push_str(&new);
                out = Some(text);
            }
            (None, Some(text)) => text.push_str(line),
            (None, None) => {}
        }
        offset += line.len();
    }
    out.map_or(Cow::Borrowed(source), Cow::Owned)
}

/// The directive keyword of `line` and the text after it, when `line` is a
/// preprocessor line.
fn directive(line: &str) -> Option<(&str, &str)> {
    let rest = line.trim_start().strip_prefix('#')?.trim_start();
    let keyword_len = rest
        .bytes()
        .position(|b| !b.is_ascii_alphabetic())
        .unwrap_or(rest.len());
    Some((rest.get(..keyword_len)?, rest.get(keyword_len..)?))
}

/// Follows `#define` and `#undef`.
fn track<'a>(line: &'a str, defined: &mut HashSet<&'a str>) {
    let Some((keyword, rest)) = directive(line) else {
        return;
    };
    let name = first_identifier(rest);
    match (keyword, name) {
        ("define", Some(name)) => {
            defined.insert(name);
        }
        ("undef", Some(name)) => {
            defined.remove(name);
        }
        (_, Some(_) | None) => {}
    }
}

/// The identifier at the start of `text`, after spaces.
fn first_identifier(text: &str) -> Option<&str> {
    let text = text.trim_start();
    let len = text
        .bytes()
        .position(|b| !(b.is_ascii_alphanumeric() || b == b'_'))
        .unwrap_or(text.len());
    text.get(..len).filter(|name| {
        name.bytes()
            .next()
            .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
    })
}

/// The rewritten line, when `line` is an `#if` or `#elif` naming something
/// undefined.
fn condition(line: &str, defined: &HashSet<&str>) -> Option<String> {
    let (keyword, _) = directive(line)?;
    if !matches!(keyword, "if" | "elif") {
        return None;
    }
    let hash = line.find('#')?;
    let keyword_at = line.get(hash..)?.find(keyword)? + hash;
    let expression_at = keyword_at + keyword.len();
    let head = line.get(..expression_at)?;
    let expression = line.get(expression_at..)?;
    let mut out = String::with_capacity(line.len());
    out.push_str(head);
    let mut changed = false;
    let mut after_defined = false;
    let bytes = expression.as_bytes();
    let mut at = 0;
    while let Some(&byte) = bytes.get(at) {
        if byte == b'/' && bytes.get(at + 1) == Some(&b'/') {
            out.push_str(expression.get(at..).unwrap_or_default());
            break;
        }
        if byte.is_ascii_digit() {
            let end = word_end(bytes, at);
            out.push_str(expression.get(at..end).unwrap_or_default());
            at = end;
            continue;
        }
        if byte.is_ascii_alphabetic() || byte == b'_' {
            let end = word_end(bytes, at);
            let name = expression.get(at..end).unwrap_or_default();
            let keep = after_defined
                || name == "defined"
                || name.starts_with("GL_")
                || name.starts_with("__")
                || defined.contains(name);
            after_defined = name == "defined";
            if keep {
                out.push_str(name);
            } else {
                out.push('0');
                changed = true;
            }
            at = end;
            continue;
        }
        if !matches!(byte, b' ' | b'\t' | b'(') {
            after_defined = false;
        }
        let end = at
            + expression
                .get(at..)
                .and_then(|s| s.chars().next())
                .map_or(1, char::len_utf8);
        out.push_str(expression.get(at..end).unwrap_or_default());
        at = end;
    }
    changed.then_some(out)
}

/// The end of the identifier or number at `from`.
fn word_end(bytes: &[u8], from: usize) -> usize {
    let mut at = from;
    while bytes
        .get(at)
        .is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_')
    {
        at += 1;
    }
    at
}

/// The state of the preprocessor's conditionals while a source is read line
/// by line: which macros are defined and whether the current line is in a
/// branch the preprocessor keeps.
///
/// The source rewrites read code the preprocessor has not run on yet, and
/// two branches of one `#if` group often declare the same thing twice (a
/// `#ifdef KODELIFE` block redeclaring the controls, in real scenes). They
/// follow this to read only the branch that will be compiled. A condition
/// is decided when it is a number, a comparison of two numbers, `defined`,
/// `#ifdef`, `#ifndef` or a negation of one; an undecided `#if` keeps its
/// first branch.
#[derive(Clone, Debug, Default)]
pub struct Branches {
    groups: Vec<Group>,
    known: HashSet<String>,
}

/// One open `#if` group.
#[derive(Clone, Copy, Debug)]
struct Group {
    /// The current branch is kept.
    active: bool,
    /// A branch of this group has been kept already.
    taken: bool,
}

impl Branches {
    /// Starts with `defined` as the macros.
    pub fn new(defined: impl IntoIterator<Item = String>) -> Self {
        Self {
            groups: Vec::new(),
            known: defined.into_iter().collect(),
        }
    }

    /// The state at the start of a program: `GL_ES` defined on ES.
    pub fn dialect(es: bool) -> Self {
        Self::new(es.then(|| "GL_ES".to_owned()))
    }

    /// The state at the start of the user's code on an ES context: `GL_ES`
    /// and the preamble's macros defined.
    pub fn for_es(preamble: &str) -> Self {
        Self::for_dialect(true, preamble)
    }

    /// The state at the start of the user's code: `GL_ES` on ES only, and
    /// the preamble's macros defined.
    pub fn for_dialect(es: bool, preamble: &str) -> Self {
        let mut branches = Self::dialect(es);
        for line in preamble.lines() {
            if let Some(rest) = line.trim_start().strip_prefix('#') {
                branches.follow(rest);
            }
        }
        branches
    }

    /// Whether the current line is in a kept branch.
    pub fn active(&self) -> bool {
        self.groups.iter().all(|group| group.active)
    }

    /// Follows one directive, given as the text after its `#`. Returns
    /// `true` when it was a conditional.
    pub fn follow(&mut self, directive: &str) -> bool {
        let line = directive.trim_start();
        let keyword_len = line
            .bytes()
            .position(|b| !b.is_ascii_alphabetic())
            .unwrap_or(line.len());
        let keyword = line.get(..keyword_len).unwrap_or_default();
        let rest = line.get(keyword_len..).unwrap_or_default();
        match keyword {
            "if" | "ifdef" | "ifndef" => {
                let parent = self.active();
                let taken = match keyword {
                    "ifdef" => self.defines(rest),
                    "ifndef" => !self.defines(rest),
                    _ => self.evaluate(rest).unwrap_or(true),
                };
                self.groups.push(Group {
                    active: parent && taken,
                    taken,
                });
                true
            }
            "elif" | "else" => {
                let Some(group) = self.groups.pop() else {
                    return true;
                };
                let parent = self.active();
                let take =
                    !group.taken && (keyword == "else" || self.evaluate(rest).unwrap_or(false));
                self.groups.push(Group {
                    active: parent && take,
                    taken: group.taken || take,
                });
                true
            }
            "endif" => {
                self.groups.pop();
                true
            }
            "define" | "undef" => {
                if self.active()
                    && let Some(name) = first_identifier(rest)
                {
                    if keyword == "define" {
                        self.known.insert(name.to_owned());
                    } else {
                        self.known.remove(name);
                    }
                }
                false
            }
            _ => false,
        }
    }

    /// Whether the name at the start of `rest` is a macro.
    fn defines(&self, rest: &str) -> bool {
        first_identifier(rest).is_some_and(|name| self.known.contains(name))
    }

    /// The value of a condition, when it can be decided.
    fn evaluate(&self, condition: &str) -> Option<bool> {
        let condition = condition
            .find("//")
            .map_or(condition, |cut| condition.get(..cut).unwrap_or(condition))
            .trim();
        if let Some(negated) = condition.strip_prefix('!') {
            return self.evaluate(negated).map(|value| !value);
        }
        if let Some(value) = number(condition) {
            return Some(value != 0);
        }
        if let Some(operand) = condition.strip_prefix("defined") {
            let operand = operand.trim();
            let name = operand
                .strip_prefix('(')
                .and_then(|inner| inner.strip_suffix(')'))
                .unwrap_or(operand)
                .trim();
            let simple =
                !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
            return simple.then(|| self.known.contains(name));
        }
        comparison(condition)
    }
}

/// An integer literal, possibly in parentheses.
fn number(text: &str) -> Option<i64> {
    let text = text.trim();
    let text = text
        .strip_prefix('(')
        .and_then(|inner| inner.strip_suffix(')'))
        .unwrap_or(text)
        .trim();
    text.parse().ok()
}

/// `a OP b` for two integer literals.
fn comparison(condition: &str) -> Option<bool> {
    for op in ["==", "!=", "<=", ">=", "<", ">"] {
        if let Some((left, right)) = condition.split_once(op) {
            let (a, b) = (number(left)?, number(right)?);
            return Some(match op {
                "==" => a == b,
                "!=" => a != b,
                "<=" => a <= b,
                ">=" => a >= b,
                "<" => a < b,
                _ => a > b,
            });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_undefined_name_in_a_condition_reads_as_zero() {
        let source =
            "float SHAPE = 2.0;\n#if SHAPE == 4\nA\n#elif TUBE >= 1 && defined(X)\nB\n#endif\n";
        assert_eq!(
            undefined_names_to_zero(source, ""),
            "float SHAPE = 2.0;\n#if 0 == 4\nA\n#elif 0 >= 1 && defined(X)\nB\n#endif\n",
            "globals are not macros; `defined`'s operand is never replaced"
        );
    }

    #[test]
    fn macros_from_the_preamble_and_earlier_lines_are_kept() {
        let source = "#define MODE 2\n#if MODE == 2 && LIB > 0 && GL_ES\n#endif\n#undef MODE\n#if MODE\n#endif\n";
        assert_eq!(
            undefined_names_to_zero(source, "#define LIB 1\n"),
            "#define MODE 2\n#if MODE == 2 && LIB > 0 && GL_ES\n#endif\n#undef MODE\n#if 0\n#endif\n",
            "defined names stay, the driver's own stay, an #undef counts"
        );
    }

    #[test]
    fn a_source_without_such_conditions_is_borrowed() {
        let source = "#ifdef FOO\n#endif\n#if defined FOO\n#endif\nfloat x = 1.0;\n";
        assert!(
            matches!(undefined_names_to_zero(source, ""), Cow::Borrowed(_)),
            "#ifdef and `defined FOO` need nothing"
        );
    }
}

#[cfg(test)]
mod branch_tests {
    use super::*;

    fn kept(source: &str, mut branches: Branches) -> Vec<&str> {
        let mut out = Vec::new();
        for line in source.lines() {
            if let Some(rest) = line.trim_start().strip_prefix('#') {
                branches.follow(rest);
            } else if branches.active() {
                out.push(line);
            }
        }
        out
    }

    #[test]
    fn decided_conditions_follow_the_preprocessor() {
        let source = "#ifdef KODELIFE\na\n#else\nb\n#endif\n#if 0 == 4\nc\n#elif defined(GL_ES)\nd\n#endif\n#define M\n#ifndef M\ne\n#endif\n";
        assert_eq!(
            kept(source, Branches::for_es("#define texture2D texture\n")),
            vec!["b", "d"],
            "KODELIFE is not defined, GL_ES is on ES, M is defined by then"
        );
    }

    #[test]
    fn an_undecided_if_keeps_its_first_branch_only() {
        let source = "#if FOO > 1\nf\n#else\ng\n#endif\n";
        assert_eq!(
            kept(source, Branches::default()),
            vec!["f"],
            "the first branch"
        );
    }
}
