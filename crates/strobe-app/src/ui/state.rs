// ============================================================================
// Module:       strobe_app::ui::state
// Description:  The control window's presentation state: pane layout, the
//               browser's query and selection, the theme, and the machinery
//               that saves them when they change.
//
// Dependencies: crate::{workspace, prefs, palette, library, motion}
// ============================================================================

//! Presentation state, owned by the app and kept out of egui's memory
//! (UI_IMPLEMENTATION_PLAN section 3, layer B).
//!
//! egui can persist its own `Memory`, but product state hidden there cannot
//! be tested, restored in a known order, or changed by a keyboard command,
//! so none of this lives there. The panes, the browser's query and filter
//! and the theme are plain fields; [`UiState::tick`] notices when they
//! change and hands the debounced [`crate::prefs`] writer the new text.
//!
//! ## Cost per frame
//!
//! [`UiState::tick`] hashes a few fields (no allocation) and compares the
//! result with last frame's. Only when something changed does it start the
//! debounce, and only when the debounce fires does it build and serialise a
//! [`Prefs`]; the write itself happens on the writer thread. The browser's
//! filtered list is rebuilt when the query, the filter or the library
//! changes, never per frame.

use crate::library::{Health, HealthFilter, Library, matches};
use crate::palette::{Catalog, Palette};
use crate::prefs::{BrowserPref, Debounce, Prefs, Saver};
use crate::workspace::{Pane, Panes};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::time::{Duration, Instant};

/// The scene browser's workflow state.
#[derive(Debug, Default)]
pub struct BrowserState {
    /// The search text.
    pub query: String,
    /// The health filter.
    pub filter: HealthFilter,
    /// Set by a command (Ctrl+F) to focus the search field on the next
    /// frame the field is drawn.
    pub focus_search: bool,
    /// The scene highlighted in the list (not necessarily playing).
    pub selected: Option<usize>,
    /// A row (position in the visible list) to scroll into view on the next
    /// frame, set when the keyboard moves the selection.
    pub scroll_to: Option<usize>,
    /// Favourite scenes, by library name, sorted (D211).
    pub favourites: Vec<String>,
    visible: Vec<usize>,
    cached: Option<Cached>,
}

/// What the visible list was computed from.
#[derive(Debug)]
struct Cached {
    library_rev: u64,
    query: String,
    filter: HealthFilter,
    favourites: usize,
}

impl BrowserState {
    /// The library indices that pass the query and filter, in library
    /// order. Recomputed only when the query, filter or library changed.
    pub fn visible(&mut self, library: &Library, health: &[Health], rev: u64) -> &[usize] {
        let fresh = self.cached.as_ref().is_some_and(|cached| {
            cached.library_rev == rev
                && cached.filter == self.filter
                && cached.query == self.query
                && cached.favourites == self.favourites.len()
        });
        if !fresh {
            let query = self.query.trim().to_lowercase();
            self.visible.clear();
            let unknown = Health::Unknown;
            for (index, entry) in library.entries.iter().enumerate() {
                let health = health.get(index).unwrap_or(&unknown);
                let favourite = self.is_favourite(&entry.name);
                if matches(entry, health, favourite, &query, self.filter) {
                    self.visible.push(index);
                }
            }
            self.cached = Some(Cached {
                library_rev: rev,
                query: self.query.clone(),
                filter: self.filter,
                favourites: self.favourites.len(),
            });
        }
        &self.visible
    }
}

impl BrowserState {
    /// Whether the scene called `name` is a favourite.
    pub fn is_favourite(&self, name: &str) -> bool {
        self.favourites
            .binary_search_by(|f| f.as_str().cmp(name))
            .is_ok()
    }

    /// Makes `name` a favourite, or not. Saved with the layout.
    pub fn toggle_favourite(&mut self, name: &str) {
        match self.favourites.binary_search_by(|f| f.as_str().cmp(name)) {
            Ok(at) => {
                self.favourites.remove(at);
            }
            Err(at) => self.favourites.insert(at, name.to_string()),
        }
    }

    /// Moves the (fresh) visible list out so a caller can iterate it while
    /// mutating the rest of the state without copying it. Hand it back with
    /// [`Self::put_visible`].
    pub fn take_visible(&mut self, library: &Library, health: &[Health], rev: u64) -> Vec<usize> {
        let _ = self.visible(library, health, rev);
        std::mem::take(&mut self.visible)
    }

    /// Returns the list taken by [`Self::take_visible`].
    pub fn put_visible(&mut self, visible: Vec<usize>) {
        self.visible = visible;
    }
}

/// Everything about the control window's presentation that is the
/// operator's to keep.
pub struct UiState {
    /// Which panes are open, and how wide.
    pub panes: Panes,
    /// The scene browser.
    pub browser: BrowserState,
    /// Inspector groups the operator collapsed.
    pub collapsed_groups: Vec<String>,
    /// Draw Strobe's own title bar.
    pub custom_title_bar: bool,
    /// The diagnostics console shows only warnings and errors. Not saved:
    /// an operator who left it on would miss the info lines next time.
    pub console_problems_only: bool,
    /// The library folder name of the scene last loaded, reopened at the
    /// next launch (T480 Task 21).
    pub last_scene: String,
    /// The command palette. Not saved: it always starts closed.
    pub command_palette: super::command_palette::PaletteUi,
    /// The Editor's presentation state (go-to line, highlight cache).
    pub editor: super::editor::EditorUi,
    /// The Inspector's tag field while it is open. Not saved.
    pub tag_draft: Option<String>,
    /// The Settings pane's shortcut editor. Not saved.
    pub hotkeys: super::keys::HotkeyEditor,
    theme_id: String,
    reduced_motion: bool,
    catalog: Catalog,
    theme_changed: bool,
    saver: Option<Saver>,
    debounce: Debounce,
    last_fingerprint: u64,
    rng: u64,
}

impl UiState {
    /// The state saved in the preferences file, with the writer running.
    /// Reads the file (blocking), so it belongs at start-up only.
    pub fn load() -> Self {
        let path = crate::prefs::path();
        let (prefs, problem) = Prefs::load(&path);
        if let Some(problem) = problem {
            log::warn!("UI preferences not used: {problem}");
        }
        let mut state = Self::from_prefs(&prefs);
        state.saver = Some(Saver::start(path));
        state
    }

    /// The defaults, with nothing read and nothing ever written: screenshot
    /// mode, so a capture never depends on, or changes, the operator's
    /// layout.
    pub fn ephemeral() -> Self {
        Self::from_prefs(&Prefs::default())
    }

    /// State built from `prefs`, with no writer.
    pub fn from_prefs(prefs: &Prefs) -> Self {
        let catalog = Catalog::load();
        for (source, reason) in catalog.problems() {
            log::warn!("theme {source} not used: {reason}");
        }
        // The environment can force reduced motion on (an accessibility
        // switch that does not need the settings pane to reach).
        let reduced = prefs.reduced_motion || env_wants_reduced_motion();
        crate::motion::set_reduced(reduced);
        let mut state = Self {
            panes: prefs.to_panes(),
            browser: BrowserState {
                query: prefs.browser.query.clone(),
                filter: HealthFilter::from_key(&prefs.browser.filter),
                favourites: {
                    // A hand-edited file may be unsorted or repeat a name.
                    let mut names = prefs.browser.favourites.clone();
                    names.sort();
                    names.dedup();
                    names
                },
                ..BrowserState::default()
            },
            collapsed_groups: prefs.collapsed_groups.clone(),
            custom_title_bar: prefs.custom_title_bar,
            console_problems_only: false,
            command_palette: super::command_palette::PaletteUi::default(),
            editor: super::editor::EditorUi::default(),
            tag_draft: None,
            hotkeys: super::keys::HotkeyEditor::default(),
            last_scene: prefs.last_scene.clone(),
            theme_id: prefs.theme.clone(),
            reduced_motion: reduced,
            catalog,
            theme_changed: true,
            saver: None,
            debounce: Debounce::default(),
            last_fingerprint: 0,
            rng: 0x9e37_79b9_7f4a_7c15,
        };
        state.last_fingerprint = state.fingerprint();
        state
    }

    /// The saved form of this state.
    pub fn to_prefs(&self) -> Prefs {
        let mut prefs = Prefs::from_parts(
            &self.panes,
            &self.theme_id,
            self.custom_title_bar,
            self.reduced_motion,
            BrowserPref {
                query: self.browser.query.clone(),
                filter: self.browser.filter.key().to_string(),
                favourites: self.browser.favourites.clone(),
            },
            self.collapsed_groups.clone(),
        );
        prefs.last_scene.clone_from(&self.last_scene);
        prefs
    }

    /// Opens the named panes, for `--open-pane`. Unknown names are returned
    /// so the caller can say so.
    pub fn open_named(&mut self, names: &[String]) -> Vec<String> {
        let mut unknown = Vec::new();
        for name in names {
            match Pane::from_key(name) {
                Some(pane) => self.panes.open(pane),
                None => unknown.push(name.clone()),
            }
        }
        unknown
    }

    /// The theme catalog.
    pub fn catalog(&self) -> &Catalog {
        &self.catalog
    }

    /// The active theme's id.
    pub fn theme_id(&self) -> &str {
        &self.theme_id
    }

    /// The active palette.
    pub fn palette(&self) -> &Palette {
        self.catalog.get(Some(&self.theme_id))
    }

    /// Chooses a theme by id.
    pub fn set_theme(&mut self, id: &str) {
        if self.theme_id != id {
            self.theme_id = id.to_string();
            self.theme_changed = true;
        }
    }

    /// Whether the theme changed since the last call (the host then
    /// re-derives egui's style).
    pub fn take_theme_change(&mut self) -> bool {
        std::mem::take(&mut self.theme_changed)
    }

    /// Whether reduced motion is on.
    pub fn reduced_motion(&self) -> bool {
        self.reduced_motion
    }

    /// Turns reduced motion on or off.
    pub fn set_reduced_motion(&mut self, on: bool) {
        self.reduced_motion = on;
        crate::motion::set_reduced(on || env_wants_reduced_motion());
    }

    /// A pseudo-random number in `0..1`, for the inspector's Random button.
    /// xorshift64*: the operator wants variety, not cryptography, and the
    /// bank owns no generator by design.
    pub fn random_unit(&mut self) -> f32 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        let scrambled = x.wrapping_mul(0x2545_f491_4f6c_dd1d);
        (scrambled >> 40) as f32 / (1_u64 << 24) as f32
    }

    /// Whether the inspector group `name` is collapsed.
    pub fn is_collapsed(&self, name: &str) -> bool {
        self.collapsed_groups.iter().any(|group| group == name)
    }

    /// Collapses or expands the inspector group `name`.
    pub fn set_collapsed(&mut self, name: &str, collapsed: bool) {
        let at = self.collapsed_groups.iter().position(|group| group == name);
        match (collapsed, at) {
            (true, None) => self.collapsed_groups.push(name.to_string()),
            (false, Some(at)) => {
                self.collapsed_groups.remove(at);
            }
            (true, Some(_)) | (false, None) => {}
        }
    }

    fn fingerprint(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        self.panes.hash_into(&mut hasher);
        self.browser.query.hash(&mut hasher);
        self.browser.filter.key().hash(&mut hasher);
        self.browser.favourites.hash(&mut hasher);
        self.collapsed_groups.hash(&mut hasher);
        self.theme_id.hash(&mut hasher);
        self.custom_title_bar.hash(&mut hasher);
        self.reduced_motion.hash(&mut hasher);
        self.last_scene.hash(&mut hasher);
        hasher.finish()
    }

    /// Called once per control-window frame: notices a change and, when it
    /// has settled, queues a save. Cheap when nothing changed.
    pub fn tick(&mut self, now: Instant) {
        let fingerprint = self.fingerprint();
        let changed = fingerprint != self.last_fingerprint;
        self.last_fingerprint = fingerprint;
        if self.debounce.note(changed, now) {
            self.save_now();
        }
    }

    fn save_now(&self) {
        let Some(saver) = &self.saver else {
            return;
        };
        match self.to_prefs().to_text() {
            Ok(text) => saver.save(text),
            Err(error) => log::warn!("{error}"),
        }
    }

    /// Writes any pending change and stops the writer, waiting briefly for
    /// it. Called at shutdown.
    pub fn finish(mut self) {
        if self.debounce.pending() {
            self.save_now();
        }
        if let Some(saver) = self.saver.take() {
            saver.finish(Duration::from_secs(2));
        }
    }
}

/// Whether `STROBE_REDUCED_MOTION` asks for reduced motion.
fn env_wants_reduced_motion() -> bool {
    std::env::var("STROBE_REDUCED_MOTION").is_ok_and(|v| !v.is_empty() && v != "0")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Entry;
    use std::path::PathBuf;

    fn library(names: &[&str]) -> Library {
        Library {
            dir: PathBuf::from("scenes"),
            entries: names
                .iter()
                .map(|n| Entry::new((*n).to_string(), PathBuf::from(n)))
                .collect(),
        }
    }

    #[test]
    fn the_visible_list_follows_the_query_and_only_recomputes_on_change() {
        let lib = library(&["aurora", "plasma", "tunnel"]);
        let health = vec![Health::Unknown; 3];
        let mut browser = BrowserState::default();
        assert_eq!(browser.visible(&lib, &health, 1), &[0, 1, 2], "all");
        browser.query = "AUR".to_string();
        assert_eq!(browser.visible(&lib, &health, 1), &[0], "filtered");
        // Poison the cache's output: an unchanged key must not recompute.
        browser.visible.clear();
        assert!(
            browser.visible(&lib, &health, 1).is_empty(),
            "unchanged inputs were recomputed"
        );
        assert_eq!(
            browser.visible(&lib, &health, 2),
            &[0],
            "new revision recomputes"
        );
    }

    #[test]
    fn the_health_filter_narrows_the_list() {
        let lib = library(&["a", "b", "c"]);
        let health = vec![
            Health::Ready,
            Health::Failed("boom".to_string()),
            Health::Unknown,
        ];
        let mut browser = BrowserState {
            filter: HealthFilter::Failed,
            ..BrowserState::default()
        };
        assert_eq!(browser.visible(&lib, &health, 1), &[1], "needs work");
    }

    #[test]
    fn a_star_filters_saves_and_is_noticed() {
        let lib = library(&["a", "b", "c"]);
        let health = vec![Health::Unknown; 3];
        let mut state = UiState::ephemeral();
        let start = Instant::now();
        state.tick(start);
        state.browser.toggle_favourite("c");
        state.browser.toggle_favourite("a");
        state.browser.filter = HealthFilter::Favourites;
        assert_eq!(
            state.browser.visible(&lib, &health, 1),
            &[0, 2],
            "only the starred, in library order"
        );
        state.browser.toggle_favourite("a");
        assert_eq!(
            state.browser.visible(&lib, &health, 1),
            &[2],
            "unstarring refilters without a library change"
        );
        state.tick(start + Duration::from_millis(33));
        assert!(state.debounce.pending(), "a star is saved");
        assert_eq!(
            state.to_prefs().browser.favourites,
            ["c"],
            "written to the preferences"
        );
    }

    #[test]
    fn a_new_state_has_nothing_to_save() {
        let mut state = UiState::ephemeral();
        let start = Instant::now();
        for frame in 0..60_u32 {
            state.tick(start + Duration::from_millis(33) * frame);
        }
        assert!(!state.debounce.pending(), "an idle UI scheduled a write");
    }

    #[test]
    fn opening_a_pane_is_noticed_and_settles() {
        let mut state = UiState::ephemeral();
        let start = Instant::now();
        state.tick(start);
        state.panes.open(Pane::Browser);
        state.tick(start + Duration::from_millis(33));
        assert!(state.debounce.pending(), "the change was not noticed");
        state.tick(start + Duration::from_millis(33) + crate::prefs::QUIET);
        assert!(!state.debounce.pending(), "and it was not flushed");
    }

    #[test]
    fn the_query_and_filter_survive_a_round_trip_through_prefs() {
        let mut state = UiState::ephemeral();
        state.browser.query = "tunnel".to_string();
        state.browser.filter = HealthFilter::Failed;
        state.set_collapsed("Motion", true);
        state.set_theme("graphite");
        let back = UiState::from_prefs(&state.to_prefs());
        assert_eq!(back.browser.query, "tunnel", "query");
        assert_eq!(back.browser.filter, HealthFilter::Failed, "filter");
        assert!(back.is_collapsed("Motion"), "collapsed group");
        assert_eq!(back.theme_id(), "graphite", "theme");
    }

    #[test]
    fn collapsing_twice_does_not_duplicate() {
        let mut state = UiState::ephemeral();
        state.set_collapsed("A", true);
        state.set_collapsed("A", true);
        assert_eq!(state.collapsed_groups.len(), 1, "one entry");
        state.set_collapsed("A", false);
        assert!(state.collapsed_groups.is_empty(), "removed");
    }

    #[test]
    fn the_random_stream_stays_in_range_and_moves() {
        let mut state = UiState::ephemeral();
        let values: Vec<f32> = (0..200).map(|_| state.random_unit()).collect();
        assert!(
            values.iter().all(|v| (0.0..1.0).contains(v)),
            "out of range: {values:?}"
        );
        let first = values.first().copied().unwrap_or(0.0);
        assert!(values.iter().any(|v| (*v - first).abs() > 0.01), "stuck");
    }

    #[test]
    fn unknown_pane_names_are_reported_and_known_ones_open() {
        let mut state = UiState::ephemeral();
        let unknown = state.open_named(&["browser".to_string(), "nope".to_string()]);
        assert_eq!(unknown, vec!["nope".to_string()], "unknown reported");
        assert!(state.panes.is_open(Pane::Browser), "known opened");
    }
}
