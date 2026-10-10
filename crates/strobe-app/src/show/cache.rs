// ============================================================================
// Module:       strobe_app::show::cache
// Description:  The precompile cache: one slot per distinct scene in a
//               playlist, filled one scene at a time, with progress.
//
// Dependencies: strobe-playlist (SceneRef)
// ============================================================================

//! The precompiled-scene cache.
//!
//! A playlist promises that no shader is compiled while it plays (PLAN
//! section 7.4), so every distinct scene is compiled when the playlist
//! loads. [`SceneCache`] is the bookkeeping for that: a slot per scene, in
//! first-use order, each [`Slot::Pending`] until it is compiled and then
//! either holding the compiled thing or the reason it could not be made.
//!
//! The cache is generic over what it holds. The app puts a parked
//! [`Session`](crate::session::Session) in each slot (compiled programs,
//! images and script, with 1x1 render targets, D151), and the tests put
//! numbers.
//!
//! ## Filling it without a hitch
//!
//! Compiling happens on the thread that owns the GL context, so it cannot
//! run beside the frame loop. It is spread out instead: the app asks
//! [`SceneCache::next_pending`] for one scene per control-window frame,
//! compiles that, and calls [`SceneCache::fill`]. A frame is blocked for one
//! scene's compile, never for all of them, and [`SceneCache::progress`]
//! drives the progress readout between frames.
//!
//! ## Lending a slot
//!
//! A scene that is playing, warming or fading out is *out* of the cache
//! ([`SceneCache::take`] leaves [`Slot::Out`]) and goes back with
//! [`SceneCache::put_back`] when it is parked. A scene is in exactly one
//! place, so there is never a second copy of one scene's targets.

use std::collections::HashMap;
use strobe_playlist::SceneRef;

/// One scene's place in the cache.
#[derive(Debug)]
pub enum Slot<T> {
    /// Not compiled yet.
    Pending,
    /// Compiled and parked here.
    Ready(T),
    /// Compiled, and currently playing, warming or fading out elsewhere.
    Out,
    /// Could not be made; why, in one line.
    Failed(String),
}

/// How far a precompile has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Progress {
    /// Scenes finished, successfully or not.
    pub done: usize,
    /// Scenes in the playlist.
    pub total: usize,
    /// Of those finished, how many failed.
    pub failed: usize,
}

impl Progress {
    /// Whether every scene has been tried.
    pub fn complete(self) -> bool {
        self.done >= self.total
    }
}

/// A slot per distinct scene, in the order they are compiled.
#[derive(Debug)]
pub struct SceneCache<T> {
    scenes: Vec<SceneRef>,
    slots: Vec<Slot<T>>,
    index: HashMap<SceneRef, usize>,
}

impl<T> SceneCache<T> {
    /// A cache with a pending slot for each of `scenes` (duplicates are
    /// ignored; the first position wins).
    pub fn new(scenes: Vec<SceneRef>) -> Self {
        let mut unique = Vec::with_capacity(scenes.len());
        let mut index = HashMap::with_capacity(scenes.len());
        for scene in scenes {
            if !index.contains_key(&scene) {
                index.insert(scene.clone(), unique.len());
                unique.push(scene);
            }
        }
        let slots = unique.iter().map(|_| Slot::Pending).collect();
        Self {
            scenes: unique,
            slots,
            index,
        }
    }

    /// Scenes in the cache.
    pub fn len(&self) -> usize {
        self.scenes.len()
    }

    /// Whether there are no scenes at all.
    pub fn is_empty(&self) -> bool {
        self.scenes.is_empty()
    }

    /// The scene in slot `slot`.
    pub fn scene(&self, slot: usize) -> Option<&SceneRef> {
        self.scenes.get(slot)
    }

    /// The first slot still pending, in compile order.
    pub fn next_pending(&self) -> Option<usize> {
        self.slots
            .iter()
            .position(|slot| matches!(slot, Slot::Pending))
    }

    /// Records the result of compiling slot `slot`. A slot that is not
    /// pending is left alone (a result can only arrive once).
    pub fn fill(&mut self, slot: usize, result: Result<T, String>) {
        if let Some(entry) = self.slots.get_mut(slot)
            && matches!(entry, Slot::Pending)
        {
            *entry = match result {
                Ok(value) => Slot::Ready(value),
                Err(reason) => Slot::Failed(reason),
            };
        }
    }

    /// How far the precompile has got.
    pub fn progress(&self) -> Progress {
        let pending = self
            .slots
            .iter()
            .filter(|slot| matches!(slot, Slot::Pending))
            .count();
        let failed = self
            .slots
            .iter()
            .filter(|slot| matches!(slot, Slot::Failed(_)))
            .count();
        Progress {
            done: self.slots.len() - pending,
            total: self.slots.len(),
            failed,
        }
    }

    /// Takes the compiled value for `scene` out of its slot, leaving
    /// [`Slot::Out`]. `None` if the scene is unknown, failed, not compiled
    /// yet, or already out.
    pub fn take(&mut self, scene: &SceneRef) -> Option<T> {
        let slot = self.slots.get_mut(*self.index.get(scene)?)?;
        match std::mem::replace(slot, Slot::Out) {
            Slot::Ready(value) => Some(value),
            other @ (Slot::Pending | Slot::Out | Slot::Failed(_)) => {
                *slot = other;
                None
            }
        }
    }

    /// Returns a value taken with [`Self::take`]. A scene the cache does
    /// not know, or whose slot is not out, is dropped with a log line
    /// rather than overwriting what is there.
    pub fn put_back(&mut self, scene: &SceneRef, value: T) {
        let slot = self
            .index
            .get(scene)
            .and_then(|index| self.slots.get_mut(*index));
        match slot {
            Some(slot @ Slot::Out) => *slot = Slot::Ready(value),
            Some(Slot::Pending | Slot::Ready(_) | Slot::Failed(_)) | None => {
                log::warn!("{scene} was returned to the scene cache, which has no room for it");
            }
        }
    }

    /// Why `scene` could not be made, if it could not.
    pub fn failure(&self, scene: &SceneRef) -> Option<&str> {
        match self.index.get(scene).and_then(|i| self.slots.get(*i))? {
            Slot::Failed(reason) => Some(reason),
            Slot::Pending | Slot::Ready(_) | Slot::Out => None,
        }
    }

    /// Whether `scene` is compiled and parked (not out, not failed).
    pub fn is_parked(&self, scene: &SceneRef) -> bool {
        matches!(
            self.index.get(scene).and_then(|i| self.slots.get(*i)),
            Some(Slot::Ready(_))
        )
    }

    /// Moves what `old` knows about the scenes this cache shares with it:
    /// compiled scenes stay compiled, failed ones stay failed, and a scene
    /// that is out being played stays out. Scenes only this cache has are
    /// left pending. A reloaded playlist uses this to compile only what is
    /// new (D158).
    pub fn adopt_from(&mut self, old: &mut Self) {
        for (scene, slot) in self.scenes.iter().zip(self.slots.iter_mut()) {
            let Some(old_slot) = old.index.get(scene).and_then(|i| old.slots.get_mut(*i)) else {
                continue;
            };
            *slot = std::mem::replace(old_slot, Slot::Pending);
        }
    }

    /// Drops every parked value (the context they belong to is gone) and
    /// makes every slot pending again, so the precompile can run anew.
    pub fn reset(&mut self) {
        for slot in &mut self.slots {
            *slot = Slot::Pending;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, anyhow};

    fn scene(name: &str) -> SceneRef {
        SceneRef::Library(name.to_string())
    }

    fn cache(names: &[&str]) -> SceneCache<u32> {
        SceneCache::new(names.iter().map(|n| scene(n)).collect())
    }

    #[test]
    fn duplicates_collapse_and_the_order_is_first_use() -> Result<()> {
        let c = cache(&["b", "a", "b", "c", "a"]);
        assert_eq!(c.len(), 3, "three distinct scenes");
        assert_eq!(c.scene(0), Some(&scene("b")), "first use order");
        assert_eq!(c.scene(2), Some(&scene("c")), "first use order");
        Ok(())
    }

    #[test]
    fn filling_one_scene_per_step_reports_progress_to_completion() -> Result<()> {
        let mut c = cache(&["a", "b", "c"]);
        assert_eq!(
            c.progress(),
            Progress {
                done: 0,
                total: 3,
                failed: 0
            },
            "nothing compiled"
        );
        let mut seen = Vec::new();
        while let Some(slot) = c.next_pending() {
            let name = c.scene(slot).cloned().ok_or_else(|| anyhow!("no scene"))?;
            let result = if name == scene("b") {
                Err("main.glsl:3: nope".to_string())
            } else {
                Ok(slot as u32)
            };
            c.fill(slot, result);
            seen.push(c.progress().done);
        }
        assert_eq!(seen, vec![1, 2, 3], "one more scene each step");
        let progress = c.progress();
        assert!(progress.complete(), "all tried");
        assert_eq!(progress.failed, 1, "b failed");
        assert_eq!(c.failure(&scene("b")), Some("main.glsl:3: nope"), "reason");
        assert_eq!(c.failure(&scene("a")), None, "a is fine");
        Ok(())
    }

    #[test]
    fn a_scene_is_in_exactly_one_place() {
        let mut c = cache(&["a"]);
        c.fill(0, Ok(7));
        assert!(c.is_parked(&scene("a")), "parked after the compile");
        assert_eq!(c.take(&scene("a")), Some(7), "taken once");
        assert_eq!(c.take(&scene("a")), None, "not available while out");
        assert!(!c.is_parked(&scene("a")), "out is not parked");
        c.put_back(&scene("a"), 8);
        assert_eq!(c.take(&scene("a")), Some(8), "back again, with its value");
    }

    #[test]
    fn a_failed_or_unknown_scene_gives_nothing_and_a_result_arrives_once() {
        let mut c = cache(&["a"]);
        c.fill(0, Err("bad".to_string()));
        assert_eq!(c.take(&scene("a")), None, "failed scenes have no value");
        assert_eq!(c.take(&scene("zz")), None, "unknown scene");
        c.fill(0, Ok(1));
        assert_eq!(
            c.failure(&scene("a")),
            Some("bad"),
            "a second result does not overwrite the first"
        );
        c.put_back(&scene("a"), 9);
        assert!(!c.is_parked(&scene("a")), "no room for a stranger");
    }

    #[test]
    fn a_reset_makes_everything_pending_again() {
        let mut c = cache(&["a", "b"]);
        c.fill(0, Ok(1));
        c.fill(1, Err("x".to_string()));
        c.reset();
        assert_eq!(c.progress().done, 0, "nothing compiled any more");
        assert_eq!(c.next_pending(), Some(0), "starts over");
    }

    #[test]
    fn a_reload_keeps_what_is_compiled_and_compiles_only_what_is_new() {
        let mut old = cache(&["a", "b", "c"]);
        old.fill(0, Ok(1));
        old.fill(1, Err("bad".to_string()));
        old.fill(2, Ok(3));
        let _lent = old.take(&scene("c"));
        let mut new = cache(&["c", "a", "b", "d"]);
        new.adopt_from(&mut old);
        assert!(new.is_parked(&scene("a")), "a is still compiled");
        assert_eq!(new.failure(&scene("b")), Some("bad"), "b is still failed");
        assert_eq!(new.take(&scene("c")), None, "c is still out being played");
        let pending = new.next_pending().and_then(|i| new.scene(i).cloned());
        assert_eq!(pending, Some(scene("d")), "only d is left to compile");
    }

    #[test]
    fn an_empty_playlist_is_complete_at_once() {
        let c: SceneCache<u32> = SceneCache::new(Vec::new());
        assert!(c.is_empty(), "no scenes");
        assert!(c.progress().complete(), "nothing to wait for");
        assert_eq!(c.next_pending(), None, "nothing to compile");
    }
}
