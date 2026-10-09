// ============================================================================
// Module:       strobe_script::uniforms
// Description:  The uniforms a script publishes: arity-inferred types,
//               per-frame staging, and commit-or-discard on fault.
//
// Dependencies: strobe_scene::assemble (ExtraType), crate::output
// ============================================================================

//! The script's output uniforms.
//!
//! `setUniform("name", …)` declares a uniform by using it: the GLSL type is
//! the arity of the first call (one number is a `float`, four a `vec4`).
//! The type is then *fixed*. A later call with a different arity is padded
//! or truncated to it rather than changing the type, because a type change
//! would mean recompiling the shader, and a script that alternates between
//! two arities would recompile every frame.
//!
//! Writes are staged. A frame that faults halfway must not leave half its
//! uniforms behind, so values go to a `pending` slot and only
//! [`UniformStore::commit`] makes them current; [`UniformStore::discard`]
//! throws them away and the previous frame's values stay in force.

use crate::output::ScriptUniform;
use std::collections::HashMap;
use std::sync::Arc;
use strobe_scene::assemble::ExtraType;

/// One uniform.
#[derive(Debug)]
struct Slot {
    name: Arc<str>,
    ty: ExtraType,
    /// The last good value. `None` until the first successful commit.
    committed: Option<[f32; 4]>,
    /// Written this frame, not yet committed.
    pending: Option<[f32; 4]>,
}

/// What [`UniformStore::set`] did with a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SetOutcome {
    /// Stored as given.
    Stored,
    /// The uniform already has this type and the value had a different
    /// number of components, so it was padded with zeros or truncated.
    Coerced(ExtraType),
}

/// Every uniform a script has published.
#[derive(Debug, Default)]
pub(crate) struct UniformStore {
    slots: Vec<Slot>,
    index: HashMap<Arc<str>, usize>,
    /// Slots with a `pending` value, so commit and discard touch only
    /// those rather than scanning every slot.
    dirty: Vec<usize>,
    /// A slot committed for the first time since the last report.
    fresh: bool,
}

/// How many floats a type holds.
fn width(ty: ExtraType) -> usize {
    match ty {
        ExtraType::Float => 1,
        ExtraType::Vec2 => 2,
        ExtraType::Vec3 => 3,
        ExtraType::Vec4 => 4,
    }
}

/// The type `count` components make: 1 is a float, 2 a vec2, and so on.
fn type_for(count: usize) -> ExtraType {
    match count {
        0 | 1 => ExtraType::Float,
        2 => ExtraType::Vec2,
        3 => ExtraType::Vec3,
        _ => ExtraType::Vec4,
    }
}

impl UniformStore {
    /// Stages `components` (1 to 4 floats) as `name`'s value this frame,
    /// declaring the uniform if it is new.
    pub(crate) fn set(&mut self, name: &str, components: &[f32]) -> SetOutcome {
        let position = match self.index.get(name) {
            Some(&position) => position,
            None => {
                let name: Arc<str> = Arc::from(name);
                let position = self.slots.len();
                self.slots.push(Slot {
                    name: Arc::clone(&name),
                    ty: type_for(components.len()),
                    committed: None,
                    pending: None,
                });
                self.index.insert(name, position);
                position
            }
        };
        let Some(slot) = self.slots.get_mut(position) else {
            return SetOutcome::Stored;
        };
        let slot_width = width(slot.ty);
        let mut value = [0.0_f32; 4];
        for (target, source) in value.iter_mut().take(slot_width).zip(components) {
            *target = *source;
        }
        if slot.pending.is_none() {
            self.dirty.push(position);
        }
        slot.pending = Some(value);
        if components.len() == slot_width {
            SetOutcome::Stored
        } else {
            SetOutcome::Coerced(slot.ty)
        }
    }

    /// Makes this frame's staged values current.
    pub(crate) fn commit(&mut self) {
        for &position in &self.dirty {
            let Some(slot) = self.slots.get_mut(position) else {
                continue;
            };
            if let Some(value) = slot.pending.take() {
                if slot.committed.is_none() {
                    self.fresh = true;
                }
                slot.committed = Some(value);
            }
        }
        self.dirty.clear();
    }

    /// Throws this frame's staged values away; the last good ones stay.
    pub(crate) fn discard(&mut self) {
        for &position in &self.dirty {
            if let Some(slot) = self.slots.get_mut(position) {
                slot.pending = None;
            }
        }
        self.dirty.clear();
    }

    /// Whether a uniform was first committed since the last call; clears
    /// the flag.
    pub(crate) fn take_fresh(&mut self) -> bool {
        std::mem::take(&mut self.fresh)
    }

    /// Forgets that anything is fresh, and lists every committed uniform
    /// as `(name, type)`.
    pub(crate) fn announce(&mut self) -> Vec<(String, ExtraType)> {
        self.fresh = false;
        self.slots
            .iter()
            .filter(|slot| slot.committed.is_some())
            .map(|slot| (slot.name.to_string(), slot.ty))
            .collect()
    }

    /// Every committed uniform, in first-appearance order.
    pub(crate) fn snapshot(&self) -> Vec<ScriptUniform> {
        self.slots
            .iter()
            .filter_map(|slot| {
                slot.committed.map(|value| ScriptUniform {
                    name: Arc::clone(&slot.name),
                    ty: slot.ty,
                    value,
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn the_first_call_fixes_the_type() {
        let mut store = UniformStore::default();
        assert_eq!(
            store.set("a", &[1.0, 2.0]),
            SetOutcome::Stored,
            "a new vec2 is stored as given"
        );
        assert_eq!(
            store.set("a", &[5.0]),
            SetOutcome::Coerced(ExtraType::Vec2),
            "a float written to a vec2 is padded"
        );
        store.commit();
        assert_eq!(
            store.snapshot().first().map(|u| (u.ty, u.value)),
            Some((ExtraType::Vec2, [5.0, 0.0, 0.0, 0.0])),
            "the type stayed vec2"
        );
    }

    #[test]
    fn discard_keeps_the_last_good_value() -> Result<()> {
        let mut store = UniformStore::default();
        store.set("a", &[1.0]);
        store.commit();
        store.set("a", &[2.0]);
        store.discard();
        let snapshot = store.snapshot();
        let first = snapshot
            .first()
            .ok_or_else(|| anyhow::anyhow!("no uniform"))?;
        assert_eq!(first.value[0], 1.0, "the discarded write left no trace");
        Ok(())
    }

    #[test]
    fn a_uniform_is_fresh_once() {
        let mut store = UniformStore::default();
        store.set("a", &[1.0]);
        assert!(!store.take_fresh(), "staged is not yet known");
        store.commit();
        assert!(store.take_fresh(), "the first commit is fresh");
        store.set("a", &[2.0]);
        store.commit();
        assert!(!store.take_fresh(), "a later commit is not");
    }

    #[test]
    fn announcing_clears_the_fresh_flag() {
        let mut store = UniformStore::default();
        store.set("a", &[1.0, 2.0, 3.0]);
        store.commit();
        let extras = store.announce();
        assert_eq!(
            extras,
            vec![("a".to_string(), ExtraType::Vec3)],
            "the committed uniform is listed"
        );
        assert!(!store.take_fresh(), "and no longer fresh");
    }
}
