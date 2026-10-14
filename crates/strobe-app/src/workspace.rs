// ============================================================================
// Module:       strobe_app::workspace
// Description:  The pane model of Contextual Perform: which panes exist, which
//               are open, how wide, and how they fit a window. Pure data and
//               arithmetic, no egui.
//
// Dependencies: std only
// ============================================================================

//! Panes as data (UI_IMPLEMENTATION_PLAN section 6, D281).
//!
//! The control window has one **main view** and one persistent **side
//! panel**, as Synesthesia's does (UI_DESIGN section 4). A pane has an
//! identity, a dock, a width range and a place in the open order; this module
//! owns all of that and knows nothing about egui, so the rules that matter at
//! 1280x720 (how wide the side panel may be, when it becomes a drawer) are
//! unit tests.
//!
//! ## Docks
//!
//! The *main* dock holds every pane the rail switches between: the scene
//! library ([`Pane::Browser`]), [`Pane::Import`], [`Pane::Playlist`],
//! [`Pane::Editor`], [`Pane::Map`], [`Pane::Diagnostics`] and
//! [`Pane::Settings`]. It shows one pane: the most recently opened of its open
//! panes. With none open the main view is the large live preview (PLAY).
//! Closing the shown pane (Esc closes the topmost one) reveals the one that
//! was under it, so opening Diagnostics over the Library and pressing Esc
//! puts the Library back, which is what an operator in the middle of a show
//! expects.
//!
//! The *side* dock holds the [`Pane::Inspector`] alone: the preview, the
//! status strip and the Playlist, Controls, Presets and Edit tabs.
//! Open, it is a column beside the main view; closed, it is a handle strip
//! that reopens it, so the preview and the controls are never more than one
//! click away.
//!
//! ## Fitting a window
//!
//! [`arrange`] decides what is drawn. The main view takes whatever the side
//! panel leaves, and the side panel gives way in this order:
//!
//! 1. it is drawn at its width (until the operator drags it, that is
//!    [`SIDE_SHARE`] of the window, see [`Panes::side_follows`]) while the
//!    main view keeps at least [`MAIN_MIN`] beside the rail;
//! 2. otherwise it shrinks towards its minimum;
//! 3. below that it becomes an overlay drawer over the main view.
//!
//! Below [`NARROW`] the rail drops its labels.

/// A contextual pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Pane {
    /// The scene browser: search, filter, preview, load.
    Browser,
    /// Shadertoy import.
    Import,
    /// The control inspector for the playing scene.
    Inspector,
    /// Health cards and the console.
    Diagnostics,
    /// Output, audio, performance and appearance settings.
    Settings,
    /// The playlist: load, timeline, generate, and the cue commands.
    Playlist,
    /// Mappings: MIDI and OSC, learn, macros and the activity monitor.
    Map,
    /// The scene's source files: edit, save, reload (U-02).
    Editor,
}

/// Which region of the work surface a pane docks to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dock {
    /// The main view, beside the rail: one pane at a time, switched by the
    /// rail.
    Main,
    /// The persistent side panel at the far edge.
    Side,
}

impl Pane {
    /// Every pane, in a fixed order that indexes the arrays below.
    pub const ALL: [Pane; 8] = [
        Pane::Browser,
        Pane::Import,
        Pane::Inspector,
        Pane::Diagnostics,
        Pane::Settings,
        Pane::Playlist,
        Pane::Map,
        Pane::Editor,
    ];

    fn index(self) -> usize {
        match self {
            Self::Browser => 0,
            Self::Import => 1,
            Self::Inspector => 2,
            Self::Diagnostics => 3,
            Self::Settings => 4,
            Self::Playlist => 5,
            Self::Map => 6,
            Self::Editor => 7,
        }
    }

    /// The dock this pane lives in.
    pub fn dock(self) -> Dock {
        match self {
            Self::Browser
            | Self::Import
            | Self::Editor
            | Self::Diagnostics
            | Self::Settings
            | Self::Playlist
            | Self::Map => Dock::Main,
            Self::Inspector => Dock::Side,
        }
    }

    /// The pane's title.
    pub fn title(self) -> &'static str {
        match self {
            Self::Browser => "Library",
            Self::Import => "Import",
            Self::Inspector => "Panel",
            Self::Diagnostics => "Diagnostics",
            Self::Settings => "Settings",
            Self::Playlist => "Playlist",
            Self::Map => "Mappings",
            Self::Editor => "Editor",
        }
    }

    /// The key the pane is stored under in the preferences file.
    pub fn key(self) -> &'static str {
        match self {
            Self::Browser => "browser",
            Self::Import => "import",
            Self::Inspector => "inspector",
            Self::Diagnostics => "diagnostics",
            Self::Settings => "settings",
            Self::Playlist => "playlist",
            Self::Map => "map",
            Self::Editor => "editor",
        }
    }

    /// The pane stored under `key`, if there is one.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|pane| pane.key() == key)
    }

    /// The narrowest the pane may be drawn, in points. A main pane takes
    /// whatever the side panel leaves (at least [`MAIN_MIN`]); its own range
    /// is kept for the preferences file, which stores a width for every
    /// pane.
    pub fn min_width(self) -> f32 {
        match self {
            Self::Browser | Self::Import => 220.0,
            // Wide enough for a 16:9 preview a person can judge and for a
            // slider row with its number and reset beside it.
            Self::Inspector => 280.0,
            Self::Diagnostics | Self::Settings => 300.0,
            Self::Playlist => 340.0,
            Self::Map => 380.0,
            Self::Editor => 420.0,
        }
    }

    /// The width a pane opens at, in points.
    pub fn default_width(self) -> f32 {
        match self {
            Self::Browser | Self::Import => 280.0,
            // Used only once the panel has been dragged and the stored
            // width is lost; until then it follows the window (D281).
            Self::Inspector => 440.0,
            Self::Diagnostics | Self::Settings => 380.0,
            Self::Playlist => 440.0,
            Self::Map => 460.0,
            Self::Editor => 580.0,
        }
    }

    /// The widest the pane may be dragged to, in points.
    pub fn max_width(self) -> f32 {
        match self {
            Self::Browser | Self::Import => 460.0,
            Self::Inspector => 720.0,
            Self::Diagnostics | Self::Settings => 640.0,
            Self::Playlist => 720.0,
            Self::Map => 760.0,
            Self::Editor => 1100.0,
        }
    }
}

/// The least width the main view keeps beside a docked side panel: two
/// library cards at the small size, or the Editor at its narrowest line
/// length worth having. Below it the side panel shrinks, then becomes a
/// drawer (D281).
pub const MAIN_MIN: f32 = 320.0;

/// Below this window width the rail drops its labels.
pub const NARROW: f32 = 1000.0;

/// The share of the window the side panel takes while its width is the
/// default one: Synesthesia's right panel is 31% of its window, measured on
/// 1.25.4 (D281). A width the operator has dragged to is kept as dragged.
pub const SIDE_SHARE: f32 = 0.31;

/// The side panel's width in a window `window_width` wide while nobody has
/// dragged it: [`SIDE_SHARE`] of the window, inside the panel's range.
pub fn auto_side_width(window_width: f32) -> f32 {
    clamp_width(Pane::Inspector, window_width * SIDE_SHARE)
}

/// Which panes are open, how wide, and in what order they were opened.
#[derive(Clone, Debug, PartialEq)]
pub struct Panes {
    open: [bool; 8],
    width: [f32; 8],
    /// When each pane was last opened, on a counter; larger is newer.
    stamp: [u64; 8],
    clock: u64,
    /// Whether the side panel's width follows the window ([`SIDE_SHARE`]
    /// of it) because nobody has dragged it. A flag, not a magic width, so
    /// a drag that lands on any width keeps it (D281).
    side_follows: bool,
}

impl Default for Panes {
    /// The default (D281): the side panel open, and the library as the main
    /// view. The library is opened last, so Esc closes it first and leaves
    /// the panel.
    fn default() -> Self {
        let mut panes = Self {
            open: [false; 8],
            width: Pane::ALL.map(Pane::default_width),
            stamp: [0; 8],
            clock: 0,
            side_follows: true,
        };
        panes.open(Pane::Inspector);
        panes.open(Pane::Browser);
        panes
    }
}

impl Panes {
    /// Whether `pane` is open (it may still be hidden by the window's width,
    /// see [`arrange`]).
    pub fn is_open(&self, pane: Pane) -> bool {
        self.open.get(pane.index()).copied().unwrap_or(false)
    }

    /// Opens `pane` and makes it the topmost of its dock.
    pub fn open(&mut self, pane: Pane) {
        self.clock += 1;
        if let Some(open) = self.open.get_mut(pane.index()) {
            *open = true;
        }
        if let Some(stamp) = self.stamp.get_mut(pane.index()) {
            *stamp = self.clock;
        }
    }

    /// Closes `pane`.
    pub fn close(&mut self, pane: Pane) {
        if let Some(open) = self.open.get_mut(pane.index()) {
            *open = false;
        }
    }

    /// What a button for `pane` does, given whether the pane is actually
    /// *drawn* right now (an open pane can be hidden because the window is
    /// too narrow for it beside the other dock): close it if it is drawn,
    /// otherwise open it, which raises it and lets it win the room.
    pub fn toggle_drawn(&mut self, pane: Pane, drawn: bool) {
        if drawn {
            self.close(pane);
        } else {
            self.open(pane);
        }
    }

    /// Closes every pane: the main view becomes the large preview and the
    /// side panel collapses to its handle (`--no-panes`).
    pub fn close_all(&mut self) {
        self.open = [false; 8];
    }

    /// Closes every main pane, so the main view is the large live preview
    /// (the rail's PLAY). The side panel is left as it is.
    pub fn close_main(&mut self) {
        for pane in Pane::ALL {
            if pane.dock() == Dock::Main {
                self.close(pane);
            }
        }
    }

    /// Whether the side panel is open (docked or as a drawer) rather than
    /// collapsed to its handle.
    pub fn side_open(&self) -> bool {
        self.is_open(Pane::Inspector)
    }

    /// The pane showing in `dock` when there is room: the most recently
    /// opened of its open panes.
    pub fn shown_in(&self, dock: Dock) -> Option<Pane> {
        Pane::ALL
            .into_iter()
            .filter(|pane| pane.dock() == dock && self.is_open(*pane))
            .max_by_key(|pane| self.stamp.get(pane.index()).copied().unwrap_or(0))
    }

    /// The most recently opened open pane in either dock.
    pub fn topmost(&self) -> Option<Pane> {
        Pane::ALL
            .into_iter()
            .filter(|pane| self.is_open(*pane))
            .max_by_key(|pane| self.stamp.get(pane.index()).copied().unwrap_or(0))
    }

    /// Feeds everything that distinguishes one layout from another into
    /// `state`, without allocating: the UI hashes its state once a frame to
    /// notice a change.
    pub fn hash_into<H: std::hash::Hasher>(&self, state: &mut H) {
        use std::hash::Hash;
        self.open.hash(state);
        for width in self.width {
            width.to_bits().hash(state);
        }
        self.stamp.hash(state);
        self.side_follows.hash(state);
    }

    /// The open panes, oldest first: the order they were opened in, which
    /// the preferences file keeps so a restart restores which is on top.
    pub fn open_order(&self) -> Vec<Pane> {
        let mut open: Vec<Pane> = Pane::ALL
            .into_iter()
            .filter(|pane| self.is_open(*pane))
            .collect();
        open.sort_by_key(|pane| self.stamp.get(pane.index()).copied().unwrap_or(0));
        open
    }

    /// Closes the topmost open pane and says which it was (Esc).
    pub fn close_topmost(&mut self) -> Option<Pane> {
        let top = self.topmost()?;
        self.close(top);
        Some(top)
    }

    /// The pane's width in points, inside its range.
    pub fn width(&self, pane: Pane) -> f32 {
        let stored = self
            .width
            .get(pane.index())
            .copied()
            .unwrap_or_else(|| pane.default_width());
        clamp_width(pane, stored)
    }

    /// Sets the pane's width, clamped to its range. Not-a-number is ignored.
    /// The side panel stops following the window and keeps this width.
    pub fn set_width(&mut self, pane: Pane, width: f32) {
        if !width.is_finite() {
            return;
        }
        if let Some(slot) = self.width.get_mut(pane.index()) {
            *slot = clamp_width(pane, width);
            if pane == Pane::Inspector {
                self.side_follows = false;
            }
        }
    }

    /// Whether the side panel's width follows the window, because it has
    /// not been dragged since it last did.
    pub fn side_follows(&self) -> bool {
        self.side_follows
    }

    /// Makes the side panel's width follow the window again (a double-click
    /// on its edge).
    pub fn follow_window(&mut self) {
        self.side_follows = true;
        if let Some(slot) = self.width.get_mut(Pane::Inspector.index()) {
            *slot = Pane::Inspector.default_width();
        }
    }
}

fn clamp_width(pane: Pane, width: f32) -> f32 {
    if width.is_nan() {
        return pane.default_width();
    }
    width.clamp(pane.min_width(), pane.max_width())
}

/// What is drawn this frame, for a window of a given width.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Arrangement {
    /// The pane in the main view; `None` is the large live preview (PLAY).
    pub main: Option<Pane>,
    /// The side panel's drawn width when it is open, docked or as a drawer.
    /// `None` when it is collapsed to its handle.
    pub side: Option<f32>,
    /// Whether the open side panel is drawn as an overlay drawer over the
    /// main view instead of beside it (a narrow window).
    pub overlay: bool,
    /// Whether the rail shows its labels.
    pub rail_labels: bool,
}

impl Arrangement {
    /// Whether `pane` is the one drawn in its dock right now.
    pub fn shows(&self, pane: Pane) -> bool {
        match pane.dock() {
            Dock::Main => self.main == Some(pane),
            Dock::Side => self.side.is_some(),
        }
    }

    /// The width the docked side panel takes from the main view.
    pub fn docked_width(&self) -> f32 {
        if self.overlay {
            return 0.0;
        }
        self.side.unwrap_or(0.0)
    }
}

/// The widest the side panel may be dragged without squeezing the main view
/// below [`MAIN_MIN`]: never less than the panel's own minimum, so a drag
/// always has somewhere to go.
pub fn drag_limit(window_width: f32, rail: f32) -> f32 {
    let avail = (window_width - rail).max(0.0);
    let room = avail - MAIN_MIN;
    room.max(Pane::Inspector.min_width())
        .min(Pane::Inspector.max_width())
}

/// Decides what is drawn, and how wide the side panel is, for a window
/// `window_width` points wide whose rail is `rail` points wide. See the
/// module docs for the rules.
pub fn arrange(panes: &Panes, window_width: f32, rail: f32) -> Arrangement {
    let rail_labels = window_width >= NARROW;
    let avail = (window_width - rail).max(0.0);
    let main = panes.shown_in(Dock::Main);
    let mut overlay = false;
    let side = panes.side_open().then(|| {
        let wanted = if panes.side_follows() {
            auto_side_width(window_width)
        } else {
            panes.width(Pane::Inspector)
        };
        let room = avail - MAIN_MIN;
        if wanted <= room {
            wanted
        } else if room >= Pane::Inspector.min_width() {
            room
        } else {
            overlay = true;
            Pane::Inspector.min_width().min(avail * 0.9)
        }
    });
    Arrangement {
        main,
        side,
        overlay,
        rail_labels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAIL: f32 = 60.0;

    fn with(open: &[Pane]) -> Panes {
        let mut panes = Panes::default();
        panes.close_all();
        for pane in open {
            panes.open(*pane);
        }
        panes
    }

    #[test]
    fn the_default_opens_the_library_and_the_side_panel() {
        let panes = Panes::default();
        for pane in Pane::ALL {
            assert_eq!(
                panes.is_open(pane),
                matches!(pane, Pane::Inspector | Pane::Browser),
                "{pane:?} in the default layout"
            );
        }
        assert_eq!(
            panes.topmost(),
            Some(Pane::Browser),
            "Esc closes the library first and keeps the panel"
        );
    }

    #[test]
    fn every_pane_but_the_inspector_is_a_main_view() {
        for pane in Pane::ALL {
            let expected = if pane == Pane::Inspector {
                Dock::Side
            } else {
                Dock::Main
            };
            assert_eq!(pane.dock(), expected, "{pane:?}");
        }
    }

    #[test]
    fn the_main_view_shows_the_newest_open_pane_and_play_when_none() {
        let layout = arrange(&with(&[Pane::Browser, Pane::Settings]), 1440.0, RAIL);
        assert_eq!(layout.main, Some(Pane::Settings), "newest wins");
        assert!(layout.shows(Pane::Settings) && !layout.shows(Pane::Browser));
        let mut panes = with(&[Pane::Browser, Pane::Inspector]);
        panes.close_main();
        let layout = arrange(&panes, 1440.0, RAIL);
        assert_eq!(layout.main, None, "PLAY: the large preview");
        assert!(layout.side.is_some(), "and the panel stays");
    }

    #[test]
    fn an_undragged_panel_docks_at_a_third_of_the_window() {
        for width in [1280.0_f32, 1366.0, 1440.0, 1920.0, 2560.0] {
            let layout = arrange(&Panes::default(), width, RAIL);
            assert_eq!(
                layout.side,
                Some(auto_side_width(width)),
                "at {width}: {layout:?}"
            );
            assert!(!layout.overlay, "docked at {width}");
            let share = layout.docked_width() / width;
            assert!(
                width >= 2000.0 || (0.28..=0.33).contains(&share),
                "the panel is about 31% of the window at {width}, not {share}"
            );
            let main = width - RAIL - layout.docked_width();
            assert!(main >= MAIN_MIN, "the main view keeps {main} at {width}");
        }
    }

    #[test]
    fn a_dragged_panel_keeps_the_width_it_was_dragged_to() {
        // 440 is the panel's default width: a drag that lands on it is a
        // drag like any other, not a return to following the window.
        for dragged in [360.0, Pane::Inspector.default_width()] {
            let mut panes = Panes::default();
            assert!(panes.side_follows(), "undragged, it follows the window");
            panes.set_width(Pane::Inspector, dragged);
            assert!(!panes.side_follows(), "dragged to {dragged}");
            let layout = arrange(&panes, 1920.0, RAIL);
            assert_eq!(
                layout.side,
                Some(dragged),
                "dragged to {dragged}, so not following the window"
            );
        }
        let mut panes = Panes::default();
        panes.set_width(Pane::Inspector, 360.0);
        panes.follow_window();
        assert_eq!(
            arrange(&panes, 1920.0, RAIL).side,
            Some(auto_side_width(1920.0)),
            "and a double-click on its edge makes it follow again"
        );
    }

    #[test]
    fn the_main_view_always_keeps_its_minimum_when_docked() {
        for width in [700.0_f32, 800.0, 900.0, 1000.0, 1280.0, 1920.0] {
            let mut panes = Panes::default();
            panes.set_width(Pane::Inspector, Pane::Inspector.max_width());
            let layout = arrange(&panes, width, RAIL);
            if layout.overlay {
                continue;
            }
            let main = width - RAIL - layout.docked_width();
            assert!(
                main >= MAIN_MIN - 0.5,
                "at {width}: the main view keeps only {main}"
            );
        }
    }

    #[test]
    fn a_very_narrow_window_turns_the_panel_into_a_drawer() {
        let layout = arrange(&Panes::default(), 600.0, RAIL);
        assert!(layout.overlay, "{layout:?}");
        assert!(!layout.rail_labels, "labels go first");
        assert_eq!(layout.docked_width(), 0.0, "a drawer takes no layout width");
        assert!(
            layout.side.is_some_and(|w| w <= (600.0 - RAIL) * 0.9),
            "the drawer leaves a strip of the main view: {layout:?}"
        );
    }

    #[test]
    fn the_panel_shrinks_before_it_becomes_a_drawer() {
        // 683 wide (1024 at 150%): 623 beside the rail, 303 left for the
        // panel once the main view has its 320, above the 280 minimum.
        let layout = arrange(&Panes::default(), 683.0, RAIL);
        let drawn = layout.side.unwrap_or(0.0);
        assert!(!layout.overlay, "{layout:?}");
        assert!(
            drawn < Pane::Inspector.default_width() && drawn >= Pane::Inspector.min_width(),
            "{drawn}"
        );
    }

    #[test]
    fn a_collapsed_panel_takes_no_width() {
        let layout = arrange(&with(&[Pane::Browser]), 1440.0, RAIL);
        assert_eq!(layout.side, None, "collapsed to its handle");
        assert!(!layout.shows(Pane::Inspector));
        assert_eq!(layout.docked_width(), 0.0);
    }

    #[test]
    fn dragging_the_panel_wider_never_squeezes_the_main_view() {
        let limit = drag_limit(1440.0, RAIL);
        let mut panes = Panes::default();
        panes.set_width(Pane::Inspector, limit);
        let layout = arrange(&panes, 1440.0, RAIL);
        assert!(!layout.overlay, "{layout:?}");
        assert!(
            1440.0 - RAIL - layout.docked_width() >= MAIN_MIN - 0.5,
            "dragging to the limit ({limit}) squeezed the main view: {layout:?}"
        );
        // And the limit is never below the panel's own minimum.
        assert!(drag_limit(300.0, RAIL) >= Pane::Inspector.min_width());
    }

    #[test]
    fn opening_over_a_pane_and_closing_reveals_the_one_beneath() {
        let mut panes = with(&[Pane::Browser]);
        panes.open(Pane::Diagnostics);
        assert_eq!(panes.shown_in(Dock::Main), Some(Pane::Diagnostics), "over");
        assert_eq!(panes.close_topmost(), Some(Pane::Diagnostics), "esc");
        assert_eq!(panes.shown_in(Dock::Main), Some(Pane::Browser), "under");
        assert_eq!(panes.close_topmost(), Some(Pane::Browser), "esc again");
        assert_eq!(panes.close_topmost(), None, "nothing left to close");
    }

    #[test]
    fn a_button_for_a_pane_under_another_raises_it() {
        let mut panes = with(&[Pane::Inspector, Pane::Browser, Pane::Playlist]);
        let layout = arrange(&panes, 1280.0, RAIL);
        let drawn = layout.shows(Pane::Browser);
        assert!(!drawn, "precondition: the playlist is over the library");
        panes.toggle_drawn(Pane::Browser, drawn);
        let after = arrange(&panes, 1280.0, RAIL);
        assert_eq!(
            after.main,
            Some(Pane::Browser),
            "the library is the main view after its button was pressed: {after:?}"
        );
        assert!(
            panes.is_open(Pane::Playlist),
            "the playlist stays open beneath"
        );
    }

    #[test]
    fn widths_are_clamped_and_nonsense_is_ignored() {
        let mut panes = Panes::default();
        panes.set_width(Pane::Inspector, 10.0);
        assert_eq!(
            panes.width(Pane::Inspector),
            Pane::Inspector.min_width(),
            "min"
        );
        panes.set_width(Pane::Inspector, 9999.0);
        assert_eq!(
            panes.width(Pane::Inspector),
            Pane::Inspector.max_width(),
            "max"
        );
        panes.set_width(Pane::Inspector, 300.0);
        panes.set_width(Pane::Inspector, f32::NAN);
        assert_eq!(panes.width(Pane::Inspector), 300.0, "NaN leaves it alone");
    }

    #[test]
    fn pane_keys_round_trip() {
        for pane in Pane::ALL {
            assert_eq!(Pane::from_key(pane.key()), Some(pane), "{pane:?}");
        }
        assert_eq!(Pane::from_key("nope"), None, "unknown key");
    }

    #[test]
    fn every_width_range_is_ordered_and_holds_its_default() {
        for pane in Pane::ALL {
            assert!(pane.min_width() < pane.max_width(), "{pane:?} range");
            assert!(
                (pane.min_width()..=pane.max_width()).contains(&pane.default_width()),
                "{pane:?} default outside its range"
            );
        }
    }
}
