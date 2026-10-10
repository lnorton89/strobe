// ============================================================================
// Module:       strobe_app::workspace
// Description:  The pane model of Contextual Perform: which panes exist, which
//               are open, how wide, and how they fit a window. Pure data and
//               arithmetic, no egui.
//
// Dependencies: std only
// ============================================================================

//! Panes as data (UI_IMPLEMENTATION_PLAN section 6).
//!
//! Perform is one stable work surface (the preview, the transport, the
//! macros) with *contextual* panes around it. A pane has an identity, a
//! dock, a width range and a place in the open order; this module owns all
//! of that and knows nothing about egui, so the rules that matter at 1280x720
//! (which panes may coexist, which one gives way) are unit tests.
//!
//! ## Docks
//!
//! There are two docks. The *left* dock holds the scene [`Pane::Browser`]
//! and [`Pane::Import`]; the *right* dock holds the [`Pane::Inspector`],
//! [`Pane::Diagnostics`] and [`Pane::Settings`]. A dock shows one pane: the
//! most recently opened of its open panes. Closing that pane (Esc closes the
//! topmost one) reveals the one that was under it, so opening Diagnostics
//! over the Inspector and pressing Esc puts the Inspector back, which is
//! what an operator in the middle of a show expects.
//!
//! ## Fitting a window
//!
//! [`arrange`] decides what is drawn. The plan's collapse order is: hide
//! secondary metadata (the panes' own business), collapse the browser,
//! collapse the inspector, drop the rail labels, and never the preview,
//! the transport or the blackout. In terms of docks:
//!
//! 1. both docks are shown if the preview keeps at least
//!    [`PREVIEW_SHARE`] of the width beside the rail;
//! 2. otherwise the dock whose pane was opened *less* recently gives way
//!    (the browser and the inspector compete for the same contextual
//!    region, UI_IMPLEMENTATION_PLAN section 7);
//! 3. a lone pane that still does not fit shrinks towards its minimum;
//! 4. below that the pane becomes an overlay drawer over the preview.

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
}

/// Which side of the work surface a pane docks to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dock {
    /// Beside the rail.
    Left,
    /// At the far edge.
    Right,
}

impl Pane {
    /// Every pane, in a fixed order that indexes the arrays below.
    pub const ALL: [Pane; 5] = [
        Pane::Browser,
        Pane::Import,
        Pane::Inspector,
        Pane::Diagnostics,
        Pane::Settings,
    ];

    fn index(self) -> usize {
        match self {
            Self::Browser => 0,
            Self::Import => 1,
            Self::Inspector => 2,
            Self::Diagnostics => 3,
            Self::Settings => 4,
        }
    }

    /// The dock this pane lives in.
    pub fn dock(self) -> Dock {
        match self {
            Self::Browser | Self::Import => Dock::Left,
            Self::Inspector | Self::Diagnostics | Self::Settings => Dock::Right,
        }
    }

    /// The pane's title.
    pub fn title(self) -> &'static str {
        match self {
            Self::Browser => "Scenes",
            Self::Import => "Import",
            Self::Inspector => "Controls",
            Self::Diagnostics => "Diagnostics",
            Self::Settings => "Settings",
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
        }
    }

    /// The pane stored under `key`, if there is one.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|pane| pane.key() == key)
    }

    /// The narrowest the pane may be drawn, in points.
    pub fn min_width(self) -> f32 {
        match self {
            Self::Browser | Self::Import => 220.0,
            Self::Inspector => 260.0,
            Self::Diagnostics | Self::Settings => 300.0,
        }
    }

    /// The width a pane opens at, in points.
    pub fn default_width(self) -> f32 {
        match self {
            Self::Browser | Self::Import => 280.0,
            Self::Inspector => 300.0,
            Self::Diagnostics | Self::Settings => 380.0,
        }
    }

    /// The widest the pane may be dragged to, in points.
    pub fn max_width(self) -> f32 {
        match self {
            Self::Browser | Self::Import => 460.0,
            Self::Inspector => 480.0,
            Self::Diagnostics | Self::Settings => 640.0,
        }
    }
}

/// The share of the width beside the rail that the preview keeps before a
/// second dock is allowed to take space from it.
///
/// 0.58 is where, at 1440 wide, the browser and the inspector (280 + 300)
/// still fit beside a preview that keeps 58% of the width, and where, at
/// 1280 and 1366, they do not: those windows show one contextual pane at a
/// time, as UI_IMPLEMENTATION_PLAN section 7 asks. D120 names the reopen
/// condition if the 1280x720 screenshots disagree.
pub const PREVIEW_SHARE: f32 = 0.58;

/// Below this window width the rail drops its labels and a pane becomes an
/// overlay drawer.
pub const NARROW: f32 = 1000.0;

/// Which panes are open, how wide, and in what order they were opened.
#[derive(Clone, Debug, PartialEq)]
pub struct Panes {
    open: [bool; 5],
    width: [f32; 5],
    /// When each pane was last opened, on a counter; larger is newer.
    stamp: [u64; 5],
    clock: u64,
}

impl Default for Panes {
    /// The Perform default (mockup direction B): the inspector open, the
    /// browser closed until asked for.
    fn default() -> Self {
        let mut panes = Self {
            open: [false; 5],
            width: Pane::ALL.map(Pane::default_width),
            stamp: [0; 5],
            clock: 0,
        };
        panes.open(Pane::Inspector);
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

    /// Opens `pane` if it is closed or buried under a newer pane in its
    /// dock, closes it if it is the one showing.
    pub fn toggle(&mut self, pane: Pane) {
        if self.is_open(pane) && self.shown_in(pane.dock()) == Some(pane) {
            self.close(pane);
        } else {
            self.open(pane);
        }
    }

    /// Closes every pane, for a larger preview.
    pub fn close_all(&mut self) {
        self.open = [false; 5];
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
    pub fn set_width(&mut self, pane: Pane, width: f32) {
        if !width.is_finite() {
            return;
        }
        if let Some(slot) = self.width.get_mut(pane.index()) {
            *slot = clamp_width(pane, width);
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
    /// The pane in the left dock and its drawn width.
    pub left: Option<(Pane, f32)>,
    /// The pane in the right dock and its drawn width.
    pub right: Option<(Pane, f32)>,
    /// Whether the docked pane is drawn as an overlay drawer over the
    /// preview instead of beside it (a very narrow window).
    pub overlay: bool,
    /// Whether the rail shows its labels.
    pub rail_labels: bool,
}

impl Arrangement {
    /// The width the docked panes take from the work surface.
    pub fn docked_width(&self) -> f32 {
        if self.overlay {
            return 0.0;
        }
        self.left.map_or(0.0, |(_, w)| w) + self.right.map_or(0.0, |(_, w)| w)
    }
}

/// Decides which panes are drawn, and how wide, for a window `window_width`
/// points wide whose rail is `rail` points wide. See the module docs for
/// the rules.
pub fn arrange(panes: &Panes, window_width: f32, rail: f32) -> Arrangement {
    let rail_labels = window_width >= NARROW;
    let avail = (window_width - rail).max(0.0);
    let mut left = panes
        .shown_in(Dock::Left)
        .map(|pane| (pane, panes.width(pane)));
    let mut right = panes
        .shown_in(Dock::Right)
        .map(|pane| (pane, panes.width(pane)));
    let preview_min = PREVIEW_SHARE * avail;

    if let (Some((l, lw)), Some((r, rw))) = (left, right)
        && avail - lw - rw < preview_min
    {
        // The dock whose pane was opened less recently gives way.
        let left_newer = panes.stamp.get(l.index()) > panes.stamp.get(r.index());
        if left_newer {
            right = None;
        } else {
            left = None;
        }
    }

    // A lone dock may shrink towards its minimum to leave the preview its
    // share; if even the minimum leaves too little, it becomes a drawer.
    let mut overlay = false;
    for (pane, width) in [&mut left, &mut right].into_iter().flatten() {
        let room = avail - preview_min;
        if *width > room {
            if room >= pane.min_width() {
                *width = room;
            } else {
                *width = pane.min_width().min(avail * 0.9);
                overlay = true;
            }
        }
    }
    // Two docks never share a window narrow enough to overlay.
    if overlay {
        let keep_left = match (left, right) {
            (Some((l, _)), Some((r, _))) => panes.stamp.get(l.index()) > panes.stamp.get(r.index()),
            (Some(_), None) => true,
            (None, _) => false,
        };
        if keep_left {
            right = None;
        } else {
            left = None;
        }
    }

    Arrangement {
        left,
        right,
        overlay,
        rail_labels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const RAIL: f32 = 56.0;

    fn with(open: &[Pane]) -> Panes {
        let mut panes = Panes::default();
        panes.close_all();
        for pane in open {
            panes.open(*pane);
        }
        panes
    }

    #[test]
    fn the_default_opens_the_inspector_only() {
        let panes = Panes::default();
        for pane in Pane::ALL {
            assert_eq!(
                panes.is_open(pane),
                pane == Pane::Inspector,
                "{pane:?} in the default layout"
            );
        }
    }

    #[test]
    fn at_1440_the_browser_and_inspector_coexist() {
        let panes = with(&[Pane::Inspector, Pane::Browser]);
        let layout = arrange(&panes, 1440.0, RAIL);
        assert_eq!(layout.left.map(|(p, _)| p), Some(Pane::Browser), "left");
        assert_eq!(layout.right.map(|(p, _)| p), Some(Pane::Inspector), "right");
        assert!(!layout.overlay, "docked, not a drawer");
    }

    #[test]
    fn at_1280_the_newer_pane_wins_the_contextual_region() {
        let panes = with(&[Pane::Inspector, Pane::Browser]);
        let layout = arrange(&panes, 1280.0, RAIL);
        assert_eq!(layout.left.map(|(p, _)| p), Some(Pane::Browser), "browser");
        assert!(layout.right.is_none(), "inspector gives way: {layout:?}");

        let panes = with(&[Pane::Browser, Pane::Inspector]);
        let layout = arrange(&panes, 1280.0, RAIL);
        assert!(layout.left.is_none(), "browser gives way: {layout:?}");
        assert_eq!(
            layout.right.map(|(p, _)| p),
            Some(Pane::Inspector),
            "inspector"
        );
    }

    #[test]
    fn at_1920_everything_open_fits() {
        let panes = with(&[Pane::Inspector, Pane::Browser]);
        let layout = arrange(&panes, 1920.0, RAIL);
        assert!(
            layout.left.is_some() && layout.right.is_some(),
            "{layout:?}"
        );
    }

    #[test]
    fn the_preview_always_keeps_its_share_when_docked() {
        for width in [900.0_f32, 1000.0, 1280.0, 1366.0, 1440.0, 1920.0] {
            for open in [
                vec![Pane::Inspector],
                vec![Pane::Browser],
                vec![Pane::Inspector, Pane::Browser],
                vec![Pane::Settings, Pane::Import],
            ] {
                let layout = arrange(&with(&open), width, RAIL);
                if layout.overlay {
                    continue;
                }
                let preview = width - RAIL - layout.docked_width();
                assert!(
                    preview >= PREVIEW_SHARE * (width - RAIL) - 0.5,
                    "{open:?} at {width}: the preview keeps only {preview}"
                );
            }
        }
    }

    #[test]
    fn a_very_narrow_window_turns_the_pane_into_a_drawer() {
        let layout = arrange(&with(&[Pane::Inspector]), 640.0, RAIL);
        assert!(layout.overlay, "{layout:?}");
        assert!(!layout.rail_labels, "labels go first");
        assert_eq!(layout.docked_width(), 0.0, "a drawer takes no layout width");
    }

    #[test]
    fn a_lone_pane_shrinks_before_it_becomes_a_drawer() {
        // 1100 wide: 0.58 * 1044 leaves 438 for the pane, above its
        // 260 minimum but below the 480 maximum it was dragged to.
        let mut panes = with(&[Pane::Inspector]);
        panes.set_width(Pane::Inspector, 480.0);
        let layout = arrange(&panes, 1100.0, RAIL);
        let drawn = layout.right.map(|(_, w)| w).unwrap_or(0.0);
        assert!(!layout.overlay, "{layout:?}");
        assert!(
            drawn < 480.0 && drawn >= Pane::Inspector.min_width(),
            "{drawn}"
        );
    }

    #[test]
    fn opening_over_a_pane_and_closing_reveals_the_one_beneath() {
        let mut panes = with(&[Pane::Inspector]);
        panes.open(Pane::Diagnostics);
        assert_eq!(panes.shown_in(Dock::Right), Some(Pane::Diagnostics), "over");
        assert_eq!(panes.close_topmost(), Some(Pane::Diagnostics), "esc");
        assert_eq!(panes.shown_in(Dock::Right), Some(Pane::Inspector), "under");
        assert_eq!(panes.close_topmost(), Some(Pane::Inspector), "esc again");
        assert_eq!(panes.close_topmost(), None, "nothing left to close");
    }

    #[test]
    fn toggling_a_buried_pane_raises_it_instead_of_closing_it() {
        let mut panes = with(&[Pane::Inspector]);
        panes.open(Pane::Settings);
        panes.toggle(Pane::Inspector);
        assert_eq!(
            panes.shown_in(Dock::Right),
            Some(Pane::Inspector),
            "a click on a hidden pane's button shows it"
        );
        panes.toggle(Pane::Inspector);
        assert!(!panes.is_open(Pane::Inspector), "and a second click closes");
    }

    #[test]
    fn widths_are_clamped_and_nonsense_is_ignored() {
        let mut panes = Panes::default();
        panes.set_width(Pane::Browser, 10.0);
        assert_eq!(panes.width(Pane::Browser), Pane::Browser.min_width(), "min");
        panes.set_width(Pane::Browser, 9999.0);
        assert_eq!(panes.width(Pane::Browser), Pane::Browser.max_width(), "max");
        panes.set_width(Pane::Browser, 300.0);
        panes.set_width(Pane::Browser, f32::NAN);
        assert_eq!(panes.width(Pane::Browser), 300.0, "NaN leaves it alone");
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
