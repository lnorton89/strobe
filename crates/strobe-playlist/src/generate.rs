// ============================================================================
// Module:       strobe_playlist::generate
// Description:  The deterministic section generator: a library snapshot, a
//               pool filter, a target length and a seed in; cues out.
//
// Dependencies: crate::model, crate::duration, crate::rng, crate::timeline
// ============================================================================

//! Generating a section.
//!
//! "A 4-hour playlist from the `calm` tag in two minutes" is the point of
//! this module. [`generate`] is a pure function: the same library snapshot,
//! spec and seed give the same cues on every machine and every build, so
//! the UI can preview a section, throw it away, and later regenerate
//! exactly it from the seed it showed.
//!
//! # How a section is built
//!
//! 1. The pool is the library snapshot filtered by [`PoolFilter`] and
//!    **sorted by name**, so the order of the snapshot (a directory
//!    listing, a database query) cannot change the result.
//! 2. Repeatedly: choose a scene uniformly among those not in the last *w*
//!    picks; draw the cue's duration from the duration rule; choose its
//!    transition from the transition rules. Stop when the running total
//!    reaches the target.
//! 3. Every drawn value is written into the cue explicitly, so the section
//!    reads the same in the file as in the preview and does not depend on
//!    the playlist's `shuffle` seed afterwards.
//!
//! # The no-repeat window
//!
//! *w* is the requested window, shrunk to `pool size - 1` when the pool is
//! smaller (a 3-scene pool with a window of 10 can still avoid immediate
//! repeats; it cannot avoid all of them). A pool of one scene repeats it.
//! [`Generated::effective_window`] reports the window actually applied.
//! `GenerateSpec::recent` seeds the history with the scenes that came
//! before this section, so the boundary between sections obeys the window
//! too.
//!
//! # The tail
//!
//! With a rule that resolves to seconds the section is exactly the target
//! long: the last cue is clipped, or, if less than [`MIN_TAIL_SECONDS`]
//! would remain, absorbed by the cue before it. With a bar or beat rule the
//! tempo is unknown here, so the total is estimated at `fallback_bpm` and
//! overshoots by less than one cue.

use crate::duration::{BEATS_PER_BAR, DurationSpec, Length};
use crate::model::{Cue, SceneRef, Section, TransitionSpec};
use crate::rng::SplitMix64;
use crate::timeline::MIN_CUE_SECONDS;

/// A tail shorter than this is folded into the previous cue instead of
/// becoming a cue of its own.
pub const MIN_TAIL_SECONDS: f64 = 10.0;

/// More cues than this in one section means the spec is a mistake.
const MAX_CUES: usize = 10_000;

/// One scene as the generator sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryScene {
    /// The library name cues will refer to.
    pub name: String,
    /// Tags, matched case-insensitively.
    pub tags: Vec<String>,
    /// The library folder it lives in (`/`-separated, empty for the root).
    pub folder: String,
}

impl LibraryScene {
    /// A scene with tags in the root folder.
    pub fn new(name: &str, tags: &[&str]) -> Self {
        Self {
            name: name.to_string(),
            tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
            folder: String::new(),
        }
    }
}

/// The scenes available to a generator.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LibrarySnapshot {
    /// Every scene, in any order.
    pub scenes: Vec<LibraryScene>,
}

/// Which scenes may be picked.
///
/// A scene is in the pool if it matches *any* of the include lists (or all
/// three are empty, meaning "everything") and matches *none* of the
/// exclude lists.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PoolFilter {
    /// Scenes by exact name.
    pub names: Vec<String>,
    /// Folders; a scene in a subfolder matches too.
    pub folders: Vec<String>,
    /// Scenes carrying any of these tags.
    pub tags: Vec<String>,
    /// Scenes never to pick, by name.
    pub exclude_names: Vec<String>,
    /// Scenes never to pick if they carry any of these tags.
    pub exclude_tags: Vec<String>,
}

impl PoolFilter {
    /// Whether `scene` is in the pool.
    pub fn admits(&self, scene: &LibraryScene) -> bool {
        let has_tag = |wanted: &[String]| {
            scene
                .tags
                .iter()
                .any(|tag| wanted.iter().any(|w| w.eq_ignore_ascii_case(tag)))
        };
        let included = (self.names.is_empty() && self.folders.is_empty() && self.tags.is_empty())
            || self
                .names
                .iter()
                .any(|name| name.eq_ignore_ascii_case(&scene.name))
            || self
                .folders
                .iter()
                .any(|folder| in_folder(&scene.folder, folder))
            || has_tag(&self.tags);
        let excluded = self
            .exclude_names
            .iter()
            .any(|name| name.eq_ignore_ascii_case(&scene.name))
            || has_tag(&self.exclude_tags);
        included && !excluded
    }
}

/// Whether `scene_folder` is `folder` or inside it.
fn in_folder(scene_folder: &str, folder: &str) -> bool {
    let normal = |text: &str| {
        text.replace('\\', "/")
            .trim_matches('/')
            .to_ascii_lowercase()
    };
    let (scene_folder, folder) = (normal(scene_folder), normal(folder));
    scene_folder == folder
        || (!folder.is_empty() && scene_folder.starts_with(&format!("{folder}/")))
}

/// What to generate.
#[derive(Debug, Clone, PartialEq)]
pub struct GenerateSpec {
    /// The new section's name.
    pub section_name: String,
    /// Which scenes to draw from.
    pub pool: PoolFilter,
    /// How long the section should be, in seconds.
    pub target_seconds: f64,
    /// The duration rule each cue draws from.
    pub duration: DurationSpec,
    /// Transition rules; empty leaves cues to inherit, one is used for
    /// every cue, several are picked from at random per cue.
    pub transitions: Vec<TransitionSpec>,
    /// How many recent scenes not to repeat.
    pub no_repeat_window: usize,
    /// Scenes played just before this section, oldest first.
    pub recent: Vec<String>,
    /// The seed. Everything random comes from it.
    pub seed: u64,
    /// The tempo beat lengths are estimated at.
    pub fallback_bpm: f64,
}

impl GenerateSpec {
    /// A spec with the usual defaults: the whole library, 3-minute cues,
    /// inherited transitions, a window of 5, 120 BPM.
    pub fn new(section_name: &str, target_seconds: f64, seed: u64) -> Self {
        Self {
            section_name: section_name.to_string(),
            pool: PoolFilter::default(),
            target_seconds,
            duration: DurationSpec::Seconds(180.0),
            transitions: Vec::new(),
            no_repeat_window: 5,
            recent: Vec::new(),
            seed,
            fallback_bpm: 120.0,
        }
    }
}

/// A generated section and what was applied to make it.
#[derive(Debug, Clone, PartialEq)]
pub struct Generated {
    /// The section, ready to append to a playlist.
    pub section: Section,
    /// Its length at the fallback BPM, in seconds.
    pub estimated_seconds: f64,
    /// How many scenes were in the pool.
    pub pool_size: usize,
    /// The no-repeat window actually used (see the module docs).
    pub effective_window: usize,
}

/// Why nothing was generated.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GenerateError {
    /// The filter admitted no scene.
    #[error("no scene in the library matches the pool")]
    EmptyPool,
    /// The target length is not a positive number.
    #[error("the target length must be a positive number of seconds")]
    BadTarget,
    /// The spec would need an absurd number of cues.
    #[error("more than {MAX_CUES} cues would be needed; lengthen the cue duration")]
    TooManyCues,
}

/// Generates a section. Pure in its arguments; see the module docs.
pub fn generate(
    library: &LibrarySnapshot,
    spec: &GenerateSpec,
) -> Result<Generated, GenerateError> {
    if !(spec.target_seconds.is_finite() && spec.target_seconds > 0.0) {
        return Err(GenerateError::BadTarget);
    }
    let mut pool: Vec<&LibraryScene> = library
        .scenes
        .iter()
        .filter(|scene| spec.pool.admits(scene))
        .collect();
    pool.sort_by(|a, b| a.name.cmp(&b.name));
    pool.dedup_by(|a, b| a.name == b.name);
    if pool.is_empty() {
        return Err(GenerateError::EmptyPool);
    }

    let window = spec.no_repeat_window.min(pool.len() - 1);
    // Indices into `pool`, oldest first. Scenes from `recent` that are not
    // in the pool cannot be picked anyway and are dropped.
    let mut history: Vec<usize> = spec
        .recent
        .iter()
        .filter_map(|name| pool.iter().position(|scene| scene.name == *name))
        .collect();

    let mut rng = SplitMix64::new(spec.seed);
    let bpm = spec.fallback_bpm;
    let mut section = Section::new(&spec.section_name);
    let mut total = 0.0;

    while total < spec.target_seconds {
        if section.cues.len() >= MAX_CUES {
            return Err(GenerateError::TooManyCues);
        }
        let blocked = history.len().saturating_sub(window);
        let blocked = history.get(blocked..).unwrap_or(&[]);
        let candidates: Vec<usize> = (0..pool.len()).filter(|i| !blocked.contains(i)).collect();
        // `window < pool.len()` guarantees at least one candidate; the
        // fallback keeps the function total regardless.
        let pick = candidates
            .get(rng.below(candidates.len() as u64) as usize)
            .copied()
            .unwrap_or(0);
        history.push(pick);

        let length = spec.duration.resolve(&mut rng, bpm);
        let mut duration = concrete(length);
        // Measured from the written duration (whole seconds), not the draw,
        // so the cues add up to what the section says it is.
        let mut seconds = concrete_length(&duration).seconds(bpm).max(MIN_CUE_SECONDS);
        let remaining = spec.target_seconds - total;
        if !length.is_beat_based() && seconds > remaining {
            if remaining < MIN_TAIL_SECONDS && extend_last(&mut section, &mut total, remaining) {
                break;
            }
            seconds = remaining.max(MIN_CUE_SECONDS);
            duration = DurationSpec::Seconds(seconds);
        }

        let transition = match spec.transitions.as_slice() {
            [] => None,
            [only] => Some(only.clone()),
            many => many.get(rng.below(many.len() as u64) as usize).cloned(),
        };
        let name = pool
            .get(pick)
            .map_or_else(String::new, |scene| scene.name.clone());
        let mut cue = Cue::new(SceneRef::Library(name));
        cue.settings.duration = Some(duration);
        cue.settings.transition = transition;
        section.cues.push(cue);
        total += seconds;
    }

    Ok(Generated {
        section,
        estimated_seconds: total,
        pool_size: pool.len(),
        effective_window: window,
    })
}

/// A drawn length written back as a readable spec: whole seconds, or bars
/// when the beats divide into whole bars.
fn concrete(length: Length) -> DurationSpec {
    match length {
        Length::Seconds(seconds) => DurationSpec::Seconds(seconds.round().max(1.0)),
        Length::Beats(beats) if beats % BEATS_PER_BAR == 0 => {
            DurationSpec::Bars(beats / BEATS_PER_BAR)
        }
        Length::Beats(beats) => DurationSpec::Beats(beats),
    }
}

/// The length a concrete (non-random) duration spec means.
fn concrete_length(duration: &DurationSpec) -> Length {
    match duration {
        DurationSpec::Seconds(seconds) => Length::Seconds(*seconds),
        DurationSpec::Bars(bars) => Length::Beats(bars.saturating_mul(BEATS_PER_BAR)),
        DurationSpec::Beats(beats) => Length::Beats(*beats),
        // `concrete` never produces a random spec.
        DurationSpec::Random { .. } => Length::Seconds(0.0),
    }
}

/// Lengthens the section's last cue by `extra` seconds, if it is counted in
/// seconds. Returns whether it did.
fn extend_last(section: &mut Section, total: &mut f64, extra: f64) -> bool {
    let Some(last) = section.cues.last_mut() else {
        return false;
    };
    match &last.settings.duration {
        Some(DurationSpec::Seconds(seconds)) => {
            last.settings.duration = Some(DurationSpec::Seconds(seconds + extra));
            *total += extra;
            true
        }
        Some(DurationSpec::Bars(_) | DurationSpec::Beats(_) | DurationSpec::Random { .. })
        | None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{TransitionKind, TransitionSpec};
    use anyhow::{Result, anyhow};

    fn library() -> LibrarySnapshot {
        let mut scenes = vec![
            LibraryScene::new("aurora", &["calm", "colour"]),
            LibraryScene::new("bloom", &["calm"]),
            LibraryScene::new("cathedral", &["dark", "3d"]),
            LibraryScene::new("domain-warp", &["colour", "fast"]),
            LibraryScene::new("embers", &["warm", "calm"]),
            LibraryScene::new("feedback", &["fast", "trails"]),
            LibraryScene::new("geode", &["3d", "calm"]),
            LibraryScene::new("halo", &["dark"]),
            LibraryScene::new("ink", &["calm", "dark"]),
            LibraryScene::new("kaleido", &["fast", "colour"]),
            LibraryScene::new("lattice", &["3d"]),
            LibraryScene::new("moire", &["fast"]),
        ];
        for scene in scenes
            .iter_mut()
            .filter(|s| matches!(s.name.as_str(), "geode" | "lattice"))
        {
            scene.folder = "3d/solids".to_string();
        }
        LibrarySnapshot { scenes }
    }

    fn names(generated: &Generated) -> Vec<String> {
        generated
            .section
            .cues
            .iter()
            .map(|cue| cue.scene.to_text())
            .collect()
    }

    fn seconds_of(cue: &Cue) -> f64 {
        match &cue.settings.duration {
            Some(DurationSpec::Seconds(s)) => *s,
            Some(DurationSpec::Bars(_) | DurationSpec::Beats(_) | DurationSpec::Random { .. })
            | None => f64::NAN,
        }
    }

    fn rand_2_to_4_minutes() -> DurationSpec {
        DurationSpec::Random {
            min: Box::new(DurationSpec::Seconds(120.0)),
            max: Box::new(DurationSpec::Seconds(240.0)),
        }
    }

    #[test]
    fn golden_output_for_a_fixed_seed() -> Result<()> {
        // Pins the whole algorithm: pool order, pick rule, draw order and the
        // PRNG. If this changes, saved seeds no longer reproduce their
        // sections; change it only on purpose, and say so in DECISIONS.md.
        let mut spec = GenerateSpec::new("Golden", 1200.0, 1337);
        spec.duration = rand_2_to_4_minutes();
        spec.no_repeat_window = 4;
        spec.transitions = vec![TransitionSpec::crossfade(4.0), TransitionSpec::cut()];
        let generated = generate(&library(), &spec)?;
        let got: Vec<String> = generated
            .section
            .cues
            .iter()
            .map(|cue| {
                format!(
                    "{} {} {}",
                    cue.scene,
                    cue.settings
                        .duration
                        .as_ref()
                        .map_or_else(String::new, ToString::to_string),
                    match cue.settings.transition.as_ref().map(|t| &t.kind) {
                        Some(TransitionKind::Cut) => "cut",
                        Some(TransitionKind::Crossfade) => "crossfade",
                        Some(TransitionKind::FadeThroughBlack | TransitionKind::Shader(_))
                        | None => "?",
                    }
                )
            })
            .collect();
        let expected: Vec<&str> = GOLDEN.to_vec();
        assert_eq!(
            got, expected,
            "the generator's output for seed 1337 changed"
        );
        assert_eq!(
            generated.estimated_seconds, 1200.0,
            "a seconds rule lands exactly on the target"
        );
        Ok(())
    }

    /// The recorded output of `golden_output_for_a_fixed_seed`: scene,
    /// duration, transition for each cue. Seed 1337, a 20-minute target,
    /// `rand(2m, 4m)` cues, a window of 4 and a crossfade/cut transition pair.
    const GOLDEN: &[&str] = &[
        "ink 215s crossfade",
        "feedback 147s cut",
        "domain-warp 131s cut",
        "bloom 213s crossfade",
        "geode 208s crossfade",
        "moire 183s cut",
        "cathedral 103s cut",
    ];

    #[test]
    fn the_same_inputs_give_the_same_section_whatever_the_snapshot_order() -> Result<()> {
        let mut spec = GenerateSpec::new("S", 3600.0, 7);
        spec.duration = rand_2_to_4_minutes();
        let forward = generate(&library(), &spec)?;
        let mut reversed = library();
        reversed.scenes.reverse();
        let backward = generate(&reversed, &spec)?;
        assert_eq!(forward, backward, "snapshot order must not matter");
        assert_eq!(
            forward,
            generate(&library(), &spec)?,
            "a second run must match"
        );
        Ok(())
    }

    #[test]
    fn different_seeds_give_different_sections() -> Result<()> {
        let spec = |seed| {
            let mut spec = GenerateSpec::new("S", 3600.0, seed);
            spec.duration = rand_2_to_4_minutes();
            spec
        };
        assert_ne!(
            names(&generate(&library(), &spec(1))?),
            names(&generate(&library(), &spec(2))?),
            "two seeds produced the same order"
        );
        Ok(())
    }

    #[test]
    fn a_seconds_rule_lands_exactly_on_the_target() -> Result<()> {
        for seed in 0..50u64 {
            let mut spec = GenerateSpec::new("S", 4.0 * 3600.0 + 7.0, seed);
            spec.duration = rand_2_to_4_minutes();
            let generated = generate(&library(), &spec)?;
            let sum: f64 = generated.section.cues.iter().map(seconds_of).sum();
            assert!(
                (sum - spec.target_seconds).abs() < 1e-6,
                "seed {seed}: cues sum to {sum}, wanted {}",
                spec.target_seconds
            );
            assert_eq!(
                generated.estimated_seconds, sum,
                "seed {seed}: estimate disagrees with the cues"
            );
            let shortest = generated
                .section
                .cues
                .iter()
                .map(seconds_of)
                .fold(f64::MAX, f64::min);
            assert!(
                shortest >= MIN_TAIL_SECONDS.min(120.0),
                "seed {seed}: a sliver cue of {shortest}s"
            );
        }
        Ok(())
    }

    #[test]
    fn a_four_hour_section_has_about_eighty_cues_at_three_minutes() -> Result<()> {
        let generated = generate(&library(), &GenerateSpec::new("S", 4.0 * 3600.0, 1))?;
        assert_eq!(generated.section.cues.len(), 80, "4 h of 3 min cues");
        Ok(())
    }

    #[test]
    fn the_no_repeat_window_holds_whenever_the_pool_allows() -> Result<()> {
        // Property: for many seeds, windows and pool sizes, no scene appears
        // twice within `effective_window` consecutive cues, including
        // against the `recent` history.
        for seed in 0..40u64 {
            for window in [0usize, 1, 3, 5, 8, 11, 30] {
                for pool_size in [1usize, 2, 3, 6, 12] {
                    let mut lib = library();
                    lib.scenes.truncate(pool_size);
                    let mut spec = GenerateSpec::new("S", 3000.0, seed);
                    spec.no_repeat_window = window;
                    spec.duration = DurationSpec::Seconds(60.0);
                    spec.recent = lib.scenes.iter().take(2).map(|s| s.name.clone()).collect();
                    let generated = generate(&lib, &spec)?;
                    let effective = generated.effective_window;
                    assert_eq!(effective, window.min(pool_size - 1), "effective window");
                    let sequence: Vec<String> = spec
                        .recent
                        .iter()
                        .cloned()
                        .chain(names(&generated))
                        .collect();
                    for (at, name) in sequence.iter().enumerate().skip(spec.recent.len()) {
                        let from = at.saturating_sub(effective);
                        let earlier = sequence.get(from..at).unwrap_or(&[]);
                        assert!(
                            !earlier.contains(name),
                            "seed {seed}, window {window}, pool {pool_size}: {name} repeats within {effective} cues at position {at}"
                        );
                    }
                }
            }
        }
        Ok(())
    }

    #[test]
    fn a_pool_smaller_than_the_window_still_avoids_immediate_repeats() -> Result<()> {
        let mut lib = library();
        lib.scenes.truncate(3);
        let mut spec = GenerateSpec::new("S", 3000.0, 5);
        spec.no_repeat_window = 10;
        spec.duration = DurationSpec::Seconds(60.0);
        let generated = generate(&lib, &spec)?;
        assert_eq!(generated.effective_window, 2, "window shrinks to pool - 1");
        let sequence = names(&generated);
        for pair in sequence.windows(2) {
            assert_ne!(
                pair.first(),
                pair.get(1),
                "back-to-back repeat in {sequence:?}"
            );
        }
        Ok(())
    }

    #[test]
    fn a_pool_of_one_repeats_it() -> Result<()> {
        let mut spec = GenerateSpec::new("S", 600.0, 1);
        spec.pool.names = vec!["halo".into()];
        spec.no_repeat_window = 5;
        let generated = generate(&library(), &spec)?;
        assert_eq!(generated.effective_window, 0, "nothing else to pick");
        assert!(
            names(&generated).iter().all(|n| n == "halo"),
            "only halo can play"
        );
        assert!(
            generated.section.cues.len() > 1,
            "and it repeats to fill the time"
        );
        Ok(())
    }

    #[test]
    fn pool_filters_combine_include_and_exclude() -> Result<()> {
        let lib = library();
        let pool_names = |filter: PoolFilter| -> Result<Vec<String>> {
            let mut spec = GenerateSpec::new("S", 36_000.0, 3);
            spec.pool = filter;
            spec.duration = DurationSpec::Seconds(60.0);
            spec.no_repeat_window = 0;
            let mut seen: Vec<String> = names(&generate(&lib, &spec)?);
            seen.sort();
            seen.dedup();
            Ok(seen)
        };
        assert_eq!(
            pool_names(PoolFilter {
                tags: vec!["CALM".into()],
                ..PoolFilter::default()
            })?,
            vec!["aurora", "bloom", "embers", "geode", "ink"],
            "tags match case-insensitively"
        );
        assert_eq!(
            pool_names(PoolFilter {
                folders: vec!["3d".into()],
                ..PoolFilter::default()
            })?,
            vec!["geode", "lattice"],
            "a parent folder includes its subfolders"
        );
        assert_eq!(
            pool_names(PoolFilter {
                tags: vec!["calm".into()],
                names: vec!["moire".into()],
                exclude_tags: vec!["dark".into()],
                exclude_names: vec!["bloom".into()],
                ..PoolFilter::default()
            })?,
            vec!["aurora", "embers", "geode", "moire"],
            "include by tag or name, then exclude by tag and name"
        );
        Ok(())
    }

    #[test]
    fn empty_pools_and_bad_targets_are_errors() {
        let mut spec = GenerateSpec::new("S", 60.0, 1);
        spec.pool.tags = vec!["nonexistent".into()];
        assert_eq!(
            generate(&library(), &spec).err(),
            Some(GenerateError::EmptyPool),
            "no match"
        );
        for target in [0.0, -5.0, f64::NAN, f64::INFINITY] {
            let spec = GenerateSpec::new("S", target, 1);
            assert_eq!(
                generate(&library(), &spec).err(),
                Some(GenerateError::BadTarget),
                "target {target}"
            );
        }
        assert_eq!(
            generate(
                &LibrarySnapshot::default(),
                &GenerateSpec::new("S", 60.0, 1)
            )
            .err(),
            Some(GenerateError::EmptyPool),
            "empty library"
        );
    }

    #[test]
    fn an_absurd_spec_fails_instead_of_running_away() {
        let mut spec = GenerateSpec::new("S", 1.0e9, 1);
        spec.duration = DurationSpec::Seconds(1.0);
        assert_eq!(
            generate(&library(), &spec).err(),
            Some(GenerateError::TooManyCues),
            "a billion one-second cues"
        );
    }

    #[test]
    fn bar_rules_keep_bars_and_may_overshoot_by_under_one_cue() -> Result<()> {
        let mut spec = GenerateSpec::new("S", 600.0, 9);
        spec.duration = DurationSpec::Random {
            min: Box::new(DurationSpec::Bars(16)),
            max: Box::new(DurationSpec::Bars(32)),
        };
        let generated = generate(&library(), &spec)?;
        for cue in &generated.section.cues {
            match &cue.settings.duration {
                Some(DurationSpec::Bars(n)) => {
                    assert!((16..=32).contains(n), "{n} bars out of range")
                }
                other => return Err(anyhow!("expected bars, got {other:?}")),
            }
        }
        // 32 bars at 120 BPM is 64 s: the overshoot is under one such cue.
        assert!(generated.estimated_seconds >= 600.0, "reaches the target");
        assert!(
            generated.estimated_seconds < 600.0 + 64.0,
            "overshoots by less than a cue"
        );
        Ok(())
    }

    #[test]
    fn transitions_are_inherited_fixed_or_drawn() -> Result<()> {
        let mut spec = GenerateSpec::new("S", 3000.0, 4);
        spec.duration = DurationSpec::Seconds(60.0);
        let none = generate(&library(), &spec)?;
        assert!(
            none.section
                .cues
                .iter()
                .all(|c| c.settings.transition.is_none()),
            "no rule leaves cues to inherit"
        );
        spec.transitions = vec![TransitionSpec::cut()];
        let one = generate(&library(), &spec)?;
        assert!(
            one.section
                .cues
                .iter()
                .all(|c| c.settings.transition == Some(TransitionSpec::cut())),
            "one rule is used throughout"
        );
        spec.transitions = vec![TransitionSpec::cut(), TransitionSpec::crossfade(3.0)];
        let both = generate(&library(), &spec)?;
        let cuts = both
            .section
            .cues
            .iter()
            .filter(|c| c.settings.transition == Some(TransitionSpec::cut()))
            .count();
        assert!(
            cuts > 0 && cuts < both.section.cues.len(),
            "both rules should be drawn, got {cuts} cuts"
        );
        Ok(())
    }
}
