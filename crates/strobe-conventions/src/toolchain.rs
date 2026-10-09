// ============================================================================
// Module:       strobe_conventions::toolchain
// Description:  Checks that rust-toolchain.toml and every CI workflow name the
//               same Rust version, so local and CI lint identically.
//
// Dependencies: anyhow, std::fs; crate::sources (the workspace root)
// ============================================================================

//! One Rust version, named in agreeing places.
//!
//! `rust-toolchain.toml` is what rustup obeys; the workflows also name a
//! version so the setup action installs it up front. Several places, one
//! fact — the shape that drifts. A workflow on a floating `@stable` moves
//! to each new release on its own, and the symptom is a lint firing in CI
//! that cannot fire locally: a red build on a morning nobody changed
//! anything.

use crate::sources::workspace_root;
use anyhow::{Result, anyhow};
use std::fs;

/// The `channel = "..."` value from `rust-toolchain.toml`.
pub fn pinned_channel() -> Result<String> {
    let text = fs::read_to_string(workspace_root()?.join("rust-toolchain.toml"))?;
    for line in text.lines() {
        let Some(rest) = line.trim().strip_prefix("channel") else {
            continue;
        };
        let Some(rest) = rest.trim_start().strip_prefix('=') else {
            continue;
        };
        return Ok(rest.trim().trim_matches('"').to_owned());
    }
    Err(anyhow!("rust-toolchain.toml has no `channel` line"))
}

/// Every workflow under `.github/workflows/`, as `(path, text)` pairs.
pub fn workflows() -> Result<Vec<(String, String)>> {
    let dir = workspace_root()?.join(".github").join("workflows");
    let mut found = Vec::new();
    for entry in fs::read_dir(&dir)? {
        let path = entry?.path();
        if path
            .extension()
            .is_some_and(|ext| ext == "yml" || ext == "yaml")
        {
            let name = path.to_string_lossy().replace('\\', "/");
            found.push((name, fs::read_to_string(&path)?));
        }
    }
    found.sort();
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_workflow_pins_the_same_toolchain_as_the_repo() -> Result<()> {
        let pinned = pinned_channel()?;
        assert!(
            !pinned.is_empty(),
            "rust-toolchain.toml names an empty channel"
        );
        let workflows = workflows()?;
        assert!(!workflows.is_empty(), "no CI workflow found to check");
        for (name, text) in workflows {
            for declared in text
                .lines()
                .filter_map(|line| line.trim().strip_prefix("toolchain:"))
                .map(|rest| rest.trim().trim_matches('"'))
            {
                assert_eq!(
                    declared, pinned,
                    "{name} installs Rust {declared} but rust-toolchain.toml pins {pinned}"
                );
            }
        }
        Ok(())
    }

    /// The release workflow is the one that ships a binary, so a toolchain
    /// drift there matters most and is easiest to miss: it only runs on a
    /// tag. `workflows()` reads the whole directory, so it is covered the
    /// moment the file exists; this pins that it *is* covered, by name, and
    /// that each of the two workflows that compile code actually names the
    /// toolchain it builds with.
    #[test]
    fn the_ci_and_release_workflows_are_both_checked_and_name_a_toolchain() -> Result<()> {
        let workflows = workflows()?;
        for expected in ["ci.yml", "release.yml"] {
            let Some((name, text)) = workflows.iter().find(|(name, _)| name.ends_with(expected))
            else {
                return Err(anyhow!("no .github/workflows/{expected} was found"));
            };
            assert!(
                text.lines()
                    .any(|line| line.trim().starts_with("toolchain:")),
                "{name} never names a toolchain to install, so it would build with \
                 whatever the runner image ships"
            );
        }
        Ok(())
    }

    #[test]
    fn no_workflow_installs_a_floating_stable_toolchain() -> Result<()> {
        for (name, text) in workflows()? {
            assert!(
                !text.contains("rust-toolchain@stable"),
                "{name} uses dtolnay/rust-toolchain@stable; pin the version instead"
            );
        }
        Ok(())
    }
}
