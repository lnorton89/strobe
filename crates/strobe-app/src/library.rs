// ============================================================================
// Module:       strobe_app::library
// Description:  The scene library: the folders under --library, listed for
//               the browser and found by name for --scene, plus each
//               scene's health and the browser's search and filter.
//
// Dependencies: strobe-scene (load::scan, folder_title, tags)
// ============================================================================

//! The scene library.
//!
//! A library is a folder of scene folders. Scanning lists them (a folder
//! containing `main.glsl`) and reads each one's `TAGS` (D205); a scene is
//! read and compiled when it is selected, so a broken scene never stops the
//! browser from listing. Reading tags cannot fail a scan: a missing or
//! broken `scene.json` is a scene with no tags.
//!
//! ## Health and search
//!
//! A scan only lists folders, so a scene's [`Health`] is what the app has
//! *seen*: unknown until it is loaded, ready after it loads, failed (with
//! the first line of the error) after it does not. The browser filters by
//! it ([`HealthFilter`]) and searches names and tags ([`matches`]); both are cheap
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
    /// The scene's `TAGS`, normalised (`strobe_scene::tags`).
    pub tags: Vec<String>,
    /// The scene's picture (`IMAGE_PATH`), when it names one inside its
    /// folder (D212). Whether the file exists is found out when it is drawn.
    pub thumbnail: Option<PathBuf>,
}

impl Entry {
    /// An entry for the scene folder `dir` called `name`, with no tags.
    pub fn new(name: String, dir: PathBuf) -> Self {
        let lower = name.to_lowercase();
        Self {
            name,
            dir,
            lower,
            tags: Vec::new(),
            thumbnail: None,
        }
    }

    /// This entry with these tags.
    pub fn with_tags(mut self, tags: &[&str]) -> Self {
        self.tags = strobe_scene::tags::normalized(tags.iter().copied());
        self
    }

    /// Reads the tags and the picture from the folder's `scene.json` again
    /// (at a scan, and after the Editor or the tag field wrote it). Absent
    /// or broken is none.
    pub fn reread_tags(&mut self) {
        let text = std::fs::read_to_string(self.dir.join("scene.json")).unwrap_or_default();
        self.tags = strobe_scene::tags::read(&text);
        self.thumbnail = image_path(&text).map(|relative| self.dir.join(relative));
    }
}

/// `IMAGE_PATH` from `scene.json` text, when it is a path inside the scene
/// folder (`strobe_scene::scene::is_folder_relative`); a path that leaves
/// the folder is ignored, as the scene loader ignores it.
fn image_path(json: &str) -> Option<String> {
    let value: serde_json::Value =
        serde_json::from_str(json.trim_start_matches('\u{feff}')).ok()?;
    value
        .get("IMAGE_PATH")?
        .as_str()
        .map(str::trim)
        .filter(|path| strobe_scene::scene::is_folder_relative(path))
        .map(str::to_string)
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
    /// Scenes starred as favourites (D211).
    Favourites,
}

impl HealthFilter {
    /// Every filter, in the order the browser offers them.
    pub const ALL: [HealthFilter; 4] = [Self::All, Self::Favourites, Self::Ready, Self::Failed];

    /// The key the preferences file stores.
    pub fn key(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Ready => "ready",
            Self::Failed => "failed",
            Self::Favourites => "favourites",
        }
    }

    /// The chip's label.
    pub fn label(self) -> &'static str {
        match self {
            Self::All => "All",
            Self::Ready => "Ready",
            Self::Failed => "Needs work",
            Self::Favourites => "Favourites",
        }
    }

    /// The filter stored under `key`, or `All`.
    pub fn from_key(key: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|filter| filter.key() == key)
            .unwrap_or_default()
    }

    /// Whether a scene of this health, a favourite or not, passes.
    pub fn admits(self, health: &Health, favourite: bool) -> bool {
        match self {
            Self::All => true,
            Self::Ready => *health == Health::Ready,
            Self::Failed => matches!(health, Health::Failed(_)),
            Self::Favourites => favourite,
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
/// The query is a list of words, all of which must appear in the name or
/// in one of the tags, in any order: `dark tunnel` finds a scene tagged
/// `dark` whose name says tunnel. `broken` and `favorite`-style keywords
/// are not special yet; the health filter covers "broken".
pub fn matches(
    entry: &Entry,
    health: &Health,
    favourite: bool,
    query_lower: &str,
    filter: HealthFilter,
) -> bool {
    filter.admits(health, favourite)
        && query_lower.split_whitespace().all(|word| {
            entry.lower.contains(word) || entry.tags.iter().any(|tag| tag.contains(word))
        })
}

/// The scenes found under one folder.
#[derive(Debug, Default)]
pub struct Library {
    /// The folder scanned.
    pub dir: PathBuf,
    /// Scenes sorted by name.
    pub entries: Vec<Entry>,
}

/// The library folder `path` names. A relative path that is not a folder
/// under the current directory is looked for beside the program, so a
/// packaged build finds its bundled `scenes` whatever folder it is started
/// from (a terminal, a shortcut, `PATH`; T480 Task 21). A path found in
/// neither place is returned unchanged, and the scan reports it.
pub fn resolve(path: &Path) -> PathBuf {
    let beside = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    resolve_with(path, beside.as_deref())
}

/// [`resolve`] with the program's folder given.
pub fn resolve_with(path: &Path, program_dir: Option<&Path>) -> PathBuf {
    if path.is_absolute() || path.is_dir() {
        return path.to_path_buf();
    }
    match program_dir.map(|dir| dir.join(path)) {
        Some(candidate) if candidate.is_dir() => candidate,
        Some(_) | None => path.to_path_buf(),
    }
}

impl Library {
    /// Lists the scenes under `dir`.
    pub fn scan(dir: &Path) -> Result<Self, String> {
        let folders =
            strobe_scene::load::scan(dir).map_err(|e| format!("library {}: {e}", dir.display()))?;
        let entries = folders
            .into_iter()
            .map(|folder| {
                let mut entry = Entry::new(strobe_scene::load::folder_title(&folder), folder);
                entry.reread_tags();
                entry
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

    #[test]
    fn a_relative_library_is_found_beside_the_program() -> anyhow::Result<()> {
        let root = std::env::temp_dir().join(format!("strobe-resolve-{}", std::process::id()));
        let program = root.join("app");
        std::fs::create_dir_all(program.join("bundled-scenes-for-test"))?;
        let found = resolve_with(Path::new("bundled-scenes-for-test"), Some(&program));
        assert_eq!(
            found,
            program.join("bundled-scenes-for-test"),
            "not under the current directory, so beside the program"
        );
        let missing = resolve_with(Path::new("nowhere-at-all"), Some(&program));
        assert_eq!(
            missing,
            PathBuf::from("nowhere-at-all"),
            "found nowhere: unchanged, for the scan to report"
        );
        let absolute = root.join("elsewhere");
        assert_eq!(
            resolve_with(&absolute, Some(&program)),
            absolute,
            "an absolute path is taken as given"
        );
        std::fs::remove_dir_all(&root)?;
        Ok(())
    }

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
            matches(&entry, &ready, false, "", HealthFilter::All),
            "empty query"
        );
        assert!(
            matches(&entry, &ready, false, "warp domain", HealthFilter::All),
            "any order"
        );
        assert!(
            matches(&entry, &ready, false, "05", HealthFilter::All),
            "prefix"
        );
        assert!(
            !matches(&entry, &ready, false, "tunnel", HealthFilter::All),
            "no match"
        );
        assert!(
            !matches(&entry, &ready, false, "warp tunnel", HealthFilter::All),
            "all words"
        );
        let tagged = entry.with_tags(&["Dark", "audio reactive"]);
        assert!(
            matches(&tagged, &ready, false, "dark warp", HealthFilter::All),
            "a tag and a name word"
        );
        assert!(
            matches(&tagged, &ready, false, "reactive", HealthFilter::All),
            "part of a tag"
        );
    }

    #[test]
    fn a_scan_reads_tags_and_a_broken_scene_json_is_no_tags() -> anyhow::Result<()> {
        let root = std::env::temp_dir().join(format!("strobe-lib-tags-{}", std::process::id()));
        for (name, json) in [
            ("a.synScene", r#"{ "TAGS": ["Dark", "3D"] }"#),
            ("b.synScene", "{ broken"),
        ] {
            let dir = root.join(name);
            std::fs::create_dir_all(&dir)?;
            std::fs::write(
                dir.join("main.glsl"),
                "vec4 renderMain(void) { return vec4(1.0); }",
            )?;
            std::fs::write(dir.join("scene.json"), json)?;
        }
        std::fs::write(
            root.join("a.synScene").join("scene.json"),
            r#"{ "TAGS": ["Dark", "3D"], "IMAGE_PATH": "thumb.png" }"#,
        )?;
        let library = Library::scan(&root).map_err(anyhow::Error::msg)?;
        std::fs::remove_dir_all(&root)?;
        assert_eq!(
            library.entries.first().and_then(|e| e.thumbnail.clone()),
            Some(root.join("a.synScene").join("thumb.png")),
            "IMAGE_PATH, inside the folder"
        );
        assert_eq!(
            image_path(r#"{ "IMAGE_PATH": "../elsewhere.png" }"#),
            None,
            "a path out of the folder is ignored"
        );
        let tags: Vec<&[String]> = library.entries.iter().map(|e| e.tags.as_slice()).collect();
        assert_eq!(
            tags,
            [&["dark".to_string(), "3d".to_string()][..], &[][..]],
            "read and normalised; broken is none"
        );
        Ok(())
    }

    #[test]
    fn the_favourites_filter_shows_only_starred_scenes() {
        let entry = Entry::new("a".to_string(), PathBuf::from("x"));
        let ready = Health::Ready;
        assert!(
            matches(&entry, &ready, true, "", HealthFilter::Favourites),
            "a favourite shows"
        );
        assert!(
            !matches(&entry, &ready, false, "", HealthFilter::Favourites),
            "another does not"
        );
        assert!(
            matches(&entry, &ready, false, "", HealthFilter::All),
            "All ignores the star"
        );
        assert_eq!(
            HealthFilter::from_key("favourites"),
            HealthFilter::Favourites,
            "saved and read back by key"
        );
    }

    #[test]
    fn the_health_filter_separates_working_from_broken() {
        let entry = Entry::new("a".to_string(), PathBuf::from("x"));
        let failed = Health::Failed("main.glsl:3".to_string());
        assert!(
            matches(&entry, &failed, false, "", HealthFilter::Failed),
            "failed shows"
        );
        assert!(
            !matches(&entry, &failed, false, "", HealthFilter::Ready),
            "failed hidden"
        );
        assert!(
            !matches(&entry, &Health::Unknown, false, "", HealthFilter::Failed),
            "unknown is not failed"
        );
        assert!(
            matches(&entry, &Health::Unknown, false, "", HealthFilter::All),
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
