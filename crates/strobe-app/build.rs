// ============================================================================
// Module:       build (build script)
// Description:  Embeds the brand icon in the Windows executable as a
//               resource, so Explorer and the taskbar show it.
//
// Dependencies: embed-resource
// ============================================================================

//! The executable's icon (backlog R-06, D241).
//!
//! The window's own icon is set at run time (`icon.rs`); this is the one a
//! file manager shows before the program runs. `embed_resource` compiles
//! `assets/strobe.rc` with the Windows SDK's `rc.exe` (or `windres`) when
//! the target is Windows and does nothing elsewhere. A Windows build with
//! no resource compiler still builds, with a warning; a resource that fails
//! to compile fails the build, since the release would then ship without
//! its icon unnoticed.

use embed_resource::CompilationResult;
use std::path::Path;

fn main() {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let icon = Path::new(&manifest).join("../../assets/brand/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=assets/strobe.rc");
    println!("cargo:rerun-if-changed={}", icon.display());
    // rc.exe reads forward slashes in a path, and backslashes in a string
    // would be escapes.
    let path = icon.display().to_string().replace('\\', "/");
    let define = format!("ICON_PATH=\"{path}\"");
    match embed_resource::compile("assets/strobe.rc", [define.as_str()]) {
        CompilationResult::NotWindows | CompilationResult::Ok => {}
        CompilationResult::NotAttempted(why) => {
            println!("cargo:warning=strobe.exe gets no icon: {why}");
        }
        CompilationResult::Failed(why) => {
            eprintln!("strobe.exe's icon resource did not compile: {why}");
            std::process::exit(1);
        }
    }
}
