// ============================================================================
// Module:       build (build script)
// Description:  Captures the git commit, dirty flag, target and profile so
//               every log and crash report says exactly which build it is.
//
// Dependencies: std::process (git), cargo's build-script environment
// ============================================================================

//! Build metadata.
//!
//! Runs `git rev-parse` and `git status` once per build. A checkout with no
//! git (a source tarball) or no `git` binary is not an error: the hash is
//! `unknown` and the build is reported as clean. `STROBE_GIT_HASH` in the
//! environment overrides the lookup, for release pipelines that build from
//! an exported tree.

use std::path::PathBuf;
use std::process::Command;

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8(output.stdout).ok()?;
    Some(text.trim().to_string())
}

fn main() {
    println!("cargo:rerun-if-env-changed=STROBE_GIT_HASH");
    println!("cargo:rerun-if-changed=build.rs");

    let hash = std::env::var("STROBE_GIT_HASH")
        .ok()
        .filter(|value| !value.is_empty())
        .or_else(|| git(&["rev-parse", "--short=12", "HEAD"]))
        .unwrap_or_else(|| "unknown".to_string());
    // Tracked files only: build outputs and scratch files are not "dirty".
    // `--no-optional-locks` keeps `git status` from rewriting the index to
    // refresh its stat cache.
    let dirty = git(&[
        "--no-optional-locks",
        "status",
        "--porcelain",
        "--untracked-files=no",
    ])
    .is_some_and(|status| !status.is_empty());

    // Re-run when the commit moves: HEAD changes on a checkout, and its
    // reflog gains a line on every commit, reset, merge or rebase. Not the
    // index: git rewrites it whenever anything (an editor, a git pane, a
    // `git status`) refreshes its stat cache, and every rewrite rebuilt this
    // crate and, through it, relinked the app. Only paths that exist are
    // named, since cargo treats a missing one as always changed.
    // `--git-path` resolves worktrees, where `.git` is a file and HEAD lives
    // elsewhere.
    for name in ["HEAD", "logs/HEAD"] {
        if let Some(path) = git(&["rev-parse", "--git-path", name]).map(PathBuf::from)
            && path.exists()
        {
            println!("cargo:rerun-if-changed={}", path.display());
        }
    }

    println!("cargo:rustc-env=STROBE_BUILD_GIT_HASH={hash}");
    println!("cargo:rustc-env=STROBE_BUILD_GIT_DIRTY={}", u8::from(dirty));
    println!(
        "cargo:rustc-env=STROBE_BUILD_TARGET={}",
        std::env::var("TARGET").unwrap_or_else(|_| "unknown".to_string())
    );
    println!(
        "cargo:rustc-env=STROBE_BUILD_PROFILE={}",
        std::env::var("PROFILE").unwrap_or_else(|_| "unknown".to_string())
    );
}
