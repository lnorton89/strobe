// ============================================================================
// Module:       strobe_scene::promote::expand
// Description:  Types a call to a function-like macro through its expansion
//               (GLSL ES only), and maps the conversions it needs back onto
//               the call's arguments or, failing that, inlines the call.
//
// Dependencies: std::collections::HashMap; super (Walker, Edit, apply),
//               super::lex
// ============================================================================

//! Function-like macros on the ES dialect (C-02, D261).
//!
//! `#define S(a, b, t) smoothstep(a, b, t)` called as `S(0, 1., x)` is
//! `smoothstep(0, 1., x)` to the compiler, which desktop GLSL accepts and
//! GLSL ES rejects. The walker cannot see that through the call, so on ES
//! it builds the expansion the preprocessor would, one token at a time,
//! remembering where each token came from: the macro's body, or which use
//! of which parameter and which token of the call's argument. It then types
//! the expansion with a walker of its own and looks at the edits:
//!
//! - **Every edit lands in an argument, and every use of a parameter wants
//!   the same edits:** they are made in the argument at the call site.
//!   `S(0, 1., x)` becomes `S(0.0, 1., x)`; the macro is untouched.
//! - **Otherwise** (an edit in the body, as `#define sat(x) clamp(x, 0, 1)`
//!   called with a float needs, or a parameter used once as an `int` and
//!   once as a `float`): the call is replaced by its expansion with the
//!   edits written in. The body is never edited, because another call may
//!   need it as it is. The replacement keeps the call's line count, and is
//!   not made when the expansion names a macro the preprocessor would have
//!   left alone inside it (a self-referencing macro).
//!
//! Anything the walker cannot follow (a body that is not an expression, a
//! wrong argument count, a macro defined twice) leaves the call as it was,
//! typed unknown, which is the D199 behaviour.

use super::lex::{Kind, Token, lex};
use super::{CLOSE, Edit, Expr, FnMacro, Macro, OPEN, REPLACE, Source, Ty, Walker, apply, sorted};
use std::collections::HashMap;

/// How many function-like macros may expand inside one another.
const MAX_NESTING: usize = 16;

/// How many macro calls one program may expand. A parameter used `n` times
/// types its argument `n` times, so nested calls multiply; past this budget
/// a call is left untyped, as on desktop. Real scenes use a few hundred.
const MAX_EXPANSIONS: usize = 20_000;

/// Where a token of an expansion came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    /// The macro's replacement text.
    Body,
    /// Use `occurrence` of parameter `param`: token `token` of the caller.
    Arg {
        param: usize,
        occurrence: usize,
        token: usize,
    },
}

/// A macro call's expansion, as text and tokens.
struct Expansion {
    text: String,
    tokens: Vec<Token>,
    origins: Vec<Origin>,
    /// How many times each parameter is used.
    uses: Vec<usize>,
}

impl Expansion {
    fn push(&mut self, text: &str, kind: Kind, origin: Origin) {
        if !self.text.is_empty() {
            self.text.push(' ');
        }
        let start = self.text.len();
        self.text.push_str(text);
        self.tokens.push(Token {
            kind,
            start,
            end: self.text.len(),
        });
        self.origins.push(origin);
    }
}

impl Walker<'_> {
    /// Types the call to `name` at token `start` (its `(` at `self.at`) as
    /// the preprocessor would expand it, when `name` is a function-like
    /// macro this walker follows. On success the call has been consumed and
    /// its conversions recorded; on `None` nothing has changed.
    pub(super) fn expand_macro(&mut self, start: usize, name: &str) -> Option<Expr> {
        let symbols = self.symbols;
        if !symbols.expand
            || self.expanding.len() >= MAX_NESTING
            || self.expanding.iter().any(|n| n == name)
        {
            return None;
        }
        let Some(Macro::Function(Some(definition))) = symbols.macros.get(name) else {
            return None;
        };
        let expanded = symbols.expansions.get();
        if expanded >= MAX_EXPANSIONS {
            return None;
        }
        symbols.expansions.set(expanded + 1);
        let open = self.at;
        if !self.is("(") {
            return None;
        }
        let close = self.source.skip_balanced(open).checked_sub(1)?;
        if close >= self.end || !self.source.is(close, ")") {
            return None;
        }
        let args = self.macro_arguments(open, close, definition.params.len())?;
        let expansion = self.expansion(definition, &args)?;

        let source = Source {
            text: &expansion.text,
            tokens: &expansion.tokens,
        };
        let mut inner = Walker::new(source, symbols, 0, expansion.tokens.len());
        inner.scopes = self.scopes.clone();
        inner.depth = self.depth;
        inner.expanding = self.expanding.clone();
        inner.expanding.push(name.to_owned());
        let value = inner.expression().ok()?;
        let statement = inner.eat(";");
        inner.finished().ok()?;
        if !inner.selects.is_empty() {
            return None;
        }
        let edits = std::mem::take(&mut inner.edits);

        match at_call_site(&expansion, &edits) {
            Some(mapped) => {
                let mapped: Option<Vec<Edit>> = mapped
                    .into_iter()
                    .map(|edit| self.to_caller(edit))
                    .collect();
                self.edits.extend(mapped?);
            }
            None => {
                let inlined = self.inline(&expansion, edits, start, close, name)?;
                self.edits.push(inlined);
            }
        }
        self.at = close + 1;
        Some(Expr {
            ty: if statement { Ty::Unknown } else { value.ty },
            start,
            end: close + 1,
            literals: None,
        })
    }

    /// The token ranges of the arguments between `open` and `close`, when
    /// there are `count` of them and none is empty.
    fn macro_arguments(
        &self,
        open: usize,
        close: usize,
        count: usize,
    ) -> Option<Vec<(usize, usize)>> {
        let mut args = Vec::new();
        let mut from = open + 1;
        let mut at = open + 1;
        while at < close {
            if self.source.is(at, ",") {
                args.push((from, at));
                from = at + 1;
                at += 1;
            } else if self.source.is(at, "(") || self.source.is(at, "[") || self.source.is(at, "{")
            {
                at = self.source.skip_balanced(at);
            } else {
                at += 1;
            }
        }
        if count == 0 {
            return (open + 1 == close).then(Vec::new);
        }
        args.push((from, close));
        (args.len() == count && args.iter().all(|(a, b)| a < b)).then_some(args)
    }

    /// The tokens the preprocessor makes of `definition` called with
    /// `args`, before rescanning.
    fn expansion(&self, definition: &FnMacro, args: &[(usize, usize)]) -> Option<Expansion> {
        let body = lex(&definition.body);
        let body_source = Source {
            text: &definition.body,
            tokens: &body.tokens,
        };
        let mut out = Expansion {
            text: String::new(),
            tokens: Vec::new(),
            origins: Vec::new(),
            uses: vec![0; definition.params.len()],
        };
        for (index, token) in body.tokens.iter().enumerate() {
            let text = body_source.text(index)?;
            if text == "\\" {
                continue;
            }
            let param = (token.kind == Kind::Ident)
                .then(|| definition.params.iter().position(|p| p == text))
                .flatten();
            let Some(param) = param else {
                out.push(text, token.kind, Origin::Body);
                continue;
            };
            let (first, last) = *args.get(param)?;
            let occurrence = *out.uses.get(param)?;
            for caller in first..last {
                let kind = self.source.tokens.get(caller)?.kind;
                let origin = Origin::Arg {
                    param,
                    occurrence,
                    token: caller,
                };
                out.push(self.source.text(caller)?, kind, origin);
            }
            *out.uses.get_mut(param)? += 1;
        }
        Some(out)
    }

    /// An edit expressed against the caller's tokens.
    fn to_caller(&self, edit: Mapped) -> Option<Edit> {
        let first = self.source.tokens.get(edit.first)?;
        let last = self.source.tokens.get(edit.last)?;
        Some(match edit.order {
            CLOSE => Edit::insert(first.end, CLOSE, edit.text),
            OPEN => Edit::insert(first.start, OPEN, edit.text),
            REPLACE => Edit::replace(first.start, last.end, edit.text),
            _ => return None,
        })
    }

    /// The call at tokens `start..=close` replaced by its expansion with
    /// `edits` written in, padded with the newlines it covered, or `None`
    /// where inlining would change what the preprocessor does.
    fn inline(
        &self,
        expansion: &Expansion,
        edits: Vec<Edit>,
        start: usize,
        close: usize,
        name: &str,
    ) -> Option<Edit> {
        // The inlined text is rescanned with no macro disabled; inside the
        // real expansion, these names are not expanded again.
        let rescans_differently = expansion.tokens.iter().any(|token| {
            token.kind == Kind::Ident
                && expansion
                    .text
                    .get(token.start..token.end)
                    .is_some_and(|word| word == name || self.expanding.iter().any(|n| n == word))
        });
        if rescans_differently {
            return None;
        }
        let first = self.source.tokens.get(start)?;
        let last = self.source.tokens.get(close)?;
        let original = self.source.text.get(first.start..last.end)?;
        if original.contains('#') {
            return None;
        }
        let mut text = String::from(" ");
        text.push_str(&apply(&expansion.text, edits));
        text.push(' ');
        for _ in 0..original.matches('\n').count() {
            text.push('\n');
        }
        Some(Edit::replace(first.start, last.end, text))
    }
}

/// An edit of an expansion, moved to the caller's tokens.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Mapped {
    order: u8,
    /// The caller's token the edit starts at (an opener's or a
    /// replacement's) or ends at (a closer's).
    first: usize,
    /// The caller's token a replacement ends at; `first` otherwise.
    last: usize,
    text: String,
}

/// The expansion's edits as edits of the call's arguments, when all of them
/// land in arguments and every use of each parameter wants the same ones.
fn at_call_site(expansion: &Expansion, edits: &[Edit]) -> Option<Vec<Mapped>> {
    let mut starts = HashMap::new();
    let mut ends = HashMap::new();
    for (index, token) in expansion.tokens.iter().enumerate() {
        starts.insert(token.start, index);
        ends.insert(token.end, index);
    }
    let origin = |offsets: &HashMap<usize, usize>, at: usize| {
        offsets
            .get(&at)
            .and_then(|&index| expansion.origins.get(index))
            .copied()
    };
    // Per parameter, per use: the edits that use needs.
    let mut wanted: HashMap<(usize, usize), Vec<Mapped>> = HashMap::new();
    for edit in sorted(edits.to_vec()) {
        let (from, to) = match edit.order {
            CLOSE => (origin(&ends, edit.at)?, origin(&ends, edit.at)?),
            OPEN => (origin(&starts, edit.at)?, origin(&starts, edit.at)?),
            _ => (origin(&starts, edit.at)?, origin(&ends, edit.end)?),
        };
        let (
            Origin::Arg {
                param,
                occurrence,
                token: first,
            },
            Origin::Arg {
                param: to_param,
                occurrence: to_occurrence,
                token: last,
            },
        ) = (from, to)
        else {
            return None;
        };
        if (param, occurrence) != (to_param, to_occurrence) {
            return None;
        }
        wanted.entry((param, occurrence)).or_default().push(Mapped {
            order: edit.order,
            first,
            last,
            text: edit.text,
        });
    }
    let mut mapped = Vec::new();
    for (param, &uses) in expansion.uses.iter().enumerate() {
        let first_use = wanted.get(&(param, 0)).cloned().unwrap_or_default();
        for occurrence in 1..uses {
            let this_use = wanted
                .get(&(param, occurrence))
                .cloned()
                .unwrap_or_default();
            if this_use != first_use {
                return None;
            }
        }
        mapped.extend(first_use);
    }
    Some(mapped)
}
