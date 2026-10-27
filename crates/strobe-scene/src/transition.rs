// ============================================================================
// Module:       strobe_scene::transition
// Description:  The live values of a scene's HARD_TRANSITIONS and
//               SMOOTH_TRANSITIONS uniforms: a new look on each song change.
//
// Dependencies: crate::scene
// ============================================================================

//! What a song change does to a scene (D489).
//!
//! SSF gives a scene two ways to change its look when the music does:
//!
//! - **`HARD_TRANSITIONS`**: each listed uniform jumps to a value chosen at
//!   random from its `VALUES`, all of them at once. A value listed twice is
//!   twice as likely.
//! - **`SMOOTH_TRANSITIONS`**: each family of uniforms has exactly one at 1
//!   and the rest at 0. A song change picks another member at random and
//!   crossfades to it, linearly, over the family's `DURATION` seconds.
//!
//! [`Transitions`] holds those values for one loaded scene. The app calls
//! [`Transitions::fire`] when it hears a song change (or the operator asks
//! for one) and [`Transitions::step`] once a frame; [`Transitions::values`]
//! is in [`SceneDef::transition_uniforms`] order, ready for the renderer.
//!
//! **A change always changes something.** A hard transition never picks the
//! entry it is on, and a family never crossfades to the member that is
//! already on, so a song change is never invisible by chance. SSF says
//! "randomly chooses a new value"; whether it may repeat is Q33.
//!
//! **A crossfade that is interrupted carries on from where it is.** Every
//! member moves toward its target (1 for the new member, 0 for the rest) at
//! `1 / DURATION` per second, so an uninterrupted change is exactly the
//! documented linear crossfade, and a second change mid-fade turns the
//! picture around without a jump.
//!
//! Before the first change a scene shows its first value and its first
//! family member, as it did before transitions ran. Nothing here
//! allocates after [`Transitions::new`].

use crate::scene::SceneDef;

/// One hard transition's state.
#[derive(Debug, Clone)]
struct Hard {
    /// The uniform's index in the values.
    slot: usize,
    /// The values it chooses between, never empty.
    values: Vec<f32>,
    /// The entry it shows now.
    current: usize,
}

/// One smooth family's state.
#[derive(Debug, Clone, Copy)]
struct Smooth {
    /// The first member's index in the values.
    first: usize,
    /// How many members, at least one.
    len: usize,
    /// Seconds a full crossfade takes; `0` cuts.
    duration: f32,
    /// The member that is on, or fading in.
    on: usize,
}

/// The live transition uniforms of one scene.
#[derive(Debug, Clone)]
pub struct Transitions {
    values: Vec<f32>,
    hard: Vec<Hard>,
    smooth: Vec<Smooth>,
    /// xorshift64* state, never zero.
    rng: u64,
}

impl Transitions {
    /// The scene's transitions at their starting values. `seed` picks the
    /// sequence of looks; two scenes loaded with the same seed change alike.
    pub fn new(def: &SceneDef, seed: u64) -> Self {
        let values: Vec<f32> = def.transition_uniforms.iter().map(|t| t.initial).collect();
        let hard = def
            .hard_transitions
            .iter()
            .filter(|h| h.slot < values.len() && !h.values.is_empty())
            .map(|h| Hard {
                slot: h.slot,
                values: h.values.clone(),
                current: 0,
            })
            .collect();
        let smooth = def
            .smooth_transitions
            .iter()
            .filter(|s| s.len > 0 && s.first + s.len <= values.len())
            .map(|s| Smooth {
                first: s.first,
                len: s.len,
                duration: s.duration,
                on: 0,
            })
            .collect();
        Self {
            values,
            hard,
            smooth,
            // SplitMix64's increment keeps a zero seed off xorshift's one
            // fixed point.
            rng: (seed ^ 0x9e37_79b9_7f4a_7c15) | 1,
        }
    }

    /// The values, in [`SceneDef::transition_uniforms`] order.
    pub fn values(&self) -> &[f32] {
        &self.values
    }

    /// Whether a song change can change anything: some hard transition has
    /// two entries, or some family two members.
    pub fn can_change(&self) -> bool {
        self.hard.iter().any(|h| h.values.len() > 1) || self.smooth.iter().any(|s| s.len > 1)
    }

    /// A song change: every hard transition jumps to another entry and every
    /// family starts crossfading to another member.
    pub fn fire(&mut self) {
        for index in 0..self.hard.len() {
            let len = self.hard[index].values.len();
            let current = self.hard[index].current;
            let next = self.other_than(current, len);
            let hard = &mut self.hard[index];
            hard.current = next;
            if let (Some(value), Some(slot)) = (
                hard.values.get(next).copied(),
                self.values.get_mut(hard.slot),
            ) {
                *slot = value;
            }
        }
        for index in 0..self.smooth.len() {
            let family = self.smooth[index];
            self.smooth[index].on = self.other_than(family.on, family.len);
        }
        // A cut lands now, not on the next step.
        self.step(0.0);
    }

    /// Moves every crossfade on by `dt` seconds.
    pub fn step(&mut self, dt: f32) {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        for family in &self.smooth {
            let Some(members) = self.values.get_mut(family.first..family.first + family.len) else {
                continue;
            };
            let reach = if family.duration > 0.0 {
                dt / family.duration
            } else {
                1.0
            };
            for (i, value) in members.iter_mut().enumerate() {
                let target = if i == family.on { 1.0 } else { 0.0 };
                *value += (target - *value).clamp(-reach, reach);
            }
        }
    }

    /// A random index in `0..len` other than `current`; `current` itself
    /// when there is no other.
    fn other_than(&mut self, current: usize, len: usize) -> usize {
        if len < 2 {
            return current.min(len.saturating_sub(1));
        }
        // Skip over `current`: a uniform draw from the other `len - 1`.
        let draw = (self.next_u64() % (len as u64 - 1)) as usize;
        if draw >= current { draw + 1 } else { draw }
    }

    /// xorshift64*.
    fn next_u64(&mut self) -> u64 {
        let mut x = self.rng;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.rng = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    fn scene(json: &str) -> Result<SceneDef> {
        Ok(SceneDef::parse(json, "s")?)
    }

    const GLITCH: &str = r#"{
        "HARD_TRANSITIONS": [
            {"UNIFORM": "sizeTri", "VALUES": [90.0, 30.0, 200.0]},
            {"UNIFORM": "style", "VALUES": [2.0, 1.0, 0.0]}
        ],
        "SMOOTH_TRANSITIONS": [ {"UNIFORMS": ["c0", "c1", "c2", "c3"], "DURATION": 10} ]
    }"#;

    #[test]
    fn before_a_change_a_scene_shows_its_first_values() -> Result<()> {
        let transitions = Transitions::new(&scene(GLITCH)?, 7);
        assert_eq!(
            transitions.values(),
            &[90.0, 2.0, 1.0, 0.0, 0.0, 0.0],
            "first values and the first member on"
        );
        Ok(())
    }

    #[test]
    fn a_hard_transition_always_jumps_to_another_listed_value() -> Result<()> {
        let mut transitions = Transitions::new(&scene(GLITCH)?, 1);
        let mut seen = Vec::new();
        for _ in 0..200 {
            let before = transitions.values()[0];
            transitions.fire();
            let after = transitions.values()[0];
            assert_ne!(before, after, "a change never stays put");
            assert!(
                [90.0, 30.0, 200.0].contains(&after),
                "{after} is one of the listed values"
            );
            if !seen.contains(&after) {
                seen.push(after);
            }
        }
        assert_eq!(seen.len(), 3, "every value comes up: {seen:?}");
        Ok(())
    }

    #[test]
    fn a_value_listed_twice_still_changes_entry() -> Result<()> {
        let def = scene(r#"{ "HARD_TRANSITIONS": [ {"UNIFORM": "d", "VALUES": [1, 1, -1]} ] }"#)?;
        let mut transitions = Transitions::new(&def, 3);
        let mut ones = 0;
        for _ in 0..300 {
            transitions.fire();
            if transitions.values()[0] == 1.0 {
                ones += 1;
            }
        }
        // From entry 0 or 1 the draw is between the other two (one `1`, one
        // `-1`); from entry 2 it is between the two `1`s. Both values come up.
        assert!(
            (100..300).contains(&ones),
            "the doubled value is likelier but not certain: {ones} of 300"
        );
        Ok(())
    }

    #[test]
    fn a_family_crossfades_linearly_over_its_duration() -> Result<()> {
        let mut transitions = Transitions::new(&scene(GLITCH)?, 5);
        transitions.fire();
        let family = &transitions.values()[2..6];
        assert_eq!(family, &[1.0, 0.0, 0.0, 0.0], "nothing moves before a step");
        transitions.step(5.0);
        let half = &transitions.values()[2..6];
        assert!((half[0] - 0.5).abs() < 1e-6, "halfway out: {half:?}");
        let rising: Vec<f32> = half[1..].iter().copied().filter(|&v| v > 0.0).collect();
        assert_eq!(rising.len(), 1, "one new member fades in: {half:?}");
        assert!((rising[0] - 0.5).abs() < 1e-6, "halfway in: {half:?}");
        transitions.step(5.0);
        let done = &transitions.values()[2..6];
        assert_eq!(done[0], 0.0, "the old member is off: {done:?}");
        assert_eq!(
            done.iter().filter(|&&v| v == 1.0).count(),
            1,
            "exactly one member is on: {done:?}"
        );
        assert_eq!(done.iter().sum::<f32>(), 1.0, "one-hot: {done:?}");
        Ok(())
    }

    #[test]
    fn a_change_mid_fade_turns_around_without_a_jump() -> Result<()> {
        let def =
            scene(r#"{ "SMOOTH_TRANSITIONS": [ {"UNIFORMS": ["a", "b"], "DURATION": 2} ] }"#)?;
        let mut transitions = Transitions::new(&def, 9);
        transitions.fire();
        transitions.step(0.5);
        assert_eq!(
            transitions.values(),
            &[0.75, 0.25],
            "a quarter of the way to b"
        );
        transitions.fire();
        assert_eq!(
            transitions.values(),
            &[0.75, 0.25],
            "no jump at the second change"
        );
        transitions.step(0.5);
        assert_eq!(
            transitions.values(),
            &[1.0, 0.0],
            "back to a, at the same rate"
        );
        Ok(())
    }

    #[test]
    fn a_zero_duration_cuts_and_a_missing_one_takes_three_seconds() -> Result<()> {
        let def = scene(
            r#"{ "SMOOTH_TRANSITIONS": [
                {"UNIFORMS": ["a", "b"], "DURATION": 0},
                {"UNIFORMS": ["c", "d"]}
            ] }"#,
        )?;
        let mut transitions = Transitions::new(&def, 2);
        transitions.fire();
        assert_eq!(
            &transitions.values()[..2],
            &[0.0, 1.0],
            "a cut lands at the change"
        );
        transitions.step(1.5);
        assert_eq!(
            &transitions.values()[2..],
            &[0.5, 0.5],
            "halfway at 1.5 s of 3"
        );
        Ok(())
    }

    #[test]
    fn a_scene_with_nothing_to_choose_does_not_change() -> Result<()> {
        let def = scene(
            r#"{ "HARD_TRANSITIONS": [ {"UNIFORM": "a", "VALUES": [4]}, {"UNIFORM": "b", "VALUES": []} ],
                 "SMOOTH_TRANSITIONS": [ {"UNIFORMS": ["c"]} ] }"#,
        )?;
        let mut transitions = Transitions::new(&def, 4);
        assert!(!transitions.can_change(), "one value and one member each");
        transitions.fire();
        transitions.step(10.0);
        assert_eq!(transitions.values(), &[4.0, 0.0, 1.0], "unchanged");
        assert!(
            Transitions::new(&scene(GLITCH)?, 4).can_change(),
            "glitch has choices"
        );
        Ok(())
    }

    #[test]
    fn the_same_seed_gives_the_same_looks() -> Result<()> {
        let def = scene(GLITCH)?;
        let mut a = Transitions::new(&def, 11);
        let mut b = Transitions::new(&def, 11);
        for _ in 0..20 {
            a.fire();
            b.fire();
            a.step(1.0);
            b.step(1.0);
            assert_eq!(a.values(), b.values(), "same seed, same sequence");
        }
        Ok(())
    }
}
