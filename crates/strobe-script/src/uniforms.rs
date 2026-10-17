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
//! ## Sealing
//!
//! Shaders are compiled with the uniforms known at load, and a show never
//! compiles while it plays (D161). [`UniformStore::seal`] marks that
//! moment: the uniforms committed so far are the declared set, and a
//! `set` of any other name is [`SetOutcome::Refused`] and stores nothing.
//! Declared uniforms keep updating as before.
//!
//! Writes are staged. A frame that faults halfway must not leave half its
//! uniforms behind, so values go to a `pending` slot and only
//! [`UniformStore::commit`] makes them current; [`UniformStore::discard`]
//! throws them away and the previous frame's values stay in force.
//!
//! ## Binding
//!
//! The app uploads the script's uniforms in the order its shaders declared
//! them, which is not the order the script first set them in (a name the
//! scene already declares is left out). [`UniformStore::bind`] resolves that
//! list of names to slots once, when the store is sealed, and
//! [`UniformStore::bound_into`] then reads the values in that order with no
//! name lookups and, into a buffer that has grown to size, no allocation. A
//! bound name the script has not set yet (a restarted script that declares it
//! only later) is resolved when its slot is created, which is the only time
//! binding costs anything after the seal.

use crate::output::ScriptUniform;
use std::collections::{HashMap, HashSet};
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
    /// The store is sealed and this name was not declared before the seal;
    /// nothing was stored.
    Refused,
}

/// Every uniform a script has published.
#[derive(Debug, Default)]
pub(crate) struct UniformStore {
    slots: Vec<Slot>,
    index: HashMap<Arc<str>, usize>,
    /// Slots with a `pending` value, so commit and discard touch only
    /// those rather than scanning every slot.
    dirty: Vec<usize>,
    /// The names declared when the store was sealed; `None` until then.
    declared: Option<HashSet<Arc<str>>>,
    /// For each name given to [`UniformStore::bind`], in that order, its
    /// slot, or `None` while the script has not created it.
    bound: Vec<Option<usize>>,
    /// Bound names still without a slot, each with its place in `bound`.
    unresolved: Vec<(Arc<str>, usize)>,
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
        if self
            .declared
            .as_ref()
            .is_some_and(|declared| !declared.contains(name))
        {
            return SetOutcome::Refused;
        }
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
                self.resolve(&name, position);
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

    /// Points every bound name still waiting for a slot at `position`, the
    /// slot just created for `name`.
    fn resolve(&mut self, name: &Arc<str>, position: usize) {
        if self.unresolved.is_empty() {
            return;
        }
        let bound = &mut self.bound;
        self.unresolved.retain(|(waiting, place)| {
            let matched = waiting == name;
            if matched && let Some(entry) = bound.get_mut(*place) {
                *entry = Some(position);
            }
            !matched
        });
    }

    /// Makes this frame's staged values current.
    pub(crate) fn commit(&mut self) {
        for &position in &self.dirty {
            let Some(slot) = self.slots.get_mut(position) else {
                continue;
            };
            if let Some(value) = slot.pending.take() {
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

    /// Seals the store and lists every committed uniform as
    /// `(name, type)`: those are the declared set from now on.
    pub(crate) fn seal(&mut self) -> Vec<(String, ExtraType)> {
        let committed: Vec<&Slot> = self
            .slots
            .iter()
            .filter(|slot| slot.committed.is_some())
            .collect();
        self.declared = Some(
            committed
                .iter()
                .map(|slot| Arc::clone(&slot.name))
                .collect(),
        );
        committed
            .iter()
            .map(|slot| (slot.name.to_string(), slot.ty))
            .collect()
    }

    /// Seals the store with exactly `names` as the declared set, whatever
    /// has been committed: for a script started again under shaders that
    /// were compiled for an earlier start of it. A name in `names` not set
    /// yet may still be set; any other new name is refused.
    ///
    /// The names are also [bound](Self::bind), in the order given, because
    /// they are the shaders' uniforms in the order the shaders declared them.
    pub(crate) fn seal_to<'a>(&mut self, names: impl IntoIterator<Item = &'a str>) {
        let names: Vec<&str> = names.into_iter().collect();
        self.declared = Some(names.iter().copied().map(Arc::from).collect());
        self.bind(names);
    }

    /// Resolves `names` to slots, once, so that [`Self::bound_into`] can read
    /// their values in this order without looking anything up. Replaces an
    /// earlier binding. A name with no slot yet is resolved when the script
    /// creates one.
    pub(crate) fn bind<'a>(&mut self, names: impl IntoIterator<Item = &'a str>) {
        self.bound.clear();
        self.unresolved.clear();
        for (place, name) in names.into_iter().enumerate() {
            let slot = self.index.get(name).copied();
            if slot.is_none() {
                self.unresolved.push((Arc::from(name), place));
            }
            self.bound.push(slot);
        }
    }

    /// Writes the committed value of each bound name into `out`, in the
    /// order they were bound; a name the script has not committed reads as
    /// zeros. Clears `out` first and reuses its capacity.
    pub(crate) fn bound_into(&self, out: &mut Vec<[f32; 4]>) {
        out.clear();
        out.extend(self.bound.iter().map(|slot| {
            slot.and_then(|position| self.slots.get(position))
                .and_then(|slot| slot.committed)
                .unwrap_or([0.0; 4])
        }));
    }

    /// Every committed uniform, in first-appearance order.
    #[cfg(test)]
    pub(crate) fn snapshot(&self) -> Vec<ScriptUniform> {
        let mut out = Vec::new();
        self.snapshot_into(&mut out);
        out
    }

    /// [`Self::snapshot`] into `out`, which is cleared first and keeps its
    /// capacity, so a frame after the first allocates nothing: a name is
    /// shared, not copied.
    pub(crate) fn snapshot_into(&self, out: &mut Vec<ScriptUniform>) {
        out.clear();
        out.extend(self.slots.iter().filter_map(|slot| {
            slot.committed.map(|value| ScriptUniform {
                name: Arc::clone(&slot.name),
                ty: slot.ty,
                value,
            })
        }));
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
    fn a_sealed_store_refuses_new_names_and_keeps_updating_declared_ones() {
        let mut store = UniformStore::default();
        store.set("a", &[1.0, 2.0, 3.0]);
        store.commit();
        let extras = store.seal();
        assert_eq!(
            extras,
            vec![("a".to_string(), ExtraType::Vec3)],
            "the committed uniform is the declared set"
        );
        assert_eq!(
            store.set("late", &[1.0]),
            SetOutcome::Refused,
            "a name first used after the seal is refused"
        );
        assert_eq!(
            store.set("a", &[4.0, 5.0, 6.0]),
            SetOutcome::Stored,
            "a declared name still updates"
        );
        store.commit();
        let names: Vec<String> = store
            .snapshot()
            .iter()
            .map(|uniform| uniform.name.to_string())
            .collect();
        assert_eq!(names, ["a"], "the refused name never appears");
    }

    #[test]
    fn sealing_to_a_list_declares_exactly_that_list() {
        let mut store = UniformStore::default();
        store.set("mine", &[1.0]);
        store.commit();
        store.seal_to(["compiled"]);
        assert_eq!(
            store.set("compiled", &[2.0, 3.0]),
            SetOutcome::Stored,
            "a listed name not set before the seal is still accepted"
        );
        assert_eq!(
            store.set("mine", &[1.0]),
            SetOutcome::Refused,
            "a committed name the list leaves out is refused"
        );
    }

    #[test]
    fn a_name_staged_but_never_committed_before_the_seal_is_not_declared() {
        let mut store = UniformStore::default();
        store.set("half", &[1.0]);
        store.discard();
        assert!(store.seal().is_empty(), "nothing was committed");
        assert_eq!(
            store.set("half", &[1.0]),
            SetOutcome::Refused,
            "the shaders were compiled without it"
        );
    }

    /// The values `bound_into` reads, as a plain vector.
    fn bound(store: &UniformStore) -> Vec<[f32; 4]> {
        let mut out = Vec::new();
        store.bound_into(&mut out);
        out
    }

    #[test]
    fn bound_values_come_in_the_bound_order_not_the_order_they_were_set_in() {
        let mut store = UniformStore::default();
        store.set("first", &[1.0]);
        store.set("clash", &[9.0]);
        store.set("last", &[3.0, 4.0]);
        store.commit();
        store.seal();
        // The shaders were built without `clash`, and in a different order.
        store.bind(["last", "first"]);
        assert_eq!(
            bound(&store),
            [[3.0, 4.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]],
            "bound order, `clash` left out"
        );
        store.set("first", &[7.0]);
        assert_eq!(
            bound(&store)[1],
            [1.0, 0.0, 0.0, 0.0],
            "a staged value is not current until it is committed"
        );
        store.commit();
        assert_eq!(bound(&store)[1], [7.0, 0.0, 0.0, 0.0], "and then it is");
    }

    #[test]
    fn a_bound_name_the_script_sets_later_is_found_when_its_slot_appears() {
        let mut store = UniformStore::default();
        store.set("other", &[1.0]);
        store.commit();
        // A script started again: `late` is declared by the shaders but this
        // run has not set it yet.
        store.seal_to(["late", "other"]);
        assert_eq!(
            bound(&store),
            [[0.0; 4], [1.0, 0.0, 0.0, 0.0]],
            "an unset name reads as zeros"
        );
        assert_eq!(
            store.set("late", &[5.0, 6.0]),
            SetOutcome::Stored,
            "a declared name may still be created"
        );
        assert_eq!(
            bound(&store)[0],
            [0.0; 4],
            "staged, so still zero until the frame commits"
        );
        store.commit();
        assert_eq!(
            bound(&store),
            [[5.0, 6.0, 0.0, 0.0], [1.0, 0.0, 0.0, 0.0]],
            "found without any lookup by name"
        );
        assert!(
            store.unresolved.is_empty(),
            "nothing is left waiting: {:?}",
            store.unresolved
        );
    }

    #[test]
    fn a_discarded_first_write_leaves_a_bound_name_at_zero() {
        let mut store = UniformStore::default();
        store.seal_to(["a"]);
        store.set("a", &[2.0]);
        store.discard();
        assert_eq!(
            bound(&store),
            [[0.0; 4]],
            "a faulted frame published nothing"
        );
        store.set("a", &[3.0]);
        store.commit();
        assert_eq!(
            bound(&store),
            [[3.0, 0.0, 0.0, 0.0]],
            "the next good frame did"
        );
    }

    #[test]
    fn steady_state_reads_reuse_their_buffers() {
        let mut store = UniformStore::default();
        for name in ["a", "b", "c"] {
            store.set(name, &[1.0, 2.0]);
        }
        store.commit();
        store.seal();
        store.bind(["c", "a", "b"]);
        let mut uniforms = Vec::new();
        let mut values = Vec::new();
        store.snapshot_into(&mut uniforms);
        store.bound_into(&mut values);
        let (uniforms_at, uniforms_capacity) = (uniforms.as_ptr(), uniforms.capacity());
        let (values_at, values_capacity) = (values.as_ptr(), values.capacity());
        for frame in 0..100_u16 {
            for name in ["a", "b", "c"] {
                store.set(name, &[f32::from(frame), 2.0]);
            }
            store.commit();
            store.snapshot_into(&mut uniforms);
            store.bound_into(&mut values);
            assert_eq!(uniforms.len(), 3, "frame {frame}: every uniform listed");
            assert_eq!(values.len(), 3, "frame {frame}: every bound value read");
        }
        assert_eq!(
            (uniforms.as_ptr(), uniforms.capacity()),
            (uniforms_at, uniforms_capacity),
            "the uniform list was refilled in place"
        );
        assert_eq!(
            (values.as_ptr(), values.capacity()),
            (values_at, values_capacity),
            "the bound values were refilled in place"
        );
    }
}
