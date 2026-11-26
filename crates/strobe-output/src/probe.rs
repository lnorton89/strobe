// ============================================================================
// Module:       strobe_output::probe
// Description:  Finds the NDI runtime and the Spout library on this machine,
//               so a failure can say what to install before a sender is made.
//
// Dependencies: std::env, std::fs, std::path
// ============================================================================

//! Finding the optional runtime before it is needed.
//!
//! Neither sender needs a build-time SDK (D570): NDI loads its runtime at
//! runtime, and Spout's `SpoutLibrary.dll` is found by the loader. That makes
//! both features free to compile, and it moves the bad news to the moment a
//! person turns one on. These two probes turn that news into a sentence
//! before a sender is created: "NDI output needs the NDI 6 runtime; install
//! NDI Tools from https://ndi.video" reads better than an SDK error from
//! inside a show.
//!
//! The search order is the loader's own where it can be: the program's own
//! folder first (so a runtime can ship beside `strobe.exe`), then the
//! runtime's environment variable, then the usual install folders. A probe
//! only reads directories; it never loads a library, so it cannot crash the
//! show and it costs nothing when an output is off.

use std::path::{Path, PathBuf};

/// The NDI 6 runtime's file name on this platform.
#[cfg(target_os = "windows")]
pub const NDI_LIBRARY: &str = "Processing.NDI.Lib.x64.dll";

/// The NDI 6 runtime's file name on this platform.
#[cfg(target_os = "linux")]
pub const NDI_LIBRARY: &str = "libndi.so.6";

/// The NDI 6 runtime's file name on this platform.
#[cfg(target_os = "macos")]
pub const NDI_LIBRARY: &str = "libndi.dylib";

/// The NDI 6 runtime's file name on this platform.
#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
pub const NDI_LIBRARY: &str = "Processing.NDI.Lib.x64.dll";

/// The Spout library's file name, Windows' only (Spout is Windows-only).
pub const SPOUT_LIBRARY: &str = "SpoutLibrary.dll";

/// The environment variable NDI's loader reads: an absolute folder holding
/// its runtime.
pub const NDI_RUNTIME_DIR: &str = "NDI_RUNTIME_DIR_V6";

/// The NDI 6 runtime, if this machine has one an x86_64 build can load.
///
/// Found beside the program, in [`NDI_RUNTIME_DIR`], or in the NDI tools'
/// usual install folders. A 32-bit runtime (Synesthesia ships one inside its
/// own folder) is not this process's and is not reported.
pub fn ndi_runtime() -> Option<PathBuf> {
    let mut folders: Vec<PathBuf> = Vec::new();
    if let Some(exe) = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
    {
        folders.push(exe);
    }
    if let Some(env) = std::env::var_os(NDI_RUNTIME_DIR) {
        folders.push(PathBuf::from(env));
    }
    #[cfg(target_os = "windows")]
    folders.extend(windows_ndi_folders());
    for folder in folders {
        if let Some(found) = find_in(&folder, &[NDI_LIBRARY]) {
            return Some(found);
        }
    }
    None
}

/// The Spout library, if this machine has one (Windows only: Spout shares
/// Direct3D textures, and elsewhere this is always `None`).
pub fn spout_library() -> Option<PathBuf> {
    #[cfg(not(target_os = "windows"))]
    {
        None
    }
    #[cfg(target_os = "windows")]
    {
        let mut folders: Vec<PathBuf> = Vec::new();
        if let Some(exe) = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(Path::to_path_buf))
        {
            folders.push(exe);
        }
        for variable in ["PATH", "SPOUT_LIBRARY_DIR"] {
            if let Some(value) = std::env::var_os(variable) {
                folders.extend(std::env::split_paths(&value));
            }
        }
        for folder in folders {
            if let Some(found) = find_in(&folder, &[SPOUT_LIBRARY]) {
                return Some(found);
            }
        }
        None
    }
}

/// The exact file `names` in `folder`, in the order given.
fn find_in(folder: &Path, names: &[&str]) -> Option<PathBuf> {
    names
        .iter()
        .map(|name| folder.join(name))
        .find(|path| path.is_file())
}

/// The folders the NDI tools install their runtime into, newest first.
#[cfg(target_os = "windows")]
fn windows_ndi_folders() -> Vec<PathBuf> {
    let mut folders = Vec::new();
    for root in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
        let Some(base) = std::env::var_os(root) else {
            continue;
        };
        let base = PathBuf::from(base).join("NDI");
        for version in ["NDI 6 Runtime", "NDI 5 Runtime"] {
            folders.push(base.join(version).join("v6"));
            folders.push(base.join(version).join("v5"));
        }
    }
    folders
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A fresh empty folder under the system's temporary folder.
    fn scratch() -> Result<PathBuf> {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let unique = format!(
            "strobe-output-probe-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::SeqCst)
        );
        let folder = std::env::temp_dir().join(unique);
        std::fs::create_dir_all(&folder)?;
        Ok(folder)
    }

    #[test]
    fn a_library_beside_the_program_is_found_by_name() -> Result<()> {
        let folder = scratch()?;
        std::fs::write(folder.join(NDI_LIBRARY), b"not really a library")?;
        let found = find_in(&folder, &[NDI_LIBRARY]);
        assert_eq!(
            found.as_deref(),
            Some(folder.join(NDI_LIBRARY).as_path()),
            "the file that is there is the one reported"
        );
        std::fs::remove_dir_all(&folder)?;
        Ok(())
    }

    #[test]
    fn a_folder_without_the_file_reports_nothing() -> Result<()> {
        let folder = scratch()?;
        std::fs::write(folder.join("SpoutSender.exe"), b"another program")?;
        assert_eq!(
            find_in(&folder, &[SPOUT_LIBRARY]),
            None,
            "a different file is not the library"
        );
        std::fs::remove_dir_all(&folder)?;
        Ok(())
    }

    #[test]
    fn the_probes_answer_without_panicking_anywhere() {
        // The answer depends on the machine; the contract is that asking is
        // safe and costs nothing. A path that comes back must be a file.
        if let Some(path) = ndi_runtime() {
            assert!(path.is_file(), "an NDI runtime that is reported exists");
            assert!(
                path.file_name().is_some_and(|name| name == NDI_LIBRARY),
                "and it is the x86_64 runtime, not a 32-bit one inside another app"
            );
        }
        if let Some(path) = spout_library() {
            assert!(path.is_file(), "a Spout library that is reported exists");
            assert!(
                path.file_name().is_some_and(|name| name == SPOUT_LIBRARY),
                "and it is SpoutLibrary.dll"
            );
        }
    }

    #[test]
    #[cfg(not(target_os = "windows"))]
    fn spout_is_windows_only() {
        assert_eq!(
            spout_library(),
            None,
            "Spout shares Direct3D textures; off Windows there is nothing to find"
        );
    }
}
