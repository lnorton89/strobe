// ============================================================================
// Module:       strobe_scene::promote::select
// Description:  Rewrites a call whose sampler argument is chosen with `?:`
//               into a `?:` of calls, which every GLSL dialect accepts.
//
// Dependencies: super (Walker, Edit, Source)
// ============================================================================

//! A sampler chosen with `?:` (C-02, D261).
//!
//! GLSL does not allow a sampler (an opaque type) as an operand of `?:`.
//! NVIDIA's desktop compiler accepts `texture(i == 0 ? a : b, uv)` anyway;
//! Mesa and ANGLE reject it. The call moves inside the choice instead:
//!
//! ```text
//! texture(i == 0 ? a : b, uv)  ->  (i == 0 ? texture(a, uv) : texture(b, uv))
//! ```
//!
//! which is the same value on every driver: one branch runs, so the other
//! arguments are still evaluated once. Nested choices
//! (`c ? a : d ? b : e`) and parentheses around the choice are followed.
//! It applies to the call of any built-in or user function, and only when
//! exactly one argument is such a choice. Everything else (two such
//! arguments, a choice that is not an argument, a function-like macro on
//! desktop) is left as written and reported by [`super::Promoted`].
//!
//! The call is replaced whole, with the conversions already found inside
//! it written in. The replacement is one line, followed by the newlines
//! the call spanned, so no later line moves.

use super::{CLOSE, Edit, OPEN, REPLACE, Source, Walker, sorted};

/// A `?:` found in a token range.
struct Split {
    /// First token of the condition.
    condition: usize,
    /// The `?`.
    question: usize,
    /// The `:` that matches it.
    colon: usize,
    /// One past the last token of the `else` branch.
    end: usize,
}

impl Walker<'_> {
    /// Rewrites the call at tokens `start..end` when exactly one of its
    /// sampler arguments (`samplers`, token ranges) is a `?:`. `mark` is
    /// the number of edits made before the call's arguments were parsed:
    /// every edit after it is inside the call.
    pub(super) fn distribute(
        &mut self,
        start: usize,
        end: usize,
        samplers: &[(usize, usize)],
        mark: usize,
    ) {
        let mut choices = samplers
            .iter()
            .copied()
            .filter(|&(lo, hi)| split(&self.source, lo, hi).is_some());
        let (Some((lo, hi)), None) = (choices.next(), choices.next()) else {
            return;
        };
        let Some(replacement) = self.distributed(start, end, lo, hi, mark) else {
            return;
        };
        self.edits.truncate(mark);
        self.edits.push(replacement);
        self.selects.retain(|&(a, b)| !(lo <= a && b <= hi));
    }

    /// The replacement for the call at `start..end` with the choice at
    /// `lo..hi` moved outside it.
    fn distributed(
        &self,
        start: usize,
        end: usize,
        lo: usize,
        hi: usize,
        mark: usize,
    ) -> Option<Edit> {
        let first = self.source.tokens.get(start)?;
        let last = self.source.tokens.get(end.checked_sub(1)?)?;
        let original = self.source.text.get(first.start..last.end)?;
        if original.contains('#') {
            return None;
        }
        let edits = sorted(self.edits.get(mark..)?.to_vec());
        let mut used = vec![false; edits.len()];
        let prefix = render(&self.source, start, lo, &edits, &mut used)?;
        let suffix = render(&self.source, hi, end, &edits, &mut used)?;
        let mut text = choice(&self.source, lo, hi, (&prefix, &suffix), &edits, &mut used)?;
        // An edit with no token to sit on would be lost.
        if used.contains(&false) {
            return None;
        }
        for _ in 0..original.matches('\n').count() {
            text.push('\n');
        }
        Some(Edit::replace(first.start, last.end, text))
    }
}

/// The `?:` that makes up tokens `lo..hi`, inside any parentheses that
/// enclose all of it.
fn split(source: &Source<'_>, mut lo: usize, mut hi: usize) -> Option<Split> {
    while lo < hi && source.is(lo, "(") && source.skip_balanced(lo) == hi {
        lo += 1;
        hi -= 1;
    }
    let mut at = lo;
    let mut question = None;
    while at < hi {
        if source.is(at, "(") || source.is(at, "[") || source.is(at, "{") {
            at = source.skip_balanced(at);
            continue;
        }
        if source.is(at, "?") {
            question = Some(at);
            break;
        }
        at += 1;
    }
    let question = question?;
    let mut nested = 0_usize;
    let mut at = question + 1;
    while at < hi {
        if source.is(at, "(") || source.is(at, "[") || source.is(at, "{") {
            at = source.skip_balanced(at);
            continue;
        }
        if source.is(at, "?") {
            nested += 1;
        } else if source.is(at, ":") {
            if nested == 0 {
                return (lo < question && question + 1 < at && at + 1 < hi).then_some(Split {
                    condition: lo,
                    question,
                    colon: at,
                    end: hi,
                });
            }
            nested -= 1;
        }
        at += 1;
    }
    None
}

/// The choice at `lo..hi` with the call (`prefix`, the choice's branch,
/// `suffix`) at every leaf.
fn choice(
    source: &Source<'_>,
    lo: usize,
    hi: usize,
    call: (&str, &str),
    edits: &[Edit],
    used: &mut [bool],
) -> Option<String> {
    let Some(parts) = split(source, lo, hi) else {
        let leaf = render(source, lo, hi, edits, used)?;
        return Some(format!("{} {leaf} {}", call.0, call.1));
    };
    let condition = render(source, parts.condition, parts.question, edits, used)?;
    let yes = choice(source, parts.question + 1, parts.colon, call, edits, used)?;
    let no = choice(source, parts.colon + 1, parts.end, call, edits, used)?;
    Some(format!("({condition} ? {yes} : {no})"))
}

/// Tokens `from..to`, one space apart, with the edits that sit on them
/// written in. Marks each edit it writes in `used`.
fn render(
    source: &Source<'_>,
    from: usize,
    to: usize,
    edits: &[Edit],
    used: &mut [bool],
) -> Option<String> {
    let mut out = String::new();
    let mut index = from;
    while index < to {
        let token = source.tokens.get(index)?;
        if !out.is_empty() {
            out.push(' ');
        }
        for (edit, done) in edits.iter().zip(used.iter_mut()) {
            if edit.at == token.start && edit.order == OPEN {
                out.push_str(&edit.text);
                *done = true;
            }
        }
        let mut last = index;
        let replacement = edits
            .iter()
            .zip(used.iter_mut())
            .find(|(edit, _)| edit.at == token.start && edit.order == REPLACE);
        if let Some((edit, done)) = replacement {
            last =
                (index..to).find(|&j| source.tokens.get(j).is_some_and(|t| t.end == edit.end))?;
            out.push_str(edit.text.trim_end_matches('\n'));
            *done = true;
        } else {
            out.push_str(source.text(index)?);
        }
        let end = source.tokens.get(last)?.end;
        for (edit, done) in edits.iter().zip(used.iter_mut()) {
            if edit.at == end && edit.order == CLOSE {
                out.push_str(&edit.text);
                *done = true;
            }
        }
        index = last + 1;
    }
    Some(out)
}
