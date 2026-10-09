// ============================================================================
// Module:       strobe_conventions::actions
// Description:  Checks that every workflow pins its actions to full commit
//               SHAs with a version comment, and declares its permissions.
//
// Dependencies: anyhow; crate::toolchain (the workflow files)
// ============================================================================

//! Supply-chain hygiene for `.github/workflows/`.
//!
//! Two rules, both about what a workflow is allowed to trust:
//!
//! - **Actions are pinned to a full 40-hex commit SHA, followed by a
//!   comment naming the version.** A tag (`@v4`) is mutable: whoever
//!   controls the action's repository can move it, and the next run of a
//!   workflow holding a write-capable token executes whatever it now
//!   points at. A SHA cannot move, and the trailing comment is what lets a
//!   human (and Dependabot, which rewrites both together) see which
//!   release it is. Local actions (`./…`) are exempt.
//! - **Every workflow declares top-level `permissions:`.** Without it the
//!   token gets the repository's default, which may be read-write. Naming
//!   it makes least privilege the starting point and every elevation a
//!   visible line in a job.
//!
//! The check is textual, like the toolchain one beside it: workflow YAML
//! here is simple enough that a line scan is exact, and it keeps the crate
//! free of a YAML dependency.

/// The length of a full git commit SHA in hex digits.
const SHA_LEN: usize = 40;

/// Problems with one `uses:` line, or none.
///
/// `line` is the whole line as it appears in the file.
pub fn uses_problem(line: &str) -> Option<String> {
    let trimmed = line.trim_start().trim_start_matches("- ").trim_start();
    let rest = trimmed.strip_prefix("uses:")?.trim();
    // Local actions and reusable-workflow paths inside this repository are
    // versioned with the commit that calls them.
    if rest.starts_with("./") {
        return None;
    }
    let (reference, comment) = match rest.split_once('#') {
        Some((reference, comment)) => (reference.trim(), Some(comment.trim())),
        None => (rest, None),
    };
    let Some((_, revision)) = reference.rsplit_once('@') else {
        return Some(format!("`{reference}` has no @revision at all"));
    };
    let is_sha = revision.len() == SHA_LEN && revision.chars().all(|c| c.is_ascii_hexdigit());
    if !is_sha {
        return Some(format!(
            "`{reference}` is pinned to `{revision}`, a mutable ref; pin the full commit SHA"
        ));
    }
    match comment {
        Some(text) if !text.is_empty() => None,
        Some(_) | None => Some(format!(
            "`{reference}` is pinned by SHA but has no trailing `# <version>` comment"
        )),
    }
}

/// Every problem in one workflow's text.
pub fn problems(name: &str, text: &str) -> Vec<String> {
    let mut found: Vec<String> = text
        .lines()
        .filter_map(|line| uses_problem(line).map(|problem| format!("{name}: {problem}")))
        .collect();
    if !text.lines().any(|line| line.starts_with("permissions:")) {
        found.push(format!(
            "{name}: no top-level `permissions:`; declare `contents: read` and elevate per job"
        ));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolchain::workflows;
    use anyhow::Result;

    const SHA: &str = "3d3c42e5aac5ba805825da76410c181273ba90b1";

    #[test]
    fn every_workflow_pins_its_actions_and_declares_permissions() -> Result<()> {
        let workflows = workflows()?;
        assert!(!workflows.is_empty(), "no workflow found to check");
        let failures: Vec<String> = workflows
            .iter()
            .flat_map(|(name, text)| problems(name, text))
            .collect();
        assert!(
            failures.is_empty(),
            "{} workflow problem(s):\n{}",
            failures.len(),
            failures.join("\n")
        );
        Ok(())
    }

    #[test]
    fn a_pinned_action_with_a_version_comment_passes() {
        let line = format!("      - uses: actions/checkout@{SHA} # v7.0.1");
        assert_eq!(
            uses_problem(&line),
            None,
            "a pinned, commented action is fine"
        );
        let inline = format!("        uses: actions/upload-artifact@{SHA} # v7.0.1");
        assert_eq!(uses_problem(&inline), None, "the key form is fine too");
    }

    #[test]
    fn a_mutable_tag_is_rejected() {
        let problem = uses_problem("      - uses: actions/checkout@v4");
        assert!(
            problem.is_some_and(|text| text.contains("mutable")),
            "a tag must be reported as mutable"
        );
        let branch = uses_problem("      - uses: dtolnay/rust-toolchain@stable");
        assert!(branch.is_some(), "a branch name must be reported");
    }

    #[test]
    fn a_short_sha_is_rejected() {
        let line = "      - uses: actions/checkout@3d3c42e # v7.0.1";
        assert!(
            uses_problem(line).is_some(),
            "an abbreviated SHA is not a pin"
        );
    }

    #[test]
    fn a_pin_without_a_version_comment_is_rejected() {
        let line = format!("      - uses: actions/checkout@{SHA}");
        let problem = uses_problem(&line);
        assert!(
            problem.is_some_and(|text| text.contains("comment")),
            "a bare SHA hides which release it is"
        );
    }

    #[test]
    fn local_actions_are_exempt() {
        assert_eq!(
            uses_problem("      - uses: ./.github/actions/setup"),
            None,
            "a local action is versioned with its caller"
        );
    }

    #[test]
    fn a_workflow_without_permissions_is_rejected() {
        let found = problems("x.yml", "name: X\non: push\njobs: {}\n");
        assert!(
            found.iter().any(|text| text.contains("permissions")),
            "a missing top-level permissions block must be reported, got {found:?}"
        );
        let fine = problems("x.yml", "name: X\npermissions:\n  contents: read\n");
        assert!(fine.is_empty(), "a declared block passes, got {fine:?}");
    }
}
