// ============================================================================
// Module:       strobe_app::ui::icon
// Description:  Painting crate::icon's geometry with egui, and the sizes the
//               app draws an icon at.
//
// Dependencies: egui (Painter, Shape); crate::icon; super::theme.
//               Ported from rustaman's `gui::ui::icon` (D5, D130).
// ============================================================================

//! The icon set, painted.
//!
//! [`crate::icon`] owns the shapes, as static data with tests that run
//! anywhere. This module maps them onto a rect and strokes them.
//!
//! ## Strokes, not fills
//!
//! Every icon is a stroked path with round caps and joins (a handful are
//! solid, like the play triangle). A stroke takes the theme's colour as one
//! argument and scales with the icon, so an icon at 12 px in a list row and
//! the same icon at 20 px in the rail carry the same visual weight; at 16 px
//! a mitre joint on a chevron renders as a spike a pixel long.

use crate::icon::{GRID, Icon, MAX_ARC_POINTS, Seg, WEIGHT, arc_points};
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, Vec2};

/// Paints `icon` inside `rect`, in `color`. The icon is centred and drawn on
/// the largest square that fits, so a caller may hand over a non-square rect
/// without the shape distorting.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    paint_rotated(painter, rect, icon, color, 0.0);
}

/// [`paint`], turned `turns` of a full rotation clockwise about the icon's
/// centre. A disclosure chevron that *rotates* between closed and open reads
/// as the same object turning, where two chevrons swapped at the halfway
/// point read as one being replaced by another.
pub fn paint_rotated(painter: &Painter, rect: Rect, icon: Icon, color: Color32, turns: f32) {
    let side = rect.width().min(rect.height());
    if side <= 0.0 {
        return;
    }
    let scale = side / GRID;
    let origin = rect.center() - Vec2::splat(side / 2.0);
    let stroke = Stroke::new((WEIGHT * scale).max(1.0), color);

    // Rotation happens in grid space, about the grid's own centre, so an
    // icon that is optically centred stays centred as it turns.
    let turns = if turns.is_finite() { turns } else { 0.0 };
    let (sin, cos) = (turns * std::f32::consts::TAU).sin_cos();
    let half = GRID / 2.0;
    let place = |&(x, y): &(f32, f32)| {
        let (dx, dy) = (x - half, y - half);
        let (rx, ry) = (dx * cos - dy * sin + half, dx * sin + dy * cos + half);
        Pos2::new(origin.x + rx * scale, origin.y + ry * scale)
    };

    for seg in icon.segments() {
        match *seg {
            Seg::Open(points) => {
                painter.add(Shape::line(points.iter().map(place).collect(), stroke));
            }
            Seg::Closed(points) => {
                let mut placed: Vec<Pos2> = points.iter().map(place).collect();
                if let Some(&first) = placed.first() {
                    placed.push(first);
                }
                painter.add(Shape::line(placed, stroke));
            }
            Seg::Solid(points) => {
                // `convex_polygon` is the cheap path, and these shapes are
                // convex. It takes its own stroke, which is `NONE`: a filled
                // shape that is also outlined is drawn half a stroke larger
                // than the grid says.
                painter.add(Shape::convex_polygon(
                    points.iter().map(place).collect(),
                    color,
                    Stroke::NONE,
                ));
            }
            Seg::Dot(x, y) => {
                painter.circle_filled(place(&(x, y)), stroke.width / 2.0, color);
            }
            Seg::Circle(cx, cy, radius) => {
                let mut buffer = [(0.0, 0.0); MAX_ARC_POINTS];
                let count = arc_points(cx, cy, radius, 0.0, std::f32::consts::TAU, &mut buffer);
                let mut placed: Vec<Pos2> = buffer.iter().take(count).map(place).collect();
                if let Some(&first) = placed.first() {
                    placed.push(first);
                }
                painter.add(Shape::line(placed, stroke));
            }
            Seg::Arc(cx, cy, radius, start, end) => {
                let mut buffer = [(0.0, 0.0); MAX_ARC_POINTS];
                let count = arc_points(cx, cy, radius, start, end, &mut buffer);
                painter.add(Shape::line(
                    buffer.iter().take(count).map(place).collect(),
                    stroke,
                ));
            }
        }
    }
}

/// Allocates a square of `size` in the current layout and paints `icon` into
/// it.
pub fn show(ui: &mut egui::Ui, icon: Icon, size: f32, color: Color32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    paint(ui.painter(), rect, icon, color);
    response
}

/// The size an icon is drawn at beside body text. Slightly smaller than the
/// text's own line height: an icon at the full line height optically
/// outweighs the label next to it.
pub const INLINE: f32 = 15.0;

/// The size for a primary destination: a rail entry.
pub const NAV: f32 = 20.0;

/// The size for a transport button's mark.
pub const TRANSPORT: f32 = 22.0;

/// The size for a disclosure arrow inside a dense row.
pub const DISCLOSURE: f32 = 12.0;

// The icon sizes have to keep their order, and these are relations between
// constants rather than facts about a running program.
const _: () = {
    assert!(
        NAV >= INLINE,
        "a nav icon smaller than an inline one inverts the hierarchy"
    );
    assert!(
        TRANSPORT >= NAV,
        "a transport mark is the largest icon on the surface"
    );
    assert!(
        DISCLOSURE < INLINE,
        "a disclosure arrow the size of an inline icon competes with the \
         row's own content"
    );
};
