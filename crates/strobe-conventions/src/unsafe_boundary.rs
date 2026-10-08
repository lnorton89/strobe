// ============================================================================
// Module:       strobe_conventions::unsafe_boundary
// Description:  Enforces the safe-leaf boundary: unsafe lives only in
//               strobe-gfx's gl module and strobe-diag's native.rs, one
//               operation per wrapper.
//
// Dependencies: anyhow; crate::sources (the workspace walk)
// ============================================================================

//! The unsafe boundary.
//!
//! Every GL call is `unsafe` in `glow`, and so is creating a context. That
//! is a lot of unsafe for one renderer, which is exactly why it is fenced:
//! it all lives in the [`ALLOWED`] places, each operation in its own
//! small, named, private safe function whose `unsafe` block holds the call
//! and nothing else, under a `// SAFETY:` comment. Nothing outside them
//! contains the word, and no test does.
//!
//! There are two places: `crates/strobe-gfx/src/gl/`, where every GL call
//! and context creation lives, and the single file
//! `crates/strobe-diag/src/native.rs`, which holds the one unsafe call that
//! installs the native crash handler (decision D41). An entry ending in `/`
//! is a directory; any other entry is exactly one file.
//!
//! Compiler lints check the local mechanics (`undocumented_unsafe_blocks`,
//! `unsafe_op_in_unsafe_fn`). This check enforces what they cannot:
//! *where* unsafe may appear, that no `pub unsafe fn` escapes, and that
//! one function never accumulates more than one unsafe operation.

use std::collections::HashMap;

/// The places allowed to hold unsafe code: a directory (trailing `/`) or
/// exactly one file.
pub const ALLOWED: [&str; 2] = [
    "crates/strobe-gfx/src/gl/",
    "crates/strobe-diag/src/native.rs",
];

/// This crate's own sources, which spell the keyword in their checks.
const SELF: &str = "crates/strobe-conventions/";

/// Whether `relative` is one of the [`ALLOWED`] places.
fn is_allowed(relative: &str) -> bool {
    ALLOWED.iter().any(|place| {
        if place.ends_with('/') {
            relative.starts_with(place)
        } else {
            relative == *place
        }
    })
}

/// Whether a source line (comments stripped) opens an unsafe boundary.
fn is_boundary(line: &str) -> bool {
    let keyword = ["un", "safe"].concat();
    let code = line.split("//").next().unwrap_or_default();
    code.contains(&format!("{keyword} {{"))
        || code.contains(&format!("{keyword} fn "))
        || code.contains(&format!("{keyword} impl "))
        || code.contains(&format!("{keyword} extern "))
}

/// The nearest named function at or above `line`.
fn enclosing_function(lines: &[&str], line: usize) -> Option<usize> {
    (0..=line).rev().find(|index| {
        let trimmed = lines.get(*index).map_or("", |text| text.trim_start());
        [
            "fn ",
            "pub fn ",
            "pub(crate) fn ",
            "pub(super) fn ",
            "const fn ",
        ]
        .iter()
        .any(|prefix| trimmed.starts_with(prefix))
    })
}

/// Every boundary violation in one file.
pub fn violations(relative: &str, text: &str) -> Vec<String> {
    let mut failures = Vec::new();
    if relative.starts_with(SELF) {
        return failures;
    }
    let keyword = ["un", "safe"].concat();
    let lines: Vec<&str> = text.lines().collect();
    let test_start = lines.iter().position(|line| line.trim() == "mod tests {");
    let mut per_function: HashMap<usize, usize> = HashMap::new();

    for (index, line) in lines.iter().enumerate() {
        if !is_boundary(line) {
            continue;
        }
        let location = format!("{relative}:{}", index + 1);
        if !is_allowed(relative) {
            failures.push(format!(
                "{location}: unsafe code must live in one of {}",
                ALLOWED.join(", ")
            ));
        }
        if relative.contains("/tests/") || test_start.is_some_and(|start| index > start) {
            failures.push(format!("{location}: test code must use safe wrappers"));
        }
        if line.contains(&format!("pub {keyword} fn")) {
            failures.push(format!("{location}: public unsafe APIs are forbidden"));
        }
        if let Some(function) = enclosing_function(&lines, index) {
            *per_function.entry(function).or_default() += 1;
        }
    }

    let mut crowded: Vec<(usize, usize)> = per_function
        .into_iter()
        .filter(|(_, count)| *count > 1)
        .collect();
    crowded.sort_unstable();
    for (line, count) in crowded {
        failures.push(format!(
            "{relative}:{}: leaf wrapper contains {count} unsafe operations",
            line + 1
        ));
    }
    failures
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::workspace_sources;
    use anyhow::Result;

    #[test]
    fn unsafe_code_stays_in_one_operation_leaf_wrappers() -> Result<()> {
        let failures: Vec<String> = workspace_sources()?
            .iter()
            .flat_map(|source| violations(&source.relative, &source.text))
            .collect();
        assert!(
            failures.is_empty(),
            "unsafe boundary violation(s):\n{}",
            failures.join("\n")
        );
        Ok(())
    }

    #[test]
    fn unsafe_outside_the_gl_module_is_reported() {
        let keyword = ["un", "safe"].concat();
        let text = format!("fn leaf() {{\n    {keyword} {{ call() }}\n}}\n");
        let found = violations("crates/strobe-scene/src/load.rs", &text);
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("must live in one of")),
            "got {found:?}"
        );
        assert!(
            violations("crates/strobe-gfx/src/gl/raw.rs", &text).is_empty(),
            "one operation in the gl module is allowed"
        );
    }

    #[test]
    fn the_diag_native_file_is_allowed_but_its_neighbours_are_not() {
        let keyword = ["un", "safe"].concat();
        let text = format!("fn leaf() {{\n    {keyword} {{ call() }}\n}}\n");
        assert!(
            violations("crates/strobe-diag/src/native.rs", &text).is_empty(),
            "one operation in strobe-diag's native.rs is allowed"
        );
        for neighbour in [
            "crates/strobe-diag/src/crash.rs",
            "crates/strobe-diag/src/native_extra.rs",
            "crates/strobe-diag/src/native.rs.bak",
        ] {
            assert!(
                !violations(neighbour, &text).is_empty(),
                "{neighbour} must not be allowed to hold unsafe code"
            );
        }
    }

    #[test]
    fn two_operations_in_the_diag_native_file_are_reported() {
        let keyword = ["un", "safe"].concat();
        let text =
            format!("fn leaf() {{\n    {keyword} {{ a() }};\n    {keyword} {{ b() }};\n}}\n");
        let found = violations("crates/strobe-diag/src/native.rs", &text);
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("2 unsafe operations")),
            "got {found:?}"
        );
    }

    #[test]
    fn two_operations_in_one_wrapper_are_reported() {
        let keyword = ["un", "safe"].concat();
        let text =
            format!("fn leaf() {{\n    {keyword} {{ a() }};\n    {keyword} {{ b() }};\n}}\n");
        let found = violations("crates/strobe-gfx/src/gl/raw.rs", &text);
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("2 unsafe operations")),
            "got {found:?}"
        );
    }

    #[test]
    fn a_public_unsafe_function_is_reported() {
        let keyword = ["un", "safe"].concat();
        let text = format!("pub {keyword} fn raw() {{}}\n");
        let found = violations("crates/strobe-gfx/src/gl/raw.rs", &text);
        assert!(
            found
                .iter()
                .any(|problem| problem.contains("public unsafe")),
            "got {found:?}"
        );
    }
}
