// ============================================================================
// Module:       xtask::changelog
// Description:  Regenerates CHANGELOG.md from the git history, grouping
//               conventional-commit subjects under the release tag that
//               shipped them.
//
// Dependencies: anyhow; std::process::Command (shells out to git). No crates
//               beyond what the workspace already pulls in — the calendar
//               conversion is inlined rather than adding a date crate.
// ============================================================================

//! `cargo xtask changelog`: the changelog is derived, not authored.
//!
//! Regenerate the file:
//!
//! ```sh
//! cargo xtask changelog
//! ```
//!
//! check the committed file against the history without rewriting it:
//!
//! ```sh
//! cargo xtask changelog --check
//! ```
//!
//! or write the section for a release that is *about to be tagged*, so
//! the tag can be placed on a commit whose changelog is already final:
//!
//! ```sh
//! cargo xtask changelog --release v0.2.0
//! ```
//!
//! This is a port of rustaman's `examples/changelog.rs`, with the same
//! behaviour and the same file format, so the two repositories' changelogs
//! read and check identically. What differs is where it lives (a workspace
//! tool rather than an example of one package), the repository URL (read
//! from the workspace manifest rather than hard-coded), and that it has
//! unit tests.
//!
//! `--release` exists because of rustaman's `v0.2.1`: the tag was placed
//! first and the changelog regenerated after, so the tag — and the source
//! archives GitHub serves for it, forever — carried a changelog listing the
//! whole release under `Unreleased`. Writing the section before tagging
//! means the tagged tree is the finished one. The commit that adds the
//! generated changelog cannot list itself (its hash does not exist while
//! the file is being written), which is why commits touching only
//! `CHANGELOG.md` are excluded from every section, on the generate and
//! check paths alike — the two must always agree.
//!
//! Every entry is a conventional-commit subject (`fix:`, `feat:`,
//! `refactor:` …) resolved against `git tag`, so the released sections
//! cannot say something the history does not. A hand-maintained changelog
//! drifts silently, and the drift is only ever discovered on release day.
//!
//! Consequences worth knowing before you edit the file by hand:
//!
//! - **Released sections are rewritten on every run.** Prose added to one
//!   is lost the next time this runs. If a release needs a human summary,
//!   the place for it is the GitHub release body.
//! - **`--check` compares released sections only.** `Unreleased` is
//!   exempt, because its content changes with every merge — and a squash
//!   or rebase rewrites the very hashes it would be pinned to. Gating a
//!   pull request on it would fail every pull request for no signal.
//! - **Merge commits are not listed.** `git log --no-merges` drops them:
//!   their subjects (`Merge pull request #1 …`) are not conventional, and
//!   the commits they bring in are listed individually, so nothing a merge
//!   carried is lost. A *squash* merge is a single ordinary commit and is
//!   listed under whatever type its subject carries — so a squashed PR
//!   title should be a conventional subject too.
//! - **A version tag no branch can reach is a hard error under `--check`.**
//!   Each release is computed as `<previous>..<tag>`, so a tag left
//!   pointing at an amended-away commit makes every range from there on
//!   wrong. See [`orphan_report`] for the symptom and the fix.

use anyhow::{Context as _, Result, anyhow, bail};
use std::fs;
use std::path::Path;
use std::process::Command;

/// The generated file, relative to the workspace root.
const CHANGELOG: &str = "CHANGELOG.md";

/// Heading for commits that have not shipped in a tagged release yet.
const UNRELEASED: &str = "## [Unreleased]";

/// Field separator for `git log --format`. `%x1f` is the ASCII unit
/// separator, which cannot occur in a commit subject.
const SEP: char = '\u{1f}';

/// Conventional-commit types, in the order their sections appear within a
/// release, mapped to the heading each lands under.
///
/// Anything whose subject does not parse as a conventional commit falls to
/// `Other`.
const SECTIONS: &[(&str, &[&str])] = &[
    ("Added", &["feat"]),
    ("Fixed", &["fix"]),
    ("Performance", &["perf"]),
    ("Changed", &["refactor", "style"]),
    ("Documentation", &["docs"]),
    ("Internal", &["build", "ci", "chore"]),
    ("Tests", &["test"]),
];

/// Heading for breaking changes, which lead a release regardless of the
/// type they were committed under.
const BREAKING: &str = "Breaking changes";

/// Heading for subjects that are not conventional commits.
const OTHER: &str = "Other";

/// One commit, already parsed into the pieces a changelog line needs.
struct Commit {
    hash: String,
    short: String,
    /// The heading this commit belongs under.
    section: &'static str,
    /// The `(scope)` of a conventional commit, if it carried one.
    scope: Option<String>,
    /// The subject with the `type(scope):` prefix stripped.
    summary: String,
}

/// One tagged release and everything that landed since the previous tag.
struct Release {
    /// The tag as git spells it, e.g. `v0.2.0`.
    tag: String,
    /// The tag without its leading `v`, e.g. `0.2.0`.
    version: String,
    /// Committer date of the tagged commit, `YYYY-MM-DD`.
    date: String,
    /// The tag this release is compared against, if any.
    previous: Option<String>,
    commits: Vec<Commit>,
}

/// Entry point for `cargo xtask changelog [--check | --release <version>]`.
///
/// Must run from the workspace root, which `main` arranges.
pub(crate) fn run(check: bool, release: Option<&str>) -> Result<()> {
    if check && release.is_some() {
        bail!("--check and --release are mutually exclusive");
    }
    if !Path::new("Cargo.toml").is_file() {
        bail!("run this from the workspace root, where Cargo.toml lives");
    }
    let manifest = fs::read_to_string("Cargo.toml").context("failed to read Cargo.toml")?;
    let repo = repository_url(&manifest)?;

    let mut releases = releases()?;

    // An orphaned tag makes every range below wrong, so it is reported
    // before any mismatch it would itself have caused.
    let orphaned = orphaned_tags(&releases)?;

    if check {
        if !orphaned.is_empty() {
            bail!("{}", orphan_report(&orphaned));
        }
        let existing = fs::read_to_string(CHANGELOG)
            .with_context(|| format!("{CHANGELOG} is missing; run `cargo xtask changelog`"))?;
        // A leading section whose tag does not exist yet is the pre-tag
        // release flow in progress, not drift — but it only passes if it
        // is exactly the release about to be cut: version newer than the
        // latest tag, matching Cargo.toml, content matching the history.
        if let Some(version) = pending_version(&existing, &releases) {
            let pending = pending_release(&version, &releases, &manifest).with_context(|| {
                format!(
                    "{CHANGELOG} has a [{version}] section but no such tag exists, \
                     and it does not validate as a release about to be cut"
                )
            })?;
            releases.insert(0, pending);
        }
        let generated = render(&releases, &[], &repo);
        if released_only(&existing) == released_only(&generated) {
            println!("{CHANGELOG} matches the git history.");
            return Ok(());
        }
        bail!(
            "{CHANGELOG} is out of date with the git history.\n\
             Regenerate it with `cargo xtask changelog` and commit the result."
        );
    }

    let unreleased = match release {
        // The pending release swallows everything that would have been
        // unreleased; the section it lands under is the one being cut.
        Some(version) => {
            let pending = pending_release(version, &releases, &manifest)?;
            releases.insert(0, pending);
            Vec::new()
        }
        None => match releases.first() {
            Some(latest) => commits(&format!("{}..HEAD", latest.tag))?,
            None => commits("HEAD")?,
        },
    };
    let generated = render(&releases, &unreleased, &repo);

    // A warning rather than an error on this path: the file still gets
    // written, so you are not locked out of regenerating while you sort the
    // tag out. `--check` is where it becomes a build failure.
    if !orphaned.is_empty() {
        eprintln!("warning: {}", orphan_report(&orphaned));
    }

    fs::write(CHANGELOG, &generated).with_context(|| format!("failed to write {CHANGELOG}"))?;
    println!(
        "wrote {CHANGELOG}: {} release(s), {} unreleased commit(s).",
        releases.len(),
        unreleased.len()
    );
    Ok(())
}

/// Runs git and returns its stdout, turning a non-zero exit into an error
/// rather than an empty string that would silently produce a blank section.
fn git(args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .with_context(|| format!("failed to run `git {}`", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!("`git {}` failed: {}", args.join(" "), stderr.trim());
    }

    String::from_utf8(output.stdout)
        .with_context(|| format!("`git {}` printed invalid UTF-8", args.join(" ")))
}

/// The `[workspace.package]` table's lines, without the header.
fn workspace_package_lines(manifest: &str) -> impl Iterator<Item = &str> {
    manifest
        .lines()
        .skip_while(|line| line.trim() != "[workspace.package]")
        .skip(1)
        .take_while(|line| !line.trim_start().starts_with('['))
}

/// A `key = "value"` entry of `[workspace.package]`.
fn workspace_package_value(manifest: &str, key: &str) -> Option<String> {
    workspace_package_lines(manifest).find_map(|line| {
        let (name, value) = line.split_once('=')?;
        if name.trim() != key {
            return None;
        }
        Some(value.trim().trim_matches('"').to_owned())
    })
}

/// The repository URL used for commit and compare links: the workspace
/// manifest's `repository` field, with any trailing slash or `.git` cut.
fn repository_url(manifest: &str) -> Result<String> {
    let url = workspace_package_value(manifest, "repository")
        .ok_or_else(|| anyhow!("Cargo.toml has no `repository` in [workspace.package]"))?;
    Ok(url
        .trim_end_matches('/')
        .trim_end_matches(".git")
        .to_owned())
}

/// Every version tag, newest release first.
fn releases() -> Result<Vec<Release>> {
    let mut tags: Vec<(String, (u64, u64, u64))> = Vec::new();
    for line in git(&["tag", "--list"])?.lines() {
        let tag = line.trim();
        if tag.is_empty() {
            continue;
        }
        if let Some(version) = parse_version(tag) {
            tags.push((tag.to_string(), version));
        }
    }

    // Oldest first, so each tag knows the one it follows.
    tags.sort_by_key(|(_, version)| *version);

    let mut releases = Vec::new();
    for (index, (tag, _)) in tags.iter().enumerate() {
        let previous = index
            .checked_sub(1)
            .and_then(|i| tags.get(i))
            .map(|(t, _)| t.clone());
        let range = match &previous {
            Some(prev) => format!("{prev}..{tag}"),
            // The first release reaches back to the root commit.
            None => tag.clone(),
        };
        releases.push(Release {
            version: tag.trim_start_matches('v').to_string(),
            date: tag_date(tag)?,
            previous,
            commits: commits(&range)?,
            tag: tag.clone(),
        });
    }

    // Newest first, which is the order a changelog is read in.
    releases.reverse();
    Ok(releases)
}

/// Every ref a tag is reachable from.
///
/// Empty means the tag points into history that no branch can reach — the
/// tagged commit was amended or rebased afterwards, and the tag was left
/// behind pointing at the discarded copy. The symptom is the release's own
/// commit showing up under `Unreleased`, because `<tag>..HEAD` spans a
/// fork rather than a range.
fn refs_containing(tag: &str) -> Result<Vec<String>> {
    let refs = git(&[
        "for-each-ref",
        "--contains",
        tag,
        "--format=%(refname)",
        "refs/heads",
        "refs/remotes",
    ])?;
    Ok(refs
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_string)
        .collect())
}

/// Version tags pointing at unreachable commits.
///
/// A checkout with no branch refs at all cannot answer the question, so
/// this reports nothing rather than condemning every tag — better to skip
/// the check than to fail a build for the shape of its clone.
fn orphaned_tags(releases: &[Release]) -> Result<Vec<String>> {
    let all_refs = git(&[
        "for-each-ref",
        "--format=%(refname)",
        "refs/heads",
        "refs/remotes",
    ])?;
    if all_refs.trim().is_empty() {
        return Ok(Vec::new());
    }

    let mut orphaned = Vec::new();
    for release in releases {
        if refs_containing(&release.tag)?.is_empty() {
            orphaned.push(release.tag.clone());
        }
    }
    Ok(orphaned)
}

/// What to tell someone who has just grown an orphaned tag.
fn orphan_report(tags: &[String]) -> String {
    let list = tags.join(", ");
    format!(
        "{list} point(s) at a commit no branch can reach.\n\
         A tag was placed, then the commit under it was amended or rebased, so the tag \
         now describes history that was thrown away. Every range computed from it is wrong.\n\
         \n\
         Re-point it at the commit that actually shipped, then force-push:\n\
         \n\
         \x20   git tag -f -a <tag> <commit> -m \"strobe <tag>\"\n\
         \x20   git push --force origin refs/tags/<tag>\n\
         \n\
         Pushing a `v*` tag re-triggers .github/workflows/release.yml. If the release is \
         already published, that run fails at `Create draft release` and changes nothing \
         (D520); the red run is expected."
    )
}

/// `v1.2.3` or `1.2.3` into a comparable tuple. Anything else is not a
/// release tag and is skipped rather than guessed at.
fn parse_version(tag: &str) -> Option<(u64, u64, u64)> {
    let mut parts = tag.trim_start_matches('v').split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

/// Committer time of the commit a tag points at, as a UTC date.
/// `^{commit}` dereferences an annotated tag, so both tag styles give
/// the same answer.
///
/// UTC rather than the committer's recorded offset, so the date
/// `--release` writes *before* the tag exists (from the wall clock, in
/// UTC) is the same date this reads back after the tag exists — with
/// local dates, a release cut near midnight in a non-UTC timezone would
/// fail its own `--check`.
fn tag_date(tag: &str) -> Result<String> {
    let spec = format!("{tag}^{{commit}}");
    let seconds: u64 = git(&["log", "-1", "--format=%ct", &spec])?
        .trim()
        .parse()
        .with_context(|| format!("{tag} has an unreadable committer timestamp"))?;
    let time = std::time::UNIX_EPOCH + std::time::Duration::from_secs(seconds);
    Ok(utc_date(time))
}

/// A `SystemTime` as a `YYYY-MM-DD` UTC date.
///
/// UTC on both the generate and the check path, so a tag pushed on one
/// side of midnight cannot disagree with a check run on the other. The
/// check compares headings with the date stripped anyway, but a
/// regeneration that flipped the date for no reason would show up as a
/// diff in every release section at once.
fn utc_date(time: std::time::SystemTime) -> String {
    let Ok(elapsed) = time.duration_since(std::time::UNIX_EPOCH) else {
        // Before 1970. Not reachable from a git committer timestamp, and a
        // wrong date is better than a panic in a tool whose whole job is
        // to be run unattended in CI.
        return "1970-01-01".to_owned();
    };
    let days = (elapsed.as_secs() / 86_400) as i64;
    let (year, month, day) = civil_from_days(days);
    format!("{year:04}-{month:02}-{day:02}")
}

/// Public-domain "days since the Unix epoch" into (year, month, day) on
/// the proleptic Gregorian calendar, by Howard Hinnant.
///
/// Reproduced here rather than pulled from a date crate: this is the
/// only place in the workspace that needs a calendar, and `chrono` or
/// `time` would be one more dependency audited by `cargo deny` for one
/// function.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let shifted = days + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = (shifted - era * 146_097) as u64;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let year = year_of_era as i64 + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u32;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

/// The section for a release that is about to be tagged: everything
/// since the latest tag, dated today, under the version the tag will
/// carry — so the tag can then be placed on a tree whose changelog is
/// already finished. See the module docs for the incident this exists to
/// prevent.
fn pending_release(version: &str, releases: &[Release], manifest: &str) -> Result<Release> {
    let tag = if version.starts_with('v') {
        version.to_string()
    } else {
        format!("v{version}")
    };
    let Some(parsed) = parse_version(&tag) else {
        bail!("`{version}` is not a version; expected something like v0.2.0");
    };
    if git(&["tag", "--list", &tag])?.trim() == tag {
        bail!("{tag} already exists — regenerate without `--release` instead");
    }
    if let Some(latest) = releases.first()
        && parse_version(&latest.tag).is_some_and(|latest_version| parsed <= latest_version)
    {
        bail!("{tag} is not newer than the latest tag, {}", latest.tag);
    }
    // The version bump commits first (see CONTRIBUTING.md), so a
    // mismatch here means the release is being cut against the wrong
    // tree — with the bump forgotten, or under the wrong number.
    let declared = workspace_package_value(manifest, "version")
        .ok_or_else(|| anyhow!("Cargo.toml has no `version` in [workspace.package]"))?;
    let wanted = tag.trim_start_matches('v');
    if declared != wanted {
        bail!(
            "Cargo.toml says `version = \"{declared}\"` but the release being cut is {tag}; \
             bump the version first"
        );
    }

    let range = match releases.first() {
        Some(latest) => format!("{}..HEAD", latest.tag),
        None => "HEAD".to_string(),
    };
    Ok(Release {
        version: wanted.to_string(),
        date: utc_date(std::time::SystemTime::now()),
        previous: releases.first().map(|r| r.tag.clone()),
        commits: commits(&range)?,
        tag,
    })
}

/// Parsed commits in a revision range, newest first. Merges are excluded —
/// they carry no subject of their own worth listing. So are commits that
/// touch only `CHANGELOG.md`: the changelog commit a release produces
/// cannot list itself (its hash does not exist while the file is being
/// generated), so such commits are defined out of every section instead,
/// which keeps the generate and check paths in exact agreement.
fn commits(range: &str) -> Result<Vec<Commit>> {
    let format = format!("--format=%H{SEP}%h{SEP}%s");
    // `--full-history` matters: a pathspec switches on git's history
    // simplification, which is allowed to prune a commit whose
    // path-limited tree matches a parent's — it silently dropped one of
    // a duplicate-subject pair from a section when this filter first
    // landed in rustaman. Full history keeps every commit whose own diff
    // touches the kept paths, which is the semantics actually wanted:
    // "skip a commit only when CHANGELOG.md is all it changed".
    let log = git(&[
        "log",
        "--no-merges",
        "--full-history",
        &format,
        range,
        "--",
        ".",
        ":(exclude)CHANGELOG.md",
    ])?;

    let mut commits = Vec::new();
    for line in log.lines() {
        let mut fields = line.split(SEP);
        let (Some(hash), Some(short), Some(subject)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        commits.push(parse_subject(hash, short, subject));
    }
    Ok(commits)
}

/// Splits a `type(scope)!: summary` subject into its parts, falling back to
/// the `Other` section for anything that is not a conventional commit.
fn parse_subject(hash: &str, short: &str, subject: &str) -> Commit {
    let plain = Commit {
        hash: hash.to_string(),
        short: short.to_string(),
        section: OTHER,
        scope: None,
        summary: subject.trim().to_string(),
    };

    let Some((head, rest)) = subject.split_once(':') else {
        return plain;
    };
    let summary = rest.trim();
    if summary.is_empty() {
        return plain;
    }

    // A trailing `!` is the conventional-commit marker for a breaking change.
    let breaking = head.ends_with('!');
    let head = head.strip_suffix('!').unwrap_or(head);

    let (kind, scope) = match head.split_once('(') {
        Some((kind, scope)) => match scope.strip_suffix(')') {
            Some(scope) if !scope.is_empty() => (kind, Some(scope.to_string())),
            // `feat(unclosed: ...` is not a conventional commit.
            _ => return plain,
        },
        None => (head, None),
    };

    if kind.is_empty() || !kind.chars().all(|c| c.is_ascii_lowercase()) {
        return plain;
    }

    let section = if breaking {
        BREAKING
    } else {
        let mut found = OTHER;
        for (heading, kinds) in SECTIONS {
            if kinds.contains(&kind) {
                found = heading;
                break;
            }
        }
        found
    };

    Commit {
        hash: hash.to_string(),
        short: short.to_string(),
        section,
        scope,
        summary: summary.to_string(),
    }
}

/// The full file, preamble through link references.
fn render(releases: &[Release], unreleased: &[Commit], repo: &str) -> String {
    let mut out = String::new();

    out.push_str("# Changelog\n\n");
    out.push_str("All notable changes to Strobe, newest first.\n\n");
    out.push_str(
        "**This file is generated.** It is rebuilt from the git history by\n\
         `cargo xtask changelog`, so released sections are rewritten in\n\
         place and edits to them do not survive. See `CONTRIBUTING.md`.\n\n",
    );
    out.push_str(
        "The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),\n\
         and the project follows [Semantic Versioning](https://semver.org/spec/v2.0.0.html).\n",
    );

    out.push('\n');
    out.push_str(UNRELEASED);
    out.push('\n');
    if unreleased.is_empty() {
        out.push_str("\nNothing yet — the latest release is the tip of `master`.\n");
    } else {
        push_sections(&mut out, unreleased, repo);
    }

    for release in releases {
        out.push_str(&format!("\n## [{}] - {}\n", release.version, release.date));
        if release.commits.is_empty() {
            out.push_str("\nNo commits recorded against this tag.\n");
        } else {
            push_sections(&mut out, &release.commits, repo);
        }
    }

    push_links(&mut out, releases, repo);
    out
}

/// The `### Heading` blocks for one release, in `SECTIONS` order, skipping
/// any heading nothing landed under.
fn push_sections(out: &mut String, commits: &[Commit], repo: &str) {
    let mut order: Vec<&str> = vec![BREAKING];
    order.extend(SECTIONS.iter().map(|(heading, _)| *heading));
    order.push(OTHER);

    for heading in order {
        let matching: Vec<&Commit> = commits.iter().filter(|c| c.section == heading).collect();
        if matching.is_empty() {
            continue;
        }
        out.push_str(&format!("\n### {heading}\n\n"));
        for commit in matching {
            let scope = match &commit.scope {
                Some(scope) => format!("**{scope}:** "),
                None => String::new(),
            };
            out.push_str(&format!(
                "- {scope}{} ([`{}`]({repo}/commit/{}))\n",
                commit.summary, commit.short, commit.hash
            ));
        }
    }
}

/// Markdown link references, so each heading above is a link to the diff
/// that produced it.
fn push_links(out: &mut String, releases: &[Release], repo: &str) {
    out.push('\n');
    match releases.first() {
        Some(latest) => out.push_str(&format!(
            "[Unreleased]: {repo}/compare/{}...HEAD\n",
            latest.tag
        )),
        None => out.push_str(&format!("[Unreleased]: {repo}/commits/HEAD\n")),
    }

    for release in releases {
        match &release.previous {
            Some(previous) => out.push_str(&format!(
                "[{}]: {repo}/compare/{previous}...{}\n",
                release.version, release.tag
            )),
            None => out.push_str(&format!(
                "[{}]: {repo}/releases/tag/{}\n",
                release.version, release.tag
            )),
        }
    }
}

/// The version of the newest release section in the committed file when
/// no tag of that name exists — the pre-tag state of the release flow.
/// `None` when the newest section's tag exists (or there are no release
/// sections at all). Only the newest section may be pending: an older
/// section without a tag is plain drift and fails the comparison.
fn pending_version(existing: &str, releases: &[Release]) -> Option<String> {
    for line in existing.lines() {
        let Some(rest) = line.strip_prefix("## [") else {
            continue;
        };
        let Some((version, _)) = rest.split_once(']') else {
            continue;
        };
        if version == "Unreleased" {
            continue;
        }
        return (!releases.iter().any(|r| r.version == version)).then(|| version.to_string());
    }
    None
}

/// The file with its `Unreleased` section removed and release-heading
/// dates stripped, which is what `--check` compares. The `Unreleased`
/// exemption is in the module docs; dates are stripped because the
/// pre-tag section is dated by the wall clock and the post-tag one by
/// the tag, and a release cut close to UTC midnight must not fail its
/// own check over which side of it the tag landed. The next
/// regeneration refreshes the written date to the tag's.
fn released_only(text: &str) -> String {
    let mut kept = Vec::new();
    let mut skipping = false;
    for line in text.lines() {
        if line.starts_with(UNRELEASED) {
            skipping = true;
            continue;
        }
        // Any other `## ` heading ends the section, as do the link
        // references that close the file.
        if skipping && (line.starts_with("## ") || line.starts_with("[Unreleased]:")) {
            skipping = false;
        }
        if !skipping {
            kept.push(undated_heading(line));
        }
    }
    kept.join("\n")
}

/// A release heading without its ` - YYYY-MM-DD` tail; every other line
/// unchanged. Entry bullets start `- `, never `## [`, so only headings
/// can match.
fn undated_heading(line: &str) -> &str {
    if line.starts_with("## [") {
        line.split(" - ").next().unwrap_or(line)
    } else {
        line
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MANIFEST: &str = "\
[workspace]
members = [\"crates/*\"]

[workspace.package]
version = \"0.3.1\"
edition = \"2024\"
repository = \"https://github.com/lnorton89/strobe.git\"

[workspace.dependencies]
version = \"not this one\"
";

    fn commit_of(subject: &str) -> Commit {
        parse_subject("0123456789abcdef", "0123456", subject)
    }

    #[test]
    fn a_conventional_subject_is_split_into_scope_and_summary() {
        let commit = commit_of("fix(gfx): link the program once");
        assert_eq!(commit.section, "Fixed", "fix lands under Fixed");
        assert_eq!(commit.scope.as_deref(), Some("gfx"), "scope is kept");
        assert_eq!(
            commit.summary, "link the program once",
            "prefix is stripped"
        );
    }

    #[test]
    fn every_documented_type_lands_under_its_heading() {
        for (heading, kinds) in SECTIONS {
            for kind in *kinds {
                let commit = commit_of(&format!("{kind}: something"));
                assert_eq!(
                    commit.section, *heading,
                    "`{kind}:` should land under {heading}"
                );
            }
        }
    }

    #[test]
    fn a_bang_marks_a_breaking_change_whatever_the_type() {
        assert_eq!(
            commit_of("refactor(scene)!: rename PARAMS").section,
            BREAKING,
            "a trailing ! leads the release"
        );
    }

    #[test]
    fn subjects_that_are_not_conventional_fall_to_other() {
        for subject in [
            "Merge pull request #1: deep agent research",
            "just words",
            "feat(unclosed: oops",
            "Feat: capitalised type",
            "feat:",
            ": no type",
        ] {
            assert_eq!(
                commit_of(subject).section,
                OTHER,
                "`{subject}` is not a conventional commit"
            );
        }
        assert_eq!(
            commit_of("Merge pull request #1: x").summary,
            "Merge pull request #1: x",
            "an unparsed subject is kept whole"
        );
    }

    #[test]
    fn an_unknown_type_is_other_but_still_parsed() {
        let commit = commit_of("wip(app): half a window");
        assert_eq!(commit.section, OTHER, "wip is not a known type");
        assert_eq!(commit.scope.as_deref(), Some("app"), "scope still parses");
    }

    #[test]
    fn versions_parse_with_or_without_the_v() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)), "v prefix");
        assert_eq!(parse_version("1.2.3"), Some((1, 2, 3)), "bare");
        assert_eq!(parse_version("v1.2"), None, "two parts");
        assert_eq!(parse_version("v1.2.3.4"), None, "four parts");
        assert_eq!(parse_version("nightly"), None, "not a version");
        assert!(
            parse_version("v0.10.0") > parse_version("v0.9.0"),
            "versions compare numerically, not as text"
        );
    }

    #[test]
    fn the_calendar_conversion_matches_known_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1), "the epoch");
        assert_eq!(civil_from_days(11_016), (2000, 2, 29), "a leap day");
        assert_eq!(civil_from_days(20_000), (2024, 10, 4), "an arbitrary day");
        assert_eq!(civil_from_days(-1), (1969, 12, 31), "before the epoch");
    }

    #[test]
    fn the_workspace_package_table_is_read_and_nothing_else() -> Result<()> {
        assert_eq!(
            workspace_package_value(MANIFEST, "version").as_deref(),
            Some("0.3.1"),
            "version comes from [workspace.package], not a later table"
        );
        assert_eq!(
            repository_url(MANIFEST)?,
            "https://github.com/lnorton89/strobe",
            "a trailing .git is dropped from the link base"
        );
        assert_eq!(
            workspace_package_value(MANIFEST, "license"),
            None,
            "a missing key is None"
        );
        Ok(())
    }

    #[test]
    fn the_unreleased_section_and_release_dates_are_ignored_by_the_check() {
        let committed = "\
# Changelog

## [Unreleased]

### Added

- something new

## [0.1.0] - 2026-01-01

### Fixed

- a thing

[Unreleased]: https://example.com/compare/v0.1.0...HEAD
[0.1.0]: https://example.com/releases/tag/v0.1.0
";
        let regenerated = committed
            .replace("- something new", "- something else entirely")
            .replace("2026-01-01", "2026-01-02");
        assert_eq!(
            released_only(committed),
            released_only(&regenerated),
            "Unreleased content and heading dates must not matter"
        );
        let drifted = committed.replace("- a thing", "- another thing");
        assert_ne!(
            released_only(committed),
            released_only(&drifted),
            "a released entry that changed must be caught"
        );
    }

    #[test]
    fn rendering_with_nothing_unreleased_says_so_and_links_the_latest_tag() {
        let release = Release {
            tag: "v0.1.0".to_owned(),
            version: "0.1.0".to_owned(),
            date: "2026-01-01".to_owned(),
            previous: None,
            commits: vec![commit_of("feat(app): a window")],
        };
        let text = render(&[release], &[], "https://example.com/r");
        assert!(
            text.contains("Nothing yet"),
            "an empty Unreleased section should say so"
        );
        assert!(
            text.contains("[Unreleased]: https://example.com/r/compare/v0.1.0...HEAD"),
            "Unreleased compares against the latest tag"
        );
        assert!(
            text.contains(
                "- **app:** a window ([`0123456`](https://example.com/r/commit/0123456789abcdef))"
            ),
            "an entry carries its scope and commit link, got:\n{text}"
        );
    }
}
