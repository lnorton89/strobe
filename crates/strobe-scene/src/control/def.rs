// ============================================================================
// Module:       strobe_scene::control::def
// Description:  A validated control: typed, named, with its range and default
//               resolved to one component per uniform dimension.
//
// Dependencies: crate::json (raw input), crate::control::kind, crate::error
// ============================================================================

//! Control definitions.
//!
//! Real scenes are looser than the documentation, and this is where that
//! is absorbed rather than rejected:
//!
//! - `MIN`, `MAX` and `DEFAULT` are often missing. The range defaults to
//!   `0..1` and the default to the range's low end clamped into it.
//! - `MIN` is sometimes greater than `MAX`, on purpose: the slider runs
//!   backwards. That is kept as written; the normalised position `t` maps
//!   to `MIN + t * (MAX - MIN)` either way.
//! - A single number where the control has several dimensions applies to
//!   all of them.
//!
//! What *is* rejected is ambiguity a show cannot recover from: an array
//! with the wrong number of components, a dropdown with no options, a name
//! that is not a GLSL identifier.

use crate::control::kind::ControlKind;
use crate::error::SceneError;
use crate::json::{Components, RawControl};

/// The smoothing amount when `PARAMS` is absent: ~100 frames to settle.
pub const DEFAULT_PARAMS: f32 = 0.01;

/// Most components any control has (a colour's three).
pub const MAX_DIMENSIONS: usize = 3;

/// A control's per-component values, `dims` of which are meaningful.
pub type Values = [f32; MAX_DIMENSIONS];

/// One validated control.
#[derive(Debug, Clone, PartialEq)]
pub struct ControlDef {
    /// The uniform name, exactly as declared.
    pub name: String,
    /// Base type and modifiers.
    pub kind: ControlKind,
    /// Value at normalised position 0, per component.
    pub min: Values,
    /// Value at normalised position 1, per component.
    pub max: Values,
    /// The starting value, per component (for a dropdown: the index).
    pub default: Values,
    /// Smoothing amount, `(0, 1]`.
    pub params: f32,
    /// UI group, if any.
    pub group: Option<String>,
    /// Tooltip text.
    pub description: Option<String>,
    /// Dropdown option labels.
    pub labels: Vec<String>,
    /// Dropdown option values.
    pub values: Vec<f32>,
}

impl ControlDef {
    /// Validates one raw control.
    pub fn from_raw(raw: &RawControl) -> Result<Self, SceneError> {
        let name = raw.name.trim().to_string();
        if !crate::names::is_glsl_identifier(&name) {
            return Err(SceneError::InvalidName(raw.name.clone()));
        }
        if crate::names::is_reserved(&name) {
            return Err(SceneError::ReservedName(name));
        }
        let kind = ControlKind::parse(&name, &raw.ty)?;
        let dims = kind.dimensions();

        let (labels, values) = if matches!(kind, ControlKind::Dropdown { .. }) {
            dropdown_options(&name, raw)?
        } else {
            (Vec::new(), Vec::new())
        };

        let (min, max) = if matches!(kind, ControlKind::Dropdown { .. }) {
            // A dropdown's range is its option indices.
            let last = values.len().saturating_sub(1) as f32;
            ([0.0; MAX_DIMENSIONS], [last; MAX_DIMENSIONS])
        } else {
            (
                components(&name, "MIN", raw.min.as_ref(), dims, 0.0)?,
                components(&name, "MAX", raw.max.as_ref(), dims, 1.0)?,
            )
        };

        let default_source = match kind {
            ControlKind::Color { .. } => raw.def_color.as_ref().or(raw.default.as_ref()),
            ControlKind::Slider(_)
            | ControlKind::Knob(_)
            | ControlKind::Toggle { .. }
            | ControlKind::Bang(_)
            | ControlKind::Xy(_)
            | ControlKind::Dropdown { .. } => raw.default.as_ref(),
        };
        let fallback = components(&name, "MIN", None, dims, 0.0)?;
        let mut default = match default_source {
            Some(source) => components(&name, "DEFAULT", Some(source), dims, 0.0)?,
            None => fallback,
        };
        for i in 0..dims {
            let (lo, hi) = ordered(min[i], max[i]);
            default[i] = default[i].clamp(lo, hi);
        }
        if let ControlKind::Dropdown { .. } = kind {
            default[0] = default[0].round();
        }

        let params = raw.params.unwrap_or(DEFAULT_PARAMS);
        let params = if params.is_finite() && params > 0.0 {
            params.min(1.0)
        } else {
            DEFAULT_PARAMS
        };

        Ok(Self {
            name,
            kind,
            min,
            max,
            default,
            params,
            group: raw
                .ui_group
                .clone()
                .filter(|group| !group.trim().is_empty()),
            description: raw.description.clone(),
            labels,
            values,
        })
    }

    /// How many components are meaningful.
    pub fn dims(&self) -> usize {
        self.kind.dimensions()
    }

    /// The raw value at normalised position `t` for component `i`.
    pub fn denormalize(&self, i: usize, t: f32) -> f32 {
        let (Some(min), Some(max)) = (self.min.get(i), self.max.get(i)) else {
            return 0.0;
        };
        min + t * (max - min)
    }

    /// The normalised position of raw `value` for component `i`.
    pub fn normalize(&self, i: usize, value: f32) -> f32 {
        let (Some(min), Some(max)) = (self.min.get(i), self.max.get(i)) else {
            return 0.0;
        };
        let span = max - min;
        if span.abs() < f32::EPSILON {
            0.0
        } else {
            ((value - min) / span).clamp(0.0, 1.0)
        }
    }

    /// Clamps raw `value` into component `i`'s range, whichever way it runs.
    pub fn clamp(&self, i: usize, value: f32) -> f32 {
        let (Some(min), Some(max)) = (self.min.get(i), self.max.get(i)) else {
            return value;
        };
        let (lo, hi) = ordered(*min, *max);
        if value.is_nan() {
            lo
        } else {
            value.clamp(lo, hi)
        }
    }
}

/// `(lower, upper)` of two bounds written in either order.
fn ordered(a: f32, b: f32) -> (f32, f32) {
    if a <= b { (a, b) } else { (b, a) }
}

/// One `MIN` / `MAX` / `DEFAULT` value spread over `dims` components.
fn components(
    name: &str,
    key: &'static str,
    source: Option<&Components>,
    dims: usize,
    fallback: f32,
) -> Result<Values, SceneError> {
    let mut out = [fallback; MAX_DIMENSIONS];
    match source {
        None => {}
        Some(Components::One(value)) => out = [*value; MAX_DIMENSIONS],
        Some(Components::Many(values)) if values.len() == 1 => {
            out = [values.first().copied().unwrap_or(fallback); MAX_DIMENSIONS];
        }
        // A colour written with alpha: the fourth component is dropped,
        // since the uniform is a vec3.
        Some(Components::Many(values)) if values.len() == dims || values.len() == dims + 1 => {
            for (slot, value) in out.iter_mut().zip(values.iter()).take(dims) {
                *slot = *value;
            }
        }
        Some(Components::Many(values)) => {
            return Err(SceneError::Arity {
                name: name.to_string(),
                key,
                expected: dims,
                found: values.len(),
            });
        }
    }
    for slot in &mut out {
        if !slot.is_finite() {
            *slot = fallback;
        }
    }
    Ok(out)
}

/// A dropdown's labels and values, each filled in from the other.
fn dropdown_options(name: &str, raw: &RawControl) -> Result<(Vec<String>, Vec<f32>), SceneError> {
    match (&raw.labels, &raw.values) {
        (Some(labels), Some(values)) if labels.len() == values.len() && !labels.is_empty() => {
            Ok((labels.clone(), values.clone()))
        }
        (Some(labels), None) if !labels.is_empty() => Ok((
            labels.clone(),
            (0..labels.len()).map(|i| i as f32).collect(),
        )),
        (None, Some(values)) if !values.is_empty() => Ok((
            (1..=values.len()).map(|i| format!("option {i}")).collect(),
            values.clone(),
        )),
        (Some(_), Some(_)) | (Some(_), None) | (None, Some(_)) | (None, None) => {
            Err(SceneError::Dropdown(name.to_string()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    fn control(json: &str) -> Result<ControlDef> {
        let raw: RawControl = serde_json::from_str(json)?;
        Ok(ControlDef::from_raw(&raw)?)
    }

    #[test]
    fn a_bare_control_gets_a_unit_range_and_zero_default() -> Result<()> {
        let def = control(r#"{ "NAME": "amount", "TYPE": "slider" }"#)?;
        assert_eq!(
            (def.min[0], def.max[0], def.default[0]),
            (0.0, 1.0, 0.0),
            "defaults"
        );
        assert_eq!(def.params, DEFAULT_PARAMS, "default smoothing");
        Ok(())
    }

    #[test]
    fn an_inverted_range_is_kept_and_still_clamps() -> Result<()> {
        // Taken from a real scene: the slider runs from 0.99 down to 0.
        let def = control(
            r#"{ "NAME": "motion_blur", "TYPE": "slider", "MIN": 0.99, "MAX": 0.0, "DEFAULT": 0.5 }"#,
        )?;
        assert_eq!(def.min[0], 0.99, "MIN kept as written");
        assert_eq!(def.denormalize(0, 1.0), 0.0, "t = 1 reaches MAX");
        assert_eq!(
            def.clamp(0, 2.0),
            0.99,
            "clamps to the upper bound of either order"
        );
        assert_eq!(def.default[0], 0.5, "an in-range default is untouched");
        Ok(())
    }

    #[test]
    fn a_colour_reads_def_color_and_drops_alpha() -> Result<()> {
        let def =
            control(r#"{ "NAME": "tint", "TYPE": "color", "DEF_COLOR": [0.8, 0.1, 0.1, 1.0] }"#)?;
        assert_eq!(
            def.default,
            [0.8, 0.1, 0.1],
            "DEF_COLOR is the colour default"
        );
        Ok(())
    }

    #[test]
    fn a_scalar_applies_to_every_dimension() -> Result<()> {
        let def = control(r#"{ "NAME": "pos", "TYPE": "xy", "MIN": -1, "MAX": 1, "DEFAULT": 0 }"#)?;
        assert_eq!(
            (def.min[1], def.max[1]),
            (-1.0, 1.0),
            "second dimension shares the range"
        );
        Ok(())
    }

    #[test]
    fn a_wrong_arity_is_an_error_naming_the_key() -> Result<()> {
        let raw: RawControl =
            serde_json::from_str(r#"{ "NAME": "pos", "TYPE": "xy", "DEFAULT": [1, 2, 3, 4, 5] }"#)?;
        let error = ControlDef::from_raw(&raw).err().map(|e| e.to_string());
        assert!(
            error.as_deref().is_some_and(|e| e.contains("DEFAULT")),
            "got {error:?}"
        );
        Ok(())
    }

    #[test]
    fn dropdown_options_fill_in_from_either_side() -> Result<()> {
        let labels = control(r#"{ "NAME": "m", "TYPE": "dropdown", "LABELS": ["a", "b"] }"#)?;
        assert_eq!(labels.values, vec![0.0, 1.0], "values default to indices");
        let values = control(r#"{ "NAME": "m", "TYPE": "dropdown", "VALUES": [0.5, 2] }"#)?;
        assert_eq!(
            values.labels,
            vec!["option 1", "option 2"],
            "labels default to option N"
        );
        let none: RawControl = serde_json::from_str(r#"{ "NAME": "m", "TYPE": "dropdown" }"#)?;
        assert!(
            ControlDef::from_raw(&none).is_err(),
            "a dropdown with no options"
        );
        Ok(())
    }

    #[test]
    fn names_must_be_identifiers_and_not_built_ins() -> Result<()> {
        for name in ["rate in", "TIME", "syn_BassLevel"] {
            let raw: RawControl =
                serde_json::from_str(&format!(r#"{{ "NAME": "{name}", "TYPE": "slider" }}"#))?;
            assert!(
                ControlDef::from_raw(&raw).is_err(),
                "`{name}` should be rejected"
            );
        }
        Ok(())
    }
}
