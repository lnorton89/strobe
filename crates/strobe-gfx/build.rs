// ============================================================================
// Module:       build (build script)
// Description:  On a Windows dev build, puts the pinned ANGLE DLLs beside the
//               executables when they are missing or another version,
//               through the same hash-checked scripts/fetch-angle.ps1 the
//               release uses.
//
// Dependencies: std::process (powershell), cargo's build-script environment,
//               scripts/native-deps.json (the pin)
// ============================================================================

//! ANGLE beside a dev build (D498).
//!
//! The ANGLE fallback (D2, D12) needs `libEGL.dll`, `libGLESv2.dll`,
//! `d3dcompiler_47.dll` and the Vulkan loader beside the executable. The
//! release zip ships them; a checkout did not, so every new machine and
//! every new profile needed `scripts\fetch-angle.ps1` run by hand, and a
//! forgotten run showed up later as an ANGLE that would not start. This
//! crate is the one every binary that can open ANGLE depends on (the app,
//! the bench, the corpus, auto-controls), so its build script is where the
//! DLLs are made sure of.
//!
//! - **Only a Windows host building for Windows.** Elsewhere there is no
//!   ANGLE to fetch, and a cross-check from Linux has no PowerShell.
//! - **Only when missing or stale.** The profile's output folder is three
//!   levels up from `OUT_DIR` (`target/<profile>/build/<package>-<hash>/out`,
//!   with the target triple before the profile when one is given). If all
//!   four DLLs are there and the `angle.stamp` the script writes beside them
//!   names the version in `scripts/native-deps.json`, nothing runs. A DLL's
//!   name says nothing about its version, so a bumped pin would otherwise
//!   leave every existing profile on the old ANGLE (D536). The manifest is
//!   watched, so a bump reruns this script.
//! - **The same checks as the release.** It runs the script with
//!   `-Destination`, which verifies the Electron archive and every DLL
//!   against pinned SHA-256 hashes. The archive is cached in
//!   `target\angle\<version>`, so a second profile copies without
//!   downloading.
//! - **A failed fetch is a warning in a dev build.** ANGLE is the fallback;
//!   native GL works without it. A failed fetch (offline, say) is a
//!   warning, repeated on every build until ANGLE is there, naming the
//!   script to run by hand; `cargo clean -p strobe-gfx` lets the build try
//!   again. A build that must carry ANGLE sets `STROBE_REQUIRE_ANGLE`, and
//!   then a failed fetch fails the build (D544). `cargo xtask package`
//!   checks every DLL's hash against the pin regardless.
//! - **The script is watched with the manifest.** A change to
//!   `fetch-angle.ps1` (what it places, how it checks) reruns this, like a
//!   change to the pin does.
//! - **Not in CI** (`CI` set). The release workflow fetches explicitly,
//!   and the Windows test job does not need ANGLE. `STROBE_NO_ANGLE_FETCH`
//!   turns it off anywhere.
//!
//! It deliberately watches no DLL path: a path from `OUT_DIR` is absolute,
//! and an absolute path cargo records can read as changed from another
//! shell and rerun the script, which relinks everything above it (D464).
//! So the check runs when this script runs (a fresh profile, a clean, or a
//! change here); a DLL deleted by hand later needs the script by hand.

use std::path::{Path, PathBuf};
use std::process::Command;

/// What `fetch-angle.ps1` puts beside the executables.
const DLLS: [&str; 4] = [
    "libEGL.dll",
    "libGLESv2.dll",
    "d3dcompiler_47.dll",
    "vulkan-1.dll",
];

/// Turns the fetch off when set to anything.
const OPT_OUT: &str = "STROBE_NO_ANGLE_FETCH";

/// The manifest that pins ANGLE, relative to this package: cargo records a
/// relative path as given, so it never reads as changed from another shell
/// the way an absolute one can (D464).
const MANIFEST: &str = "../../scripts/native-deps.json";

/// The script that fetches and checks ANGLE, relative the same way.
const SCRIPT: &str = "../../scripts/fetch-angle.ps1";

/// Makes a failed fetch fail the build, for a build that must ship ANGLE.
const REQUIRE: &str = "STROBE_REQUIRE_ANGLE";

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={MANIFEST}");
    println!("cargo:rerun-if-changed={SCRIPT}");
    println!("cargo:rerun-if-env-changed={OPT_OUT}");
    println!("cargo:rerun-if-env-changed={REQUIRE}");
    println!("cargo:rerun-if-env-changed=CI");
    let windows_target = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
    if !cfg!(windows) || !windows_target {
        return;
    }
    if std::env::var_os(OPT_OUT).is_some() || std::env::var_os("CI").is_some() {
        return;
    }
    let Some(dest) = profile_dir() else {
        println!("cargo:warning=ANGLE: could not tell the output folder from OUT_DIR; {HAND}");
        return;
    };
    let pinned = pinned_version();
    let stamped = std::fs::read_to_string(dest.join("angle.stamp")).ok();
    let present = DLLS.iter().all(|dll| dest.join(dll).is_file());
    if present && stamped.as_deref().map(str::trim) == pinned.as_deref() {
        return;
    }
    // Success is a plain line (in the build script's output file, shown with
    // -vv), not a warning: cargo replays a workspace crate's warnings on
    // every later build, and "was fetched" would then read as news forever.
    // A failure is a warning for exactly that reason: it stays in view
    // until ANGLE is there.
    match fetch(&dest) {
        Ok(()) => println!(
            "ANGLE fetched (pinned, hash-checked) into {}",
            dest.display()
        ),
        Err(why) if std::env::var_os(REQUIRE).is_some() => {
            eprintln!("ANGLE was not fetched ({why}), and {REQUIRE} is set; {HAND}");
            std::process::exit(1);
        }
        Err(why) => println!("cargo:warning=ANGLE was not fetched: {why}; {HAND}"),
    }
}

/// What a person can do instead.
const HAND: &str = "native GL works without it; for the ANGLE fallback run \
                    `powershell -ExecutionPolicy Bypass -File scripts\\fetch-angle.ps1`, \
                    or `cargo clean -p strobe-gfx` to let the build try again";

/// The ANGLE version the manifest pins: the first `"version"` value in it,
/// which is the `angle` entry's. A plain scan rather than a JSON parser, so
/// the build script needs no dependency; `None` when the file is unreadable,
/// which makes any stamp stale and the fetch report the real error.
fn pinned_version() -> Option<String> {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR")?;
    let text = std::fs::read_to_string(Path::new(&manifest).join(MANIFEST)).ok()?;
    let after = text.split_once("\"version\"")?.1;
    let value = after.split_once('"')?.1;
    Some(value.split_once('"')?.0.to_owned())
}

/// The profile's output folder: `OUT_DIR` without `build/<package>/out`.
fn profile_dir() -> Option<PathBuf> {
    let out = PathBuf::from(std::env::var_os("OUT_DIR")?);
    out.ancestors().nth(3).map(Path::to_path_buf)
}

/// Runs the fetch script into `dest`, returning the first line of its error
/// on failure (PowerShell puts the message there).
fn fetch(dest: &Path) -> Result<(), String> {
    let manifest = std::env::var_os("CARGO_MANIFEST_DIR").ok_or("CARGO_MANIFEST_DIR is not set")?;
    let script = Path::new(&manifest)
        .join("..")
        .join("..")
        .join("scripts")
        .join("fetch-angle.ps1");
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ])
        .arg(&script)
        .arg("-Destination")
        .arg(dest)
        .output()
        .map_err(|error| format!("could not start PowerShell: {error}"))?;
    if output.status.success() {
        return Ok(());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let last = stderr
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("no error text");
    Err(format!(
        "fetch-angle.ps1 exited with {}: {last}",
        output.status
    ))
}
