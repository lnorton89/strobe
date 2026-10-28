// ============================================================================
// Module:       strobe_conventions::actions
// Description:  Checks that every workflow pins its actions to full commit
//               SHAs with a version comment, declares its permissions, runs
//               cargo `--locked`, and gives every job a timeout.
//
// Dependencies: anyhow; crate::toolchain (the workflow files)
// ============================================================================

//! Supply-chain and runner hygiene for `.github/workflows/`.
//!
//! Four rules. The first two are about what a workflow is allowed to
//! trust:
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
//! The other two keep what CI proves honest (D517):
//!
//! - **Every cargo command that resolves dependencies passes `--locked`.**
//!   Without it a Cargo.lock that no longer matches the manifests is
//!   quietly rewritten on a throwaway runner, and the run tests a
//!   dependency set nobody committed. `cargo fmt` reads no lockfile and is
//!   exempt; so is any cargo plugin not listed in [`RESOLVING`].
//! - **Every job has `timeout-minutes:`** (a job that only calls a
//!   reusable workflow cannot, and is exempt: the called jobs carry their
//!   own). GitHub's default is six hours, which a hung GUI test spends in
//!   full.
//!
//! The check is textual, like the toolchain one beside it: workflow YAML
//! here is simple enough that a line scan is exact, and it keeps the crate
//! free of a YAML dependency.

/// The length of a full git commit SHA in hex digits.
const SHA_LEN: usize = 40;

/// Cargo subcommands that resolve the dependency graph, and so would
/// rewrite a stale Cargo.lock without `--locked`. `xtask` is the workspace
/// alias for `run -p xtask`.
const RESOLVING: [&str; 8] = [
    "build", "check", "clippy", "doc", "install", "run", "test", "xtask",
];

/// The cargo command on one workflow line that resolves dependencies
/// without `--locked`, or none.
///
/// The line may run cargo under a wrapper (`xvfb-run -a … cargo run`), and
/// flags may come before the subcommand (`cargo --locked xtask`). Comment
/// lines are prose and are skipped.
pub fn unlocked_cargo(line: &str) -> Option<String> {
    let trimmed = line.trim_start();
    if trimmed.starts_with('#') {
        return None;
    }
    let mut words = trimmed.split_whitespace();
    words.by_ref().find(|word| *word == "cargo")?;
    let rest: Vec<&str> = words.collect();
    let subcommand = rest.iter().find(|word| !word.starts_with('-'))?;
    if !RESOLVING.contains(subcommand) || rest.contains(&"--locked") {
        return None;
    }
    Some(format!(
        "`cargo {subcommand}` without `--locked` (`{}`)",
        trimmed
            .trim_start_matches("- ")
            .trim_start_matches("run:")
            .trim()
    ))
}

/// The jobs in a workflow that set no `timeout-minutes:`, by id.
///
/// A job is a two-space-indented key under the top-level `jobs:` block. A
/// job that calls a reusable workflow (`uses:` at the job's own level)
/// cannot take a timeout and is skipped.
pub fn jobs_without_timeout(text: &str) -> Vec<String> {
    /// One job's id and whether it has what it needs.
    struct Job {
        id: String,
        covered: bool,
    }
    let mut jobs: Vec<Job> = Vec::new();
    let mut in_jobs = false;
    for line in text.lines() {
        if line.trim().is_empty() || line.trim_start().starts_with('#') {
            continue;
        }
        if !line.starts_with(' ') {
            in_jobs = line.trim_end() == "jobs:";
            continue;
        }
        if !in_jobs {
            continue;
        }
        let job_key = line
            .strip_prefix("  ")
            .filter(|rest| !rest.starts_with(' '))
            .and_then(|rest| rest.trim_end().strip_suffix(':'));
        if let Some(id) = job_key {
            jobs.push(Job {
                id: id.to_owned(),
                covered: false,
            });
            continue;
        }
        let own_level = line
            .strip_prefix("    ")
            .filter(|rest| !rest.starts_with(' '));
        if let (Some(key), Some(job)) = (own_level, jobs.last_mut())
            && (key.starts_with("timeout-minutes:") || key.starts_with("uses:"))
        {
            job.covered = true;
        }
    }
    jobs.into_iter()
        .filter(|job| !job.covered)
        .map(|job| job.id)
        .collect()
}

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
    found.extend(
        text.lines()
            .filter_map(unlocked_cargo)
            .map(|problem| format!("{name}: {problem}")),
    );
    found.extend(
        jobs_without_timeout(text)
            .into_iter()
            .map(|job| format!("{name}: job `{job}` has no `timeout-minutes:`")),
    );
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

    #[test]
    fn a_resolving_cargo_command_needs_locked() {
        for line in [
            "        run: cargo test --workspace --all-features",
            "      - run: cargo xtask changelog --check",
            "          xvfb-run -a -s \"-screen 0 1280x720x24\" cargo run -p strobe-app",
            "          cargo install cargo-cyclonedx --version 0.5.9",
        ] {
            assert!(
                unlocked_cargo(line).is_some(),
                "`{line}` resolves dependencies and must be reported"
            );
        }
    }

    #[test]
    fn locked_formatting_and_prose_pass() {
        for line in [
            "        run: cargo test --locked --workspace",
            "      - run: cargo --locked xtask changelog --check",
            "          xvfb-run -a cargo run --locked -p strobe-app",
            "      - run: cargo fmt --all -- --check",
            "          cargo cyclonedx \\",
            "      # `cargo xtask bench` is the way to take real numbers.",
            "      - run: echo no cargo here",
        ] {
            assert_eq!(unlocked_cargo(line), None, "`{line}` is fine");
        }
    }

    #[test]
    fn a_job_without_a_timeout_is_named() {
        let text = "\
name: X
permissions:
  contents: read
jobs:
  timed:
    runs-on: ubuntu-latest
    timeout-minutes: 10
    steps:
      - run: echo
  untimed:
    runs-on: ubuntu-latest
    steps:
      # timeout-minutes: 10 in a comment does not count
      - name: A step's own timeout does not cover the job
        timeout-minutes: 5
        run: echo
  called:
    needs: timed
    uses: ./.github/workflows/ci.yml
";
        assert_eq!(
            jobs_without_timeout(text),
            vec!["untimed".to_owned()],
            "only the job with no timeout of its own is reported"
        );
    }

    #[test]
    fn keys_outside_jobs_are_not_jobs() {
        let text = "\
on:
  push:
    branches: [master]
concurrency:
  group: x
jobs:
  only:
    timeout-minutes: 5
";
        assert!(
            jobs_without_timeout(text).is_empty(),
            "`push` and `group` are not jobs"
        );
    }
}
