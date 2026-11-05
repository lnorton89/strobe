// ============================================================================
// Module:       strobe_app::prerender
// Description:  The library's prerendered scenes: which are still to do,
//               which are parked ready, and whether a parked one still
//               matches its folder.
//
// Dependencies: std (collections, fs, time)
// ============================================================================

//! Prerendered library scenes (D542).
//!
//! A scene's first load compiles its shaders, starts its script and reads
//! its images on the thread that draws the output, which shows as a stall
//! of tens to hundreds of milliseconds. Prerendering does that work ahead:
//! at launch and whenever the library is listed again, every scene is
//! loaded at 1x1, drawn once so the driver finishes what it defers to the
//! first draw, and parked here. Loading a parked scene is then a resize and
//! a restart, which costs about a frame.
//!
//! [`Prerender`] is the bookkeeping, generic over what it parks so the
//! tests can park numbers; the app parks a
//! [`Session`](crate::session::Session) and does one scene per step (a
//! step blocks for that scene's compile, as a playlist's precompile does,
//! D151).
//!
//! - **Planned from the library.** [`Prerender::plan`] queues every scene
//!   folder not parked already, and drops parked scenes the library no
//!   longer lists. A rescan therefore prerenders only what is new.
//! - **Checked before use.** A parked scene carries a [`Stamp`] of its
//!   folder (file count, bytes, newest change) taken before it was read.
//!   [`Prerender::take`] hands it out only while the folder still matches,
//!   so an edit made in the Editor or outside Strobe is never hidden by a
//!   stale copy: the caller loads it fresh instead.

use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// What a scene folder holds, enough to tell that it changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stamp {
    files: u64,
    bytes: u64,
    newest: Option<SystemTime>,
}

/// The most files walked to stamp one folder; past it the folder is not
/// stamped, and a scene that cannot be stamped is never handed out parked.
const MAX_WALK: u64 = 4000;

/// The stamp of the scene folder `dir`: every file under it, counted,
/// summed and dated. `None` when it cannot be read or holds too much.
pub fn stamp(dir: &Path) -> Option<Stamp> {
    let mut total = Stamp {
        files: 0,
        bytes: 0,
        newest: None,
    };
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let meta = std::fs::symlink_metadata(&next).ok()?;
        total.newest = total.newest.max(meta.modified().ok());
        if meta.is_dir() {
            stack.extend(
                std::fs::read_dir(&next)
                    .ok()?
                    .filter_map(Result::ok)
                    .map(|entry| entry.path()),
            );
            continue;
        }
        total.files += 1;
        total.bytes += meta.len();
        if total.files > MAX_WALK {
            return None;
        }
    }
    Some(total)
}

/// One parked scene and the stamp its folder had when it was read.
#[derive(Debug)]
struct Parked<T> {
    item: T,
    stamp: Stamp,
}

/// The library's prerendered scenes (D542).
#[derive(Debug)]
pub struct Prerender<T> {
    /// Scene folders still to prerender, in library order.
    queue: VecDeque<PathBuf>,
    /// Prerendered scenes, by folder.
    parked: HashMap<PathBuf, Parked<T>>,
    /// Scenes in the last plan.
    total: usize,
}

impl<T> Default for Prerender<T> {
    fn default() -> Self {
        Self {
            queue: VecDeque::new(),
            parked: HashMap::new(),
            total: 0,
        }
    }
}

impl<T> Prerender<T> {
    /// Plans a prerender of `dirs` (the library's scene folders, in order):
    /// parked scenes the list no longer names are dropped, and every folder
    /// not parked is queued, except `live`, the scene playing now, which is
    /// parked when it is replaced instead.
    pub fn plan(&mut self, dirs: &[PathBuf], live: Option<&Path>) {
        self.parked.retain(|dir, _| dirs.contains(dir));
        self.queue = dirs
            .iter()
            .filter(|dir| !self.parked.contains_key(*dir) && live != Some(dir.as_path()))
            .cloned()
            .collect();
        self.total = dirs.len();
    }

    /// The next folder to prerender, taken off the queue.
    pub fn next_queued(&mut self) -> Option<PathBuf> {
        self.queue.pop_front()
    }

    /// Parks `item`, the scene in `dir`, read when the folder was `stamp`.
    /// A scene parked again replaces the one parked before.
    pub fn park(&mut self, dir: PathBuf, item: T, stamp: Stamp) {
        self.queue.retain(|queued| *queued != dir);
        self.parked.insert(dir, Parked { item, stamp });
    }

    /// Takes the parked scene in `dir` when its folder still matches
    /// `now`, the folder's stamp as it is; a parked scene whose folder has
    /// changed (or that cannot be stamped) is dropped and `None` returned,
    /// so the caller reads the folder afresh.
    pub fn take(&mut self, dir: &Path, now: Option<Stamp>) -> Option<T> {
        let parked = self.parked.remove(dir)?;
        (Some(parked.stamp) == now).then_some(parked.item)
    }

    /// Whether a scene is parked for `dir`, without taking it or checking
    /// its stamp: a load asks this to know there is no compile to wait for
    /// (D555), and [`Self::take`] still decides.
    pub fn is_parked(&self, dir: &Path) -> bool {
        self.parked.contains_key(dir)
    }

    /// Drops every parked scene and the queue.
    pub fn clear(&mut self) {
        self.queue.clear();
        self.parked.clear();
        self.total = 0;
    }

    /// How many scenes are parked.
    pub fn parked(&self) -> usize {
        self.parked.len()
    }

    /// How many scenes are still queued.
    pub fn remaining(&self) -> usize {
        self.queue.len()
    }

    /// Scenes in the last plan.
    pub fn total(&self) -> usize {
        self.total
    }
}

/// The least time between two steps that compile a scene (D542).
pub const MIN_GAP: Duration = Duration::from_millis(50);

/// For each moment a step held the frame loop, the output gets this many
/// to draw before the next step (D550). So it keeps at least four fifths of
/// the time on a machine of any speed. A fixed gap did not: on the mini
/// PC's Vega 3, compiles of up to 4.3 s with 50 ms between them left the
/// output at 7 fps for the minute the library took.
pub const DRAW_PER_STEP: u32 = 4;

/// When the step after one that ran from `began` to `ended` may start:
/// [`DRAW_PER_STEP`] times its cost later, and never sooner than
/// [`MIN_GAP`].
pub fn next_step(began: Instant, ended: Instant) -> Instant {
    let cost = ended.saturating_duration_since(began);
    let gap = MIN_GAP.max(cost.saturating_mul(DRAW_PER_STEP));
    ended.checked_add(gap).unwrap_or(ended)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    /// A scratch folder removed on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Result<Self> {
            let dir =
                std::env::temp_dir().join(format!("strobe-prerender-{}-{tag}", std::process::id()));
            if dir.exists() {
                std::fs::remove_dir_all(&dir)?;
            }
            std::fs::create_dir_all(&dir)?;
            Ok(Self(dir))
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn dirs(names: &[&str]) -> Vec<PathBuf> {
        names.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn the_output_keeps_four_fifths_of_the_time_however_slow_a_step_is() -> Result<()> {
        let began = Instant::now();
        for cost_ms in [0_u64, 5, 12, 100, 400, 4300] {
            let ended = began + Duration::from_millis(cost_ms);
            let next = next_step(began, ended);
            let gap = next.duration_since(ended);
            assert!(gap >= MIN_GAP, "{cost_ms} ms: never sooner than the floor");
            let cost = ended.duration_since(began);
            assert!(
                gap >= cost * DRAW_PER_STEP,
                "{cost_ms} ms: the output draws {DRAW_PER_STEP} times as long as the step held it"
            );
            let share = gap.as_secs_f64() / (gap + cost).as_secs_f64();
            // At exactly four times the cost the share is 0.8, which f64
            // may hold a hair under.
            assert!(
                share >= 0.8 - 1e-9,
                "{cost_ms} ms: the output's share is {share:.2}"
            );
        }
        // A cheap step keeps the old pace: the floor, not a multiple.
        let quick = began + Duration::from_millis(2);
        assert_eq!(
            next_step(began, quick).duration_since(quick),
            MIN_GAP,
            "a 2 ms step waits the 50 ms floor"
        );
        Ok(())
    }

    #[test]
    fn a_plan_queues_what_is_not_parked_or_live_and_drops_what_is_gone() -> Result<()> {
        let scratch = Scratch::new("plan")?;
        let Some(stamp) = stamp(&scratch.0) else {
            anyhow::bail!("an empty folder is stamped");
        };
        let mut prerender = Prerender::default();
        prerender.plan(&dirs(&["a", "b", "c"]), Some(Path::new("b")));
        assert_eq!(
            (prerender.total(), prerender.remaining()),
            (3, 2),
            "everything but the live scene is queued"
        );
        assert_eq!(
            prerender.next_queued(),
            Some(PathBuf::from("a")),
            "in order"
        );
        prerender.park(PathBuf::from("a"), 1, stamp);
        prerender.park(PathBuf::from("b"), 2, stamp);
        assert_eq!(
            (prerender.parked(), prerender.remaining()),
            (2, 1),
            "the live scene parked when replaced; c still to do"
        );

        prerender.plan(&dirs(&["a", "c", "d"]), None);
        assert_eq!(
            prerender.parked(),
            1,
            "b left the library, so it is dropped"
        );
        assert_eq!(
            (
                prerender.next_queued(),
                prerender.next_queued(),
                prerender.next_queued()
            ),
            (Some(PathBuf::from("c")), Some(PathBuf::from("d")), None),
            "a rescan queues only what is not parked"
        );
        prerender.clear();
        assert_eq!(
            (prerender.parked(), prerender.remaining(), prerender.total()),
            (0, 0, 0),
            "cleared"
        );
        Ok(())
    }

    #[test]
    fn a_parked_scene_is_handed_out_only_while_its_folder_is_unchanged() -> Result<()> {
        let scratch = Scratch::new("stamp")?;
        let scene = scratch.0.join("ink");
        std::fs::create_dir_all(scene.join("images"))?;
        std::fs::write(
            scene.join("main.glsl"),
            "vec4 renderMain() { return vec4(1.0); }",
        )?;
        std::fs::write(scene.join("images/a.png"), b"x")?;
        let before = stamp(&scene);
        assert!(before.is_some(), "a scene folder is stamped");

        let mut prerender = Prerender::default();
        if let Some(before) = before {
            prerender.park(scene.clone(), 7, before);
        }
        assert_eq!(
            prerender.take(&scene, stamp(&scene)),
            Some(7),
            "unchanged: handed out"
        );
        assert_eq!(prerender.take(&scene, stamp(&scene)), None, "and only once");

        if let Some(before) = before {
            prerender.park(scene.clone(), 8, before);
        }
        std::fs::write(
            scene.join("main.glsl"),
            "vec4 renderMain() { return vec4(0.5, 0.5, 0.5, 1.0); }",
        )?;
        assert_eq!(
            prerender.take(&scene, stamp(&scene)),
            None,
            "an edited scene is not handed out stale"
        );
        assert_eq!(prerender.parked(), 0, "and the stale copy is dropped");
        assert_eq!(
            stamp(&scratch.0.join("missing")),
            None,
            "a folder that is not there has no stamp"
        );
        Ok(())
    }
}
