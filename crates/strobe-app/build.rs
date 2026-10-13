// ============================================================================
// Module:       build (build script)
// Description:  Embeds the brand icon and a version block in the Windows
//               executables as resources, so Explorer and the taskbar
//               show the icon and Properties shows the version.
//
// Dependencies: embed-resource
// ============================================================================

//! The executables' icon and version (backlog R-06, D241, D244).
//!
//! The window's own icon is set at run time (`icon.rs`); this is the one a
//! file manager shows before the program runs. `embed_resource` compiles
//! each binary's script in `assets/` with the Windows SDK's `rc.exe` (or
//! `llvm-rc`/`windres`) when
//! the target is Windows and does nothing elsewhere. A Windows build with
//! no resource compiler still builds, with a warning; a resource that fails
//! to compile fails the build, since the release would then ship without
//! its icon unnoticed.

use embed_resource::{CompilationResult, ParamsMacrosAndIncludeDirs};
use std::path::Path;

/// Each executable and its resource script. The output of a compiled
/// script is named after its file stem, so two binaries cannot share one
/// `.rc` with different macros; each has a two-line script that includes
/// `strobe-common.rc`.
const BINARIES: [(&str, &str); 2] = [
    ("strobe", "assets/strobe.rc"),
    ("strobe-corpus", "assets/strobe-corpus.rc"),
];

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let icon = Path::new(&manifest).join("../../assets/brand/icon.ico");
    let assets = Path::new(&manifest).join("assets");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets");
    println!("cargo:rerun-if-changed={}", icon.display());
    // rc.exe reads forward slashes in a path, and backslashes in a string
    // would be escapes.
    let path = icon.display().to_string().replace('\\', "/");
    let version =
        |part: &str| std::env::var(format!("CARGO_PKG_VERSION_{part}")).unwrap_or_default();
    let macros = [
        format!("ICON_PATH=\"{path}\""),
        format!("VER_MAJOR={}", version("MAJOR")),
        format!("VER_MINOR={}", version("MINOR")),
        format!("VER_PATCH={}", version("PATCH")),
        format!(
            "VER_STRING=\"{}\"",
            std::env::var("CARGO_PKG_VERSION").unwrap_or_default()
        ),
    ];
    for (binary, script) in BINARIES {
        let params = ParamsMacrosAndIncludeDirs(macros.iter(), [assets.as_os_str()]);
        match embed_resource::compile_for(script, [binary], params) {
            CompilationResult::NotWindows | CompilationResult::Ok => {}
            CompilationResult::NotAttempted(why) => {
                println!("cargo:warning={binary}.exe gets no icon or version: {why}");
            }
            CompilationResult::Failed(why) => {
                eprintln!("{binary}.exe's resources did not compile: {why}");
                std::process::exit(1);
            }
        }
    }
}
