// ============================================================================
// Module:       strobe_app::icon
// Description:  Every icon in the app as geometry on a 16x16 grid: the shapes,
//               not the painting.
//
// Dependencies: std only, deliberately. See the module docs. Ported from
//               rustaman's `icon` module (D5, D130).
// ============================================================================

//! The icon set, as geometry.
//!
//! ## Why icons are drawn
//!
//! egui bundles Ubuntu Sans, Hack and an emoji subset. Between them they
//! cover Latin, Greek, Cyrillic and a few hundred emoji, and almost nothing
//! in the Miscellaneous Symbols, Geometric Shapes or Dingbats blocks, which
//! is where every icon-shaped character lives. A glyph the font lacks is
//! drawn as a replacement box, so a rail of Unicode icons is a column of
//! squares. The three ways out are an icon font (a binary, a licence, magic
//! codepoints, and the same silent failure when one is wrong), SVGs (another
//! dependency, and an icon that has to be *loaded* can fail to load), or
//! drawing them. An icon at this size is four to eight line segments. It
//! takes the theme's colour as an argument, scales to any size, cannot fail
//! to load and cannot render as a box. So: draw them.
//! `no_drawing_module_sets_an_icon_in_a_font` in [`crate::ui`] scans the
//! drawing modules for pictographic characters and fails the build for one.
//!
//! ## Why the shapes are here and the painting is not
//!
//! Nothing in this file mentions egui. The geometry is static data, so its
//! tests run anywhere, and so that three properties can be *checked* rather
//! than eyeballed: every icon stays inside its grid, is optically centred,
//! and reaches far enough across the grid to look like a member of the same
//! set. Those are the failures that make an icon set look homemade, and none
//! needs a window to detect. [`crate::ui::icon`] holds the painting.
//!
//! The geometry is `&'static` data (no `Vec` is built to draw an icon), so
//! painting a rail of them allocates only what egui's own shapes need.
//!
//! ## The grid
//!
//! Every icon is designed on a 16x16 grid with a margin, and the coordinates
//! below are in that space; the painter maps them onto whatever rect it is
//! given. Working in one grid is what makes a row of different icons look
//! like a set.

/// The design grid every icon below is drawn on.
pub const GRID: f32 = 16.0;

/// The stroke weight, in grid units.
///
/// 1.5 rather than 1.0: at a 16 px render a one-unit stroke lands on exactly
/// one physical pixel and comes out lighter than the text beside it.
pub const WEIGHT: f32 = 1.5;

/// Every icon the app draws.
///
/// One enum so that `wildcard_enum_match_arm` applies: adding a variant
/// fails the build in [`Icon::segments`] until it is actually drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Icon {
    /// The Perform workspace: a play mark in a ring.
    Perform,
    /// The Library: three books on a shelf.
    Library,
    /// The Playlist: rows with a cue marker.
    Playlist,
    /// The Editor: angle brackets and a slash.
    Editor,
    /// Mappings: two ends joined.
    Mapping,
    /// Diagnostics: a pulse trace.
    Diagnostics,
    /// Settings: two sliders.
    Settings,
    /// Previous cue.
    Prev,
    /// Next cue.
    Next,
    /// Play.
    Play,
    /// Hold (pause).
    Pause,
    /// Blackout: the power mark.
    Blackout,
    /// A collapsed disclosure control.
    ChevronRight,
    /// An expanded disclosure control.
    ChevronDown,
    /// Search.
    Search,
    /// Dismiss, cancel, close.
    Close,
    /// The window's minimise control.
    WindowMinimise,
    /// The window's maximise control.
    WindowMaximise,
    /// The window's restore control, when it is already maximised.
    WindowRestore,
    /// Something is fine.
    Check,
    /// Something needs attention.
    Warning,
    /// Something failed.
    Error,
    /// Something to know: a circle with an i.
    Info,
    /// Refresh, re-read.
    Refresh,
    /// Reset to a default: a counter-clockwise arrow.
    Reset,
    /// Copy to the clipboard.
    Copy,
    /// Open the containing folder.
    Folder,
    /// Import: an arrow into a tray.
    Import,
    /// The output window: a display on a stand.
    Output,
    /// Fullscreen: four corners.
    Fullscreen,
    /// Audio: level bars.
    Audio,
    /// The control inspector: three faders.
    Controls,
    /// A favourite: a five-pointed star.
    Star,
}

/// One stroked part of an icon, in grid space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Seg {
    /// A polyline.
    Open(&'static [(f32, f32)]),
    /// A polyline whose end joins its start.
    Closed(&'static [(f32, f32)]),
    /// A filled convex shape. Almost nothing wants this; transport marks do,
    /// because a solid triangle is what everyone reads as "play".
    Solid(&'static [(f32, f32)]),
    /// A filled dot, as a zero-length round-capped stroke.
    Dot(f32, f32),
    /// A circle: centre x, centre y, radius.
    Circle(f32, f32, f32),
    /// An arc: centre x, centre y, radius, start and end angle in radians
    /// (y points down, so positive angles run clockwise on screen).
    Arc(f32, f32, f32, f32, f32),
}

impl Icon {
    /// The parts this icon is drawn from, in grid units.
    pub fn segments(self) -> &'static [Seg] {
        match self {
            Self::Perform => &[
                Seg::Circle(8.0, 8.0, 5.6),
                Seg::Solid(&[(6.8, 5.6), (10.8, 8.0), (6.8, 10.4)]),
            ],
            Self::Library => &[
                Seg::Closed(&[(2.5, 3.5), (5.5, 3.5), (5.5, 12.5), (2.5, 12.5)]),
                Seg::Closed(&[(6.5, 3.5), (9.5, 3.5), (9.5, 12.5), (6.5, 12.5)]),
                Seg::Closed(&[(10.5, 3.5), (13.5, 3.5), (13.5, 12.5), (10.5, 12.5)]),
            ],
            Self::Playlist => &[
                Seg::Solid(&[(2.5, 3.0), (5.5, 4.5), (2.5, 6.0)]),
                Seg::Open(&[(7.5, 4.5), (13.5, 4.5)]),
                Seg::Open(&[(2.5, 8.5), (13.5, 8.5)]),
                Seg::Open(&[(2.5, 12.0), (10.0, 12.0)]),
            ],
            Self::Editor => &[
                Seg::Open(&[(6.0, 4.5), (2.5, 8.0), (6.0, 11.5)]),
                Seg::Open(&[(10.0, 4.5), (13.5, 8.0), (10.0, 11.5)]),
                Seg::Open(&[(9.0, 3.0), (7.0, 13.0)]),
            ],
            Self::Mapping => &[
                Seg::Circle(4.6, 8.0, 2.2),
                Seg::Circle(11.4, 8.0, 2.2),
                Seg::Open(&[(6.8, 8.0), (9.2, 8.0)]),
            ],
            Self::Diagnostics => &[Seg::Open(&[
                (2.0, 8.0),
                (5.0, 8.0),
                (6.6, 3.8),
                (9.2, 12.4),
                (10.8, 8.0),
                (14.0, 8.0),
            ])],
            Self::Settings => &[
                Seg::Open(&[(2.5, 5.5), (13.5, 5.5)]),
                Seg::Circle(6.0, 5.5, 1.8),
                Seg::Open(&[(2.5, 10.5), (13.5, 10.5)]),
                Seg::Circle(10.5, 10.5, 1.8),
            ],
            Self::Prev => &[
                Seg::Open(&[(3.5, 3.5), (3.5, 12.5)]),
                Seg::Solid(&[(12.5, 3.5), (12.5, 12.5), (6.0, 8.0)]),
            ],
            Self::Next => &[
                Seg::Solid(&[(3.5, 3.5), (3.5, 12.5), (10.0, 8.0)]),
                Seg::Open(&[(12.5, 3.5), (12.5, 12.5)]),
            ],
            Self::Play => &[Seg::Solid(&[(4.5, 3.0), (12.5, 8.0), (4.5, 13.0)])],
            Self::Pause => &[
                Seg::Solid(&[(4.5, 3.5), (7.0, 3.5), (7.0, 12.5), (4.5, 12.5)]),
                Seg::Solid(&[(9.0, 3.5), (11.5, 3.5), (11.5, 12.5), (9.0, 12.5)]),
            ],
            Self::Blackout => &[
                Seg::Open(&[(8.0, 2.5), (8.0, 7.5)]),
                Seg::Arc(8.0, 8.5, 5.0, -0.97, 4.11),
            ],
            Self::ChevronRight => &[Seg::Open(&[(6.5, 3.5), (11.0, 8.0), (6.5, 12.5)])],
            Self::ChevronDown => &[Seg::Open(&[(3.5, 6.5), (8.0, 11.0), (12.5, 6.5)])],
            Self::Search => &[
                Seg::Circle(7.0, 7.0, 4.0),
                Seg::Open(&[(10.0, 10.0), (13.5, 13.5)]),
            ],
            Self::Close => &[
                Seg::Open(&[(4.5, 4.5), (11.5, 11.5)]),
                Seg::Open(&[(11.5, 4.5), (4.5, 11.5)]),
            ],
            // The window controls follow the shapes Windows itself uses,
            // because these three are the one place in the app where
            // matching the platform beats being distinctive.
            Self::WindowMinimise => &[Seg::Open(&[(4.0, 8.0), (12.0, 8.0)])],
            Self::WindowMaximise => &[Seg::Closed(&[
                (4.0, 4.0),
                (12.0, 4.0),
                (12.0, 12.0),
                (4.0, 12.0),
            ])],
            Self::WindowRestore => &[
                Seg::Closed(&[(3.0, 6.0), (10.0, 6.0), (10.0, 13.0), (3.0, 13.0)]),
                Seg::Open(&[
                    (6.0, 6.0),
                    (6.0, 3.0),
                    (13.0, 3.0),
                    (13.0, 10.0),
                    (10.0, 10.0),
                ]),
            ],
            Self::Check => &[Seg::Open(&[(3.5, 8.5), (6.5, 11.5), (12.5, 5.0)])],
            Self::Warning => &[
                Seg::Closed(&[(8.0, 2.5), (14.0, 13.0), (2.0, 13.0)]),
                Seg::Open(&[(8.0, 6.5), (8.0, 9.5)]),
                Seg::Dot(8.0, 11.3),
            ],
            Self::Error => &[
                Seg::Circle(8.0, 8.0, 5.8),
                Seg::Open(&[(5.9, 5.9), (10.1, 10.1)]),
                Seg::Open(&[(10.1, 5.9), (5.9, 10.1)]),
            ],
            Self::Info => &[
                Seg::Circle(8.0, 8.0, 5.8),
                Seg::Dot(8.0, 5.3),
                Seg::Open(&[(8.0, 7.6), (8.0, 11.0)]),
            ],
            Self::Refresh => &[
                Seg::Arc(8.0, 8.0, 5.0, 0.6, 5.4),
                Seg::Open(&[(11.0, 1.5), (12.2, 4.6), (9.0, 5.2)]),
            ],
            Self::Reset => &[
                Seg::Arc(8.0, 8.0, 5.0, 2.54, -2.26),
                Seg::Open(&[(5.0, 1.5), (3.8, 4.6), (7.0, 5.2)]),
            ],
            Self::Copy => &[
                Seg::Closed(&[(6.0, 6.0), (13.0, 6.0), (13.0, 13.5), (6.0, 13.5)]),
                Seg::Open(&[(3.0, 10.0), (3.0, 2.5), (10.0, 2.5)]),
            ],
            Self::Folder => &[Seg::Closed(&[
                (2.5, 12.5),
                (2.5, 4.0),
                (6.5, 4.0),
                (8.0, 6.0),
                (13.5, 6.0),
                (13.5, 12.5),
            ])],
            Self::Import => &[
                Seg::Open(&[(3.0, 9.5), (3.0, 13.0), (13.0, 13.0), (13.0, 9.5)]),
                Seg::Open(&[(8.0, 2.5), (8.0, 10.0)]),
                Seg::Open(&[(5.0, 7.2), (8.0, 10.2), (11.0, 7.2)]),
            ],
            Self::Output => &[
                Seg::Closed(&[(2.5, 3.0), (13.5, 3.0), (13.5, 10.5), (2.5, 10.5)]),
                Seg::Open(&[(8.0, 10.5), (8.0, 13.0)]),
                Seg::Open(&[(5.5, 13.0), (10.5, 13.0)]),
            ],
            Self::Fullscreen => &[
                Seg::Open(&[(2.5, 6.0), (2.5, 2.5), (6.0, 2.5)]),
                Seg::Open(&[(10.0, 2.5), (13.5, 2.5), (13.5, 6.0)]),
                Seg::Open(&[(13.5, 10.0), (13.5, 13.5), (10.0, 13.5)]),
                Seg::Open(&[(6.0, 13.5), (2.5, 13.5), (2.5, 10.0)]),
            ],
            Self::Audio => &[
                Seg::Open(&[(3.5, 8.5), (3.5, 12.5)]),
                Seg::Open(&[(6.5, 4.0), (6.5, 12.5)]),
                Seg::Open(&[(9.5, 6.0), (9.5, 12.5)]),
                Seg::Open(&[(12.5, 9.5), (12.5, 12.5)]),
            ],
            Self::Star => &[Seg::Closed(&[
                (8.0, 2.4),
                (9.5, 6.4),
                (13.7, 6.5),
                (10.4, 9.2),
                (11.5, 13.3),
                (8.0, 10.9),
                (4.5, 13.3),
                (5.6, 9.2),
                (2.3, 6.5),
                (6.5, 6.4),
            ])],
            Self::Controls => &[
                Seg::Open(&[(4.0, 2.5), (4.0, 13.5)]),
                Seg::Circle(4.0, 10.0, 1.7),
                Seg::Open(&[(8.0, 2.5), (8.0, 13.5)]),
                Seg::Circle(8.0, 5.5, 1.7),
                Seg::Open(&[(12.0, 2.5), (12.0, 13.5)]),
                Seg::Circle(12.0, 8.5, 1.7),
            ],
        }
    }
}

/// Segments per full turn when a circle or arc becomes a polyline.
///
/// Sixteen: at the sizes these are drawn the difference between sixteen and
/// a true circle is well under a pixel, and a polygon goes through the same
/// stroking path as everything else.
pub const CIRCLE_SEGMENTS: usize = 16;

/// The most points [`arc_points`] can produce: a full turn closes on its
/// start, so one more than [`CIRCLE_SEGMENTS`].
pub const MAX_ARC_POINTS: usize = CIRCLE_SEGMENTS + 1;

/// The points of an arc of radius `radius` about `(cx, cy)` from angle
/// `start` to `end`, written into `out`; returns how many were written.
///
/// A fixed buffer rather than a `Vec`, so painting an icon does not allocate
/// to flatten a circle.
pub fn arc_points(
    cx: f32,
    cy: f32,
    radius: f32,
    start: f32,
    end: f32,
    out: &mut [(f32, f32); MAX_ARC_POINTS],
) -> usize {
    let span = end - start;
    let turns = span.abs() / std::f32::consts::TAU;
    let steps = ((turns * CIRCLE_SEGMENTS as f32).ceil() as usize).clamp(2, CIRCLE_SEGMENTS);
    for step in 0..=steps {
        let angle = start + span * step as f32 / steps as f32;
        let (sin, cos) = angle.sin_cos();
        if let Some(slot) = out.get_mut(step) {
            *slot = (cx + cos * radius, cy + sin * radius);
        }
    }
    steps + 1
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    /// Every icon the enum names, so a new variant is drawn by the tests
    /// below without anyone remembering to add it. Kept in declaration
    /// order: the check below reads each entry's discriminant.
    const ALL: [Icon; 33] = [
        Icon::Perform,
        Icon::Library,
        Icon::Playlist,
        Icon::Editor,
        Icon::Mapping,
        Icon::Diagnostics,
        Icon::Settings,
        Icon::Prev,
        Icon::Next,
        Icon::Play,
        Icon::Pause,
        Icon::Blackout,
        Icon::ChevronRight,
        Icon::ChevronDown,
        Icon::Search,
        Icon::Close,
        Icon::WindowMinimise,
        Icon::WindowMaximise,
        Icon::WindowRestore,
        Icon::Check,
        Icon::Warning,
        Icon::Error,
        Icon::Info,
        Icon::Refresh,
        Icon::Reset,
        Icon::Copy,
        Icon::Folder,
        Icon::Import,
        Icon::Output,
        Icon::Fullscreen,
        Icon::Audio,
        Icon::Controls,
        Icon::Star,
    ];

    // `ALL` names each icon exactly once, checked when the tests are
    // compiled. `Icon` is field-less with no explicit discriminants, so
    // `as usize` is each variant's position in the enum; a list that omits
    // an entry leaves a gap, and one that repeats an entry trips the
    // duplicate check.
    const _: () = {
        let mut seen = [false; ALL.len()];
        let mut index = 0;
        while index < ALL.len() {
            let variant = ALL[index] as usize;
            assert!(
                variant < seen.len(),
                "an Icon variant is missing from ALL: one of them is exempt \
                 from every geometry test below"
            );
            assert!(!seen[variant], "ALL names the same icon twice");
            seen[variant] = true;
            index += 1;
        }
        let mut variant = 0;
        while variant < seen.len() {
            assert!(seen[variant], "an Icon variant is missing from ALL");
            variant += 1;
        }
    };

    /// Every point of an icon, with circles and arcs flattened.
    fn points_of(icon: Icon) -> Vec<(f32, f32)> {
        let mut points = Vec::new();
        for seg in icon.segments() {
            match *seg {
                Seg::Open(p) | Seg::Closed(p) | Seg::Solid(p) => points.extend_from_slice(p),
                Seg::Dot(x, y) => points.push((x, y)),
                Seg::Circle(cx, cy, r) => {
                    let mut buffer = [(0.0, 0.0); MAX_ARC_POINTS];
                    let n = arc_points(cx, cy, r, 0.0, std::f32::consts::TAU, &mut buffer);
                    points.extend_from_slice(&buffer[..n]);
                }
                Seg::Arc(cx, cy, r, a, b) => {
                    let mut buffer = [(0.0, 0.0); MAX_ARC_POINTS];
                    let n = arc_points(cx, cy, r, a, b, &mut buffer);
                    points.extend_from_slice(&buffer[..n]);
                }
            }
        }
        points
    }

    /// The ink's bounding box: `(min_x, max_x, min_y, max_y)`.
    fn bounds(icon: Icon) -> Result<(f32, f32, f32, f32)> {
        let points = points_of(icon);
        let Some(&(x, y)) = points.first() else {
            anyhow::bail!("{icon:?} draws nothing at all");
        };
        let (mut min_x, mut max_x, mut min_y, mut max_y) = (x, x, y, y);
        for &(x, y) in &points {
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
        Ok((min_x, max_x, min_y, max_y))
    }

    #[test]
    fn every_icon_stays_inside_the_grid() -> Result<()> {
        for icon in ALL {
            for (x, y) in points_of(icon) {
                assert!(
                    (0.0..=GRID).contains(&x) && (0.0..=GRID).contains(&y),
                    "{icon:?} has a point at ({x}, {y}), outside the \
                     0..={GRID} grid: it will be clipped by its own rect"
                );
            }
        }
        Ok(())
    }

    /// An icon whose ink sits in a corner of the grid reads as off-centre
    /// next to one that is balanced, however carefully its rect is placed.
    #[test]
    fn every_icon_is_optically_centred() -> Result<()> {
        /// How far the ink's centre may sit from the grid's, in grid units.
        /// One unit is about a pixel at the sizes these are drawn at.
        const TOLERANCE: f32 = 1.0;
        for icon in ALL {
            let (min_x, max_x, min_y, max_y) = bounds(icon)?;
            let (cx, cy) = ((min_x + max_x) / 2.0, (min_y + max_y) / 2.0);
            assert!(
                (cx - GRID / 2.0).abs() <= TOLERANCE,
                "{icon:?} is centred at x={cx}, not {}",
                GRID / 2.0
            );
            assert!(
                (cy - GRID / 2.0).abs() <= TOLERANCE,
                "{icon:?} is centred at y={cy}, not {}",
                GRID / 2.0
            );
        }
        Ok(())
    }

    /// An icon has to reach across its grid, or it reads as smaller than
    /// the ones beside it even though its box is the same.
    #[test]
    fn every_icon_fills_enough_of_its_grid_to_match_its_neighbours() -> Result<()> {
        /// The smallest span, in grid units, on an icon's longer axis.
        const MINIMUM_SPAN: f32 = 7.0;
        for icon in ALL {
            let (min_x, max_x, min_y, max_y) = bounds(icon)?;
            let span = (max_x - min_x).max(max_y - min_y);
            assert!(
                span >= MINIMUM_SPAN,
                "{icon:?} spans only {span} grid units, so it reads as \
                 smaller than the icons beside it"
            );
        }
        Ok(())
    }

    #[test]
    fn no_two_icons_are_drawn_identically() {
        for (index, a) in ALL.iter().enumerate() {
            for b in ALL.iter().skip(index + 1) {
                assert!(
                    a.segments() != b.segments(),
                    "{a:?} and {b:?} are the same drawing"
                );
            }
        }
    }

    #[test]
    fn a_flattened_arc_never_overruns_its_buffer() {
        let mut buffer = [(0.0, 0.0); MAX_ARC_POINTS];
        // Far more than a full turn: the step count is clamped.
        let n = arc_points(8.0, 8.0, 5.0, 0.0, 100.0, &mut buffer);
        assert!(n <= MAX_ARC_POINTS, "wrote {n} points");
        let n = arc_points(8.0, 8.0, 5.0, 0.0, f32::NAN, &mut buffer);
        assert!(n <= MAX_ARC_POINTS, "NaN span wrote {n} points");
    }
}
