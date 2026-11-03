// ============================================================================
// Module:       strobe_scene::control::def
// Description:  A validated control: typed, named, with its range and default
//               resolved to one component per uniform dimension.
//
// Dependencies: crate::json (raw input), crate::control::kind, crate::error;
//               log (a dropdown kept despite a mismatch); serde_json (written
//               back as a scene.json entry)
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
//! - A dropdown whose `LABELS` and `VALUES` differ in length keeps the
//!   options both lists give, with a warning (D412).
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
        // Real Synesthesia scenes have controls with only NAME, MIN, MAX and
        // DEFAULT (7 in 65 on TORNADO, D193). With no TYPE, the default's
        // components pick the one type with that many: two make an xy pad,
        // three a colour (the only vec3), four a colour with alpha (dropped,
        // as for a typed colour), and anything else a slider (D412, Q32).
        let ty = if raw.ty.trim().is_empty() {
            match raw.default.as_ref().map(|d| d.to_vec().len()) {
                Some(2) => "xy",
                Some(3 | 4) => "color",
                Some(_) | None => "slider",
            }
        } else {
            raw.ty.as_str()
        };
        let kind = ControlKind::parse(&name, ty)?;
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

    /// This control as a `scene.json` entry that [`Self::from_raw`] reads
    /// back as an equal control. The desktop remote sends a host's controls
    /// this way, so the controlling Strobe builds the same bank and draws
    /// it itself (D513).
    pub fn to_json(&self) -> serde_json::Value {
        let dims = self.dims();
        let components = |values: &Values| match values.get(..dims) {
            Some([one]) => serde_json::json!(one),
            Some(many) => serde_json::json!(many),
            None => serde_json::Value::Null,
        };
        let mut out = serde_json::Map::new();
        out.insert("NAME".into(), self.name.clone().into());
        out.insert("TYPE".into(), self.kind.type_text().into());
        if let ControlKind::Dropdown { .. } = self.kind {
            // A dropdown's range is its option indices, made again from
            // the options.
            out.insert("LABELS".into(), serde_json::json!(self.labels));
            out.insert("VALUES".into(), serde_json::json!(self.values));
        } else {
            out.insert("MIN".into(), components(&self.min));
            out.insert("MAX".into(), components(&self.max));
        }
        out.insert("DEFAULT".into(), components(&self.default));
        out.insert("PARAMS".into(), serde_json::json!(self.params));
        if let Some(group) = &self.group {
            out.insert("UI_GROUP".into(), group.clone().into());
        }
        if let Some(description) = &self.description {
            out.insert("DESCRIPTION".into(), description.clone().into());
        }
        serde_json::Value::Object(out)
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

/// A dropdown's labels and values, each filled in from the other. An empty
/// list counts as absent.
///
/// When both are given with different lengths, the options are the ones
/// both lists give: label `i` with value `i`, as far as the shorter list
/// goes. Neither a value nor a label is invented, the scene keeps playing,
/// and the mismatch is logged (D412). What Synesthesia does with such a
/// control is not known (Q32).
fn dropdown_options(name: &str, raw: &RawControl) -> Result<(Vec<String>, Vec<f32>), SceneError> {
    let labels = raw.labels.as_ref().filter(|labels| !labels.is_empty());
    let values = raw.values.as_ref().filter(|values| !values.is_empty());
    match (labels, values) {
        (Some(labels), Some(values)) => {
            if labels.len() != values.len() {
                log::warn!(
                    "dropdown `{name}`: {} LABELS but {} VALUES; keeping the first {} of each",
                    labels.len(),
                    values.len(),
                    labels.len().min(values.len())
                );
            }
            Ok(labels
                .iter()
                .zip(values)
                .map(|(label, value)| (label.clone(), *value))
                .unzip())
        }
        (Some(labels), None) => Ok((
            labels.clone(),
            (0..labels.len()).map(|i| i as f32).collect(),
        )),
        (None, Some(values)) => Ok((
            (1..=values.len()).map(|i| format!("option {i}")).collect(),
            values.clone(),
        )),
        (None, None) => Err(SceneError::Dropdown(name.to_string())),
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
    fn a_dropdown_whose_labels_and_values_differ_keeps_the_options_both_give() -> Result<()> {
        // One list longer than the other used to fail the whole scene.
        let def = control(
            r#"{ "NAME": "m", "TYPE": "dropdown", "LABELS": ["a", "b", "c"], "VALUES": [5, 7] }"#,
        )?;
        assert_eq!(def.labels, vec!["a", "b"], "labels paired with a value");
        assert_eq!(def.values, vec![5.0, 7.0], "every value kept");
        assert_eq!(def.max[0], 1.0, "the range is the two options' indices");
        let def = control(
            r#"{ "NAME": "m", "TYPE": "dropdown", "LABELS": ["a"], "VALUES": [5, 7, 9] }"#,
        )?;
        assert_eq!(
            (def.labels.len(), def.values.len()),
            (1, 1),
            "the shorter list decides"
        );
        let def = control(r#"{ "NAME": "m", "TYPE": "dropdown", "LABELS": [], "VALUES": [5] }"#)?;
        assert_eq!(def.labels, vec!["option 1"], "an empty list is no list");
        Ok(())
    }

    #[test]
    fn an_untyped_control_with_three_or_four_default_components_is_a_colour() -> Result<()> {
        // Two components make an xy pad (D193); three are a vec3, which only
        // a colour is, and four a colour with alpha. A slider failed them.
        for default in ["[0.2, 0.4, 0.6]", "[0.2, 0.4, 0.6, 1.0]"] {
            let def = control(&format!(r#"{{ "NAME": "tint", "DEFAULT": {default} }}"#))?;
            assert!(
                matches!(def.kind, ControlKind::Color { .. }),
                "{default}: {:?}",
                def.kind
            );
            assert_eq!(def.default, [0.2, 0.4, 0.6], "{default}: the default kept");
        }
        let def = control(r#"{ "NAME": "amount", "DEFAULT": 0.5 }"#)?;
        assert!(
            matches!(def.kind, ControlKind::Slider(_)),
            "one component is still a slider"
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

    fn round_trip(def: &ControlDef) -> Result<ControlDef> {
        let raw: RawControl = serde_json::from_value(def.to_json())?;
        Ok(ControlDef::from_raw(&raw)?)
    }

    #[test]
    fn every_kind_is_written_back_as_an_equal_control() -> Result<()> {
        let sources = [
            r#"{ "NAME": "a", "TYPE": "slider", "MIN": -2, "MAX": 5, "DEFAULT": 1.5, "UI_GROUP": "g", "DESCRIPTION": "d" }"#,
            r#"{ "NAME": "b", "TYPE": "knob traveler", "MIN": 1, "MAX": 0, "PARAMS": 0.3 }"#,
            r#"{ "NAME": "c", "TYPE": "slider speed" }"#,
            r#"{ "NAME": "d", "TYPE": "toggle smooth", "DEFAULT": 1 }"#,
            r#"{ "NAME": "e", "TYPE": "bang counter smooth", "MAX": 4 }"#,
            r#"{ "NAME": "f", "TYPE": "xy smooth", "MIN": [-1, 0], "MAX": [1, 2], "DEFAULT": [0.5, 1] }"#,
            r#"{ "NAME": "g", "TYPE": "color", "DEFAULT": [0.2, 0.4, 0.6, 1.0] }"#,
            r#"{ "NAME": "h", "TYPE": "dropdown smooth", "LABELS": ["x", "y", "z"], "VALUES": [0.5, 2, 9], "DEFAULT": 2 }"#,
            r#"{ "NAME": "i", "DEFAULT": [0.1, 0.9] }"#,
        ];
        for source in sources {
            let def = control(source)?;
            assert_eq!(round_trip(&def)?, def, "{source}");
        }
        Ok(())
    }

    #[test]
    fn every_control_in_the_library_and_the_meta_bank_is_written_back_equal() -> Result<()> {
        let scenes = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scenes");
        let mut checked = 0;
        for entry in std::fs::read_dir(&scenes)? {
            let json = entry?.path().join("scene.json");
            if !json.is_file() {
                continue;
            }
            let scene: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&json)?)?;
            let controls = scene
                .get("CONTROLS")
                .and_then(serde_json::Value::as_array)
                .cloned()
                .unwrap_or_default();
            for raw in controls {
                let def = ControlDef::from_raw(&serde_json::from_value(raw)?)?;
                assert_eq!(round_trip(&def)?, def, "{}: {}", json.display(), def.name);
                checked += 1;
            }
        }
        for def in crate::meta::controls()? {
            assert_eq!(round_trip(&def)?, def, "meta: {}", def.name);
            checked += 1;
        }
        assert!(checked > 50, "the library has controls to check: {checked}");
        Ok(())
    }
}
