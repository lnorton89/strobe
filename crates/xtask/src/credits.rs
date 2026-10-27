// ============================================================================
// Module:       xtask::credits
// Description:  `cargo xtask credits`: writes THIRD_PARTY.md (the contributors
//               from the git history and every crate in strobe.exe with its
//               licence) and, for a release, the full third-party notices.
//
// Dependencies: anyhow, serde_json, std::process (cargo metadata, git),
//               std::fs
// ============================================================================

//! `cargo xtask credits [--check | --notices <file>]` (D491).
//!
//! The About page credits the people and the packages Strobe is made of.
//! A list kept by hand goes stale the first time a dependency is bumped, so
//! the list is derived:
//!
//! - **Contributors** come from `git log` with the repository's `.mailmap`
//!   (which folds one person's several identities into one name), oldest
//!   first, so the order never changes as commits land. Bots (`[bot]`
//!   names) are left out, and no address is written. The Licensor named in
//!   `LICENSE` leads the list as its author and maintainer.
//! - **Crates** are every registry package `strobe-app` links, resolved by
//!   `cargo metadata` for [`TARGET`], the build that ships, so the list is
//!   what is in `strobe.exe` and not the bench's or the tooling's. Normal
//!   dependencies only: a build script or a test harness is not shipped.
//!   One row per crate name, without its version, so a patch bump does not
//!   change the file and only a crate that arrives, leaves or changes
//!   licence does.
//! - **Shipped beside** is the ANGLE runtime the release zip carries next
//!   to `strobe.exe`, with the Electron version read from
//!   `scripts/fetch-angle.ps1`.
//!
//! `THIRD_PARTY.md` is committed, readable on GitHub, and compiled into the
//! app, which draws its tables on the About page. `--check` regenerates it
//! in memory and fails when the committed file differs; CI runs that in the
//! job that has the whole history.
//!
//! `--notices <file>` writes what the MIT, BSD and Apache licences ask to
//! travel with a binary: every shipped crate at its exact version with the
//! full text of each licence file its package carries. Identical texts are
//! written once and referred to by number. The release workflow puts it in
//! the zip as `THIRD-PARTY-NOTICES.txt`; it is not committed, because it is
//! large and its versions move with every bump.

use anyhow::{Context as _, Result, anyhow, bail};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The committed list, relative to the workspace root.
const FILE: &str = "THIRD_PARTY.md";

/// The package whose dependencies are credited.
const APP: &str = "strobe-app";

/// The platform the release is built for.
const TARGET: &str = "x86_64-pc-windows-msvc";

/// Where the ANGLE version is pinned.
const ANGLE_SCRIPT: &str = "scripts/fetch-angle.ps1";

/// Writes `THIRD_PARTY.md`, checks it, or writes the release notices.
pub(crate) fn run(check: bool, notices: Option<&Path>) -> Result<()> {
    let packages = shipped()?;
    if let Some(path) = notices {
        let text = notices_text(&packages)?;
        std::fs::write(path, text)
            .with_context(|| format!("could not write {}", path.display()))?;
        println!("wrote {} ({} crates)", path.display(), packages.len());
        return Ok(());
    }
    let fresh = list(&contributors()?, &packages, &angle_version()?);
    if check {
        let committed = std::fs::read_to_string(FILE)
            .with_context(|| format!("could not read {FILE}"))?
            .replace("\r\n", "\n");
        if committed != fresh {
            let first = committed
                .lines()
                .zip(fresh.lines())
                .find(|(old, new)| old != new)
                .map_or_else(
                    || "the files differ in length".to_owned(),
                    |(old, new)| format!("first difference: {old:?} is now {new:?}"),
                );
            bail!("{FILE} is out of date ({first}); run `cargo xtask credits` and commit it");
        }
        println!("{FILE} is up to date");
        return Ok(());
    }
    std::fs::write(FILE, &fresh).with_context(|| format!("could not write {FILE}"))?;
    println!("wrote {FILE}");
    Ok(())
}

/// One registry package in the shipped build.
struct Package {
    name: String,
    version: String,
    licence: String,
    source: String,
    /// The folder its `Cargo.toml` is in, where its licence files are.
    dir: PathBuf,
}

/// Every registry package `strobe-app` links on [`TARGET`], sorted by name
/// then version.
fn shipped() -> Result<Vec<Package>> {
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    let output = Command::new(cargo)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--locked",
            "--filter-platform",
            TARGET,
        ])
        .output()
        .context("could not run `cargo metadata`")?;
    if !output.status.success() {
        bail!(
            "`cargo metadata` failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    let metadata: Value =
        serde_json::from_slice(&output.stdout).context("`cargo metadata` printed no JSON")?;
    let packages: BTreeMap<&str, &Value> = metadata["packages"]
        .as_array()
        .context("`cargo metadata` listed no packages")?
        .iter()
        .filter_map(|package| Some((package["id"].as_str()?, package)))
        .collect();
    let nodes: BTreeMap<&str, &Value> = metadata["resolve"]["nodes"]
        .as_array()
        .context("`cargo metadata` resolved nothing")?
        .iter()
        .filter_map(|node| Some((node["id"].as_str()?, node)))
        .collect();
    let root = packages
        .iter()
        .find(|(_, package)| package["name"] == APP)
        .map(|(id, _)| *id)
        .ok_or_else(|| anyhow!("no {APP} package in the workspace"))?;

    // Every package reachable from the app through normal dependencies.
    let mut seen = BTreeSet::new();
    let mut stack = vec![root];
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let deps = nodes
            .get(id)
            .and_then(|node| node["deps"].as_array())
            .map_or(&[][..], Vec::as_slice);
        for dep in deps {
            let normal = dep["dep_kinds"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind["kind"].is_null()));
            if let (true, Some(pkg)) = (normal, dep["pkg"].as_str()) {
                stack.push(pkg);
            }
        }
    }

    let mut shipped = Vec::new();
    for id in seen {
        let Some(package) = packages.get(id) else {
            continue;
        };
        // The workspace's own crates have no source; they are Strobe.
        if package["source"].is_null() {
            continue;
        }
        let text = |key: &str| package[key].as_str().unwrap_or_default().trim().to_owned();
        let name = text("name");
        let licence = match text("license") {
            licence if !licence.is_empty() => licence,
            _ => match package["license_file"].as_str() {
                Some(file) => format!("see {file}"),
                None => "not stated".to_owned(),
            },
        };
        let source = [text("repository"), text("homepage")]
            .into_iter()
            .find(|url| !url.is_empty())
            .unwrap_or_else(|| format!("https://crates.io/crates/{name}"));
        let manifest = PathBuf::from(text("manifest_path"));
        let dir = manifest
            .parent()
            .map(Path::to_path_buf)
            .ok_or_else(|| anyhow!("{name} has no manifest folder"))?;
        shipped.push(Package {
            version: text("version"),
            licence,
            source: tidy_url(&source),
            dir,
            name,
        });
    }
    shipped.sort_by(|a, b| (&a.name, &a.version).cmp(&(&b.name, &b.version)));
    Ok(shipped)
}

/// A repository URL without the `.git` or `/` some manifests end it with.
fn tidy_url(url: &str) -> String {
    let url = url.trim_end_matches('/');
    url.strip_suffix(".git").unwrap_or(url).to_owned()
}

/// The contributors as (name, role), the Licensor first, then everyone
/// else in the order of their first commit.
fn contributors() -> Result<Vec<(String, &'static str)>> {
    let shallow = git(&["rev-parse", "--is-shallow-repository"])?;
    if shallow.trim() == "true" {
        bail!(
            "this checkout is shallow, so its history names only some contributors; \
             fetch the whole history (`git fetch --unshallow`, or `fetch-depth: 0` in CI)"
        );
    }
    let log = git(&[
        "-c",
        "log.mailmap=true",
        "log",
        "--reverse",
        "--format=%aN",
        "HEAD",
    ])?;
    let licence = std::fs::read_to_string("LICENSE").context("could not read LICENSE")?;
    let owner = licence
        .lines()
        .find_map(|line| line.strip_prefix("Licensor:"))
        .map(str::trim)
        .ok_or_else(|| anyhow!("LICENSE names no `Licensor:`"))?
        .to_owned();
    let mut people = vec![(owner.clone(), "Author and maintainer")];
    let mut named = BTreeSet::from([owner]);
    for name in log.lines().map(str::trim) {
        if name.is_empty() || name.to_ascii_lowercase().ends_with("[bot]") {
            continue;
        }
        if named.insert(name.to_owned()) {
            people.push((name.to_owned(), "Contributor"));
        }
    }
    Ok(people)
}

/// Stdout of `git <args>`.
fn git(args: &[&str]) -> Result<String> {
    let output = Command::new("git")
        .args(args)
        .output()
        .context("could not run git")?;
    if !output.status.success() {
        bail!(
            "`git {}` failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    String::from_utf8(output.stdout).context("git printed text that is not UTF-8")
}

/// The Electron release ANGLE is taken from, as pinned in [`ANGLE_SCRIPT`].
fn angle_version() -> Result<String> {
    let script = std::fs::read_to_string(ANGLE_SCRIPT)
        .with_context(|| format!("could not read {ANGLE_SCRIPT}"))?;
    script
        .lines()
        .find_map(|line| line.trim().strip_prefix("$Version = "))
        .map(|version| version.trim().trim_matches('"').to_owned())
        .ok_or_else(|| anyhow!("{ANGLE_SCRIPT} pins no `$Version`"))
}

/// A table cell: one line, no pipe.
fn cell(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .replace('|', "/")
}

/// The text of `THIRD_PARTY.md`.
fn list(people: &[(String, &str)], packages: &[Package], angle: &str) -> String {
    // One row per crate name: several versions of one crate are one
    // project. Their licences are listed together if they differ.
    let mut rows: BTreeMap<&str, (BTreeSet<&str>, &str)> = BTreeMap::new();
    for package in packages {
        let row = rows
            .entry(&package.name)
            .or_insert_with(|| (BTreeSet::new(), &package.source));
        row.0.insert(&package.licence);
    }
    let mut out = String::new();
    let _ = writeln!(
        out,
        "<!-- Generated by `cargo xtask credits` from the git history and Cargo.lock. \
         Do not edit: CI checks it. -->"
    );
    let _ = writeln!(out, "# Credits\n");
    let _ = writeln!(
        out,
        "Strobe is MIT-licensed with the Commons Clause ([LICENSE](LICENSE)). It is built on \
         the work of the people and projects below. The app shows this list under \
         Settings > About.\n"
    );
    let _ = writeln!(out, "## Contributors\n");
    let _ = writeln!(out, "| Name | Role |\n|---|---|");
    for (name, role) in people {
        let _ = writeln!(out, "| {} | {role} |", cell(name));
    }
    let _ = writeln!(out, "\n## Shipped beside strobe.exe\n");
    let _ = writeln!(
        out,
        "The release zip carries these next to the program. Their notices are in \
         `LICENSES.chromium.html`, which ships with them.\n"
    );
    let _ = writeln!(out, "| Component | Licence | Source |\n|---|---|---|");
    let _ = writeln!(
        out,
        "| ANGLE (libEGL.dll, libGLESv2.dll), from Electron {angle} | BSD-3-Clause | \
         https://chromium.googlesource.com/angle/angle |"
    );
    let _ = writeln!(
        out,
        "| d3dcompiler_47.dll, Microsoft's Direct3D shader compiler, from Electron {angle} | \
         Microsoft redistributable | https://github.com/electron/electron |"
    );
    let _ = writeln!(
        out,
        "| vulkan-1.dll, the Khronos Vulkan loader, from Electron {angle} | Apache-2.0 | \
         https://github.com/KhronosGroup/Vulkan-Loader |"
    );
    let _ = writeln!(out, "\n## Crates\n");
    let _ = writeln!(
        out,
        "Every crate compiled into `strobe.exe` (`{APP}` for `{TARGET}`), {} in all, with the \
         licence its authors published it under. The release zip's \
         `THIRD-PARTY-NOTICES.txt` has each one's version and full licence text.\n",
        rows.len()
    );
    let _ = writeln!(out, "| Crate | Licence | Source |\n|---|---|---|");
    for (name, (licences, source)) in &rows {
        // `MIT/Apache-2.0` is the slash form Cargo has deprecated for `OR`;
        // written the one way, every row reads alike.
        let licence = licences
            .iter()
            .map(|licence| licence.replace('/', " OR "))
            .collect::<Vec<_>>()
            .join("; ");
        let _ = writeln!(
            out,
            "| {} | {} | {} |",
            cell(name),
            cell(&licence),
            cell(source)
        );
    }
    out
}

/// Whether `name` is a file a package keeps its licence or notices in.
fn is_licence_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    ["licen", "copying", "notice", "copyright"]
        .iter()
        .any(|word| lower.contains(word))
        || matches!(lower.as_str(), "ofl.txt" | "ufl.txt")
}

/// A package's licence files, relative to its folder: at its top, one
/// folder down (a vendored C library keeps its own there), and every text
/// file beside a bundled font (fonts carry their licences as `.txt`).
fn licence_files(dir: &Path) -> Vec<PathBuf> {
    const SKIP: [&str; 6] = ["target", "tests", "benches", "examples", ".git", "src"];
    let mut found = Vec::new();
    let mut folders = vec![(dir.to_path_buf(), 0)];
    while let Some((folder, depth)) = folders.pop() {
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        let mut files = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if depth == 0 && !SKIP.contains(&name.as_str()) {
                    folders.push((path, depth + 1));
                }
            } else {
                files.push((name, path));
            }
        }
        let fonts = files.iter().any(|(name, _)| {
            let lower = name.to_ascii_lowercase();
            lower.ends_with(".ttf") || lower.ends_with(".otf")
        });
        for (name, path) in files {
            let text = name.to_ascii_lowercase().ends_with(".txt");
            if (is_licence_file(&name) || (fonts && text))
                && let Ok(relative) = path.strip_prefix(dir)
            {
                found.push(relative.to_path_buf());
            }
        }
    }
    found.sort();
    found
}

/// The licence identifiers in an SPDX expression, without its operators
/// and brackets: `(MIT OR Apache-2.0) AND Zlib` is MIT, Apache-2.0, Zlib.
fn spdx_ids(expression: &str) -> Vec<&str> {
    expression
        .split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '/'))
        .filter(|word| !word.is_empty() && !matches!(*word, "OR" | "AND" | "WITH"))
        .collect()
}

/// The text of `THIRD-PARTY-NOTICES.txt`.
fn notices_text(packages: &[Package]) -> Result<String> {
    const RULE: &str =
        "-------------------------------------------------------------------------------";
    // Each distinct licence text once, numbered in order of first use.
    let mut texts: Vec<String> = Vec::new();
    let mut index: BTreeMap<String, usize> = BTreeMap::new();
    let mut out = String::new();
    let _ = writeln!(out, "Third-party notices for Strobe\n");
    let _ = writeln!(
        out,
        "strobe.exe is built from the open-source crates below, each listed with its version, \
         licence and source, and the licence and notice files its package carries. Identical \
         texts are printed once, at the end, and referred to by number.\n\n\
         The ANGLE, Direct3D compiler and Vulkan loader DLLs beside strobe.exe come from \
         Electron; their notices are in LICENSES.chromium.html.\n\n\
         Strobe itself is MIT with the Commons Clause: see LICENSE.\n"
    );
    for package in packages {
        let _ = writeln!(out, "{RULE}\n{} {}", package.name, package.version);
        let _ = writeln!(out, "Licence: {}", package.licence);
        let _ = writeln!(out, "Source:  {}", package.source);
        let files = licence_files(&package.dir);
        if files.is_empty() {
            // egui's crates, for one, publish without the files their
            // repository has. The licence's standard text is the one that
            // applies; say where it is rather than guess a copyright line.
            let _ = writeln!(
                out,
                "The package carries no licence file. The standard text of each licence its \
                 manifest names is at:"
            );
            for id in spdx_ids(&package.licence) {
                let _ = writeln!(out, "  https://spdx.org/licenses/{id}.html");
            }
        }
        for file in files {
            let text = std::fs::read_to_string(package.dir.join(&file))
                .with_context(|| {
                    format!("could not read {} from {}", file.display(), package.name)
                })?
                .replace("\r\n", "\n");
            let number = match index.get(&text) {
                Some(number) => *number,
                None => {
                    texts.push(text.clone());
                    index.insert(text, texts.len());
                    texts.len()
                }
            };
            let _ = writeln!(
                out,
                "Text [{number}]: {}",
                file.display().to_string().replace('\\', "/")
            );
        }
        let _ = writeln!(out);
    }
    let _ = writeln!(out, "{RULE}\nLicence texts\n");
    for (number, text) in texts.iter().enumerate() {
        let _ = writeln!(out, "{RULE}\n[{}]\n\n{}\n", number + 1, text.trim_end());
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn licence_files_are_found_by_name_and_code_is_not() {
        for name in [
            "LICENSE",
            "LICENSE-MIT",
            "license.txt",
            "COPYING",
            "NOTICE",
            "OFL.txt",
        ] {
            assert!(is_licence_file(name), "{name} is a licence file");
        }
        for name in ["README.md", "Cargo.toml", "lib.rs", "build.rs"] {
            assert!(!is_licence_file(name), "{name} is not a licence file");
        }
    }

    #[test]
    fn an_expression_names_its_licences() {
        assert_eq!(
            spdx_ids("(MIT OR Apache-2.0) AND Zlib"),
            vec!["MIT", "Apache-2.0", "Zlib"],
            "operators and brackets dropped"
        );
        assert_eq!(
            spdx_ids("MIT/Apache-2.0"),
            vec!["MIT", "Apache-2.0"],
            "the old slash form"
        );
    }

    #[test]
    fn a_cell_is_one_line_with_no_pipe() {
        assert_eq!(
            cell("MIT |\n Apache-2.0"),
            "MIT / Apache-2.0",
            "folded and escaped"
        );
    }

    #[test]
    fn a_repository_url_loses_its_git_suffix() {
        assert_eq!(
            tidy_url("https://github.com/a/b.git/"),
            "https://github.com/a/b",
            "trailing slash and .git"
        );
        assert_eq!(tidy_url("https://x.org"), "https://x.org", "unchanged");
    }
}
