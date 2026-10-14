// ============================================================================
// Module:       strobe_app::library
// Description:  The scene library: the folders under --library, then
//               Synesthesia's own scenes read-only where it is installed,
//               listed for the browser and found by name for --scene, plus
//               each scene's health and the browser's search and filter.
//
// Dependencies: strobe-scene (load::scan, folder_title, tags, synesthesia)
// ============================================================================

//! The scene library.
//!
//! A library is a folder of scene folders. Scanning lists them (a folder
//! containing `main.glsl`) and reads each one's `TAGS` (D205); a scene is
//! read and compiled when it is selected, so a broken scene never stops the
//! browser from listing. Reading tags cannot fail a scan: a missing or
//! broken `scene.json` is a scene with no tags.
//!
//! ## Sources (D271)
//!
//! The person's own folder comes first. Where Synesthesia is installed,
//! its scene folders (`strobe_scene::synesthesia`) are listed after it,
//! unless Settings or `--no-synesthesia` says not to: the scenes it ships,
//! inside its install and read-only, then the scene folders the person
//! added in Synesthesia's own settings, which are theirs and written like
//! the library's. Each [`Entry`] knows its [`Source`], so the browser can
//! say where a scene came from and the app can refuse to write into the
//! install ([`Synesthesia::owns`]). A name in more than one place is listed
//! once per place; everything that looks a scene up by name finds the
//! library folder's own first, because it comes first. A folder listed
//! twice (Synesthesia's settings naming the library folder itself, say) is
//! listed once, as the library's.
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
use strobe_scene::synesthesia::SceneFolder;

/// Where a library scene came from (D271).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub enum Source {
    /// The person's own library folder: listed first, and written to.
    #[default]
    Own,
    /// The scenes Synesthesia ships, inside its install: listed after, and
    /// never written to.
    Synesthesia,
    /// A scene folder the person added in Synesthesia's settings: listed
    /// after the install's, and theirs, so written like the library's.
    SynesthesiaFolder,
}

impl Source {
    /// What the browser and the palette call it, when it is worth saying.
    pub fn label(self) -> Option<&'static str> {
        match self {
            Self::Own => None,
            Self::Synesthesia | Self::SynesthesiaFolder => Some("Synesthesia"),
        }
    }

    /// Whether Strobe may write into a scene from here.
    pub fn writable(self) -> bool {
        match self {
            Self::Own | Self::SynesthesiaFolder => true,
            Self::Synesthesia => false,
        }
    }

    /// Whether it is one of the Synesthesia sources.
    pub fn from_synesthesia(self) -> bool {
        match self {
            Self::Own => false,
            Self::Synesthesia | Self::SynesthesiaFolder => true,
        }
    }

    /// The search word that finds every scene from here.
    fn keyword(self) -> Option<&'static str> {
        match self {
            Self::Own => None,
            Self::Synesthesia | Self::SynesthesiaFolder => Some("synesthesia"),
        }
    }
}

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
    /// Where it came from (D271).
    pub source: Source,
    /// Who made it: the scene's `CREDIT`, or the author of the Shadertoy
    /// shader it was imported from; empty when it says nothing (D273).
    pub author: String,
    /// `author` lowercased once, for searching and sorting.
    pub author_lower: String,
}

impl Entry {
    /// An entry for the scene folder `dir` called `name`, with no tags, in
    /// the person's own library.
    pub fn new(name: String, dir: PathBuf) -> Self {
        let lower = name.to_lowercase();
        Self {
            name,
            dir,
            lower,
            tags: Vec::new(),
            thumbnail: None,
            source: Source::Own,
            author: String::new(),
            author_lower: String::new(),
        }
    }

    /// This entry, credited to `author`.
    pub fn with_author(mut self, author: &str) -> Self {
        self.set_author(author);
        self
    }

    /// Credits the scene to `author` (trimmed; blank is nobody).
    pub fn set_author(&mut self, author: &str) {
        self.author = author.trim().to_string();
        self.author_lower = self.author.to_lowercase();
    }

    /// This entry with these tags.
    pub fn with_tags(mut self, tags: &[&str]) -> Self {
        self.tags = strobe_scene::tags::normalized(tags.iter().copied());
        self
    }

    /// This entry, from `source`.
    pub fn from_source(mut self, source: Source) -> Self {
        self.source = source;
        self
    }

    /// Whether Strobe must not write into this scene's folder.
    pub fn read_only(&self) -> bool {
        !self.source.writable()
    }

    /// Reads the tags and the picture from the folder's `scene.json` again
    /// (at a scan, and after the Editor or the tag field wrote it). Absent
    /// or broken is none.
    pub fn reread_tags(&mut self) {
        let text = std::fs::read_to_string(self.dir.join("scene.json")).unwrap_or_default();
        self.tags = strobe_scene::tags::read(&text);
        self.thumbnail = image_path(&text).map(|relative| self.dir.join(relative));
        let author = author_of(&text);
        self.set_author(&author);
    }
}

/// Who `scene.json` text credits: `CREDIT`, else the `SOURCE.author` of a
/// shader imported from elsewhere; empty for neither (D273).
fn author_of(json: &str) -> String {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json.trim_start_matches('\u{feff}'))
    else {
        return String::new();
    };
    let named = |value: Option<&serde_json::Value>| {
        value
            .and_then(serde_json::Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_string)
    };
    named(value.get("CREDIT"))
        .or_else(|| named(value.get("SOURCE").and_then(|source| source.get("author"))))
        .unwrap_or_default()
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

/// What the library grid is ordered by (D273). The default is the order the
/// scan found them in: the library folder's scenes by name, then
/// Synesthesia's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SortKey {
    /// The scan's order.
    #[default]
    Scan,
    /// By name, ignoring case, folders of both sources mixed.
    Name,
    /// By author, scenes with none last, then by name.
    Author,
    /// Favourites first, then by name.
    Favourites,
}

impl SortKey {
    /// Every key, in the order the dropdown offers them.
    pub const ALL: [SortKey; 4] = [Self::Scan, Self::Name, Self::Author, Self::Favourites];

    /// The key the preferences file stores.
    pub fn key(self) -> &'static str {
        match self {
            Self::Scan => "scan",
            Self::Name => "name",
            Self::Author => "author",
            Self::Favourites => "favourites",
        }
    }

    /// The dropdown's label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Scan => "Library order",
            Self::Name => "Name",
            Self::Author => "Author",
            Self::Favourites => "Favourites first",
        }
    }

    /// The key stored under `key`, or the default.
    pub fn from_key(key: &str) -> Self {
        Self::ALL
            .into_iter()
            .find(|sort| sort.key() == key)
            .unwrap_or_default()
    }

    /// How `a` orders against `b` under this key, ascending: the entry and
    /// whether it is a favourite. Run when the visible list is recomputed (a
    /// change of query, sort or library), never per frame.
    pub fn compare(self, a: (&Entry, bool), b: (&Entry, bool)) -> std::cmp::Ordering {
        let by_name = || a.0.lower.cmp(&b.0.lower);
        match self {
            Self::Scan => std::cmp::Ordering::Equal,
            Self::Name => by_name(),
            Self::Author => match (a.0.author.is_empty(), b.0.author.is_empty()) {
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
                (true, true) => by_name(),
                (false, false) => a.0.author_lower.cmp(&b.0.author_lower).then_with(by_name),
            },
            Self::Favourites => b.1.cmp(&a.1).then_with(by_name),
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
/// The query is a list of words, all of which must appear in the name, the
/// author or one of the tags, in any order: `dark tunnel` finds a scene tagged
/// `dark` whose name says tunnel. The word `synesthesia` also finds every
/// scene listed from Synesthesia's install (D271). `broken` and
/// `favorite`-style keywords are not special yet; the health filter covers
/// "broken".
pub fn matches(
    entry: &Entry,
    health: &Health,
    favourite: bool,
    query_lower: &str,
    filter: HealthFilter,
) -> bool {
    filter.admits(health, favourite)
        && query_lower.split_whitespace().all(|word| {
            entry.lower.contains(word)
                || entry.author_lower.contains(word)
                || entry.tags.iter().any(|tag| tag.contains(word))
                || entry.source.keyword() == Some(word)
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
        Self::scan_with(dir, &[])
    }

    /// Lists the scenes under `dir`, then those in each of `synesthesia`
    /// (Synesthesia's scene folders, [`Synesthesia::shown`], D271). Only
    /// `dir` can fail the scan: a Synesthesia folder that cannot be read is
    /// logged and contributes nothing.
    pub fn scan_with(dir: &Path, synesthesia: &[SceneFolder]) -> Result<Self, String> {
        let folders =
            strobe_scene::load::scan(dir).map_err(|e| format!("library {}: {e}", dir.display()))?;
        let mut library = Self {
            dir: dir.to_path_buf(),
            entries: entries(folders, Source::Own),
        };
        library.set_synesthesia(synesthesia);
        Ok(library)
    }

    /// Replaces the entries listed from Synesthesia with the scenes in
    /// `folders`, keeping the library folder's own: how the Settings switch
    /// and a launch whose own folder could not be read reach them. A folder
    /// inside the install lists read-only entries; one of the person's
    /// (from Synesthesia's settings) is searched as deep as it says, and
    /// its entries are writable. A scene already listed is not listed again,
    /// and a folder that is the library folder itself is skipped.
    pub fn set_synesthesia(&mut self, folders: &[SceneFolder]) {
        self.entries.retain(|entry| entry.source == Source::Own);
        for folder in folders {
            if crate::intake::same_dir(&folder.path, &self.dir) {
                continue;
            }
            let source = if folder.read_only {
                Source::Synesthesia
            } else {
                Source::SynesthesiaFolder
            };
            match strobe_scene::load::scan_deep(&folder.path, folder.depth) {
                Ok(scenes) => {
                    let fresh: Vec<PathBuf> = scenes
                        .into_iter()
                        .filter(|scene| self.entries.iter().all(|entry| entry.dir != *scene))
                        .collect();
                    self.entries.extend(entries(fresh, source));
                }
                Err(error) => log::warn!(
                    "Synesthesia's scenes in {} are not listed: {error}",
                    folder.path.display()
                ),
            }
        }
    }

    /// How many entries come from `source`.
    pub fn count(&self, source: Source) -> usize {
        self.entries.iter().filter(|e| e.source == source).count()
    }

    /// How many entries come from Synesthesia, its install's and the
    /// person's folders from its settings together.
    pub fn synesthesia_count(&self) -> usize {
        self.entries
            .iter()
            .filter(|e| e.source.from_synesthesia())
            .count()
    }

    /// The scene `query` names: an exact name, else the first whose name
    /// starts with it, else the first that contains it. The person's own
    /// scenes come first, so a name both sources have finds theirs.
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

/// Library entries for these scene folders, from `source`, with their tags
/// and pictures read.
fn entries(folders: Vec<PathBuf>, source: Source) -> Vec<Entry> {
    folders
        .into_iter()
        .map(|folder| {
            let mut entry =
                Entry::new(strobe_scene::load::folder_title(&folder), folder).from_source(source);
            entry.reread_tags();
            entry
        })
        .collect()
}

/// Synesthesia's scenes as a library source (D271): the install found at
/// launch, its scene folders, and whether this run may list them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Synesthesia {
    /// The install, when one was found.
    pub install: Option<strobe_scene::synesthesia::Install>,
    /// Its scene folders that exist: the built-in ones, then the person's
    /// from its settings.
    pub folders: Vec<SceneFolder>,
    /// Why any folder its settings name is not among them, for the log.
    pub notes: Vec<String>,
    /// `--no-synesthesia`: not listed this run, whatever Settings says.
    pub off_this_run: bool,
}

impl Synesthesia {
    /// Looks for Synesthesia on this machine
    /// (`strobe_scene::synesthesia::detect`, the same answer `#include`
    /// gets) and reads its settings for the person's scene folders. What
    /// could not be read is logged here, once per look, and is never an
    /// error.
    pub fn detect(off_this_run: bool) -> Self {
        let found = Self::from_install(strobe_scene::synesthesia::detect(), off_this_run);
        for note in &found.notes {
            log::info!("Synesthesia's scene folders: {note}");
        }
        found
    }

    /// The source for `install`, reading its settings files.
    pub fn from_install(
        install: Option<strobe_scene::synesthesia::Install>,
        off_this_run: bool,
    ) -> Self {
        let found = install
            .as_ref()
            .map(strobe_scene::synesthesia::Install::scene_folders)
            .unwrap_or_default();
        Self {
            install,
            folders: found.folders,
            notes: found.notes,
            off_this_run,
        }
    }

    /// The folders to list, with the Settings switch at `setting`: none
    /// when it is off, for this run, or not installed.
    pub fn shown(&self, setting: bool) -> &[SceneFolder] {
        if setting && !self.off_this_run {
            &self.folders
        } else {
            &[]
        }
    }

    /// Whether `dir` is inside Synesthesia's install, listed or not: a
    /// folder Strobe never writes into. The person's own scene folders from
    /// Synesthesia's settings are not, unless they are inside it. On
    /// Windows the comparison ignores case, as the file system does, so a
    /// playlist that spells the path differently is still caught.
    pub fn owns(&self, dir: &Path) -> bool {
        self.install
            .as_ref()
            .is_some_and(|install| inside(dir, &install.dir))
    }

    /// One line for Settings: what is listed, or why nothing is.
    pub fn note(&self, setting: bool, listed: usize) -> String {
        let Some(install) = &self.install else {
            return "Synesthesia is not installed here, so there is nothing to show.".to_string();
        };
        let at = install.dir.display();
        let theirs = self.folders.iter().filter(|f| !f.read_only).count();
        if self.off_this_run {
            format!("Off for this run (--no-synesthesia). Installed at {at}.")
        } else if !setting {
            format!("Off: the scenes in {at} are not listed.")
        } else if self.folders.is_empty() {
            format!("Installed at {at}, with no scene folder Strobe knows.")
        } else {
            let scenes = if listed == 1 { "scene" } else { "scenes" };
            let folders = match theirs {
                0 => String::new(),
                1 => " and your scene folder in its settings".to_string(),
                n => format!(" and your {n} scene folders in its settings"),
            };
            format!("{listed} {scenes} from {at}{folders}. Its own are read-only.")
        }
    }
}

/// Whether `dir` is `root` or inside it; ignoring case on Windows.
fn inside(dir: &Path, root: &Path) -> bool {
    let lower = |path: &Path| PathBuf::from(path.to_string_lossy().to_lowercase());
    dir.starts_with(root) || (cfg!(windows) && lower(dir).starts_with(lower(root)))
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

    #[test]
    fn the_author_is_the_credit_then_the_shader_it_came_from_then_nobody() {
        assert_eq!(
            author_of(r#"{ "CREDIT": " Ada ", "SOURCE": { "author": "iq" } }"#),
            "Ada",
            "the credit wins, trimmed"
        );
        assert_eq!(
            author_of(r#"{ "SOURCE": { "kind": "shadertoy", "author": "iq" } }"#),
            "iq",
            "an imported shader names its own author"
        );
        assert_eq!(author_of(r#"{ "CREDIT": "  " }"#), "", "blank is nobody");
        assert_eq!(author_of("not json"), "", "a broken file credits nobody");
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

    #[test]
    fn a_fake_install_is_listed_after_the_library_read_only() -> anyhow::Result<()> {
        let scratch = fake::Scratch::new("listed")?;
        let synesthesia = fake::install(&scratch.0)?;
        let own = scratch.0.join("mine");
        fake::scene(&own.join("ink.synScene"), &["mine"])?;
        fake::scene(&own.join("aurora"), &[])?;
        let library =
            Library::scan_with(&own, synesthesia.shown(true)).map_err(anyhow::Error::msg)?;
        let listed: Vec<(&str, Source)> = library
            .entries
            .iter()
            .map(|entry| (entry.name.as_str(), entry.source))
            .collect();
        assert_eq!(
            listed,
            [
                ("aurora", Source::Own),
                ("ink", Source::Own),
                ("glow", Source::Synesthesia),
                ("ink", Source::Synesthesia),
                ("hello", Source::Synesthesia),
                ("deep", Source::SynesthesiaFolder),
            ],
            "the library's own first, then release and default, then the person's folder \
             from Synesthesia's settings, searched below its top level"
        );
        assert!(
            library
                .entries
                .iter()
                .all(|entry| entry.read_only() == (entry.source == Source::Synesthesia)),
            "only the scenes Synesthesia ships are read-only; the person's folder is theirs"
        );
        assert!(
            library
                .entries
                .iter()
                .filter(|entry| entry.read_only())
                .all(|entry| synesthesia.owns(&entry.dir)),
            "and every one of them is inside the install"
        );
        assert!(
            !library
                .entries
                .iter()
                .filter(|entry| !entry.read_only())
                .any(|entry| synesthesia.owns(&entry.dir)),
            "the writable ones are never inside the install"
        );
        assert_eq!(
            library.entries.get(2).map(|entry| entry.tags.clone()),
            Some(vec!["warm".to_string()]),
            "their TAGS are read and normalised like any scene's"
        );
        assert_eq!(
            library.find("ink"),
            Some(1),
            "a clash finds the person's own"
        );
        assert_eq!(
            library.find("glo"),
            Some(2),
            "and the rest are found by name"
        );
        assert_eq!(library.count(Source::Synesthesia), 3, "counted");
        assert_eq!(library.synesthesia_count(), 4, "with the person's folder");
        let searched: Vec<&str> = library
            .entries
            .iter()
            .filter(|entry| {
                matches(
                    entry,
                    &Health::Unknown,
                    false,
                    "synesthesia",
                    HealthFilter::All,
                )
            })
            .map(|entry| entry.name.as_str())
            .collect();
        assert_eq!(
            searched,
            ["glow", "ink", "hello", "deep"],
            "the word synesthesia finds every scene from Synesthesia"
        );
        assert_eq!(
            synesthesia.note(true, 4),
            format!(
                "4 scenes from {} and your scene folder in its settings. Its own are read-only.",
                scratch.0.join("install").display()
            ),
            "Settings says what is listed and from where"
        );
        Ok(())
    }

    #[test]
    fn switching_synesthesia_off_lists_only_the_library() -> anyhow::Result<()> {
        let scratch = fake::Scratch::new("off")?;
        let synesthesia = fake::install(&scratch.0)?;
        let own = scratch.0.join("mine");
        fake::scene(&own.join("aurora"), &[])?;
        let mut library =
            Library::scan_with(&own, synesthesia.shown(true)).map_err(anyhow::Error::msg)?;
        assert_eq!(
            library.entries.len(),
            5,
            "on: the library, the install and the person's Synesthesia folder"
        );
        assert!(
            synesthesia.shown(false).is_empty(),
            "the switch off lists none"
        );
        library.set_synesthesia(synesthesia.shown(false));
        let names: Vec<&str> = library.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["aurora"], "off: exactly what the library has");
        assert_eq!(
            Library::scan_with(&own, synesthesia.shown(false))
                .map_err(anyhow::Error::msg)?
                .entries,
            Library::scan(&own).map_err(anyhow::Error::msg)?.entries,
            "and the same as before Synesthesia was ever looked for"
        );
        library.set_synesthesia(synesthesia.shown(true));
        assert_eq!(library.entries.len(), 5, "on again");
        let this_run = Synesthesia {
            off_this_run: true,
            ..synesthesia.clone()
        };
        assert!(
            this_run.shown(true).is_empty(),
            "--no-synesthesia wins over the setting"
        );
        assert!(
            this_run.note(true, 0).contains("--no-synesthesia"),
            "and Settings says why"
        );
        assert!(
            synesthesia.owns(&scratch.0.join("install/release/glow.synScene")),
            "switched off, its install is still never written"
        );
        assert!(
            !synesthesia.owns(&scratch.0.join("syn-mine/packs/deep.synScene")),
            "but the person's own Synesthesia folder is theirs to write"
        );
        let missing = Synesthesia::from_install(None, false);
        assert!(
            missing.shown(true).is_empty() && !missing.owns(&own),
            "not installed: nothing listed, nothing refused"
        );
        assert!(
            missing.note(true, 0).contains("not installed"),
            "and Settings says so"
        );
        Ok(())
    }

    #[test]
    fn a_folder_of_synesthesias_that_cannot_be_read_lists_nothing_and_fails_nothing()
    -> anyhow::Result<()> {
        let scratch = fake::Scratch::new("unreadable")?;
        let own = scratch.0.join("mine");
        fake::scene(&own.join("aurora"), &[])?;
        let gone = SceneFolder {
            path: scratch.0.join("gone"),
            read_only: true,
            depth: 1,
        };
        let library = Library::scan_with(&own, &[gone]).map_err(anyhow::Error::msg)?;
        assert_eq!(
            library.entries.len(),
            1,
            "the library's own scenes still list"
        );
        Ok(())
    }

    #[test]
    fn synesthesias_settings_naming_the_library_folder_list_it_once() -> anyhow::Result<()> {
        let scratch = fake::Scratch::new("same-folder")?;
        let own = scratch.0.join("mine");
        fake::scene(&own.join("aurora"), &[])?;
        let same = SceneFolder {
            path: own.clone(),
            read_only: false,
            depth: 10,
        };
        let library = Library::scan_with(&own, &[same]).map_err(anyhow::Error::msg)?;
        let listed: Vec<Source> = library.entries.iter().map(|e| e.source).collect();
        assert_eq!(listed, [Source::Own], "once, as the library's own");
        Ok(())
    }
}

/// A fake Synesthesia install for tests (D271): original, tiny scenes in
/// the folders a real one has, found through `STROBE_SYNESTHESIA_DIR` as
/// the app finds a real one. None of Synesthesia's own files.
#[cfg(test)]
pub(crate) mod fake {
    use super::Synesthesia;
    use std::path::{Path, PathBuf};
    use strobe_scene::synesthesia::{DATA_VAR, DIR_VAR, Env, detect_with};

    /// A temporary folder, removed on drop.
    pub(crate) struct Scratch(pub(crate) PathBuf);

    impl Scratch {
        pub(crate) fn new(tag: &str) -> anyhow::Result<Self> {
            let dir = std::env::temp_dir().join(format!(
                "strobe-fake-synesthesia-{}-{tag}",
                std::process::id()
            ));
            if dir.exists() {
                std::fs::remove_dir_all(&dir)?;
            }
            std::fs::create_dir_all(&dir)?;
            Ok(Self(dir))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// Writes a scene folder at `dir` with `tags`.
    pub(crate) fn scene(dir: &Path, tags: &[&str]) -> anyhow::Result<()> {
        std::fs::create_dir_all(dir)?;
        std::fs::write(
            dir.join("main.glsl"),
            "vec4 renderMain(void) { return vec4(_uv, 0.5, 1.0); }\n",
        )?;
        let tags: Vec<String> = tags.iter().map(|tag| format!("\"{tag}\"")).collect();
        std::fs::write(
            dir.join("scene.json"),
            format!(
                "{{\n  \"TITLE\": \"fake\",\n  \"TAGS\": [{}]\n}}\n",
                tags.join(", ")
            ),
        )?;
        Ok(())
    }

    /// An install under `root/install`: `release` holds `glow` and `ink`,
    /// `default` holds `hello`. Its data folder `root/data` has a `vs.json`
    /// naming the person's scene folder `root/syn-mine`, which holds
    /// `packs/deep` two levels down. Found through `STROBE_SYNESTHESIA_DIR`
    /// and `STROBE_SYNESTHESIA_DATA`.
    pub(crate) fn install(root: &Path) -> anyhow::Result<Synesthesia> {
        let dir = root.join("install");
        let data = root.join("data");
        let theirs = root.join("syn-mine");
        scene(&dir.join("release").join("glow.synScene"), &["Warm"])?;
        scene(&dir.join("release").join("ink.synScene"), &["dark"])?;
        scene(&dir.join("default").join("hello.synScene"), &[])?;
        scene(&theirs.join("packs").join("deep.synScene"), &[])?;
        std::fs::create_dir_all(&data)?;
        std::fs::write(
            data.join("vs.json"),
            format!(
                "{{ \"primarySceneFolder\": \"\", \"sceneFolders\": [{}], \"sceneSources\": {{}} }}",
                serde_json::to_string(&theirs.display().to_string())?
            ),
        )?;
        let lookup = |name: &str| match name {
            DIR_VAR => Some(dir.clone().into_os_string()),
            DATA_VAR => Some(data.clone().into_os_string()),
            _ => None,
        };
        let found = detect_with(&Env {
            var: &lookup,
            windows: false,
        });
        anyhow::ensure!(found.is_some(), "the override was not taken");
        Ok(Synesthesia::from_install(found, false))
    }
}
