// ============================================================================
// Module:       strobe_app::prefs
// Description:  The operator's UI preferences (which panes are open and how
//               wide, the browser's query, the theme) as a small TOML file in
//               the data directory, saved off the UI thread and debounced.
//
// Dependencies: serde + toml (the file), strobe-diag (the data directory),
//               crate::workspace (the pane model)
// ============================================================================

//! UI preferences (UI_IMPLEMENTATION_PLAN section 17).
//!
//! Preferences are *workflow state*, kept apart from runtime state and from
//! egui's widget memory: pane visibility and widths, the browser's query and
//! filter, the theme and a couple of window behaviours. They live in
//! `ui.toml` under the data directory (beside the logs), never in a
//! Synesthesia-compatible scene file (UI_DESIGN section 29).
//!
//! ## When it is written
//!
//! Never from the paint path. The UI compares its state with what was last
//! saved once per frame (a few comparisons) and tells a [`Debounce`] whether
//! it changed; when the changes have been quiet for [`QUIET`] (or have gone
//! on for [`LONGEST`], as during a pane drag) the serialised text goes over
//! a channel to a [`Saver`] thread, which writes it to a temporary file and
//! renames it over the real one. A crash cannot leave half a file, and a
//! slow disk cannot stall a frame.
//!
//! ## What a bad file does
//!
//! Nothing worse than costing the preferences: an unreadable or malformed
//! file loads as the defaults and the reason goes to the log, every field
//! has a default so an old file keeps working, and unknown keys and panes
//! are ignored so an old build can read a newer file.

use crate::workspace::{Pane, Panes};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Sender, channel};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

/// The file's name in the data directory.
pub const FILE: &str = "ui.toml";

/// How long changes must be quiet before they are written.
pub const QUIET: Duration = Duration::from_millis(500);

/// The longest a stream of changes (a pane being dragged) may go unwritten.
pub const LONGEST: Duration = Duration::from_secs(5);

/// The file's format version. 2 (D281): the inspector's width is the side
/// panel's, and 0 means it follows the window; a version-1 width was the
/// old inspector's and is dropped on reading ([`Prefs::from_text`]).
const VERSION: u32 = 2;

/// One pane's saved state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PanePref {
    /// Whether the pane is open.
    pub open: bool,
    /// Its width in points; 0 for none stored (the side panel then follows
    /// the window, D281).
    pub width: f32,
}

impl Default for PanePref {
    fn default() -> Self {
        Self {
            open: false,
            width: 0.0,
        }
    }
}

/// The scene browser's saved workflow.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct BrowserPref {
    /// The search text.
    pub query: String,
    /// The health filter, by [`crate::library::HealthFilter::key`].
    pub filter: String,
    /// Favourite scenes, by library name (D211).
    pub favourites: Vec<String>,
    /// The library grid's card size (D282): `small`, `medium` or `large`.
    /// Empty means the default.
    pub cards: String,
    /// What the grid is ordered by (D282), by
    /// [`crate::library::SortKey::key`]. Empty means the scan's order.
    pub sort: String,
    /// Whether the order is reversed (D282).
    pub descending: bool,
}

impl Default for BrowserPref {
    fn default() -> Self {
        Self {
            query: String::new(),
            filter: "all".to_string(),
            favourites: Vec::new(),
            cards: String::new(),
            sort: String::new(),
            descending: false,
        }
    }
}

/// Everything saved.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// The file format version.
    pub version: u32,
    /// The theme's `id`.
    pub theme: String,
    /// Draw Strobe's own title bar instead of the system's (D132).
    pub custom_title_bar: bool,
    /// Reduce motion: no transitions, no overshoot.
    pub reduced_motion: bool,
    /// How tightly the window is packed (D220): `comfortable` or
    /// `compact`.
    pub density: String,
    /// Panes by [`Pane::key`].
    pub panes: BTreeMap<String, PanePref>,
    /// Open panes, oldest first, by [`Pane::key`].
    pub order: Vec<String>,
    /// The scene browser.
    pub browser: BrowserPref,
    /// Inspector groups the operator collapsed, by group name.
    pub collapsed_groups: Vec<String>,
    /// The library folder name of the scene last loaded, reopened at the
    /// next launch; empty for none.
    pub last_scene: String,
    /// Named pane layouts the operator saved (D215).
    pub layouts: BTreeMap<String, Layout>,
    /// The watched drop folder (D240, D265).
    pub watch: WatchPref,
    /// The side panel's tab (D283): `controls`, `playlist` or `presets`.
    /// Empty means the default.
    pub side_tab: String,
    /// The library view's page (D291): `scenes` or `controls`. Empty means
    /// the default, the scenes.
    pub library_page: String,
    /// Synesthesia's own scenes in the library (D271).
    pub synesthesia: SynesthesiaPref,
    /// Controlling another Strobe (D294): the trust file's path and this
    /// laptop's controller id.
    pub remote: RemotePref,
}

/// What the "Control another Strobe" form remembers (D294). The trust
/// file's **path** is saved, never its contents: the secret stays in the
/// file, which is the only place it should be.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct RemotePref {
    /// The trust file, as typed; empty for none.
    pub trust_file: String,
    /// Whether the form connects with the trust file rather than a code.
    pub use_trust: bool,
    /// This computer's id on a host's list: `name-1a2b3c4d`, made once and
    /// kept, so the host can tell this laptop from another and revoke it by
    /// name. Empty until the first launch makes one.
    pub controller_id: String,
}

/// Whether the library lists Synesthesia's scenes where it is installed
/// (D271).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SynesthesiaPref {
    /// List them, read-only, after the library folder's own.
    pub show: bool,
}

impl Default for SynesthesiaPref {
    /// On: a person with Synesthesia installed sees its scenes without
    /// looking for a setting, and a file from before has none.
    fn default() -> Self {
        Self { show: true }
    }
}

/// The watched drop folder, as Settings last applied it (D265).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct WatchPref {
    /// The folder, as typed; empty for none.
    pub folder: String,
    /// Whether it is watched at launch.
    pub enabled: bool,
}

/// A named arrangement of panes (D215): which are open, in which order,
/// and every pane's width.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    /// Panes by [`Pane::key`].
    pub panes: BTreeMap<String, PanePref>,
    /// Open panes, oldest first, by [`Pane::key`].
    pub order: Vec<String>,
}

impl Layout {
    /// The layout `panes` has now.
    pub fn capture(panes: &Panes) -> Self {
        Self {
            panes: Pane::ALL
                .into_iter()
                .map(|pane| {
                    // A side panel that follows the window stores no width,
                    // so it still follows after a restart (D281).
                    let follows = pane == Pane::Inspector && panes.side_follows();
                    (
                        pane.key().to_string(),
                        PanePref {
                            open: panes.is_open(pane),
                            width: if follows { 0.0 } else { panes.width(pane) },
                        },
                    )
                })
                .collect(),
            order: panes
                .open_order()
                .into_iter()
                .map(|pane| pane.key().to_string())
                .collect(),
        }
    }

    /// The live pane state this layout describes. Unknown panes are ignored
    /// and widths are clamped to the pane's range.
    pub fn to_panes(&self) -> Panes {
        let mut panes = Panes::default();
        panes.close_all();
        for (key, pref) in &self.panes {
            if let Some(pane) = Pane::from_key(key)
                && pref.width > 0.0
            {
                panes.set_width(pane, pref.width);
            }
        }
        // Open oldest first so the newest ends up on top. A pane marked
        // open but missing from `order` (a hand-edited file) opens first.
        let mut to_open: Vec<Pane> = self
            .panes
            .iter()
            .filter(|(_, pref)| pref.open)
            .filter_map(|(key, _)| Pane::from_key(key))
            .collect();
        to_open.sort_by_key(|pane| {
            self.order
                .iter()
                .position(|key| key == pane.key())
                .map_or(0, |at| at + 1)
        });
        for pane in to_open {
            panes.open(pane);
        }
        panes
    }
}

/// Whether the custom title bar is on by default here (D132): on Windows
/// and Linux, where the system bar is either unthemeable (Windows) or
/// missing altogether (GNOME's Wayland, D122); off on macOS, whose traffic
/// lights the app does not imitate.
pub const fn default_custom_title_bar() -> bool {
    cfg!(any(windows, target_os = "linux"))
}

impl Default for Prefs {
    fn default() -> Self {
        Self::from_parts(
            &Panes::default(),
            "strobe",
            default_custom_title_bar(),
            false,
            BrowserPref::default(),
            Vec::new(),
        )
    }
}

impl Prefs {
    /// Builds the saved form of the live state.
    pub fn from_parts(
        panes: &Panes,
        theme: &str,
        custom_title_bar: bool,
        reduced_motion: bool,
        browser: BrowserPref,
        collapsed_groups: Vec<String>,
    ) -> Self {
        let layout = Layout::capture(panes);
        Self {
            version: VERSION,
            theme: theme.to_string(),
            custom_title_bar,
            reduced_motion,
            density: crate::ui::theme::Density::default().key().to_string(),
            panes: layout.panes,
            order: layout.order,
            browser,
            collapsed_groups,
            last_scene: String::new(),
            layouts: BTreeMap::new(),
            watch: WatchPref::default(),
            side_tab: String::new(),
            library_page: String::new(),
            synesthesia: SynesthesiaPref::default(),
            remote: RemotePref::default(),
        }
    }

    /// The live pane state this file describes. Unknown panes are ignored
    /// and widths are clamped to the pane's range.
    pub fn to_panes(&self) -> Panes {
        Layout {
            panes: self.panes.clone(),
            order: self.order.clone(),
        }
        .to_panes()
    }

    /// Serialises to TOML text.
    pub fn to_text(&self) -> Result<String, String> {
        toml::to_string_pretty(self).map_err(|e| format!("encoding preferences: {e}"))
    }

    /// Parses TOML text, bringing an older version's up to date.
    pub fn from_text(text: &str) -> Result<Self, String> {
        let mut prefs: Self =
            toml::from_str(text).map_err(|e| format!("parsing preferences: {e}"))?;
        prefs.migrate();
        Ok(prefs)
    }

    /// Brings a file from an older version up to this one. Version 1's
    /// inspector width belonged to the old inspector pane (300 by default),
    /// so the side panel that took its place follows the window instead of
    /// keeping it (D281).
    fn migrate(&mut self) {
        if self.version < 2 {
            let layouts = self.layouts.values_mut().map(|layout| &mut layout.panes);
            for panes in std::iter::once(&mut self.panes).chain(layouts) {
                if let Some(side) = panes.get_mut(Pane::Inspector.key()) {
                    side.width = 0.0;
                }
            }
        }
        self.version = VERSION;
    }

    /// Loads the file, or the defaults. The second value says why the
    /// defaults were used when a file existed but could not be read.
    pub fn load(path: &Path) -> (Self, Option<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => match Self::from_text(&text) {
                Ok(prefs) => (prefs, None),
                Err(error) => (
                    Self::default(),
                    Some(format!("{}: {error}", path.display())),
                ),
            },
            // No file is the normal first run.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => (Self::default(), None),
            Err(error) => (
                Self::default(),
                Some(format!("reading {}: {error}", path.display())),
            ),
        }
    }
}

/// Where the preferences file lives.
pub fn path() -> PathBuf {
    strobe_diag::paths::base_dir().join(FILE)
}

/// Decides when a stream of "something changed" observations should become
/// a write. Pure: the caller supplies the clock.
#[derive(Debug, Default)]
pub struct Debounce {
    /// When the oldest unwritten change happened.
    first: Option<Instant>,
    /// When the newest one did.
    last: Option<Instant>,
}

impl Debounce {
    /// Notes one frame: `changed` is whether the state changed *this
    /// frame* (a fingerprint differs from last frame's). Returns true when
    /// the pending changes should be written now, and forgets them.
    pub fn note(&mut self, changed: bool, now: Instant) -> bool {
        if changed {
            self.first.get_or_insert(now);
            self.last = Some(now);
        }
        match (self.first, self.last) {
            (Some(first), Some(last))
                if now.duration_since(last) >= QUIET || now.duration_since(first) >= LONGEST =>
            {
                self.first = None;
                self.last = None;
                true
            }
            (Some(_), Some(_)) | (Some(_), None) | (None, Some(_)) | (None, None) => false,
        }
    }

    /// Whether a change is waiting to be written.
    pub fn pending(&self) -> bool {
        self.first.is_some()
    }
}

/// Writes preferences on a thread of its own.
pub struct Saver {
    tx: Option<Sender<String>>,
    worker: Option<JoinHandle<()>>,
}

impl Saver {
    /// Starts the writer for `path`. If the thread cannot start, saving is
    /// off for the run (and said so in the log).
    pub fn start(path: PathBuf) -> Self {
        let (tx, rx) = channel::<String>();
        let spawned = std::thread::Builder::new()
            .name("strobe-prefs".to_string())
            .spawn(move || {
                // Several queued saves collapse into the newest.
                while let Ok(mut text) = rx.recv() {
                    while let Ok(newer) = rx.try_recv() {
                        text = newer;
                    }
                    if let Err(error) = write_atomically(&path, &text) {
                        log::warn!("could not save {}: {error}", path.display());
                    }
                }
            });
        match spawned {
            Ok(worker) => Self {
                tx: Some(tx),
                worker: Some(worker),
            },
            Err(error) => {
                log::warn!("could not start the preferences writer: {error}");
                Self {
                    tx: None,
                    worker: None,
                }
            }
        }
    }

    /// Queues `text` to be written. Never blocks.
    pub fn save(&self, text: String) {
        if let Some(tx) = &self.tx {
            // A closed channel means the writer is gone; nothing to do.
            let _ = tx.send(text);
        }
    }

    /// Stops accepting saves, lets the writer finish what it has, and waits
    /// up to `limit` for it.
    pub fn finish(mut self, limit: Duration) {
        self.tx = None;
        let Some(worker) = self.worker.take() else {
            return;
        };
        let deadline = Instant::now() + limit;
        while !worker.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        if worker.is_finished() {
            // The writer has finished (checked above); a panic in it already
            // aborted the process in release builds.
            let _ = worker.join();
        } else {
            log::warn!("the preferences writer did not finish in time");
        }
    }
}

/// Writes `text` to `path` through a temporary file and a rename.
fn write_atomically(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)
            .map_err(|e| format!("creating {}: {e}", parent.display()))?;
    }
    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, text)
        .map_err(|e| format!("writing {}: {e}", temporary.display()))?;
    std::fs::rename(&temporary, path).map_err(|e| format!("replacing {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    fn sample() -> Prefs {
        let mut panes = Panes::default();
        panes.open(Pane::Browser);
        panes.set_width(Pane::Browser, 320.0);
        panes.set_width(Pane::Inspector, 340.0);
        Prefs::from_parts(
            &panes,
            "graphite",
            false,
            true,
            BrowserPref {
                query: "tunnel".to_string(),
                filter: "failed".to_string(),
                favourites: vec!["05-domain-warp".to_string()],
                cards: "large".to_string(),
                sort: "author".to_string(),
                descending: true,
            },
            vec!["Motion".to_string()],
        )
    }

    #[test]
    fn preferences_round_trip_through_toml() -> Result<()> {
        let mut prefs = sample();
        prefs.last_scene = "05-domain-warp".to_string();
        prefs.watch = WatchPref {
            folder: "D:\\VJ\\inbox".to_string(),
            enabled: true,
        };
        prefs.synesthesia = SynesthesiaPref { show: false };
        prefs.remote = RemotePref {
            trust_file: "C:\\Users\\me\\venue.trust".to_string(),
            use_trust: true,
            controller_id: "t480-1a2b3c4d".to_string(),
        };
        let text = prefs.to_text().map_err(anyhow::Error::msg)?;
        let back = Prefs::from_text(&text).map_err(anyhow::Error::msg)?;
        assert_eq!(back, prefs, "what was written is what is read");
        assert!(
            text.contains("venue.trust") && !text.to_lowercase().contains("secret"),
            "the file's path is saved and no word of its contents: {text}"
        );
        Ok(())
    }

    #[test]
    fn a_file_from_before_venue_mode_has_no_remote_section_and_loads() -> Result<()> {
        let back = Prefs::from_text("theme = \"graphite\"\n").map_err(anyhow::Error::msg)?;
        assert_eq!(back.remote, RemotePref::default(), "nothing remembered yet");
        assert!(!back.remote.use_trust, "and the code is the default");
        Ok(())
    }

    #[test]
    fn an_undragged_side_panel_still_follows_the_window_after_a_restart() -> Result<()> {
        let prefs = Prefs::default();
        let back = Prefs::from_text(&prefs.to_text().map_err(anyhow::Error::msg)?)
            .map_err(anyhow::Error::msg)?;
        assert!(back.to_panes().side_follows(), "undragged stays undragged");
        let mut dragged = Panes::default();
        dragged.set_width(Pane::Inspector, Pane::Inspector.default_width());
        let prefs = Prefs::from_parts(
            &dragged,
            "strobe",
            false,
            false,
            BrowserPref::default(),
            Vec::new(),
        );
        let back = Prefs::from_text(&prefs.to_text().map_err(anyhow::Error::msg)?)
            .map_err(anyhow::Error::msg)?;
        let panes = back.to_panes();
        assert!(
            !panes.side_follows() && panes.width(Pane::Inspector) == 440.0,
            "a drag to the default width is kept as a drag"
        );
        Ok(())
    }

    #[test]
    fn a_version_one_inspector_width_is_dropped_so_the_panel_follows_the_window() -> Result<()> {
        let text = "version = 1\n\
                    [panes.inspector]\nopen = true\nwidth = 300.0\n\
                    [panes.browser]\nopen = false\nwidth = 320.0\n\
                    [layouts.show.panes.inspector]\nopen = true\nwidth = 300.0\n";
        let prefs = Prefs::from_text(text).map_err(anyhow::Error::msg)?;
        assert_eq!(prefs.version, VERSION, "brought up to date");
        let panes = prefs.to_panes();
        assert!(panes.side_follows(), "the old inspector's 300 is dropped");
        assert_eq!(panes.width(Pane::Browser), 320.0, "other widths are kept");
        let layout = prefs.layouts.get("show").map(Layout::to_panes);
        assert!(
            layout.is_some_and(|panes| panes.side_follows()),
            "and in a saved layout"
        );
        Ok(())
    }

    #[test]
    fn the_saved_panes_restore_their_state_and_their_order() -> Result<()> {
        let prefs = sample();
        let panes = prefs.to_panes();
        assert!(panes.is_open(Pane::Browser) && panes.is_open(Pane::Inspector));
        assert_eq!(panes.width(Pane::Browser), 320.0, "browser width");
        assert_eq!(panes.width(Pane::Inspector), 340.0, "inspector width");
        assert_eq!(
            panes.topmost(),
            Some(Pane::Browser),
            "the browser was opened last, so it is on top again"
        );
        assert!(!panes.is_open(Pane::Settings), "closed panes stay closed");
        Ok(())
    }

    #[test]
    fn a_file_with_missing_fields_takes_defaults_for_them() -> Result<()> {
        let prefs = Prefs::from_text("theme = \"void\"\n").map_err(anyhow::Error::msg)?;
        assert_eq!(prefs.theme, "void", "the field that was there");
        assert_eq!(prefs.browser, BrowserPref::default(), "default browser");
        assert!(
            prefs.last_scene.is_empty(),
            "a file from before the field reopens nothing"
        );
        assert_eq!(
            prefs.watch,
            WatchPref::default(),
            "a file from before the watched folder watches nothing"
        );
        assert!(
            prefs.synesthesia.show,
            "a file from before the switch lists Synesthesia's scenes where installed"
        );
        assert_eq!(
            prefs.custom_title_bar,
            default_custom_title_bar(),
            "the platform default"
        );
        Ok(())
    }

    #[test]
    fn unknown_keys_and_panes_are_ignored() -> Result<()> {
        let text = "future = 3\n[panes.hologram]\nopen = true\nwidth = 99.0\n";
        let prefs = Prefs::from_text(text).map_err(anyhow::Error::msg)?;
        let panes = prefs.to_panes();
        for pane in Pane::ALL {
            assert!(!panes.is_open(pane), "{pane:?} opened by an unknown key");
        }
        Ok(())
    }

    #[test]
    fn nonsense_widths_are_clamped_on_the_way_in() -> Result<()> {
        let text = "[panes.browser]\nopen = true\nwidth = 99999.0\n";
        let prefs = Prefs::from_text(text).map_err(anyhow::Error::msg)?;
        let panes = prefs.to_panes();
        assert_eq!(panes.width(Pane::Browser), Pane::Browser.max_width());
        Ok(())
    }

    #[test]
    fn a_malformed_file_loads_the_defaults_and_says_why() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("strobe-prefs-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let file = dir.join(FILE);
        std::fs::write(&file, "this is [ not toml")?;
        let (prefs, problem) = Prefs::load(&file);
        std::fs::remove_dir_all(&dir)?;
        assert_eq!(prefs, Prefs::default(), "defaults");
        assert!(problem.is_some(), "the reason is reported");
        Ok(())
    }

    #[test]
    fn a_missing_file_is_a_normal_first_run() {
        let (prefs, problem) = Prefs::load(Path::new("/definitely/not/here/ui.toml"));
        assert_eq!(prefs, Prefs::default(), "defaults");
        assert!(problem.is_none(), "no complaint on a first run");
    }

    #[test]
    fn the_saver_writes_the_newest_text_and_leaves_no_temporary_file() -> Result<()> {
        let dir = std::env::temp_dir().join(format!("strobe-prefs-save-{}", std::process::id()));
        let file = dir.join(FILE);
        let saver = Saver::start(file.clone());
        saver.save("first = 1\n".to_string());
        saver.save("second = 2\n".to_string());
        saver.finish(Duration::from_secs(5));
        let text = std::fs::read_to_string(&file)?;
        let leftovers = std::fs::read_dir(&dir)?.count();
        std::fs::remove_dir_all(&dir)?;
        assert!(text.contains("second"), "the newest save wins: {text}");
        assert_eq!(leftovers, 1, "only the real file remains");
        Ok(())
    }

    #[test]
    fn an_unchanged_state_never_writes() {
        let mut debounce = Debounce::default();
        let start = Instant::now();
        for frame in 0..100_u32 {
            assert!(
                !debounce.note(false, start + Duration::from_millis(33) * frame),
                "frame {frame} wrote an unchanged state"
            );
        }
    }

    #[test]
    fn a_change_is_written_once_it_has_been_quiet() {
        let mut debounce = Debounce::default();
        let start = Instant::now();
        assert!(!debounce.note(true, start), "the change frame itself");
        assert!(debounce.pending(), "now waiting");
        assert!(
            !debounce.note(false, start + Duration::from_millis(100)),
            "too soon"
        );
        assert!(debounce.note(false, start + QUIET), "quiet long enough");
        assert!(!debounce.pending(), "and it is no longer pending");
    }

    #[test]
    fn a_stream_of_changes_is_written_every_so_often_not_every_frame() {
        let mut debounce = Debounce::default();
        let start = Instant::now();
        let mut writes = 0;
        // A pane drag: the state changes on every one of 600 frames (20 s).
        for frame in 0..600_u32 {
            let now = start + Duration::from_millis(33) * frame;
            if debounce.note(true, now) {
                writes += 1;
            }
        }
        assert!(writes >= 3, "a long drag must still be saved: {writes}");
        assert!(writes <= 5, "but not on every frame: {writes}");
    }
}
