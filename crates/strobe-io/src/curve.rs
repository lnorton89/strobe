// ============================================================================
// Module:       strobe_io::curve
// Description:  The response curves a mapping applies between a controller's
//               travel and the control's range.
//
// Dependencies: serde
// ============================================================================

//! Response curves.
//!
//! A fader's travel is linear; many controls are not. Brightness and
//! frequency feel better when the first half of the fader covers the small
//! values ([`Curve::Exp`]), and a control whose useful values crowd the top
//! feels better the other way ([`Curve::Log`]). Both are one-parameter
//! exponential families that map `0` to `0` and `1` to `1` exactly, are
//! monotonic, and are each other's inverse, which the tests pin.

use serde::{Deserialize, Serialize};

/// How strongly the curves bend: the exponent in `e^(K x)`.
const BEND: f32 = 3.0;

/// A response curve over `0..=1`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Curve {
    /// Travel and value move together.
    #[default]
    Linear,
    /// Fast at first, then slow: small movements near the bottom matter.
    Log,
    /// Slow at first, then fast: fine control of the low values.
    Exp,
}

impl Curve {
    /// Every curve, for a picker.
    pub const ALL: [Curve; 3] = [Self::Linear, Self::Log, Self::Exp];

    /// The curve's label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Linear => "Linear",
            Self::Log => "Log",
            Self::Exp => "Exp",
        }
    }

    /// `x` (clamped to `0..=1`) through the curve. A NaN is `0`.
    pub fn apply(self, x: f32) -> f32 {
        let x = if x.is_nan() { 0.0 } else { x.clamp(0.0, 1.0) };
        let span = BEND.exp() - 1.0;
        match self {
            Self::Linear => x,
            Self::Exp => ((BEND * x).exp() - 1.0) / span,
            Self::Log => (1.0 + x * span).ln() / BEND,
        }
    }

    /// The travel that [`Self::apply`] maps to `y`: what a motorised fader
    /// or an LED ring must show for a value (MIDI feedback, D257). `Exp` and
    /// `Log` are each other's inverse.
    pub fn inverse(self, y: f32) -> f32 {
        match self {
            Self::Linear => Self::Linear.apply(y),
            Self::Exp => Self::Log.apply(y),
            Self::Log => Self::Exp.apply(y),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_curve_keeps_the_ends() {
        for curve in Curve::ALL {
            assert!(curve.apply(0.0).abs() < 1e-6, "{curve:?} at 0");
            assert!((curve.apply(1.0) - 1.0).abs() < 1e-6, "{curve:?} at 1");
        }
    }

    #[test]
    fn exp_is_below_the_diagonal_and_log_above() {
        for step in 1..10 {
            let x = step as f32 / 10.0;
            assert!(Curve::Exp.apply(x) < x, "exp at {x}");
            assert!(Curve::Log.apply(x) > x, "log at {x}");
            assert!(
                (Curve::Linear.apply(x) - x).abs() < f32::EPSILON,
                "linear at {x}"
            );
        }
    }

    #[test]
    fn log_undoes_exp() {
        for step in 0..=20 {
            let x = step as f32 / 20.0;
            let round = Curve::Log.apply(Curve::Exp.apply(x));
            assert!((round - x).abs() < 1e-4, "{x} -> {round}");
        }
    }

    #[test]
    fn curves_are_monotonic() {
        for curve in Curve::ALL {
            let mut last = -1.0;
            for step in 0..=100 {
                let y = curve.apply(step as f32 / 100.0);
                assert!(y >= last, "{curve:?} fell at step {step}");
                last = y;
            }
        }
    }

    #[test]
    fn out_of_range_and_nan_inputs_are_pinned() {
        for curve in Curve::ALL {
            assert!(curve.apply(-3.0).abs() < 1e-6, "{curve:?} below");
            assert!((curve.apply(9.0) - 1.0).abs() < 1e-6, "{curve:?} above");
            assert!(curve.apply(f32::NAN).abs() < 1e-6, "{curve:?} NaN");
        }
    }
}
