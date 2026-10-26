// ============================================================================
// Module:       strobe_app::ui::motion
// Description:  The egui binding for crate::motion: the helpers that read and
//               advance an animation held in the Context.
//
// Dependencies: egui (the animation state lives in its Context); crate::motion.
//               Ported from rustaman's `gui::ui::motion` (D5, D130).
// ============================================================================

//! Animation, as egui holds it.
//!
//! [`crate::motion`] owns the curves, the four named durations, the reduced-
//! motion switch and [`Tween`]; it has no egui in it. This module is the
//! part that needs a `Context`: egui keeps a value per [`egui::Id`] and
//! advances it itself, which is what makes an animation survive the
//! immediate-mode redraw that has no memory of the previous frame.
//!
//! **This is the only file in the app allowed to call `ctx.animate_*`.**
//! `no_drawing_module_animates_by_hand` in [`super`] fails the build for a
//! call anywhere else, because a call site that reaches past this module
//! picked its own duration. Every duration passes through
//! [`crate::motion::effective`], so reduced motion needs no cooperation from
//! any widget.
//!
//! ## Ids must name the thing, not its position
//!
//! Every helper takes an [`egui::Id`], and it must be derived from the
//! *thing* being animated rather than from where it currently sits: an id
//! built from a loop index animates the position, so removing the third
//! scene from a list makes every row below it flash.
//!
//! ## Animation and the repaint rate
//!
//! The control window repaints at a fixed 30 Hz (D96's schedule), so an
//! animation costs nothing extra: egui advances it on frames that are drawn
//! anyway. Values that arrive (`settled`) return their *target* the first
//! time an id is seen, so a row scrolling into view shows its real value
//! instead of animating up from zero.

pub use crate::motion::{ENTER, INSTANT, QUICK, SETTLE, Tween, ease_in_out, ease_out, spring};

use crate::motion::{ease_in_out as symmetric, ease_out as responsive, effective};
use egui::{Context, Id, Ui};

/// How far a boolean animation has progressed, 0..=1, eased out.
#[must_use]
pub fn toggle(ctx: &Context, id: Id, on: bool, seconds: f32) -> f32 {
    responsive(ctx.animate_bool_with_time(id, on, effective(seconds)))
}

/// [`toggle`], for the common case of a hover.
#[must_use]
pub fn hover(ui: &Ui, id: Id, hovered: bool) -> f32 {
    toggle(ui.ctx(), id, hovered, INSTANT)
}

/// [`toggle`] with the symmetric curve, for a state change no pointer is
/// driving.
#[must_use]
pub fn transition(ctx: &Context, id: Id, on: bool, seconds: f32) -> f32 {
    symmetric(ctx.animate_bool_with_time(id, on, effective(seconds)))
}

/// A continuous value that chases its target over [`SETTLE`].
#[must_use]
pub fn value(ctx: &Context, id: Id, target: f32) -> f32 {
    settled(ctx, id, target, SETTLE)
}

/// [`value`] with an explicit duration.
#[must_use]
pub fn settled(ctx: &Context, id: Id, target: f32, seconds: f32) -> f32 {
    // A non-finite target would be stored and then returned forever,
    // because every later comparison against it is false: the animation
    // would never converge and the control would be stuck.
    let target = if target.is_finite() { target } else { 0.0 };
    ctx.animate_value_with_time(id, target, effective(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_non_finite_target_settles_at_zero_rather_than_getting_stuck() {
        let ctx = Context::default();
        let id = Id::unique("nonsense-target");
        let result = settled(&ctx, id, f32::NAN, SETTLE);
        assert!(
            result.is_finite(),
            "a NaN target produced a non-finite animated value: {result}"
        );
        let result = settled(&ctx, id, f32::INFINITY, SETTLE);
        assert!(
            result.is_finite(),
            "an infinite target produced a non-finite animated value: {result}"
        );
    }

    #[test]
    fn a_settled_value_arrives_at_its_finite_target() {
        let ctx = Context::default();
        let id = Id::unique("real-target");
        let result = settled(&ctx, id, 42.0, SETTLE);
        assert!(
            (result - 42.0).abs() < f32::EPSILON,
            "expected the first observation to return the target, got {result}"
        );
    }
}
