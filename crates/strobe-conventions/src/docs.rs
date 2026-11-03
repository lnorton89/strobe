// ============================================================================
// Module:       strobe_conventions::docs
// Description:  Checks the Markdown: every relative link resolves to a file,
//               every decision number cited exists in docs/decisions/, and
//               each decision file is named for the number it defines.
//
// Dependencies: anyhow, std::fs; crate::sources (the workspace root)
// ============================================================================

//! The Markdown's links and decision numbers (D497).
//!
//! The docs are how the next person (or agent) finds the reasoning, and two
//! kinds of rot make them lie quietly: a link to a file that moved, and a
//! decision number that was never written or was renumbered (D491 and D492
//! were swapped once). Both are cheap to check:
//!
//! - Every inline link or image (`[text](target)`) whose target is a
//!   relative path resolves to a file or directory, from the linking file's
//!   own directory. Web links and in-page anchors are not followed; an
//!   anchor on a file link is ignored.
//! - Every `D<number>` cited names a decision: a file
//!   `docs/decisions/D<number>.md` whose heading is `# D<number>: …`
//!   (D531; one file per decision, so two branches that each add one do not
//!   conflict). The numbering has gaps on purpose (blocks were reserved per
//!   phase), so a cited number in a gap is caught too.
//! - Each decision file defines exactly the number in its name.
//!
//! The files are every `.md` under `docs/`, the repository root's, and each
//! crate's `README.md`. `CHANGELOG.md` is generated from commit subjects
//! (D50) and is left alone. Fenced code blocks are skipped: their text is
//! code, not prose.

use crate::sources::workspace_root;
use anyhow::Result;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// The decision log's folder, relative to the root: one file per decision.
pub const DECISIONS: &str = "docs/decisions";

/// Generated, never hand-edited, so never checked.
const GENERATED: [&str; 1] = ["CHANGELOG.md"];

/// One Markdown file: its root-relative path and its text.
#[derive(Debug)]
pub struct Page {
    /// Path relative to the workspace root, `/`-separated.
    pub relative: String,
    /// The file's contents.
    pub text: String,
}

/// Every `.md` under `dir`, recursively.
fn markdown_under(dir: &Path, found: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            markdown_under(&path, found)?;
        } else if path.extension().is_some_and(|ext| ext == "md") {
            found.push(path);
        }
    }
    Ok(())
}

/// The pages the rule covers, sorted.
pub fn pages() -> Result<Vec<Page>> {
    let root = workspace_root()?;
    let mut paths = Vec::new();
    markdown_under(&root.join("docs"), &mut paths)?;
    for entry in fs::read_dir(&root)? {
        let path = entry?.path();
        let generated = path
            .file_name()
            .is_some_and(|name| GENERATED.iter().any(|skip| name == *skip));
        if path.is_file() && path.extension().is_some_and(|ext| ext == "md") && !generated {
            paths.push(path);
        }
    }
    for entry in fs::read_dir(root.join("crates"))? {
        let readme = entry?.path().join("README.md");
        if readme.is_file() {
            paths.push(readme);
        }
    }
    let mut pages = Vec::new();
    for path in paths {
        let relative = path
            .strip_prefix(&root)?
            .to_string_lossy()
            .replace('\\', "/");
        let text = fs::read_to_string(&path)?;
        pages.push(Page { relative, text });
    }
    pages.sort_by(|a, b| a.relative.cmp(&b.relative));
    Ok(pages)
}

/// The prose lines of a Markdown text, numbered from 1, without fenced
/// code blocks.
fn prose(text: &str) -> Vec<(usize, &str)> {
    let mut fenced = false;
    let mut lines = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if !fenced {
            lines.push((index + 1, line));
        }
    }
    lines
}

/// The decision numbers a decision file defines: each heading that opens
/// with `D<number>`.
pub fn defined_decisions(decisions: &str) -> BTreeSet<u32> {
    decisions
        .lines()
        .filter_map(|line| {
            let title = line.trim_start_matches('#');
            if title.len() == line.len() {
                return None;
            }
            let digits: String = title
                .trim_start()
                .strip_prefix('D')?
                .chars()
                .take_while(char::is_ascii_digit)
                .collect();
            digits.parse().ok()
        })
        .collect()
}

/// Every decision file under [`DECISIONS`]: its file name and its text.
pub fn decision_files(root: &Path) -> Result<Vec<(String, String)>> {
    let mut files = Vec::new();
    for entry in fs::read_dir(root.join(DECISIONS))? {
        let path = entry?.path();
        if path.extension().is_some_and(|ext| ext == "md") {
            let name = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            files.push((name, fs::read_to_string(&path)?));
        }
    }
    files.sort();
    Ok(files)
}

/// The decision numbers the decision files define.
pub fn decision_numbers(root: &Path) -> Result<BTreeSet<u32>> {
    Ok(decision_files(root)?
        .iter()
        .flat_map(|(_, text)| defined_decisions(text))
        .collect())
}

/// A decision file whose name is not `D<n>.md` for the one number its
/// headings define, described; `None` when it matches.
pub fn misnamed_decision(name: &str, text: &str) -> Option<String> {
    let defined = defined_decisions(text);
    let only = defined.iter().next().filter(|_| defined.len() == 1);
    match only {
        Some(number) if name == format!("D{number}.md") => None,
        Some(number) => Some(format!(
            "{DECISIONS}/{name} defines D{number}, so it should be D{number}.md"
        )),
        None => Some(format!(
            "{DECISIONS}/{name} should define exactly one decision, and defines {defined:?}"
        )),
    }
}

/// Every `D<number>` a line cites: a `D` not preceded by a letter, digit
/// or `_`, then one to four digits not followed by a letter, digit or `_`
/// (so `D3D11` and `HDR` are not citations).
pub fn cited_decisions(line: &str) -> Vec<u32> {
    let word = |c: char| c.is_alphanumeric() || c == '_';
    let chars: Vec<char> = line.chars().collect();
    let mut cited = Vec::new();
    for (index, c) in chars.iter().enumerate() {
        if *c != 'D' {
            continue;
        }
        let before = index
            .checked_sub(1)
            .and_then(|previous| chars.get(previous))
            .copied();
        if before.is_some_and(word) {
            continue;
        }
        let digits: String = chars
            .get(index + 1..)
            .unwrap_or_default()
            .iter()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        let after = chars.get(index + 1 + digits.len()).copied();
        if digits.is_empty() || digits.len() > 4 || after.is_some_and(word) {
            continue;
        }
        if let Ok(number) = digits.parse() {
            cited.push(number);
        }
    }
    cited
}

/// A line without its inline code spans (`` `a[i](x)` `` is code, not a
/// link). An unclosed backtick leaves the rest of the line as it is.
fn without_code_spans(line: &str) -> String {
    let mut kept = String::new();
    let mut rest = line;
    while let Some(open) = rest.find('`') {
        let Some(close) = rest.get(open + 1..).and_then(|after| after.find('`')) else {
            break;
        };
        kept.push_str(rest.get(..open).unwrap_or_default());
        rest = rest.get(open + 1 + close + 1..).unwrap_or_default();
    }
    kept.push_str(rest);
    kept
}

/// Every inline link or image target on a line: the text between `](` and
/// the next `)`, without a `<...>` wrapper or a `"title"`.
pub fn link_targets(line: &str) -> Vec<&str> {
    let mut targets = Vec::new();
    let mut rest = line;
    while let Some(start) = rest.find("](") {
        let after = rest.get(start + 2..).unwrap_or_default();
        let Some(end) = after.find(')') else {
            break;
        };
        let target = after.get(..end).unwrap_or_default().trim();
        let target = target.split(" \"").next().unwrap_or_default();
        let target = target.trim_start_matches('<').trim_end_matches('>');
        if !target.is_empty() {
            targets.push(target);
        }
        rest = after.get(end..).unwrap_or_default();
    }
    targets
}

/// The local file a link target names, if it names one: not a web link,
/// mail address or in-page anchor. The anchor is dropped.
fn local_path(target: &str) -> Option<String> {
    if target.starts_with('#') {
        return None;
    }
    let scheme = target.split_once(':').is_some_and(|(scheme, _)| {
        !scheme.is_empty()
            && scheme
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "+.-".contains(c))
    });
    if scheme {
        return None;
    }
    let path = target.split('#').next().unwrap_or_default();
    (!path.is_empty()).then(|| path.replace("%20", " "))
}

/// Every dead link and unknown decision on one page. `exists` answers for
/// a root-relative path, so the rule can be tested without files.
pub fn problems(
    page: &Page,
    defined: &BTreeSet<u32>,
    exists: &dyn Fn(&str) -> bool,
) -> Vec<String> {
    let mut found = Vec::new();
    let dir = page.relative.rsplit_once('/').map_or("", |(dir, _)| dir);
    for (number, line) in prose(&page.text) {
        let location = format!("{}:{number}", page.relative);
        for decision in cited_decisions(line) {
            if !defined.contains(&decision) {
                found.push(format!(
                    "{location}: cites D{decision}, which {DECISIONS} does not define"
                ));
            }
        }
        let line = without_code_spans(line);
        for target in link_targets(&line) {
            let Some(path) = local_path(target) else {
                continue;
            };
            let resolved = resolve(dir, &path);
            if !resolved.as_deref().is_some_and(exists) {
                found.push(format!(
                    "{location}: links to `{target}`, which does not exist"
                ));
            }
        }
    }
    found
}

/// `path` relative to the directory `dir` (both root-relative), with `.`
/// and `..` folded; `None` for a path that climbs out of the root. A path
/// starting with `/` is taken from the root, as GitHub does.
fn resolve(dir: &str, path: &str) -> Option<String> {
    let mut parts: Vec<&str> = if path.starts_with('/') {
        Vec::new()
    } else {
        dir.split('/').filter(|part| !part.is_empty()).collect()
    };
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            name => parts.push(name),
        }
    }
    Some(parts.join("/"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_decision_file_is_named_for_its_number() -> Result<()> {
        let root = workspace_root()?;
        let misnamed: Vec<String> = decision_files(&root)?
            .iter()
            .filter_map(|(name, text)| misnamed_decision(name, text))
            .collect();
        assert!(
            misnamed.is_empty(),
            "{}",
            misnamed.join(
                "
"
            )
        );
        Ok(())
    }

    #[test]
    fn a_decision_file_must_match_its_heading() {
        assert_eq!(
            misnamed_decision(
                "D7.md",
                "# D7: seven
"
            ),
            None,
            "matches"
        );
        assert!(
            misnamed_decision(
                "D8.md",
                "# D7: seven
"
            )
            .is_some(),
            "a wrong name is caught"
        );
        assert!(
            misnamed_decision(
                "D7.md",
                "# D7: a

# D9: b
"
            )
            .is_some(),
            "two decisions in one file are caught"
        );
    }

    #[test]
    fn every_relative_link_resolves_and_every_cited_decision_exists() -> Result<()> {
        let root = workspace_root()?;
        let defined = decision_numbers(&root)?;
        let exists = |relative: &str| root.join(relative).exists();
        let pages = pages()?;
        let failures: Vec<String> = pages
            .iter()
            .flat_map(|page| problems(page, &defined, &exists))
            .collect();
        assert!(
            defined.len() > 300 && pages.len() > 20,
            "found {} decisions and {} pages; has the layout moved?",
            defined.len(),
            pages.len()
        );
        assert!(
            failures.is_empty(),
            "{} Markdown problem(s):\n{}",
            failures.len(),
            failures.join("\n")
        );
        Ok(())
    }

    #[test]
    fn decision_headings_are_read_and_citations_found() {
        let defined = defined_decisions(
            "# Decisions\n### D1: native GL (2026-10-08)\n### D12: x\nD99 in prose\n",
        );
        assert_eq!(defined, BTreeSet::from([1, 12]), "headings only");
        assert_eq!(
            cited_decisions("(D41, D2-D5) on D3D11, HDR, D12x, see D7."),
            vec![41, 2, 5, 7],
            "D3D11, HDR and D12x are not citations"
        );
    }

    #[test]
    fn link_targets_are_found_and_resolved() {
        assert_eq!(
            link_targets("see [a](../b.md#x) and ![p](<img/p q.png> \"t\") (not this)"),
            vec!["../b.md#x", "img/p q.png"],
            "two targets"
        );
        assert_eq!(
            without_code_spans("`v[4](x)` and [a](b.md) and `open"),
            " and [a](b.md) and `open",
            "a closed span goes, an unclosed backtick stays"
        );
        assert_eq!(local_path("https://x.org/a"), None, "web link");
        assert_eq!(local_path("mailto:a@b"), None, "mail");
        assert_eq!(local_path("#anchor"), None, "in-page");
        assert_eq!(
            local_path("a.md#x"),
            Some("a.md".to_string()),
            "anchor dropped"
        );
        assert_eq!(
            resolve("crates/strobe-gfx", "../../docs/D.md"),
            Some("docs/D.md".to_string()),
            "up and over"
        );
        assert_eq!(resolve("docs", "../../x.md"), None, "out of the root");
    }

    #[test]
    fn a_dead_link_and_an_unknown_decision_are_reported_outside_code() {
        let page = Page {
            relative: "docs/ROADMAP.md".to_string(),
            text: "Per D1 and D2, see [plan](PLAN.md) and [gone](GONE.md).\n\
                   ```\nD3 [x](NOPE.md) in a fence\n```\n"
                .to_string(),
        };
        let defined = BTreeSet::from([1]);
        let exists = |relative: &str| relative == "docs/PLAN.md";
        let found = problems(&page, &defined, &exists);
        assert_eq!(found.len(), 2, "D2 and GONE.md only: {found:?}");
        assert!(
            found.iter().any(|problem| problem.contains("cites D2")),
            "got {found:?}"
        );
        assert!(
            found.iter().any(|problem| problem.contains("GONE.md")),
            "got {found:?}"
        );
    }
}
