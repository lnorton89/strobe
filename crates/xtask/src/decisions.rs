// ============================================================================
// Module:       xtask::decisions
// Description:  `cargo xtask decisions`: writes docs/DECISIONS_BY_AREA.md, the
//               decision records indexed by the crates and areas each one
//               names, and checks it in CI.
//
// Dependencies: anyhow, std::fs
// ============================================================================

//! The decisions, by area (D539).
//!
//! `docs/decisions/` holds one file per decision, numbered in the order
//! they were made, several hundred of them. Before changing a crate you
//! want the decisions about that crate, not all of them. This index lists
//! each decision under every crate it names (`strobe-remote` or
//! `strobe_remote`), under "CI and releases" when it names a workflow, and
//! under "Not tied to one crate" when it names none. The numbers and the
//! files stay as they are. It is generated, so it can never fall behind:
//! the `generated` lane fails when the committed file differs from a fresh
//! one.

use anyhow::{Context as _, Result, bail};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::Path;

/// The generated file, relative to the workspace root.
const INDEX: &str = "docs/DECISIONS_BY_AREA.md";

/// The decision files, relative to the workspace root.
const FOLDER: &str = "docs/decisions";

/// The area for a decision that names a workflow.
const CI: &str = "CI and releases";

/// The area for a decision that names no crate and no workflow.
const NONE: &str = "Not tied to one crate";

/// Writes the index, or with `check` fails when the committed one differs.
pub(crate) fn run(check: bool) -> Result<()> {
    let root = crate::workspace_root()?;
    let crates = crate_names(&root)?;
    let decisions = read(&root.join(FOLDER))?;
    let text = render(&decisions, &crates);
    let path = root.join(INDEX);
    if check {
        let committed = std::fs::read_to_string(&path).unwrap_or_default();
        if committed.replace("\r\n", "\n") != text {
            bail!("{INDEX} is out of date; run `cargo xtask decisions` and commit it");
        }
        return Ok(());
    }
    std::fs::write(&path, text).with_context(|| format!("cannot write {INDEX}"))?;
    println!("wrote {INDEX} ({} decisions)", decisions.len());
    Ok(())
}

/// The workspace's crates, by folder name.
fn crate_names(root: &Path) -> Result<Vec<String>> {
    let mut names: Vec<String> = std::fs::read_dir(root.join("crates"))?
        .filter_map(Result::ok)
        .filter(|entry| entry.path().join("Cargo.toml").is_file())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    Ok(names)
}

/// One decision: its number, its title, and its whole text.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Decision {
    number: u32,
    title: String,
    text: String,
}

/// Every `D<n>.md` in the folder, in number order.
fn read(folder: &Path) -> Result<Vec<Decision>> {
    let mut decisions = Vec::new();
    for entry in std::fs::read_dir(folder).with_context(|| format!("cannot list {FOLDER}"))? {
        let path = entry?.path();
        let Some(number) = path
            .file_stem()
            .and_then(|stem| stem.to_str())
            .and_then(|stem| stem.strip_prefix('D'))
            .and_then(|digits| digits.parse::<u32>().ok())
        else {
            continue;
        };
        let text = std::fs::read_to_string(&path)?;
        decisions.push(Decision {
            number,
            title: title(number, &text),
            text,
        });
    }
    decisions.sort_by_key(|d| d.number);
    Ok(decisions)
}

/// The heading without `# D<n>: ` and without its closing `(date, who)`.
fn title(number: u32, text: &str) -> String {
    let heading = text.lines().next().unwrap_or_default();
    let prefix = format!("# D{number}:");
    let rest = heading.strip_prefix(&prefix).unwrap_or(heading).trim();
    match rest.rfind(" (") {
        Some(at) if rest.ends_with(')') => rest[..at].trim().to_owned(),
        Some(_) | None => rest.to_owned(),
    }
}

/// The areas a decision's text names: each crate, by its package or its
/// library name, and CI when it names a workflow.
fn areas(text: &str, crates: &[String]) -> BTreeSet<String> {
    let mut found: BTreeSet<String> = crates
        .iter()
        .filter(|krate| {
            let library = krate.replace('-', "_");
            names(text, krate) || names(text, &library)
        })
        .cloned()
        .collect();
    if ["ci.yml", "release.yml", "scheduled.yml", "workflow"]
        .iter()
        .any(|needle| text.contains(needle))
    {
        found.insert(CI.to_owned());
    }
    found
}

/// Whether `text` names `word` as a whole word (so `strobe-io` is not found
/// inside `strobe-iox`).
fn names(text: &str, word: &str) -> bool {
    let is_word = |c: char| c.is_ascii_alphanumeric() || c == '_' || c == '-';
    text.match_indices(word).any(|(at, _)| {
        let before = text[..at].chars().next_back();
        let after = text[at + word.len()..].chars().next();
        !before.is_some_and(is_word) && !after.is_some_and(is_word)
    })
}

/// The index's text.
fn render(decisions: &[Decision], crates: &[String]) -> String {
    let mut by_area: BTreeMap<String, Vec<&Decision>> = BTreeMap::new();
    for decision in decisions {
        let found = areas(&decision.text, crates);
        if found.is_empty() {
            by_area.entry(NONE.to_owned()).or_default().push(decision);
        }
        for area in found {
            by_area.entry(area).or_default().push(decision);
        }
    }
    let mut out = String::from(
        "# Decisions by area\n\n\
         Generated by `cargo xtask decisions` from [`decisions/`](decisions/); do not edit\n\
         it by hand (D539). Each decision is listed under every crate it names, under\n\
         \"CI and releases\" when it names a workflow, and under \"Not tied to one\n\
         crate\" when it names none. The log itself, and how to add to it, is\n\
         [`DECISIONS.md`](DECISIONS.md).\n\n",
    );
    let order: Vec<&String> = by_area
        .keys()
        .filter(|area| area.as_str() != CI && area.as_str() != NONE)
        .chain(by_area.keys().filter(|area| area.as_str() == CI))
        .chain(by_area.keys().filter(|area| area.as_str() == NONE))
        .collect();
    for area in &order {
        let _ = writeln!(out, "- [{area}](#{})", anchor(area));
    }
    for area in order {
        let list = by_area.get(area).map(Vec::as_slice).unwrap_or_default();
        let _ = write!(out, "\n## {area}\n\n{} decision(s).\n\n", list.len());
        for decision in list {
            let _ = writeln!(
                out,
                "- [D{0}](decisions/D{0}.md): {1}",
                decision.number, decision.title
            );
        }
    }
    out
}

/// GitHub's anchor for a heading: lower case, spaces to hyphens, other
/// punctuation dropped.
fn anchor(heading: &str) -> String {
    heading
        .chars()
        .filter_map(|c| match c {
            ' ' => Some('-'),
            c if c.is_ascii_alphanumeric() || c == '-' || c == '_' => Some(c.to_ascii_lowercase()),
            _ => None,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn crates() -> Vec<String> {
        ["strobe-io", "strobe-remote", "xtask"]
            .iter()
            .map(|s| (*s).to_owned())
            .collect()
    }

    #[test]
    fn a_title_loses_its_number_and_its_date() {
        assert_eq!(
            title(
                12,
                "# D12: ANGLE is the fallback (2026-10-01, owner)\n\nbody"
            ),
            "ANGLE is the fallback",
            "number and parenthetical gone"
        );
        assert_eq!(title(3, "# D3: plain\n"), "plain", "no date");
    }

    #[test]
    fn areas_are_whole_crate_names_in_either_spelling() {
        let found = areas(
            "strobe_remote's pairing and `cargo xtask ci`, not strobe-iox; see ci.yml",
            &crates(),
        );
        assert_eq!(
            found,
            BTreeSet::from([
                "strobe-remote".to_owned(),
                "xtask".to_owned(),
                CI.to_owned()
            ]),
            "library spelling counts, a longer word does not"
        );
    }

    #[test]
    fn a_decision_naming_nothing_has_its_own_section() {
        let decisions = vec![
            Decision {
                number: 1,
                title: "one".to_owned(),
                text: "about strobe-io".to_owned(),
            },
            Decision {
                number: 2,
                title: "two".to_owned(),
                text: "about nothing".to_owned(),
            },
        ];
        let text = render(&decisions, &crates());
        assert!(
            text.contains("## strobe-io\n\n1 decision(s).\n\n- [D1](decisions/D1.md): one"),
            "{text}"
        );
        assert!(
            text.contains(
                "## Not tied to one crate\n\n1 decision(s).\n\n- [D2](decisions/D2.md): two"
            ),
            "{text}"
        );
        assert!(
            text.contains("- [Not tied to one crate](#not-tied-to-one-crate)"),
            "{text}"
        );
    }
}
