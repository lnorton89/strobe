// ============================================================================
// Module:       strobe_app::library
// Description:  The scene library: the folders under --library, listed for
//               the browser and found by name for --scene.
//
// Dependencies: strobe-scene (load::scan, folder_title)
// ============================================================================

//! The scene library.
//!
//! A library is a folder of scene folders. Scanning only lists them (a
//! folder containing `main.glsl`); a scene is read and compiled when it is
//! selected, so a broken scene never stops the browser from listing.

use std::path::{Path, PathBuf};

/// One scene folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The folder name without a `.synScene` suffix.
    pub name: String,
    /// The folder.
    pub dir: PathBuf,
}

/// The scenes found under one folder.
#[derive(Debug, Default)]
pub struct Library {
    /// The folder scanned.
    pub dir: PathBuf,
    /// Scenes sorted by name.
    pub entries: Vec<Entry>,
}

impl Library {
    /// Lists the scenes under `dir`.
    pub fn scan(dir: &Path) -> Result<Self, String> {
        let folders =
            strobe_scene::load::scan(dir).map_err(|e| format!("library {}: {e}", dir.display()))?;
        let entries = folders
            .into_iter()
            .map(|folder| Entry {
                name: strobe_scene::load::folder_title(&folder),
                dir: folder,
            })
            .collect();
        Ok(Self {
            dir: dir.to_path_buf(),
            entries,
        })
    }

    /// The scene `query` names: an exact name, else the first whose name
    /// starts with it, else the first that contains it.
    pub fn find(&self, query: &str) -> Option<usize> {
        let by = |matches: fn(&str, &str) -> bool| {
            self.entries
                .iter()
                .position(|entry| matches(&entry.name, query))
        };
        by(|name, q| name == q)
            .or_else(|| by(|name, q| name.starts_with(q)))
            .or_else(|| by(|name, q| name.contains(q)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn library(names: &[&str]) -> Library {
        Library {
            dir: PathBuf::from("scenes"),
            entries: names
                .iter()
                .map(|name| Entry {
                    name: (*name).to_string(),
                    dir: PathBuf::from(name),
                })
                .collect(),
        }
    }

    #[test]
    fn find_prefers_exact_then_prefix_then_substring() {
        let lib = library(&["05-domain-warp", "05", "warp-tunnel"]);
        assert_eq!(lib.find("05"), Some(1), "exact");
        assert_eq!(lib.find("05-d"), Some(0), "prefix");
        assert_eq!(lib.find("tunnel"), Some(2), "substring");
        assert_eq!(lib.find("nope"), None, "missing");
    }
}
