// ============================================================================
// Module:       strobe_scene::promote
// Description:  The type-aware int→float rewrite GLSL ES needs: finds every
//               place desktop GLSL converts an int to a float implicitly and
//               writes the conversion out.
//
// Dependencies: std::collections::HashMap
// ============================================================================

//! Implicit int→float conversion, made explicit (Q8, D199).
//!
//! Desktop GLSL converts `int` to `float` wherever a float is expected:
//! `x * 2`, `max(0, y)`, `float f = 1;`, `return 0;` in a float function.
//! GLSL ES never does, and real Synesthesia scenes lean on it everywhere: on
//! the owner's 90-scene corpus, 65 scenes failed on the ES dialect for this
//! one reason (D195). This module finds those places and writes the
//! conversion out, so the ANGLE path compiles what the native path does.
//!
//! ## How
//!
//! [`int_to_float`] lexes the user's code ([`lex`]), collects every
//! declaration it can see (the generated preamble and the SSF library are
//! passed in as context, so uniforms and library overloads are known), then
//! walks each function body and global initializer with a small expression
//! parser that types every subexpression ([`types`]). Where an integer
//! expression meets a float — an operand of `+ - * /`, a comparison, an
//! assignment, an initializer, a `return`, a ternary branch, the argument
//! of a float built-in or of a user function whose only matching overload
//! takes a float — it is converted:
//!
//! - a literal becomes a float literal: `2` → `2.0`. A literal-only
//!   expression built with `+ - *` and unary minus is converted literal by
//!   literal, which is exact.
//! - anything else is wrapped: `i` → `float(i)`, an `ivec2` → `vec2(...)`.
//!   An integer division is wrapped, not rewritten, so `1/2` stays `0` as it
//!   is on desktop.
//!
//! ## Function-like macros (ES only, D261)
//!
//! On the ES dialect a call to a function-like macro is typed through its
//! expansion ([`expand`]): `#define S(a, b, t) smoothstep(a, b, t)` called
//! as `S(0, 1., x)` needs `0.0`, and the edit lands in the argument at the
//! call site. When the expansion needs an edit in the macro's body, or two
//! uses of one parameter disagree, that one call is replaced by its
//! expansion with the conversions written in, on the same line. Desktop
//! output never changes for a macro: desktop GLSL converts on its own.
//!
//! ## A sampler chosen with `?:` (every dialect, D261)
//!
//! GLSL forbids a sampler as an operand of `?:`; NVIDIA accepts it, Mesa
//! and ANGLE do not. `texture(c ? a : b, uv)` is rewritten as
//! `(c ? texture(a, uv) : texture(b, uv))` for any built-in or user
//! function ([`select`]), which keeps the meaning on every driver. A
//! sampler `?:` anywhere else is returned in [`Promoted::unsupported`].
//!
//! ## What it will not do
//!
//! It never edits against a type it does not know. A desktop macro with
//! arguments, a name it cannot resolve, or code it cannot parse makes the
//! surrounding expression [`types::Ty::Unknown`], and a function body it
//! cannot parse keeps its text unchanged. A missed conversion leaves a
//! scene failing as it did; a wrong one could break a scene that compiled.
//! Edits never add or remove a line, so driver errors still map to the
//! right line of `main.glsl`.
//!
//! It runs on every dialect (D199): strict desktop compilers (Mesa,
//! Apple's) reject some of the same implicit forms, such as an `int`
//! returned from a `float` function.

mod expand;
mod lex;
mod select;
mod types;

use crate::directives::Branches;
use lex::{Define, Kind, Token, lex, lex_with};
use std::borrow::Cow;
use std::cell::Cell;
use std::collections::HashMap;
use types::{Builtin, Scalar, Ty, builtin_function, builtin_type, is_qualifier};

/// The rewritten user code and what the rewrite could not handle.
#[derive(Debug)]
pub struct Promoted<'a> {
    /// The user's code with every conversion written out, or the input
    /// itself when nothing needed converting.
    pub code: Cow<'a, str>,
    /// 1-based lines of the user's code where a sampler is an operand of
    /// `?:` somewhere other than a function argument. GLSL ES rejects that
    /// form and the rewrite does not attempt it.
    pub unsupported: Vec<u32>,
}

/// Rewrites `user` so every implicit int→float conversion is explicit.
/// `context` is the source that precedes it in the program (the generated
/// preamble and the SSF library): its declarations are known, and it is
/// never edited. `es` says whether the driver defines `GL_ES`. Returns
/// `user` unchanged when nothing needs converting.
pub fn int_to_float<'a>(user: &'a str, context: &str, es: bool) -> Cow<'a, str> {
    rewrite(user, context, es).code
}

/// [`int_to_float`], plus the places it could not make valid for GLSL ES.
/// With `es`, calls to function-like macros are typed through their
/// expansions as well (D261).
pub fn rewrite<'a>(user: &'a str, context: &str, es: bool) -> Promoted<'a> {
    let mut branches = Branches::dialect(es);
    let context_lexed = lex_with(context, &mut branches);
    let user_lexed = lex_with(user, &mut branches);
    let mut symbols = Symbols {
        expand: es,
        ..Symbols::default()
    };
    for (name, ty) in [
        ("gl_FragCoord", Ty::vector(Scalar::Float, 4)),
        ("gl_PointCoord", Ty::vector(Scalar::Float, 2)),
        ("gl_FragDepth", Ty::FLOAT),
        ("gl_FrontFacing", Ty::BOOL),
    ] {
        symbols.globals.insert(name.to_owned(), ty);
    }
    for define in context_lexed.defines.iter().chain(&user_lexed.defines) {
        symbols.add_macro(define);
    }
    for name in context_lexed.undefs.iter().chain(&user_lexed.undefs) {
        symbols.undefine(name);
    }
    let context_source = Source {
        text: context,
        tokens: &context_lexed.tokens,
    };
    let user_source = Source {
        text: user,
        tokens: &user_lexed.tokens,
    };
    collect(&context_source, &mut symbols);
    collect(&user_source, &mut symbols);
    let (edits, selects) = infer(&user_source, &symbols);
    let mut unsupported: Vec<u32> = selects
        .iter()
        .filter_map(|&offset| {
            let before = user.get(..offset)?;
            u32::try_from(before.matches('\n').count() + 1).ok()
        })
        .collect();
    unsupported.sort_unstable();
    unsupported.dedup();
    let code = if edits.is_empty() {
        Cow::Borrowed(user)
    } else {
        Cow::Owned(apply(user, edits))
    };
    Promoted { code, unsupported }
}

/// Text and its tokens.
#[derive(Clone, Copy)]
struct Source<'s> {
    text: &'s str,
    tokens: &'s [Token],
}

impl<'s> Source<'s> {
    /// The text of token `index`.
    fn text(&self, index: usize) -> Option<&'s str> {
        let token = self.tokens.get(index)?;
        self.text.get(token.start..token.end)
    }

    /// The identifier at `index`.
    fn word(&self, index: usize) -> Option<&'s str> {
        let token = self.tokens.get(index)?;
        (token.kind == Kind::Ident)
            .then(|| self.text.get(token.start..token.end))
            .flatten()
    }

    /// `true` when token `index` is the punctuation `punct`.
    fn is(&self, index: usize, punct: &str) -> bool {
        self.tokens
            .get(index)
            .is_some_and(|t| t.kind == Kind::Punct)
            && self.text(index) == Some(punct)
    }

    /// The index after the bracket that closes the one at `open`.
    fn skip_balanced(&self, open: usize) -> usize {
        let mut depth = 0_usize;
        let mut at = open;
        while at < self.tokens.len() {
            match self.text(at) {
                Some("(" | "[" | "{")
                    if self.tokens.get(at).map(|t| t.kind) == Some(Kind::Punct) =>
                {
                    depth += 1;
                }
                Some(")" | "]" | "}")
                    if self.tokens.get(at).map(|t| t.kind) == Some(Kind::Punct) =>
                {
                    depth = depth.saturating_sub(1);
                    if depth == 0 {
                        return at + 1;
                    }
                }
                Some(_) | None => {}
            }
            at += 1;
        }
        self.tokens.len()
    }

    /// The index after the `;` that ends the statement at `from`, or after
    /// the `}` of a braced block that ends it.
    fn skip_statement(&self, from: usize) -> usize {
        let mut at = from;
        while at < self.tokens.len() {
            if self.is(at, ";") {
                return at + 1;
            }
            if self.is(at, "(") || self.is(at, "[") {
                at = self.skip_balanced(at);
                continue;
            }
            if self.is(at, "{") {
                return self.skip_balanced(at);
            }
            at += 1;
        }
        self.tokens.len().max(from + 1)
    }

    /// The index of the first `,` or `;` at bracket depth 0 from `from`.
    fn declarator_end(&self, from: usize) -> usize {
        let mut at = from;
        while at < self.tokens.len() {
            if self.is(at, ",") || self.is(at, ";") {
                return at;
            }
            if self.is(at, "(") || self.is(at, "[") || self.is(at, "{") {
                at = self.skip_balanced(at);
                continue;
            }
            at += 1;
        }
        self.tokens.len()
    }
}

/// A function's signature.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Sig {
    ret: Ty,
    /// Each parameter's type, and whether it is `out` or `inout` (an
    /// argument that cannot be converted).
    params: Vec<(Ty, bool)>,
}

/// What a macro name stands for.
#[derive(Clone, Debug)]
enum Macro {
    /// An object-like macro: its replacement text, typed on use.
    Object(String),
    /// A function-like macro: its definition, when the walker can follow
    /// it (see [`expand`]); `None` when it cannot.
    Function(Option<FnMacro>),
}

/// A function-like macro the walker can expand.
#[derive(Clone, Debug)]
struct FnMacro {
    params: Vec<String>,
    body: String,
}

/// Everything declared at global scope.
#[derive(Default)]
struct Symbols {
    globals: HashMap<String, Ty>,
    functions: HashMap<String, Vec<Sig>>,
    structs: HashMap<String, Vec<(String, Ty)>>,
    macros: HashMap<String, Macro>,
    /// Whether calls to function-like macros are typed through their
    /// expansions: the ES dialect only.
    expand: bool,
    /// How many macro calls have been expanded so far, against a budget
    /// that keeps nested macros from costing exponential time.
    expansions: Cell<usize>,
}

impl Symbols {
    fn add_macro(&mut self, define: &Define<'_>) {
        let entry = if define.function_like {
            // A macro defined twice (or `#undef`ed, below) is not followed:
            // which definition a call sees depends on where it is. Nor is a
            // body that stringizes or pastes.
            let followable = !self.macros.contains_key(define.name)
                && !define.body.contains('#')
                && define.params.iter().all(|p| is_identifier(p));
            Macro::Function(followable.then(|| FnMacro {
                params: define.params.iter().map(|p| (*p).to_owned()).collect(),
                body: define.body.to_owned(),
            }))
        } else {
            Macro::Object(define.body.to_owned())
        };
        self.macros.insert(define.name.to_owned(), entry);
    }

    /// An `#undef`: a function-like macro of that name is no longer
    /// followed, as a call may come before or after it.
    fn undefine(&mut self, name: &str) {
        if let Some(entry @ Macro::Function(Some(_))) = self.macros.get_mut(name) {
            *entry = Macro::Function(None);
        }
    }

    fn add_function(&mut self, name: &str, sig: Sig) {
        let overloads = self.functions.entry(name.to_owned()).or_default();
        if !overloads.contains(&sig) {
            overloads.push(sig);
        }
    }

    /// The type a type name stands for: built-in or a struct.
    fn type_named(&self, name: &str) -> Option<Ty> {
        builtin_type(name).or_else(|| {
            self.structs
                .contains_key(name)
                .then(|| Ty::Struct(name.to_owned()))
        })
    }

    /// The type at `index` (with an array suffix), and the index after it.
    fn type_at(&self, source: &Source<'_>, index: usize) -> Option<(Ty, usize)> {
        let ty = self.type_named(source.word(index)?)?;
        if source.is(index + 1, "[") {
            return Some((Ty::Array(Box::new(ty)), source.skip_balanced(index + 1)));
        }
        Some((ty, index + 1))
    }
}

/// One top-level declaration.
enum Item {
    Struct {
        name: String,
        fields: Vec<(String, Ty)>,
        instances: Vec<String>,
    },
    Vars(Vec<Var>),
    Function {
        name: String,
        sig: Sig,
        names: Vec<Option<String>>,
        /// The token range inside the braces.
        body: Option<(usize, usize)>,
    },
    Skip,
}

/// One declared variable and its initializer's token range.
struct Var {
    name: String,
    ty: Ty,
    init: Option<(usize, usize)>,
}

/// Reads the top-level item at `index`.
fn next_item(source: &Source<'_>, symbols: &Symbols, index: usize) -> (Item, usize) {
    if source.is(index, ";") {
        return (Item::Skip, index + 1);
    }
    let skip = || (Item::Skip, source.skip_statement(index));
    let mut at = index;
    loop {
        match source.word(at) {
            Some("layout") if source.is(at + 1, "(") => at = source.skip_balanced(at + 1),
            Some("precision") => return skip(),
            Some(word) if is_qualifier(word) => at += 1,
            Some(_) | None => break,
        }
    }
    if source.word(at) == Some("struct") {
        return parse_struct(source, symbols, at).unwrap_or_else(skip);
    }
    let Some((ty, after_type)) = symbols.type_at(source, at) else {
        return skip();
    };
    let Some(name) = source.word(after_type) else {
        return skip();
    };
    if source.is(after_type + 1, "(") {
        return parse_function(source, symbols, ty, name, after_type + 1).unwrap_or_else(skip);
    }
    parse_vars(source, ty, after_type).unwrap_or_else(skip)
}

fn parse_struct(source: &Source<'_>, symbols: &Symbols, at: usize) -> Option<(Item, usize)> {
    let name = source.word(at + 1)?;
    if !source.is(at + 2, "{") {
        return None;
    }
    let close = source.skip_balanced(at + 2).checked_sub(1)?;
    let mut fields = Vec::new();
    let mut cursor = at + 3;
    while cursor < close {
        while source.word(cursor).is_some_and(is_qualifier) {
            cursor += 1;
        }
        let (ty, mut next) = symbols.type_at(source, cursor)?;
        loop {
            let field = source.word(next)?;
            next += 1;
            let mut field_ty = ty.clone();
            if source.is(next, "[") {
                next = source.skip_balanced(next);
                field_ty = Ty::Array(Box::new(field_ty));
            }
            fields.push((field.to_owned(), field_ty));
            if source.is(next, ",") {
                next += 1;
                continue;
            }
            if source.is(next, ";") {
                next += 1;
                break;
            }
            return None;
        }
        cursor = next;
    }
    let mut instances = Vec::new();
    let mut next = close + 1;
    while let Some(instance) = source.word(next) {
        instances.push(instance.to_owned());
        next += 1;
        if source.is(next, ",") {
            next += 1;
        }
    }
    if !source.is(next, ";") {
        return None;
    }
    Some((
        Item::Struct {
            name: name.to_owned(),
            fields,
            instances,
        },
        next + 1,
    ))
}

fn parse_function(
    source: &Source<'_>,
    symbols: &Symbols,
    ret: Ty,
    name: &str,
    open: usize,
) -> Option<(Item, usize)> {
    let mut params = Vec::new();
    let mut names = Vec::new();
    let mut at = open + 1;
    if source.word(at) == Some("void") && source.is(at + 1, ")") {
        at += 1;
    }
    while !source.is(at, ")") {
        let mut out = false;
        while let Some(word) = source.word(at).filter(|w| is_qualifier(w)) {
            out |= matches!(word, "out" | "inout");
            at += 1;
        }
        let (mut ty, next) = symbols.type_at(source, at)?;
        at = next;
        let mut param_name = None;
        if let Some(word) = source.word(at) {
            param_name = Some(word.to_owned());
            at += 1;
            if source.is(at, "[") {
                at = source.skip_balanced(at);
                ty = Ty::Array(Box::new(ty));
            }
        }
        params.push((ty, out));
        names.push(param_name);
        if source.is(at, ",") {
            at += 1;
        } else if !source.is(at, ")") {
            return None;
        }
    }
    let after = at + 1;
    let sig = Sig { ret, params };
    let name = name.to_owned();
    if source.is(after, ";") {
        let item = Item::Function {
            name,
            sig,
            names,
            body: None,
        };
        return Some((item, after + 1));
    }
    if !source.is(after, "{") {
        return None;
    }
    let end = source.skip_balanced(after);
    let item = Item::Function {
        name,
        sig,
        names,
        body: Some((after + 1, end.saturating_sub(1))),
    };
    Some((item, end))
}

fn parse_vars(source: &Source<'_>, ty: Ty, mut at: usize) -> Option<(Item, usize)> {
    let mut vars = Vec::new();
    loop {
        let name = source.word(at)?;
        at += 1;
        let mut var_ty = ty.clone();
        if source.is(at, "[") {
            at = source.skip_balanced(at);
            var_ty = Ty::Array(Box::new(var_ty));
        }
        let mut init = None;
        if source.is(at, "=") {
            let end = source.declarator_end(at + 1);
            init = Some((at + 1, end));
            at = end;
        }
        vars.push(Var {
            name: name.to_owned(),
            ty: var_ty,
            init,
        });
        if source.is(at, ",") {
            at += 1;
            continue;
        }
        if source.is(at, ";") {
            return Some((Item::Vars(vars), at + 1));
        }
        return None;
    }
}

/// Records every global declaration in `source`.
fn collect(source: &Source<'_>, symbols: &mut Symbols) {
    let mut at = 0;
    while at < source.tokens.len() {
        let (item, next) = next_item(source, symbols, at);
        match item {
            Item::Struct {
                name,
                fields,
                instances,
            } => {
                for instance in instances {
                    symbols.globals.insert(instance, Ty::Struct(name.clone()));
                }
                symbols.structs.insert(name, fields);
            }
            Item::Vars(vars) => {
                for var in vars {
                    symbols.globals.insert(var.name, var.ty);
                }
            }
            Item::Function { name, sig, .. } => symbols.add_function(&name, sig),
            Item::Skip => {}
        }
        at = next.max(at + 1);
    }
}

/// `true` for a GLSL identifier.
fn is_identifier(text: &str) -> bool {
    text.bytes()
        .next()
        .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
        && text.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Every conversion `source` needs, and the byte offsets of the sampler
/// `?:` it could not rewrite.
fn infer(source: &Source<'_>, symbols: &Symbols) -> (Vec<Edit>, Vec<usize>) {
    let mut edits = Vec::new();
    let mut selects = Vec::new();
    let mut at = 0;
    while at < source.tokens.len() {
        let (item, next) = next_item(source, symbols, at);
        match item {
            Item::Vars(vars) => {
                for var in vars {
                    let Some((start, end)) = var.init else {
                        continue;
                    };
                    let mut walker = Walker::new(*source, symbols, start, end);
                    let parsed = walker.assignment().and_then(|init| {
                        walker.finished()?;
                        walker.coerce_to(init, &var.ty);
                        Ok(())
                    });
                    if parsed.is_ok() {
                        edits.append(&mut walker.edits);
                        selects.extend(walker.select_offsets());
                    }
                }
            }
            Item::Function {
                sig,
                names,
                body: Some((start, end)),
                ..
            } => {
                let mut walker = Walker::new(*source, symbols, start, end);
                walker.ret = sig.ret.clone();
                let mut scope = HashMap::new();
                for ((ty, _), name) in sig.params.iter().zip(names) {
                    if let Some(name) = name {
                        scope.insert(name, ty.clone());
                    }
                }
                walker.scopes.push(scope);
                if walker.statements().is_ok() {
                    edits.append(&mut walker.edits);
                    selects.extend(walker.select_offsets());
                }
            }
            Item::Function { body: None, .. } | Item::Struct { .. } | Item::Skip => {}
        }
        at = next.max(at + 1);
    }
    (edits, selects)
}

/// [`Edit::order`] of text that closes something (`)`, `.0`).
const CLOSE: u8 = 0;
/// [`Edit::order`] of text that opens something (`float(`).
const OPEN: u8 = 1;
/// [`Edit::order`] of a replacement of `at..end`.
const REPLACE: u8 = 2;

/// One change to the user's text: an insertion, or the replacement of
/// whole tokens.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Edit {
    /// Byte offset.
    at: usize,
    /// [`CLOSE`], [`OPEN`] or [`REPLACE`]: at one offset, closers come
    /// first, then openers, then a replacement, so they nest.
    order: u8,
    /// End of the replaced bytes; `at` for an insertion.
    end: usize,
    text: String,
}

impl Edit {
    /// Inserts `text` at `at`.
    fn insert(at: usize, order: u8, text: String) -> Self {
        Self {
            at,
            order,
            end: at,
            text,
        }
    }

    /// Replaces bytes `at..end`, which cover whole tokens, with `text`.
    fn replace(at: usize, end: usize, text: String) -> Self {
        Self {
            at,
            order: REPLACE,
            end,
            text,
        }
    }
}

/// `edits` in the order they apply, duplicates removed.
fn sorted(mut edits: Vec<Edit>) -> Vec<Edit> {
    edits.sort_by_key(|edit| (edit.at, edit.order));
    edits.dedup();
    edits
}

/// Applies `edits` to `text`.
fn apply(text: &str, edits: Vec<Edit>) -> String {
    let edits = sorted(edits);
    let extra: usize = edits.iter().map(|edit| edit.text.len()).sum();
    let mut out = String::with_capacity(text.len() + extra);
    let mut copied = 0;
    for edit in edits {
        // An edit inside a replaced range is part of the replacement.
        if edit.at < copied {
            continue;
        }
        if let Some(chunk) = text.get(copied..edit.at) {
            out.push_str(chunk);
            copied = edit.at;
        }
        out.push_str(&edit.text);
        if edit.order == REPLACE {
            copied = edit.end.max(copied);
        }
    }
    out.push_str(text.get(copied..).unwrap_or_default());
    out
}

/// A parse the walker could not follow. The enclosing function keeps its
/// text unchanged.
#[derive(Debug)]
struct Stop;

type Parsed<T> = Result<T, Stop>;

/// A typed subexpression.
#[derive(Clone, Debug)]
struct Expr {
    ty: Ty,
    /// First token.
    start: usize,
    /// One past the last token.
    end: usize,
    /// For an expression made only of integer literals with `+ - *`, unary
    /// minus and parentheses: the literals' token indices. Converting each
    /// one is exact; anything else is wrapped.
    literals: Option<Vec<usize>>,
}

/// Binary operators by binding power, loosest first.
const BINARY: [&[&str]; 10] = [
    &["||"],
    &["^^"],
    &["&&"],
    &["|"],
    &["^"],
    &["&"],
    &["==", "!="],
    &["<", ">", "<=", ">="],
    &["<<", ">>"],
    &["+", "-"],
];

/// `*`, `/` and `%` bind tighter than every operator in [`BINARY`].
const MULTIPLICATIVE: [&str; 3] = ["*", "/", "%"];

const ASSIGNMENT: [&str; 11] = [
    "=", "+=", "-=", "*=", "/=", "%=", "<<=", ">>=", "&=", "^=", "|=",
];

/// The audio textures the documentation calls 1D samplers, which the ES
/// preamble declares as 2D textures one texel tall.
const ONE_DIMENSIONAL: [&str; 2] = ["syn_Spectrum", "syn_LevelTrail"];

/// How deep expressions and macro expansions may nest before the walker
/// gives up.
const MAX_DEPTH: u32 = 200;

/// Types the statements and expressions in one token range.
struct Walker<'s> {
    source: Source<'s>,
    symbols: &'s Symbols,
    at: usize,
    end: usize,
    scopes: Vec<HashMap<String, Ty>>,
    edits: Vec<Edit>,
    ret: Ty,
    depth: u32,
    /// The function-like macros being expanded around this walk, which
    /// the preprocessor does not expand again inside it.
    expanding: Vec<String>,
    /// Token ranges of the sampler `?:` not rewritten (yet) by [`select`].
    selects: Vec<(usize, usize)>,
}

impl<'s> Walker<'s> {
    fn new(source: Source<'s>, symbols: &'s Symbols, start: usize, end: usize) -> Self {
        Self {
            source,
            symbols,
            at: start,
            end,
            scopes: Vec::new(),
            edits: Vec::new(),
            ret: Ty::Void,
            depth: 0,
            expanding: Vec::new(),
            selects: Vec::new(),
        }
    }

    /// The byte offsets of the sampler `?:` left unrewritten.
    fn select_offsets(&self) -> Vec<usize> {
        self.selects
            .iter()
            .filter_map(|&(start, _)| self.source.tokens.get(start).map(|t| t.start))
            .collect()
    }

    fn word(&self) -> Option<&'s str> {
        (self.at < self.end)
            .then(|| self.source.word(self.at))
            .flatten()
    }

    fn word_at(&self, index: usize) -> Option<&'s str> {
        (index < self.end)
            .then(|| self.source.word(index))
            .flatten()
    }

    fn is(&self, punct: &str) -> bool {
        self.at < self.end && self.source.is(self.at, punct)
    }

    fn is_at(&self, index: usize, punct: &str) -> bool {
        index < self.end && self.source.is(index, punct)
    }

    fn eat(&mut self, punct: &str) -> bool {
        let found = self.is(punct);
        if found {
            self.at += 1;
        }
        found
    }

    fn expect(&mut self, punct: &str) -> Parsed<()> {
        if self.eat(punct) { Ok(()) } else { Err(Stop) }
    }

    fn eat_word(&mut self, word: &str) -> bool {
        let found = self.word() == Some(word);
        if found {
            self.at += 1;
        }
        found
    }

    fn finished(&self) -> Parsed<()> {
        if self.at == self.end {
            Ok(())
        } else {
            Err(Stop)
        }
    }

    fn deeper(&mut self) -> Parsed<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            Err(Stop)
        } else {
            Ok(())
        }
    }

    // ---- statements ------------------------------------------------------

    /// Statements until the end of the range.
    fn statements(&mut self) -> Parsed<()> {
        while self.at < self.end {
            self.statement()?;
        }
        Ok(())
    }

    fn statement(&mut self) -> Parsed<()> {
        if self.eat("{") {
            self.scopes.push(HashMap::new());
            while !self.is("}") {
                if self.at >= self.end {
                    return Err(Stop);
                }
                self.statement()?;
            }
            self.at += 1;
            self.scopes.pop();
            return Ok(());
        }
        if self.eat(";") {
            return Ok(());
        }
        match self.word() {
            Some("if") => {
                self.at += 1;
                self.condition()?;
                self.statement()?;
                if self.eat_word("else") {
                    self.statement()?;
                }
                Ok(())
            }
            Some("for") => self.for_loop(),
            Some("while") => {
                self.at += 1;
                self.condition()?;
                self.statement()
            }
            Some("do") => {
                self.at += 1;
                self.statement()?;
                if !self.eat_word("while") {
                    return Err(Stop);
                }
                self.condition()?;
                self.expect(";")
            }
            Some("return") => {
                self.at += 1;
                if !self.eat(";") {
                    let value = self.expression()?;
                    let ret = self.ret.clone();
                    self.coerce_to(value, &ret);
                    self.expect(";")?;
                }
                Ok(())
            }
            Some("break" | "continue" | "discard") => {
                self.at += 1;
                self.expect(";")
            }
            Some("switch") => self.switch(),
            Some("precision") => {
                self.at = self.source.skip_statement(self.at).min(self.end);
                Ok(())
            }
            Some("struct") => Err(Stop),
            Some(_) | None => {
                if self.declaration_starts() {
                    self.declaration()
                } else {
                    self.expression()?;
                    self.expect(";")
                }
            }
        }
    }

    fn condition(&mut self) -> Parsed<()> {
        self.expect("(")?;
        self.expression()?;
        self.expect(")")
    }

    fn for_loop(&mut self) -> Parsed<()> {
        self.at += 1;
        self.expect("(")?;
        self.scopes.push(HashMap::new());
        if !self.eat(";") {
            if self.declaration_starts() {
                self.declaration()?;
            } else {
                self.expression()?;
                self.expect(";")?;
            }
        }
        if !self.eat(";") {
            self.expression()?;
            self.expect(";")?;
        }
        if !self.is(")") {
            self.expression()?;
        }
        self.expect(")")?;
        self.statement()?;
        self.scopes.pop();
        Ok(())
    }

    fn switch(&mut self) -> Parsed<()> {
        self.at += 1;
        self.condition()?;
        self.expect("{")?;
        self.scopes.push(HashMap::new());
        while !self.eat("}") {
            if self.at >= self.end {
                return Err(Stop);
            }
            if self.eat_word("case") {
                self.ternary()?;
                self.expect(":")?;
            } else if self.eat_word("default") {
                self.expect(":")?;
            } else {
                self.statement()?;
            }
        }
        self.scopes.pop();
        Ok(())
    }

    /// `true` when the statement here declares variables.
    fn declaration_starts(&self) -> bool {
        let mut at = self.at;
        let mut qualified = false;
        while self.word_at(at).is_some_and(is_qualifier) {
            at += 1;
            qualified = true;
        }
        let Some(name) = self.word_at(at) else {
            return false;
        };
        if self.symbols.type_named(name).is_none() {
            return false;
        }
        let mut next = at + 1;
        if self.is_at(next, "[") {
            next = self.source.skip_balanced(next);
        }
        qualified || self.word_at(next).is_some()
    }

    fn declaration(&mut self) -> Parsed<()> {
        while self.word().is_some_and(is_qualifier) {
            self.at += 1;
        }
        let (ty, next) = self.symbols.type_at(&self.source, self.at).ok_or(Stop)?;
        self.at = next;
        loop {
            let name = self.word().ok_or(Stop)?;
            self.at += 1;
            let mut var_ty = ty.clone();
            if self.is("[") {
                self.at = self.source.skip_balanced(self.at);
                var_ty = Ty::Array(Box::new(var_ty));
            }
            if self.eat("=") {
                let init = self.assignment()?;
                self.coerce_to(init, &var_ty);
            }
            if let Some(scope) = self.scopes.last_mut() {
                scope.insert(name.to_owned(), var_ty);
            }
            if self.eat(",") {
                continue;
            }
            return self.expect(";");
        }
    }

    // ---- expressions -----------------------------------------------------

    /// `a, b, c`: the type is the last one's.
    fn expression(&mut self) -> Parsed<Expr> {
        let mut value = self.assignment()?;
        while self.eat(",") {
            let start = value.start;
            value = self.assignment()?;
            value.start = start;
            value.literals = None;
        }
        Ok(value)
    }

    fn assignment(&mut self) -> Parsed<Expr> {
        let target = self.ternary()?;
        let Some(op) = ASSIGNMENT.iter().find(|op| self.is(op)) else {
            return Ok(target);
        };
        self.at += 1;
        let value = self.assignment()?;
        let end = value.end;
        match *op {
            "=" => {
                self.coerce_to(value, &target.ty);
            }
            "+=" | "-=" | "*=" | "/=" if target.ty.is_float() => {
                self.coerce_float(value);
            }
            _ => {}
        }
        Ok(Expr {
            ty: target.ty,
            start: target.start,
            end,
            literals: None,
        })
    }

    fn ternary(&mut self) -> Parsed<Expr> {
        let condition = self.binary(0)?;
        if !self.eat("?") {
            return Ok(condition);
        }
        let yes = self.assignment()?;
        self.expect(":")?;
        let no = self.assignment()?;
        let (yes, no) = self.balance(yes, no);
        let ty = if yes.ty == no.ty { yes.ty } else { Ty::Unknown };
        if matches!(ty, Ty::Sampler(_)) {
            self.selects.push((condition.start, no.end));
        }
        let literals = match (yes.literals, no.literals) {
            (Some(mut a), Some(b)) => {
                a.extend(b);
                Some(a)
            }
            (Some(_) | None, _) => None,
        };
        Ok(Expr {
            ty,
            start: condition.start,
            end: no.end,
            literals,
        })
    }

    /// When one side is a float and the other an integer, converts the
    /// integer side.
    fn balance(&mut self, left: Expr, right: Expr) -> (Expr, Expr) {
        if left.ty.is_float() && right.ty.is_integer() && fits(&right.ty, &left.ty) {
            let right = self.coerce_float(right);
            (left, right)
        } else if right.ty.is_float() && left.ty.is_integer() && fits(&left.ty, &right.ty) {
            let left = self.coerce_float(left);
            (left, right)
        } else {
            (left, right)
        }
    }

    fn binary(&mut self, level: usize) -> Parsed<Expr> {
        let Some(ops) = BINARY.get(level) else {
            return self.multiplicative();
        };
        let mut left = self.binary(level + 1)?;
        while let Some(op) = ops.iter().find(|op| self.is(op)) {
            self.at += 1;
            let right = self.binary(level + 1)?;
            left = self.combine(op, left, right);
        }
        Ok(left)
    }

    fn multiplicative(&mut self) -> Parsed<Expr> {
        let mut left = self.unary()?;
        while let Some(op) = MULTIPLICATIVE.iter().find(|op| self.is(op)) {
            self.at += 1;
            let right = self.unary()?;
            left = self.combine(op, left, right);
        }
        Ok(left)
    }

    /// Types `left op right`, converting an integer side where desktop GLSL
    /// would.
    fn combine(&mut self, op: &str, left: Expr, right: Expr) -> Expr {
        let start = left.start;
        let end = right.end;
        let converts = matches!(
            op,
            "+" | "-" | "*" | "/" | "<" | ">" | "<=" | ">=" | "==" | "!="
        );
        let (left, right) = if converts {
            self.balance(left, right)
        } else {
            (left, right)
        };
        let ty = match op {
            "+" | "-" | "*" | "/" => arithmetic(op, &left.ty, &right.ty),
            "%" | "<<" | ">>" | "&" | "|" | "^" => integer_op(&left.ty, &right.ty),
            _ => Ty::BOOL,
        };
        let literals = match (op, left.literals, right.literals) {
            ("+" | "-" | "*", Some(mut a), Some(b)) => {
                a.extend(b);
                Some(a)
            }
            (_, Some(_) | None, _) => None,
        };
        Expr {
            ty,
            start,
            end,
            literals,
        }
    }

    fn unary(&mut self) -> Parsed<Expr> {
        self.deeper()?;
        let start = self.at;
        let result = if self.eat("-") || self.eat("+") {
            let mut operand = self.unary()?;
            operand.start = start;
            Ok(operand)
        } else if self.eat("!") {
            let operand = self.unary()?;
            Ok(Expr {
                ty: Ty::BOOL,
                start,
                end: operand.end,
                literals: None,
            })
        } else if self.eat("~") || self.eat("++") || self.eat("--") {
            let operand = self.unary()?;
            Ok(Expr {
                ty: operand.ty,
                start,
                end: operand.end,
                literals: None,
            })
        } else {
            self.postfix()
        };
        self.depth -= 1;
        result
    }

    fn postfix(&mut self) -> Parsed<Expr> {
        let mut value = self.primary()?;
        loop {
            if self.eat("[") {
                self.expression()?;
                self.expect("]")?;
                value.ty = element(&value.ty);
            } else if self.eat(".") {
                let field = self.word().ok_or(Stop)?;
                self.at += 1;
                if self.eat("(") {
                    self.expect(")")?;
                    value.ty = Ty::INT;
                } else {
                    value.ty = self.field(&value.ty, field);
                }
            } else if self.eat("++") || self.eat("--") {
            } else {
                return Ok(value);
            }
            value.end = self.at;
            value.literals = None;
        }
    }

    fn field(&self, ty: &Ty, field: &str) -> Ty {
        if let Ty::Struct(name) = ty {
            return self
                .symbols
                .structs
                .get(name)
                .and_then(|fields| fields.iter().find(|(f, _)| f == field))
                .map_or(Ty::Unknown, |(_, ty)| ty.clone());
        }
        swizzle(ty, field)
    }

    fn primary(&mut self) -> Parsed<Expr> {
        let start = self.at;
        if start >= self.end {
            return Err(Stop);
        }
        let token = self.source.tokens.get(start).copied().ok_or(Stop)?;
        let single = |ty: Ty, literals: Option<Vec<usize>>| Expr {
            ty,
            start,
            end: start + 1,
            literals,
        };
        match token.kind {
            Kind::Int => {
                self.at += 1;
                let unsigned = self
                    .source
                    .text(start)
                    .is_some_and(|text| text.ends_with(['u', 'U']));
                let ty = if unsigned {
                    Ty::scalar(Scalar::Uint)
                } else {
                    Ty::INT
                };
                return Ok(single(ty, Some(vec![start])));
            }
            Kind::Float => {
                self.at += 1;
                return Ok(single(Ty::FLOAT, None));
            }
            Kind::Punct => {
                if self.eat("(") {
                    let inner = self.expression()?;
                    self.expect(")")?;
                    return Ok(Expr {
                        ty: inner.ty,
                        start,
                        end: self.at,
                        literals: inner.literals,
                    });
                }
                return Err(Stop);
            }
            Kind::Ident => {}
        }
        let name = self.word().ok_or(Stop)?;
        self.at += 1;
        if matches!(name, "true" | "false") {
            return Ok(single(Ty::BOOL, None));
        }
        if let Some(ty) = self.symbols.type_named(name) {
            return self.constructor(start, ty);
        }
        if self.is("(") {
            return self.call(start, name);
        }
        let ty = self.variable(name, 0);
        Ok(single(ty, None))
    }

    /// `T(...)` and `T[n](...)`.
    fn constructor(&mut self, start: usize, ty: Ty) -> Parsed<Expr> {
        if self.is("[") {
            self.at = self.source.skip_balanced(self.at);
            let args = self.arguments()?;
            for arg in args {
                self.coerce_to(arg, &ty);
            }
            return Ok(Expr {
                ty: Ty::Array(Box::new(ty)),
                start,
                end: self.at,
                literals: None,
            });
        }
        let args = self.arguments()?;
        if let Ty::Struct(name) = &ty
            && let Some(fields) = self.symbols.structs.get(name)
            && fields.len() == args.len()
        {
            for (arg, (_, field_ty)) in args.into_iter().zip(fields) {
                self.coerce_to(arg, field_ty);
            }
        }
        Ok(Expr {
            ty,
            start,
            end: self.at,
            literals: None,
        })
    }

    /// `(a, b, ...)` after a callee.
    fn arguments(&mut self) -> Parsed<Vec<Expr>> {
        self.expect("(")?;
        let mut args = Vec::new();
        if self.eat_word("void") || self.eat(")") {
            if self.at > 0 && self.source.is(self.at - 1, ")") {
                return Ok(args);
            }
            self.expect(")")?;
            return Ok(args);
        }
        loop {
            args.push(self.assignment()?);
            if self.eat(",") {
                continue;
            }
            self.expect(")")?;
            return Ok(args);
        }
    }

    fn call(&mut self, start: usize, name: &str) -> Parsed<Expr> {
        if let Some(expanded) = self.expand_macro(start, name) {
            return Ok(expanded);
        }
        let mark = self.edits.len();
        let resolved = self.callee(name, 0);
        let args = self.arguments()?;
        let end = self.at;
        let samplers: Vec<(usize, usize)> = args
            .iter()
            .filter(|arg| matches!(arg.ty, Ty::Sampler(_)))
            .map(|arg| (arg.start, arg.end))
            .collect();
        let ty = match resolved {
            Callee::Opaque => Ty::Unknown,
            Callee::Named(name) => {
                let ty = if let Some(rule) = builtin_function(&name) {
                    self.one_dimensional(rule, &args);
                    self.builtin(rule, args)
                } else if let Some(overloads) = self.symbols.functions.get(&name) {
                    self.user_call(overloads, args)
                } else {
                    Ty::Unknown
                };
                self.distribute(start, end, &samplers, mark);
                ty
            }
        };
        Ok(Expr {
            ty,
            start,
            end,
            literals: None,
        })
    }

    /// The function a called name reaches through object-like macros.
    fn callee(&self, name: &str, depth: u32) -> Callee {
        match self.symbols.macros.get(name) {
            None => Callee::Named(name.to_owned()),
            // Inside its own expansion a macro's name is not expanded again.
            Some(Macro::Function(_)) if self.expanding.iter().any(|n| n == name) => {
                Callee::Named(name.to_owned())
            }
            Some(Macro::Function(_)) => Callee::Opaque,
            Some(Macro::Object(body)) => {
                let body = body.trim();
                let is_name = !body.is_empty()
                    && body.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_');
                if is_name && depth < MAX_DEPTH && body != name {
                    self.callee(body, depth + 1)
                } else {
                    Callee::Opaque
                }
            }
        }
    }

    fn builtin(&mut self, rule: Builtin, args: Vec<Expr>) -> Ty {
        match rule {
            Builtin::FloatWise | Builtin::FloatScalar | Builtin::Cross => {
                let args: Vec<Expr> = args.into_iter().map(|a| self.coerce_float(a)).collect();
                match rule {
                    Builtin::FloatScalar => Ty::FLOAT,
                    Builtin::Cross => Ty::vector(Scalar::Float, 3),
                    Builtin::FloatWise
                    | Builtin::MixedWise
                    | Builtin::Texture(_)
                    | Builtin::Fetch
                    | Builtin::TextureSize
                    | Builtin::BoolScalar
                    | Builtin::BoolWise => widest(&args),
                }
            }
            Builtin::MixedWise => {
                let any_float = args.iter().any(|a| a.ty.is_float());
                let args: Vec<Expr> = if any_float {
                    args.into_iter().map(|a| self.coerce_float(a)).collect()
                } else {
                    args
                };
                widest(&args)
            }
            Builtin::Texture(scalar) => {
                if let Some(index) = scalar {
                    for (position, arg) in args.into_iter().enumerate() {
                        if position == index && arg.ty.is_scalar() {
                            self.coerce_float(arg);
                        }
                    }
                }
                Ty::vector(Scalar::Float, 4)
            }
            Builtin::Fetch => Ty::vector(Scalar::Float, 4),
            Builtin::TextureSize => match args.first().map(|a| &a.ty) {
                Some(Ty::Sampler(1)) => Ty::INT,
                Some(Ty::Sampler(n)) => Ty::vector(Scalar::Int, *n),
                Some(_) | None => Ty::Unknown,
            },
            Builtin::BoolScalar => Ty::BOOL,
            Builtin::BoolWise => args
                .first()
                .map_or(Ty::Unknown, |a| a.ty.with_kind(Scalar::Bool)),
        }
    }

    /// Resolves a call to a user or library function the way desktop GLSL
    /// does: an exact match first, otherwise the one overload reachable
    /// with the fewest int→float conversions.
    fn user_call(&mut self, overloads: &[Sig], args: Vec<Expr>) -> Ty {
        let mut best: Vec<(&Sig, Vec<usize>)> = Vec::new();
        let mut fewest = usize::MAX;
        for sig in overloads.iter().filter(|s| s.params.len() == args.len()) {
            let Some(conversions) = conversions(sig, &args) else {
                continue;
            };
            match conversions.len().cmp(&fewest) {
                std::cmp::Ordering::Less => {
                    fewest = conversions.len();
                    best = vec![(sig, conversions)];
                }
                std::cmp::Ordering::Equal => best.push((sig, conversions)),
                std::cmp::Ordering::Greater => {}
            }
        }
        match best.as_slice() {
            [(sig, conversions)] => {
                let ret = sig.ret.clone();
                let conversions = conversions.clone();
                for (index, arg) in args.into_iter().enumerate() {
                    if conversions.contains(&index) {
                        self.coerce_float(arg);
                    }
                }
                ret
            }
            [(first, _), rest @ ..] if rest.iter().all(|(sig, _)| sig.ret == first.ret) => {
                first.ret.clone()
            }
            [_, ..] | [] => Ty::Unknown,
        }
    }

    /// The type of a variable or object-like macro.
    fn variable(&self, name: &str, depth: u32) -> Ty {
        if let Some(entry) = self.symbols.macros.get(name) {
            return match entry {
                Macro::Function(_) => Ty::Unknown,
                Macro::Object(body) => self.macro_type(name, body, depth),
            };
        }
        for scope in self.scopes.iter().rev() {
            if let Some(ty) = scope.get(name) {
                return ty.clone();
            }
        }
        self.symbols
            .globals
            .get(name)
            .cloned()
            .unwrap_or(Ty::Unknown)
    }

    /// Types an object-like macro's replacement as an expression.
    fn macro_type(&self, name: &str, body: &str, depth: u32) -> Ty {
        if depth >= 8 || body.is_empty() {
            return Ty::Unknown;
        }
        let lexed = lex(body);
        let source = Source {
            text: body,
            tokens: &lexed.tokens,
        };
        // A macro naming itself is left to the preprocessor.
        if lexed.tokens.len() == 1 && source.word(0) == Some(name) {
            return Ty::Unknown;
        }
        let mut walker = Walker::new(source, self.symbols, 0, lexed.tokens.len());
        walker.scopes = self.scopes.clone();
        walker.expanding = self.expanding.clone();
        walker.depth = MAX_DEPTH.saturating_sub(50).max(self.depth);
        match walker.macro_expression() {
            Ok(ty) => ty,
            Err(Stop) => Ty::Unknown,
        }
    }

    /// A whole macro body as one expression. Names inside it resolve
    /// through `variable` as anywhere else; a nested macro chain is short
    /// in practice, and `MAX_DEPTH` bounds the parse itself.
    fn macro_expression(&mut self) -> Parsed<Ty> {
        let value = self.expression()?;
        self.finished()?;
        Ok(value.ty)
    }

    // ---- conversions -----------------------------------------------------

    /// Converts `value` to `target` when that is an int→float conversion of
    /// the same shape.
    fn coerce_to(&mut self, value: Expr, target: &Ty) -> Expr {
        if target.is_float() && value.ty.is_integer() && value.ty.shape() == target.shape() {
            self.coerce_float(value)
        } else {
            value
        }
    }

    /// Converts an integer `value` to the float type of the same shape.
    fn coerce_float(&mut self, value: Expr) -> Expr {
        if !value.ty.is_integer() {
            return value;
        }
        let Some(float) = value.ty.as_float() else {
            return value;
        };
        match (&value.literals, value.ty.float_constructor()) {
            (Some(literals), _) if value.ty.is_scalar() => {
                for &literal in literals {
                    self.promote_literal(literal);
                }
            }
            (Some(_) | None, Some(constructor)) => {
                self.wrap(value.start, value.end, constructor);
            }
            (Some(_) | None, None) => return value,
        }
        Expr {
            ty: float,
            literals: None,
            ..value
        }
    }

    /// `2` → `2.0`; anything else that is an integer literal → `float(...)`.
    fn promote_literal(&mut self, index: usize) {
        let Some(text) = self.source.text(index) else {
            return;
        };
        let plain =
            text.bytes().all(|b| b.is_ascii_digit()) && (text == "0" || !text.starts_with('0'));
        if plain {
            if let Some(token) = self.source.tokens.get(index) {
                self.edits
                    .push(Edit::insert(token.end, CLOSE, ".0".to_owned()));
            }
        } else {
            self.wrap(index, index + 1, "float");
        }
    }

    /// Wraps tokens `start..end` in `constructor(...)`.
    /// `texture(syn_Spectrum, x)`: the audio textures are 1D on desktop and
    /// 2D one texel tall on ES (Q1), so a scalar coordinate becomes
    /// `vec2(x, 0.5)` (`ivec2(x, 0)` for `texelFetch`).
    fn one_dimensional(&mut self, rule: Builtin, args: &[Expr]) {
        let (Some(sampler), Some(coordinate)) = (args.first(), args.get(1)) else {
            return;
        };
        let named = sampler.end == sampler.start + 1
            && self
                .source
                .word(sampler.start)
                .is_some_and(|name| ONE_DIMENSIONAL.contains(&name));
        if !named || !coordinate.ty.is_scalar() || sampler.ty != Ty::Sampler(2) {
            return;
        }
        let (open, close) = match rule {
            Builtin::Texture(_) if coordinate.ty.is_float() => ("vec2(", ", 0.5)"),
            Builtin::Fetch if coordinate.ty.is_integer() => ("ivec2(", ", 0)"),
            Builtin::Texture(_)
            | Builtin::Fetch
            | Builtin::FloatWise
            | Builtin::MixedWise
            | Builtin::FloatScalar
            | Builtin::Cross
            | Builtin::TextureSize
            | Builtin::BoolScalar
            | Builtin::BoolWise => return,
        };
        self.wrap_with(coordinate.start, coordinate.end, open, close);
    }

    fn wrap(&mut self, start: usize, end: usize, constructor: &str) {
        self.wrap_with(start, end, &format!("{constructor}("), ")");
    }

    /// Puts `open` before tokens `start..end` and `close` after them.
    fn wrap_with(&mut self, start: usize, end: usize, open: &str, close: &str) {
        let (Some(first), Some(last)) = (
            self.source.tokens.get(start),
            end.checked_sub(1).and_then(|i| self.source.tokens.get(i)),
        ) else {
            return;
        };
        self.edits
            .push(Edit::insert(first.start, OPEN, open.to_owned()));
        self.edits
            .push(Edit::insert(last.end, CLOSE, close.to_owned()));
    }
}

/// Where a call goes.
enum Callee {
    /// A function by name (after following macro aliases).
    Named(String),
    /// A function-like macro or anything else not followed.
    Opaque,
}

/// The int→float conversions that make `args` match `sig`, or `None` when
/// they cannot.
fn conversions(sig: &Sig, args: &[Expr]) -> Option<Vec<usize>> {
    let mut needed = Vec::new();
    for (index, ((param, out), arg)) in sig.params.iter().zip(args).enumerate() {
        if arg.ty == Ty::Unknown || arg.ty == *param {
            continue;
        }
        let converts =
            !out && param.is_float() && arg.ty.is_integer() && arg.ty.shape() == param.shape();
        if !converts {
            return None;
        }
        needed.push(index);
    }
    Some(needed)
}

/// `true` when an integer of shape `int` can meet a float of shape `float`
/// in an operator: the same shape, or a scalar on either side.
fn fits(int: &Ty, float: &Ty) -> bool {
    int.is_scalar() || float.is_scalar() || int.shape() == float.shape()
}

/// The result of `+ - * /`.
fn arithmetic(op: &str, left: &Ty, right: &Ty) -> Ty {
    // The right operand's rows never decide the result: a matrix product
    // takes its rows from the left, and a vector's from either side.
    let (Some((left_rows, left_cols)), Some((_, right_cols))) = (left.shape(), right.shape())
    else {
        return Ty::Unknown;
    };
    if left.kind() != right.kind() {
        return Ty::Unknown;
    }
    let scalar = left.kind().unwrap_or(Scalar::Float);
    if left.is_scalar() {
        return right.clone();
    }
    if right.is_scalar() {
        return left.clone();
    }
    if op == "*" {
        let left_matrix = left_cols > 1;
        let right_matrix = right_cols > 1;
        return match (left_matrix, right_matrix) {
            (true, false) => Ty::vector(scalar, left_rows),
            (false, true) => Ty::vector(scalar, right_cols),
            (true, true) => Ty::Num {
                scalar,
                rows: left_rows,
                cols: right_cols,
            },
            (false, false) => left.clone(),
        };
    }
    left.clone()
}

/// The result of `% << >> & | ^`.
fn integer_op(left: &Ty, right: &Ty) -> Ty {
    if left.is_scalar() {
        right.clone()
    } else {
        left.clone()
    }
}

/// The type of `value[i]`.
fn element(ty: &Ty) -> Ty {
    match ty {
        Ty::Array(inner) => (**inner).clone(),
        Ty::Num {
            scalar, rows, cols, ..
        } => {
            if *cols > 1 {
                Ty::vector(*scalar, *rows)
            } else if *rows > 1 {
                Ty::scalar(*scalar)
            } else {
                Ty::Unknown
            }
        }
        Ty::Sampler(_) | Ty::Struct(_) | Ty::Void | Ty::Unknown => Ty::Unknown,
    }
}

/// The type of `value.xyz`.
fn swizzle(ty: &Ty, field: &str) -> Ty {
    let Ty::Num {
        scalar,
        rows,
        cols: 1,
    } = ty
    else {
        return Ty::Unknown;
    };
    let sets = ["xyzw", "rgba", "stpq"];
    let valid = !field.is_empty()
        && field.len() <= 4
        && *rows > 1
        && sets
            .iter()
            .any(|set| field.chars().all(|c| set.contains(c)));
    if !valid {
        return Ty::Unknown;
    }
    let len = u8::try_from(field.len()).unwrap_or(1);
    Ty::vector(*scalar, len)
}

/// The widest number among `args`, as a built-in's result; unknown when an
/// argument is.
fn widest(args: &[Expr]) -> Ty {
    let mut best: Option<&Ty> = None;
    for arg in args {
        let Some((rows, cols)) = arg.ty.shape() else {
            return Ty::Unknown;
        };
        let size = u16::from(rows) * u16::from(cols);
        let wider = best
            .and_then(Ty::shape)
            .is_none_or(|(r, c)| u16::from(r) * u16::from(c) < size);
        if wider {
            best = Some(&arg.ty);
        }
    }
    best.cloned().unwrap_or(Ty::Unknown)
}

#[cfg(test)]
mod tests;
