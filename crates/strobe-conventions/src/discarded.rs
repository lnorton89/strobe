// ============================================================================
// Module:       strobe_conventions::discarded
// Description:  Every result thrown away outside tests says why: a
//               `let _ =` or a trailing `.ok();` carries a comment.
//
// Dependencies: anyhow; crate::sources (the workspace walk)
// ============================================================================

//! Discarded results.
//!
//! `let _ = f();` and `f().ok();` compile silently whatever `f` returns, so
//! a reader cannot tell a deliberate "nothing useful to do with this
//! failure" from an error that was dropped by accident. An audit of the
//! workspace (2026-10-13) found both, side by side:
//!
//! - deliberate: a send to a thread that has already gone, best-effort
//!   cleanup of a temporary file, a crash handler that must not fail;
//! - accidental: a rollback whose failure left a scene hidden under another
//!   name with no word, a damaged file reported as moved when the move
//!   failed, an existing `scene.json` replaced because reading it failed.
//!
//! So the rule is: outside tests, a statement that throws a result away
//! carries a `//` comment saying why, on its own line or directly above it
//! (D267). Anything that cannot honestly get one is handled instead.
//!
//! `write!` and `writeln!` are exempt: into a `String` they cannot fail,
//! and the few that write to a stream are best-effort diagnostics.

/// Whether a line (trimmed) is only a comment.
fn is_comment(line: &str) -> bool {
    line.starts_with("//")
}

/// Whether `code` (one trimmed line) throws a result away.
fn discards(code: &str) -> bool {
    let formatting = code.contains("write!(") || code.contains("writeln!(");
    if formatting {
        return false;
    }
    code.starts_with("let _ =") || code.ends_with(".ok();")
}

/// Whether a line ends a statement or opens or closes a block, so the line
/// after it starts a new statement.
fn ends_statement(code: &str) -> bool {
    code.is_empty() || code.ends_with(';') || code.ends_with('{') || code.ends_with('}')
}

/// The places in `text` (a non-test source) that discard a result with no
/// comment saying why, as `relative:line: code`.
pub fn violations(relative: &str, text: &str) -> Vec<String> {
    let lines: Vec<&str> = text.lines().map(str::trim).collect();
    let mut found = Vec::new();
    for (index, line) in lines.iter().enumerate() {
        // Test code says what it means by its assertions.
        if *line == "#[cfg(test)]" {
            break;
        }
        if is_comment(line) {
            continue;
        }
        let code = line.split(" //").next().unwrap_or_default().trim_end();
        if !discards(code) {
            continue;
        }
        if line.contains("//") {
            continue;
        }
        // A chained call can end with `.ok();` lines below its start: find
        // the line the statement begins on.
        let mut start = index;
        while start > 0 {
            let above = lines.get(start - 1).copied().unwrap_or_default();
            if is_comment(above) || ends_statement(above) {
                break;
            }
            start -= 1;
        }
        let statement = lines.get(start..=index).unwrap_or_default();
        // `let x = f().ok();` or `x = f().ok();` keeps the value as an
        // `Option`: a conversion, not a discard.
        let first = statement.first().copied().unwrap_or_default();
        let assigns = !first.starts_with("let _ =")
            && (first.starts_with("let ") || first.contains(" = ") || first.contains("= |"));
        if assigns {
            continue;
        }
        let commented_inside = statement.iter().any(|l| l.contains("//"));
        let commented_above =
            start > 0 && lines.get(start - 1).is_some_and(|above| is_comment(above));
        if !commented_inside && !commented_above {
            found.push(format!("{relative}:{}: {code}", index + 1));
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::workspace_sources;
    use anyhow::Result;

    #[test]
    fn every_discarded_result_outside_tests_says_why() -> Result<()> {
        let failures: Vec<String> = workspace_sources()?
            .iter()
            .filter(|source| source.relative.contains("/src/"))
            .flat_map(|source| violations(&source.relative, &source.text))
            .collect();
        assert!(
            failures.is_empty(),
            "{} result(s) thrown away with no comment saying why (D267); handle \
             the error, or say on the line or just above why there is nothing \
             to do:\n{}",
            failures.len(),
            failures.join("\n")
        );
        Ok(())
    }

    #[test]
    fn a_bare_discard_is_reported_and_a_commented_one_is_not() -> Result<()> {
        let bare = "fn f() {\n    let _ = std::fs::remove_file(path);\n}\n";
        assert_eq!(
            violations("crates/x/src/a.rs", bare).len(),
            1,
            "a bare `let _ =` is reported"
        );
        let above = "fn f() {\n    // Best effort: a stale file is harmless.\n    let _ = std::fs::remove_file(path);\n}\n";
        assert!(
            violations("crates/x/src/a.rs", above).is_empty(),
            "a comment above is a reason"
        );
        let inline = "fn f() {\n    tx.send(x).ok(); // the receiver is gone\n}\n";
        assert!(
            violations("crates/x/src/a.rs", inline).is_empty(),
            "a comment on the line is a reason"
        );
        Ok(())
    }

    #[test]
    fn a_chain_ending_in_ok_is_judged_from_its_first_line() -> Result<()> {
        let chained = "fn f() {\n    // The thread is optional.\n    self.sender = Sender::start()\n        .map_err(log)\n        .ok();\n}\n";
        assert!(
            violations("crates/x/src/a.rs", chained).is_empty(),
            "the comment above the statement's first line covers the chain"
        );
        let bare = "fn f() {\n    let x = 1;\n    client\n        .ping()\n        .ok();\n}\n";
        assert_eq!(
            violations("crates/x/src/a.rs", bare).len(),
            1,
            "an uncommented chain is reported"
        );
        Ok(())
    }

    #[test]
    fn keeping_a_result_as_an_option_is_not_a_discard() -> Result<()> {
        let text = "fn f() {\n    let value = std::env::var(ENV).ok();\n    self.sender = Sender::start()\n        .ok();\n}\n";
        assert!(
            violations("crates/x/src/a.rs", text).is_empty(),
            "an assigned `.ok()` keeps the value"
        );
        Ok(())
    }

    #[test]
    fn formatting_and_test_code_are_exempt() -> Result<()> {
        let text = "fn f() {\n    let _ = writeln!(out, \"x\");\n    writeln!(text, \"y\").ok();\n}\n#[cfg(test)]\nmod tests {\n    fn g() { let _ = h(); }\n}\n";
        assert!(
            violations("crates/x/src/a.rs", text).is_empty(),
            "write!/writeln! and everything after #[cfg(test)] are exempt"
        );
        Ok(())
    }
}
