// ============================================================================
// Module:       strobe_scene::control::kind
// Description:  The control TYPE string, tokenised: seven base types and the
//               smooth / speed / counter modifiers each one accepts.
//
// Dependencies: crate::error
// ============================================================================

//! Control types.
//!
//! A `TYPE` is a base word followed by modifiers: `"slider"`, `"slider
//! smooth"`, `"bang counter smooth"`. The tokenizer is strict about the
//! words — an unknown one is an error naming the control, not a slider by
//! default — and strict about combinations (`toggle speed` means nothing).
//! It is lenient about spacing and case, because those carry no meaning.

use crate::error::SceneError;

/// How a continuous control's output follows its input.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Output is the input.
    Plain,
    /// Output eases toward the input.
    Smooth,
    /// Output integrates the input: the control sets a rate per second.
    Speed,
}

/// What a bang does on a press.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Bang {
    /// `MAX` for one frame, then `MIN`.
    Plain,
    /// A brief rise-and-fall envelope.
    Smooth,
    /// Counts presses.
    Counter,
    /// Counts presses, easing between counts.
    CounterSmooth,
}

/// A control's base type and modifiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlKind {
    /// A float slider.
    Slider(Mode),
    /// A float knob; behaves as a slider.
    Knob(Mode),
    /// Switches between `MIN` and `MAX`.
    Toggle {
        /// Ease between the two.
        smooth: bool,
    },
    /// A momentary trigger.
    Bang(Bang),
    /// A `vec2` pad.
    Xy(Mode),
    /// A `vec3` RGB colour.
    Color {
        /// Ease between colours.
        smooth: bool,
    },
    /// A choice from a list of labelled values.
    Dropdown {
        /// Ease between values.
        smooth: bool,
    },
}

impl ControlKind {
    /// Parses a `TYPE` string for the control called `name`.
    pub fn parse(name: &str, ty: &str) -> Result<Self, SceneError> {
        let lower = ty.to_ascii_lowercase();
        let mut words = lower.split_whitespace();
        let unknown = || SceneError::UnknownControlType {
            name: name.to_string(),
            ty: ty.to_string(),
        };
        let base = words.next().ok_or_else(unknown)?;
        let mut smooth = false;
        let mut speed = false;
        let mut counter = false;
        for word in words {
            match word {
                "smooth" => smooth = true,
                "speed" => speed = true,
                "counter" => counter = true,
                _ => return Err(unknown()),
            }
        }
        let invalid = |modifier: &str| SceneError::InvalidModifier {
            name: name.to_string(),
            base: base.to_string(),
            modifier: modifier.to_string(),
        };
        let mode = || match (smooth, speed) {
            (true, true) => Err(invalid("speed")),
            (false, true) => Ok(Mode::Speed),
            (true, false) => Ok(Mode::Smooth),
            (false, false) => Ok(Mode::Plain),
        };

        if counter && base != "bang" {
            return Err(invalid("counter"));
        }
        let no_speed = |kind: Self| {
            if speed {
                Err(invalid("speed"))
            } else {
                Ok(kind)
            }
        };
        match base {
            "slider" => Ok(Self::Slider(mode()?)),
            "knob" => Ok(Self::Knob(mode()?)),
            "xy" => Ok(Self::Xy(mode()?)),
            "toggle" => no_speed(Self::Toggle { smooth }),
            "color" => no_speed(Self::Color { smooth }),
            "dropdown" => no_speed(Self::Dropdown { smooth }),
            "bang" => no_speed(Self::Bang(match (counter, smooth) {
                (false, false) => Bang::Plain,
                (false, true) => Bang::Smooth,
                (true, false) => Bang::Counter,
                (true, true) => Bang::CounterSmooth,
            })),
            _ => Err(unknown()),
        }
    }

    /// How many float components the control's uniform has.
    pub fn dimensions(self) -> usize {
        match self {
            Self::Xy(_) => 2,
            Self::Color { .. } => 3,
            Self::Slider(_)
            | Self::Knob(_)
            | Self::Toggle { .. }
            | Self::Bang(_)
            | Self::Dropdown { .. } => 1,
        }
    }

    /// The GLSL type of the control's uniform.
    pub fn glsl_type(self) -> &'static str {
        match self.dimensions() {
            2 => "vec2",
            3 => "vec3",
            _ => "float",
        }
    }

    /// Whether the output eases toward its target.
    pub fn is_smooth(self) -> bool {
        match self {
            Self::Slider(mode) | Self::Knob(mode) | Self::Xy(mode) => mode == Mode::Smooth,
            Self::Toggle { smooth } | Self::Color { smooth } | Self::Dropdown { smooth } => smooth,
            Self::Bang(bang) => matches!(bang, Bang::Smooth | Bang::CounterSmooth),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn every_documented_type_parses() -> Result<()> {
        let cases = [
            ("slider", ControlKind::Slider(Mode::Plain)),
            ("slider smooth", ControlKind::Slider(Mode::Smooth)),
            ("slider speed", ControlKind::Slider(Mode::Speed)),
            ("knob", ControlKind::Knob(Mode::Plain)),
            ("knob smooth", ControlKind::Knob(Mode::Smooth)),
            ("knob speed", ControlKind::Knob(Mode::Speed)),
            ("toggle", ControlKind::Toggle { smooth: false }),
            ("toggle smooth", ControlKind::Toggle { smooth: true }),
            ("bang", ControlKind::Bang(Bang::Plain)),
            ("bang smooth", ControlKind::Bang(Bang::Smooth)),
            ("bang counter", ControlKind::Bang(Bang::Counter)),
            (
                "bang counter smooth",
                ControlKind::Bang(Bang::CounterSmooth),
            ),
            ("xy", ControlKind::Xy(Mode::Plain)),
            ("xy smooth", ControlKind::Xy(Mode::Smooth)),
            ("xy speed", ControlKind::Xy(Mode::Speed)),
            ("color", ControlKind::Color { smooth: false }),
            ("color smooth", ControlKind::Color { smooth: true }),
            ("dropdown", ControlKind::Dropdown { smooth: false }),
            ("dropdown smooth", ControlKind::Dropdown { smooth: true }),
        ];
        for (ty, expected) in cases {
            assert_eq!(ControlKind::parse("c", ty)?, expected, "`{ty}`");
        }
        Ok(())
    }

    #[test]
    fn case_and_spacing_carry_no_meaning() -> Result<()> {
        assert_eq!(
            ControlKind::parse("c", "  Slider   SMOOTH ")?,
            ControlKind::Slider(Mode::Smooth),
            "case and extra spaces are tolerated"
        );
        Ok(())
    }

    #[test]
    fn unknown_words_and_meaningless_combinations_are_rejected() {
        for ty in [
            "",
            "slidr",
            "slider fast",
            "toggle speed",
            "slider counter",
            "xy smooth speed",
        ] {
            assert!(
                ControlKind::parse("c", ty).is_err(),
                "`{ty}` should not parse"
            );
        }
    }

    #[test]
    fn uniform_types_follow_dimensions() -> Result<()> {
        assert_eq!(ControlKind::parse("c", "xy")?.glsl_type(), "vec2", "xy");
        assert_eq!(
            ControlKind::parse("c", "color")?.glsl_type(),
            "vec3",
            "color"
        );
        assert_eq!(
            ControlKind::parse("c", "dropdown")?.glsl_type(),
            "float",
            "dropdown"
        );
        Ok(())
    }
}
