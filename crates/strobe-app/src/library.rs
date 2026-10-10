// ============================================================================
// Module:       strobe_app::library
// Description:  The scene library: the folders under --library, listed for
//               the browser and found by name for --scene, plus each
//               scene's health and the browser's search and filter.
//
// Dependencies: strobe-scene (load::scan, folder_title)
// ============================================================================

//! The scene library.
//!
//! A library is a folder of scene folders. Scanning only lists them (a
//! folder containing `main.glsl`); a scene is read and compiled when it is
//! selected, so a broken scene never stops the browser from listing.
//!
//! ## Health and search
//!
//! A scan only lists folders, so a scene's [`Health`] is what the app has
//! *seen*: unknown until it is loaded, ready after it loads, failed (with
//! the first line of the error) after it does not. The browser filters by
//! it ([`HealthFilter`]) and searches names ([`matches`]); both are cheap
//! enough to run when the query changes and cached by the browser until it
//! does.

use std::path::{Path, PathBuf};

/// One scene folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// The folder name without a `.synScene` suffix.
    pub name: String,
    /// The folder.
    pub dir: PathBuf,
    /// `name` lowercased once, so searching does not allocate per entry.
    pub lower: String,
}

impl Entry {
    /// An entry for the scene folder `dir` called `name`.
    pub fn new(name: String, dir: PathBuf) -> Self {
        let lower = name.to_lowercase();
        Self { name, dir, lower }
    }
}

/// What the app knows about whether a scene works.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Health {
    /// Never loaded in this run.
    #[default]
    Unknown,
    /// Loaded and compiled.
    Ready,
    /// Failed to load; the first line of why.
    Failed(String),
}

impl Health {
    /// A short label, always paired with an icon (never colour alone).
    pub fn label(&self) -> &'static str {
        match self {
            Self::Unknown => "Not loaded yet",
            Self::Ready => "Ready",
            Self::Failed(_) => "Failed",
        }
    }
}

/// The browser's health filter.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HealthFilter {
    /// Everything.
    #[default]
    All,
    /// Scenes that loaded.
    Ready,
    /// Scenes that failed ("needs work").
    Failed,
}

impl HealthFilter {
    /// Every filter, in the order the browser offers them.
    pub const ALL: [HealthFilter; 3] = [Self::All, Self::Ready, Self::Failed];

    /// The key the preferences file stores.
    pub fn key(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Ready => "ready",
            Self::Failed => "failed",
        }
    }

    /// The chip's label.
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Ready => "Ready",
            Self::Failed => "Needs work",
        }
    }

    /// The filter stored under `key`, or `All`.
    pub fn from_key(key: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|filter| filter.key() == key)
            .unwrap_or_default()
    }

    /// Whether a scene of this health passes.
    pub fn admits(self, health: &Health) -> bool {
        match self {
            Self::All => true,
            Self::Ready => *health == Health::Ready,
            Self::Failed => matches!(health, Health::Failed(_)),
        }
    }
}

/// The scene a step of `delta` (1 for next, -1 for previous) from `current`
/// lands on in a library of `len` scenes: wrapping at both ends, skipping
/// any scene `is_failed` calls broken, and never landing back on `current`
/// unless it is the only one that works. With nothing playing, next is the
/// first scene and previous the last. `None` when there is nothing to step
/// to.
pub fn step(
    current: Option<usize>,
    len: usize,
    delta: isize,
    is_failed: impl Fn(usize) -> bool,
) -> Option<usize> {
    if len == 0 || delta == 0 {
        return None;
    }
    let len_i = isize::try_from(len).ok()?;
    let start = match current {
        Some(index) => isize::try_from(index).ok()?,
        None if delta > 0 => -1,
        None => len_i,
    };
    let step = delta.signum();
    let mut at = start;
    for _ in 0..len {
        at = (at + step).rem_euclid(len_i);
        let candidate = usize::try_from(at).ok()?;
        if !is_failed(candidate) {
            return Some(candidate);
        }
    }
    None
}

/// Whether `entry` passes the search `query_lower` (already lowercased and
/// trimmed; empty matches everything) and the health `filter`.
///
/// The query is a list of words, all of which must appear in the name, in
/// any order. `broken` and `favorite`-style keywords are not special yet
/// (UI_DESIGN section 12 lists them for when scenes carry metadata); the
/// health filter covers "broken".
pub fn matches(entry: &Entry, health: &Health, query_lower: &str, filter: HealthFilter) -> bool {
    filter.admits(health)
        && query_lower
            .split_whitespace()
            .all(|word| entry.lower.contains(word))
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
            .map(|folder| Entry::new(strobe_scene::load::folder_title(&folder), folder))
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
                .map(|name| Entry::new((*name).to_string(), PathBuf::from(name)))
                .collect(),
        }
    }

    #[test]
    fn stepping_wraps_and_skips_broken_scenes() {
        let none_failed = |_| false;
        assert_eq!(step(Some(0), 3, 1, none_failed), Some(1), "next");
        assert_eq!(step(Some(2), 3, 1, none_failed), Some(0), "next wraps");
        assert_eq!(step(Some(0), 3, -1, none_failed), Some(2), "previous wraps");
        assert_eq!(
            step(None, 3, 1, none_failed),
            Some(0),
            "nothing playing: first"
        );
        assert_eq!(
            step(None, 3, -1, none_failed),
            Some(2),
            "nothing playing: last"
        );
        let middle_failed = |i| i == 1;
        assert_eq!(
            step(Some(0), 3, 1, middle_failed),
            Some(2),
            "skips a broken one"
        );
        assert_eq!(
            step(Some(2), 3, -1, middle_failed),
            Some(0),
            "skips going back"
        );
        assert_eq!(step(Some(0), 3, 1, |_| true), None, "all broken");
        assert_eq!(step(Some(0), 0, 1, none_failed), None, "empty library");
        assert_eq!(
            step(Some(0), 1, 1, none_failed),
            Some(0),
            "a lone scene steps to itself"
        );
        assert_eq!(step(Some(0), 3, 0, none_failed), None, "no step");
    }

    #[test]
    fn search_needs_every_word_in_any_order_and_ignores_case() {
        let entry = Entry::new("05-Domain-Warp".to_string(), PathBuf::from("x"));
        let ready = Health::Ready;
        assert!(
            matches(&entry, &ready, "", HealthFilter::All),
            "empty query"
        );
        assert!(
            matches(&entry, &ready, "warp domain", HealthFilter::All),
            "any order"
        );
        assert!(matches(&entry, &ready, "05", HealthFilter::All), "prefix");
        assert!(
            !matches(&entry, &ready, "tunnel", HealthFilter::All),
            "no match"
        );
        assert!(
            !matches(&entry, &ready, "warp tunnel", HealthFilter::All),
            "all words"
        );
    }

    #[test]
    fn the_health_filter_separates_working_from_broken() {
        let entry = Entry::new("a".to_string(), PathBuf::from("x"));
        let failed = Health::Failed("main.glsl:3".to_string());
        assert!(
            matches(&entry, &failed, "", HealthFilter::Failed),
            "failed shows"
        );
        assert!(
            !matches(&entry, &failed, "", HealthFilter::Ready),
            "failed hidden"
        );
        assert!(
            !matches(&entry, &Health::Unknown, "", HealthFilter::Failed),
            "unknown is not failed"
        );
        assert!(
            matches(&entry, &Health::Unknown, "", HealthFilter::All),
            "all admits unknown"
        );
    }

    #[test]
    fn filters_round_trip_through_their_keys() {
        for filter in HealthFilter::ALL {
            assert_eq!(HealthFilter::from_key(filter.key()), filter, "{filter:?}");
        }
        assert_eq!(
            HealthFilter::from_key("???"),
            HealthFilter::All,
            "unknown key"
        );
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
