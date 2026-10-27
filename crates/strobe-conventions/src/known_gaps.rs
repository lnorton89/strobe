// ============================================================================
// Module:       strobe_conventions::known_gaps
// Description:  Checks that every gap a crate README lists under "Status and
//               known gaps" is tracked in the backlog, is an open question,
//               or is by design, and that the backlog IDs it cites exist.
//
// Dependencies: anyhow, std::fs; crate::sources (the workspace root)
// ============================================================================

//! One register of open work (D490).
//!
//! Each crate's README ends with "Status and known gaps": what works, on
//! which machine, and what does not yet. Those sections were written crate
//! by crate before `docs/IMPLEMENTATION_BACKLOG.md` became the register of
//! open work, nothing tied the two together, and gaps ended up written in
//! one place only, or went stale in one while the other moved on.
//!
//! The rule: a bullet in that section that says something is missing,
//! unverified or still to do names where it is tracked. That is a backlog
//! item (`backlog R-01`), an open question (`Q10`), or the words "by
//! design" beside the decision that made it so. And every backlog ID a
//! README cites must be an item in the backlog, so a renamed or invented ID
//! fails too.
//!
//! What counts as a gap is a list of phrases ([`GAP_PHRASES`]): blunt, and
//! easy to satisfy honestly. The check is about the pointer existing; the
//! backlog item is where the work is described.

use crate::sources::workspace_root;
use anyhow::{Result, anyhow};
use std::fs;

/// Words that mark a bullet as a gap, matched case-insensitively.
pub const GAP_PHRASES: [&str; 17] = [
    "not built",
    "not yet",
    "is not built",
    "unverified",
    "unmeasured",
    "untested",
    "left:",
    "still to",
    "still open",
    "still wanted",
    "not generated",
    "not enforced",
    "not checked",
    "not reproduced",
    "has not",
    "have not",
    "no midi controller",
];

/// The section's heading.
const HEADING: &str = "## Status and known gaps";

/// The epics' letters, as in `R-01`.
const EPICS: &[u8] = b"RUCIPMLGD";

/// Every backlog ID cited in `text`: an epic letter, `-`, two digits, not
/// inside a longer word.
pub fn backlog_ids(text: &str) -> Vec<String> {
    let bytes = text.as_bytes();
    let mut found = Vec::new();
    for start in 0..bytes.len().saturating_sub(3) {
        let Some(&[letter, dash, tens, ones]) = bytes.get(start..start + 4) else {
            continue;
        };
        let before_ok = start == 0
            || bytes
                .get(start - 1)
                .is_some_and(|b| !b.is_ascii_alphanumeric());
        let after_ok = bytes
            .get(start + 4)
            .is_none_or(|b| !b.is_ascii_alphanumeric());
        if EPICS.contains(&letter)
            && dash == b'-'
            && tens.is_ascii_digit()
            && ones.is_ascii_digit()
            && before_ok
            && after_ok
        {
            found.push(String::from_utf8_lossy(&[letter, dash, tens, ones]).into_owned());
        }
    }
    found
}

/// Whether `text` names an open question (`Q` and a number).
fn cites_question(text: &str) -> bool {
    let bytes = text.as_bytes();
    bytes.windows(2).enumerate().any(|(i, pair)| {
        matches!(pair, [b'Q', digit] if digit.is_ascii_digit())
            && (i == 0 || bytes.get(i - 1).is_some_and(|b| !b.is_ascii_alphanumeric()))
    })
}

/// The bullets of `readme`'s "Status and known gaps" section, each joined
/// into one line. Empty when the README has no such section.
pub fn gap_bullets(readme: &str) -> Vec<String> {
    let mut bullets: Vec<String> = Vec::new();
    let mut inside = false;
    for line in readme.lines() {
        if line.starts_with("## ") {
            inside = line.trim() == HEADING;
            continue;
        }
        if !inside {
            continue;
        }
        if let Some(rest) = line.strip_prefix("- ") {
            bullets.push(rest.to_string());
        } else if line.starts_with("  ")
            && let Some(last) = bullets.last_mut()
        {
            last.push(' ');
            last.push_str(line.trim());
        }
    }
    bullets
}

/// Why `bullet` breaks the rule, if it does.
pub fn untracked(bullet: &str) -> Option<String> {
    let lower = bullet.to_lowercase();
    let phrase = GAP_PHRASES.iter().find(|p| lower.contains(*p))?;
    let tracked =
        !backlog_ids(bullet).is_empty() || cites_question(bullet) || lower.contains("by design");
    (!tracked).then(|| format!("says \"{phrase}\" but names no backlog item, question or decision"))
}

/// Every crate README, as `(path, text)` pairs.
pub fn readmes() -> Result<Vec<(String, String)>> {
    let crates = workspace_root()?.join("crates");
    let mut found = Vec::new();
    for entry in fs::read_dir(&crates)? {
        let path = entry?.path().join("README.md");
        if path.is_file() {
            let name = path.to_string_lossy().replace('\\', "/");
            found.push((name, fs::read_to_string(&path)?));
        }
    }
    found.sort();
    Ok(found)
}

/// The backlog's item IDs, from its `### X-NN` headings.
pub fn backlog_items() -> Result<Vec<String>> {
    let path = workspace_root()?
        .join("docs")
        .join("IMPLEMENTATION_BACKLOG.md");
    let text = fs::read_to_string(&path).map_err(|e| anyhow!("{}: {e}", path.display()))?;
    Ok(text
        .lines()
        .filter_map(|line| line.strip_prefix("### "))
        .flat_map(|heading| backlog_ids(heading).into_iter().take(1))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_known_gap_names_where_it_is_tracked() -> Result<()> {
        let mut problems = Vec::new();
        for (path, text) in readmes()? {
            for bullet in gap_bullets(&text) {
                if let Some(why) = untracked(&bullet) {
                    let start: String = bullet.chars().take(70).collect();
                    problems.push(format!("{path}: \"{start}...\" {why}"));
                }
            }
        }
        assert!(
            problems.is_empty(),
            "add the gap to docs/IMPLEMENTATION_BACKLOG.md and cite it (D490):\n{}",
            problems.join("\n")
        );
        Ok(())
    }

    #[test]
    fn every_backlog_id_a_readme_cites_is_an_item() -> Result<()> {
        let items = backlog_items()?;
        assert!(
            items.len() > 20,
            "the backlog's items were found: {items:?}"
        );
        let mut missing = Vec::new();
        for (path, text) in readmes()? {
            for id in backlog_ids(&text) {
                if !items.contains(&id) {
                    missing.push(format!("{path}: {id}"));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "IDs with no `### ` item in the backlog:\n{}",
            missing.join("\n")
        );
        Ok(())
    }

    #[test]
    fn a_gap_needs_a_pointer_and_a_status_line_does_not() {
        assert!(
            untracked("Video and webcam are not built.").is_some(),
            "a bare gap"
        );
        assert!(
            untracked("Video and webcam are not built (backlog M-04).").is_none(),
            "a backlog item"
        );
        assert!(
            untracked("The layout is unverified (Q10).").is_none(),
            "an open question"
        );
        assert!(
            untracked("No scripts run, by design (D239).").is_none(),
            "by design"
        );
        assert!(
            untracked("Built and tested on the T480 (Task 8).").is_none(),
            "not a gap"
        );
    }

    #[test]
    fn ids_are_whole_words() {
        assert_eq!(
            backlog_ids("backlog R-01, U-09; not XR-01 or R-012 or D-5"),
            ["R-01", "U-09"],
            "two IDs"
        );
        assert!(cites_question("(Q10)"), "a question");
        assert!(!cites_question("FAQ1"), "inside a word");
    }

    #[test]
    fn bullets_are_read_from_the_section_only() {
        let readme = "# x\n\n## Status and known gaps\n\nPointer.\n\n- one\n  more\n- two\n\n## Next\n\n- not this\n";
        assert_eq!(
            gap_bullets(readme),
            ["one more", "two"],
            "the section's bullets"
        );
    }
}
