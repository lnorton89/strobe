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
    /// Adds a macro and returns its id.
    pub fn add(&mut self, mut new: Macro) -> u32 {
        self.repair();
        self.next_id += 1;
        new.id = self.next_id;
        self.macros.push(new);
        self.next_id
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
    /// hand may have left zeros or duplicates.
    pub fn repair(&mut self) {
        let mut seen = std::collections::HashSet::new();
        let mut next = self
            .next_id
            .max(self.macros.iter().map(|m| m.id).max().unwrap_or(0));
        for m in &mut self.macros {
            if m.id == 0 || !seen.insert(m.id) {
                next += 1;
                m.id = next;
                seen.insert(next);
            }
        }
        self.next_id = next;
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
