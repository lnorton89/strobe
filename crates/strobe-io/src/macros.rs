// ============================================================================
// Module:       strobe_io::macros
// Description:  Performance Macros: the operator's curated, persistent set of
//               bindings, pinned and ordered by hand, that survive a scene
//               change by going unavailable instead of disappearing.
//
// Dependencies: crate::target; serde
// ============================================================================

//! Performance Macros, stage 2 (UI_DESIGN section 8, D175).
//!
//! A macro is not a copy of a control. It is a *stable operator binding*:
//! a name, a [`Target`] (resolved against whatever scene is playing), a pin
//! and a place in the order. The set is global to the show and saved with
//! the rest of show control. Changing scene never deletes or reorders a
//! macro; one whose control is missing from the new scene is simply
//! unavailable until a scene that has it plays.
//!
//! A macro can also be a mapping's target ([`Target::Macro`]), which is how
//! a MIDI fader or an OSC address drives "the thing in macro slot 2" without
//! caring what it is today. The macro's optional `range` narrows what the
//! mapping may reach, and `aliases` are other names to try for a control
//! (so one macro can be "speed" in one scene and "rate" in the next).
//!
//! Ids are never reused, so a mapping that names a deleted macro stays
//! broken instead of silently driving a new one.

use crate::target::Target;
use serde::{Deserialize, Serialize};

/// How many identity colours a macro can take (indices `0..COLOURS` into
/// the theme's series ramp; the app draws them).
pub const COLOURS: u8 = 8;

/// The longest glide a macro may have, in seconds (D227).
pub const MAX_GLIDE: f32 = 10.0;

/// The narrowest reach a macro's range may have, as a fraction: a range
/// that collapses to a point would make every fader a constant.
pub const MIN_REACH: f32 = 0.01;

/// One Performance Macro.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Macro {
    /// Its id, unique within the set and never reused.
    #[serde(default)]
    pub id: u32,
    /// What the card says.
    pub name: String,
    /// What it acts on (never another macro).
    pub target: Target,
    /// Other names to try for a scene control, in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub aliases: Vec<String>,
    /// Shown in the strip under the preview.
    #[serde(default = "yes")]
    pub pinned: bool,
    /// Switched on. Off: shown muted, and ignores mappings.
    #[serde(default = "yes")]
    pub enabled: bool,
    /// The part of the control's range a mapping may reach, as fractions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<[f32; 2]>,
    /// An identity colour, as an index into the theme's accents.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<u8>,
    /// How long, in seconds, the control takes to follow the macro (D227):
    /// a time constant, so a jumpy fader or a key still moves the picture
    /// smoothly. `None` writes straight through, as the control's own
    /// smoothing decides.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glide: Option<f32>,
    /// A keyboard key that drives it while held (D228), as the app writes
    /// a chord (`Shift+F5`, `Q`). Down is the top of its reach, up the
    /// bottom.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
}

fn yes() -> bool {
    true
}

impl Macro {
    /// A pinned, enabled macro with no id yet (the set gives it one).
    pub fn new(name: &str, target: Target) -> Self {
        Self {
            id: 0,
            name: name.to_string(),
            target,
            aliases: Vec::new(),
            pinned: true,
            enabled: true,
            range: None,
            color: None,
            glide: None,
            key: None,
        }
    }
}

/// The ordered set of macros.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MacroSet {
    macros: Vec<Macro>,
    next_id: u32,
}

impl MacroSet {
    /// Adds a macro and returns its id. Ids are never reused (see the module
    /// docs), except after a hand-edited counter left none to hand out
    /// (D376). With no id free at all (it would take four
    /// billion macros) nothing is added and the result is 0, which no macro
    /// has.
    pub fn add(&mut self, mut new: Macro) -> u32 {
        self.repair();
        let Some(id) = self.fresh_id() else {
            return 0;
        };
        new.id = id;
        self.macros.push(new);
        id
    }

    /// An id no macro in the set has, and the counter moved past it where it
    /// can be.
    ///
    /// Normally the counter plus one. A file edited by hand can leave the
    /// counter at `u32::MAX`, where that is an overflow: the counter then
    /// picks up again past the highest id in the set, which is what
    /// [`Self::repair`] promises of it. Only a set that holds `u32::MAX`
    /// itself has no id past its highest, and gets the smallest free one.
    fn fresh_id(&mut self) -> Option<u32> {
        if let Some(next) = self.next_id.checked_add(1) {
            self.next_id = next;
            return Some(next);
        }
        let highest = self.macros.iter().map(|m| m.id).max().unwrap_or(0);
        if let Some(next) = highest.checked_add(1) {
            self.next_id = next;
            return Some(next);
        }
        let taken: std::collections::HashSet<u32> = self.macros.iter().map(|m| m.id).collect();
        (1..=u32::MAX).find(|id| !taken.contains(id))
    }

    /// Adds a macro for `target` unless the set already has one for it
    /// (pinning twice is not two cards). Returns the macro's id.
    pub fn pin(&mut self, name: &str, target: Target) -> u32 {
        if let Some(existing) = self.macros.iter_mut().find(|m| m.target == target) {
            existing.pinned = true;
            return existing.id;
        }
        self.add(Macro::new(name, target))
    }

    /// The macro with this id.
    pub fn get(&self, id: u32) -> Option<&Macro> {
        self.macros.iter().find(|m| m.id == id)
    }

    fn get_mut(&mut self, id: u32) -> Option<&mut Macro> {
        self.macros.iter_mut().find(|m| m.id == id)
    }

    /// Every macro, in order.
    pub fn all(&self) -> &[Macro] {
        &self.macros
    }

    /// The pinned macros, in order.
    pub fn pinned(&self) -> impl Iterator<Item = &Macro> {
        self.macros.iter().filter(|m| m.pinned)
    }

    /// How many macros there are.
    pub fn len(&self) -> usize {
        self.macros.len()
    }

    /// Whether there are none.
    pub fn is_empty(&self) -> bool {
        self.macros.is_empty()
    }

    /// Deletes a macro. Its id is not reused.
    pub fn remove(&mut self, id: u32) {
        self.macros.retain(|m| m.id != id);
    }

    /// Moves a macro `delta` places (negative is earlier), staying within
    /// the set. Returns whether it moved.
    pub fn move_by(&mut self, id: u32, delta: isize) -> bool {
        let Some(from) = self.macros.iter().position(|m| m.id == id) else {
            return false;
        };
        let to = from
            .saturating_add_signed(delta)
            .min(self.macros.len().saturating_sub(1));
        if to == from {
            return false;
        }
        let moved = self.macros.remove(from);
        self.macros.insert(to, moved);
        true
    }

    /// Pins or unpins.
    pub fn set_pinned(&mut self, id: u32, pinned: bool) {
        if let Some(m) = self.get_mut(id) {
            m.pinned = pinned;
        }
    }

    /// Switches a macro on or off.
    pub fn set_enabled(&mut self, id: u32, enabled: bool) {
        if let Some(m) = self.get_mut(id) {
            m.enabled = enabled;
        }
    }

    /// Renames a macro.
    pub fn rename(&mut self, id: u32, name: &str) {
        if let Some(m) = self.get_mut(id) {
            m.name = name.to_string();
        }
    }

    /// Sets the names tried when the macro's own is not in a scene.
    pub fn set_aliases(&mut self, id: u32, aliases: Vec<String>) {
        if let Some(m) = self.get_mut(id) {
            m.aliases = aliases;
        }
    }

    /// Sets the part of the control's range a mapping may reach, as
    /// fractions (D210). The ends are clamped to `0..1` and put in order,
    /// and kept at least [`MIN_REACH`] apart; the full range is stored as
    /// `None`, so the file holds only real changes.
    pub fn set_range(&mut self, id: u32, range: Option<[f32; 2]>) {
        let range = range.and_then(|[a, b]| {
            let (a, b) = (a.clamp(0.0, 1.0), b.clamp(0.0, 1.0));
            if !(a.is_finite() && b.is_finite()) {
                return None;
            }
            let (mut low, mut high) = if a <= b { (a, b) } else { (b, a) };
            if high - low < MIN_REACH {
                high = (low + MIN_REACH).min(1.0);
                low = high - MIN_REACH;
            }
            let full = low <= f32::EPSILON && high >= 1.0 - f32::EPSILON;
            (!full).then_some([low, high])
        });
        if let Some(m) = self.get_mut(id) {
            m.range = range;
        }
    }

    /// Sets or clears macro `id`'s glide, in seconds; zero, negative and
    /// non-finite clear it, and it is capped at [`MAX_GLIDE`].
    pub fn set_glide(&mut self, id: u32, glide: Option<f32>) {
        let glide = glide
            .filter(|seconds| seconds.is_finite() && *seconds > 0.0)
            .map(|seconds| seconds.min(MAX_GLIDE));
        if let Some(m) = self.get_mut(id) {
            m.glide = glide;
        }
    }

    /// Sets or clears macro `id`'s key. A key another macro holds is taken
    /// from it, so one key never drives two macros.
    pub fn set_key(&mut self, id: u32, key: Option<String>) {
        let key = key.map(|k| k.trim().to_string()).filter(|k| !k.is_empty());
        if let Some(wanted) = key.as_deref() {
            for other in self.macros.iter_mut().filter(|m| m.id != id) {
                if other
                    .key
                    .as_deref()
                    .is_some_and(|k| k.eq_ignore_ascii_case(wanted))
                {
                    other.key = None;
                }
            }
        }
        if let Some(m) = self.get_mut(id) {
            m.key = key;
        }
    }

    /// Sets the identity colour, an index below [`COLOURS`]; anything
    /// else is no colour.
    pub fn set_color(&mut self, id: u32, color: Option<u8>) {
        if let Some(m) = self.get_mut(id) {
            m.color = color.filter(|c| *c < COLOURS);
        }
    }

    /// The macro that drives `target`, if there is one.
    pub fn for_target(&self, target: &Target) -> Option<&Macro> {
        self.macros.iter().find(|m| m.target == *target)
    }

    /// Points a macro at a different target. A macro cannot point at a
    /// macro; that is refused (returns false).
    pub fn retarget(&mut self, id: u32, target: Target) -> bool {
        if matches!(target, Target::Macro { .. }) {
            return false;
        }
        match self.get_mut(id) {
            Some(m) => {
                m.target = target;
                true
            }
            None => false,
        }
    }

    /// Makes ids unique and `next_id` past all of them: a file edited by
    /// hand may have left zeros, duplicates or a counter at the top of its
    /// range.
    pub fn repair(&mut self) {
        let mut seen = std::collections::HashSet::new();
        let mut stale = Vec::new();
        for (index, m) in self.macros.iter().enumerate() {
            if m.id == 0 || !seen.insert(m.id) {
                stale.push(index);
            }
        }
        let highest = self.macros.iter().map(|m| m.id).max().unwrap_or(0);
        self.next_id = self.next_id.max(highest);
        for index in stale {
            // None only when every id is taken, which cannot happen.
            let Some(id) = self.fresh_id() else {
                break;
            };
            if let Some(m) = self.macros.get_mut(index) {
                m.id = id;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    fn set() -> MacroSet {
        let mut set = MacroSet::default();
        set.add(Macro::new("A", Target::control("a")));
        set.add(Macro::new("B", Target::control("b")));
        set.add(Macro::new("C", Target::control("c")));
        set
    }

    fn names(set: &MacroSet) -> Vec<&str> {
        set.all().iter().map(|m| m.name.as_str()).collect()
    }

    #[test]
    fn ids_are_assigned_in_order_and_never_reused() {
        let mut set = set();
        let ids: Vec<u32> = set.all().iter().map(|m| m.id).collect();
        assert_eq!(ids, vec![1, 2, 3], "ids");
        set.remove(3);
        let next = set.add(Macro::new("D", Target::control("d")));
        assert_eq!(next, 4, "3 is not handed out again");
    }

    #[test]
    fn reordering_moves_one_macro_and_stops_at_the_ends() {
        let mut set = set();
        assert!(set.move_by(3, -1), "C up");
        assert_eq!(names(&set), vec!["A", "C", "B"], "after C up");
        assert!(set.move_by(1, 5), "A far down clamps to the end");
        assert_eq!(names(&set), vec!["C", "B", "A"], "after A down");
        assert!(!set.move_by(1, 1), "already last");
        assert!(
            !set.move_by(2, -9) || names(&set)[0] == "B",
            "clamped to the top"
        );
        assert!(!set.move_by(99, 1), "unknown id");
    }

    #[test]
    fn pinning_a_target_twice_is_one_card() {
        let mut set = set();
        set.set_pinned(1, false);
        let again = set.pin("Another A", Target::control("a"));
        assert_eq!(again, 1, "the existing macro");
        assert_eq!(set.len(), 3, "no new card");
        assert!(
            set.get(1).is_some_and(|m| m.pinned),
            "and it is pinned again"
        );
        let fresh = set.pin("D", Target::control("d"));
        assert_eq!(fresh, 4, "a new target is a new macro");
    }

    #[test]
    fn unpinned_macros_stay_in_the_set_but_leave_the_strip() {
        let mut set = set();
        set.set_pinned(2, false);
        let pinned: Vec<&str> = set.pinned().map(|m| m.name.as_str()).collect();
        assert_eq!(pinned, vec!["A", "C"], "strip");
        assert_eq!(set.len(), 3, "set");
    }

    #[test]
    fn a_macro_cannot_be_retargeted_at_a_macro() {
        let mut set = set();
        assert!(!set.retarget(1, Target::Macro { id: 2 }), "refused");
        assert!(set.retarget(1, Target::meta("hue")), "meta is fine");
        assert_eq!(
            set.get(1).map(|m| m.target.clone()),
            Some(Target::meta("hue")),
            "retargeted"
        );
    }

    #[test]
    fn a_hand_edited_file_with_zero_or_duplicate_ids_is_repaired() -> Result<()> {
        let text = r#"{ "macros": [
            { "name": "A", "target": { "kind": "control", "name": "a" } },
            { "id": 5, "name": "B", "target": { "kind": "control", "name": "b" } },
            { "id": 5, "name": "C", "target": { "kind": "control", "name": "c" } }
        ] }"#;
        let mut set: MacroSet = serde_json::from_str(text)?;
        set.repair();
        let mut ids: Vec<u32> = set.all().iter().map(|m| m.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), 3, "unique: {ids:?}");
        assert!(ids.iter().all(|id| *id > 0), "non-zero: {ids:?}");
        assert!(
            set.all().iter().all(|m| m.pinned && m.enabled),
            "defaults are on"
        );
        let next = set.add(Macro::new("D", Target::control("d")));
        assert!(!ids.contains(&next), "a fresh id");
        Ok(())
    }

    fn ids(set: &MacroSet) -> Vec<u32> {
        set.all().iter().map(|m| m.id).collect()
    }

    #[test]
    fn a_counter_at_the_top_of_its_range_picks_up_past_the_existing_ids() -> Result<()> {
        // A hand-edited file: `next_id` is u32::MAX, so `+ 1` would overflow
        // (a panic in a debug build, a wrap to 0 in a release one).
        let text = r#"{ "macros": [
            { "id": 1, "name": "A", "target": { "kind": "control", "name": "a" } },
            { "id": 7, "name": "B", "target": { "kind": "control", "name": "b" } }
        ], "next_id": 4294967295 }"#;
        let mut set: MacroSet = serde_json::from_str(text)?;
        assert_eq!(
            set.add(Macro::new("C", Target::control("c"))),
            8,
            "past the highest id in the set"
        );
        assert_eq!(
            set.add(Macro::new("D", Target::control("d"))),
            9,
            "and the counter works normally again"
        );
        assert_eq!(ids(&set), vec![1, 7, 8, 9], "all unique");
        Ok(())
    }

    #[test]
    fn a_macro_holding_the_highest_possible_id_leaves_the_smallest_free_one() {
        let mut set = set();
        set.next_id = u32::MAX;
        if let Some(last) = set.macros.last_mut() {
            last.id = u32::MAX;
        }
        // ids are now 1, 2, MAX.
        assert_eq!(set.add(Macro::new("D", Target::control("d"))), 3, "the gap");
        assert_eq!(
            set.add(Macro::new("E", Target::control("e"))),
            4,
            "the next"
        );
        let mut all = ids(&set);
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), 5, "no id twice: {all:?}");
    }

    #[test]
    fn repairing_duplicates_with_the_counter_at_the_top_does_not_overflow() -> Result<()> {
        let text = r#"{ "macros": [
            { "id": 5, "name": "A", "target": { "kind": "control", "name": "a" } },
            { "id": 5, "name": "B", "target": { "kind": "control", "name": "b" } },
            { "id": 0, "name": "C", "target": { "kind": "control", "name": "c" } }
        ], "next_id": 4294967295 }"#;
        let mut set: MacroSet = serde_json::from_str(text)?;
        set.repair();
        let mut all = ids(&set);
        all.sort_unstable();
        all.dedup();
        assert_eq!(all.len(), 3, "unique: {all:?}");
        assert!(all.iter().all(|id| *id > 0), "non-zero: {all:?}");
        Ok(())
    }

    #[test]
    fn a_range_is_ordered_clamped_kept_open_and_full_is_none() -> Result<()> {
        let mut set = MacroSet::default();
        let id = set.add(Macro::new("Speed", Target::control("speed")));
        let range = |set: &MacroSet| set.get(id).and_then(|m| m.range);
        set.set_range(id, Some([0.8, 0.2]));
        assert_eq!(range(&set), Some([0.2, 0.8]), "put in order");
        set.set_range(id, Some([-1.0, 0.5]));
        assert_eq!(range(&set), Some([0.0, 0.5]), "clamped");
        set.set_range(id, Some([0.5, 0.5]));
        let kept = range(&set).ok_or_else(|| anyhow::anyhow!("a range"))?;
        assert!(
            kept[1] - kept[0] >= MIN_REACH - 1e-6,
            "never a point: {kept:?}"
        );
        set.set_range(id, Some([1.0, 1.0]));
        let top = range(&set).ok_or_else(|| anyhow::anyhow!("a range"))?;
        assert!(
            top[1] <= 1.0 && top[1] - top[0] >= MIN_REACH - 1e-6,
            "at the top: {top:?}"
        );
        set.set_range(id, Some([0.0, 1.0]));
        assert_eq!(range(&set), None, "the full range is no range");
        set.set_color(id, Some(3));
        assert_eq!(set.get(id).and_then(|m| m.color), Some(3), "a colour");
        set.set_color(id, Some(COLOURS));
        assert_eq!(
            set.get(id).and_then(|m| m.color),
            None,
            "out of the palette is none"
        );
        assert_eq!(
            set.for_target(&Target::control("speed")).map(|m| m.id),
            Some(id),
            "found by target"
        );
        Ok(())
    }

    #[test]
    fn the_set_round_trips_through_json() -> Result<()> {
        let mut set = set();
        set.set_aliases(2, vec!["bee".into()]);
        set.set_enabled(3, false);
        let text = serde_json::to_string(&set)?;
        assert_eq!(serde_json::from_str::<MacroSet>(&text)?, set, "round trip");
        Ok(())
    }
}
