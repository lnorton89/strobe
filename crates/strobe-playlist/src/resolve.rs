// ============================================================================
// Module:       strobe_playlist::resolve
// Description:  Inheritance resolution: playlist, section and cue settings
//               flattened into fully resolved cues; the playlist's resume
//               identity; the precompile list.
//
// Dependencies: crate::model, crate::duration
// ============================================================================

//! Resolving a playlist.
//!
//! [`resolve`] walks a [`Playlist`] and produces, for every cue, a
//! [`ResolvedCue`] in which nothing is optional: duration, advance mode,
//! transition and meta values have each been taken from the nearest level
//! that set them (cue, then section, then playlist, then the built-ins
//! below). Everything downstream (timeline, clock, UI) reads resolved cues
//! and never repeats the inheritance rules.
//!
//! The resolved cue also records *which level* each setting came from
//! ([`SettingOrigins`]), so the UI can show an inherited value as such and
//! offer "reset to inherited" without redoing the walk.
//!
//! `meta` is the one field merged rather than replaced: a section that sets
//! `brightness` and a cue that sets `hue` gives a cue with both.

use crate::duration::DurationSpec;
use crate::model::{
    AdvanceMode, ControlValue, CueSettings, LoopMode, MetaOverrides, Playlist, SceneRef,
    TransitionSpec, WallClockAnchor,
};
use std::collections::{BTreeMap, HashSet};

/// The duration a cue gets when nothing above it says.
fn builtin_duration() -> DurationSpec {
    const SECONDS: f64 = 180.0;
    DurationSpec::Seconds(SECONDS)
}

/// The transition a cue gets when nothing above it says.
fn builtin_transition() -> TransitionSpec {
    const SECONDS: f64 = 2.0;
    TransitionSpec::crossfade(SECONDS)
}

/// Which level supplied a resolved setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Nothing set it: the built-in default.
    Builtin,
    /// The playlist's `defaults`.
    Playlist,
    /// The cue's section.
    Section,
    /// The cue itself.
    Cue,
}

/// Where each of a cue's inheritable settings came from. For `meta`, the
/// most specific level that contributed any key.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SettingOrigins {
    /// Origin of the duration.
    pub duration: Origin,
    /// Origin of the advance mode.
    pub advance: Origin,
    /// Origin of the transition.
    pub transition: Origin,
    /// Most specific origin of any meta value.
    pub meta: Origin,
}

/// A cue with every inheritable field decided.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedCue {
    /// Index of the cue's section in the playlist as authored.
    pub section: usize,
    /// Index of the cue inside that section as authored.
    pub cue: usize,
    /// Whether the cue plays (disabled cues are resolved but skipped by the
    /// timeline).
    pub enabled: bool,
    /// The scene.
    pub scene: SceneRef,
    /// The scene preset to load, if any.
    pub preset: Option<String>,
    /// How long the cue lasts.
    pub duration: DurationSpec,
    /// What ends it.
    pub advance: AdvanceMode,
    /// How the show arrives at it.
    pub transition: TransitionSpec,
    /// Meta-control values, merged down the levels.
    pub meta: MetaOverrides,
    /// Scene control values set by the cue itself.
    pub overrides: BTreeMap<String, ControlValue>,
    /// Where each setting came from.
    pub origins: SettingOrigins,
}

/// A playlist with every cue resolved, in play order.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedPlaylist {
    /// The playlist's name.
    pub name: String,
    /// See [`identity`].
    pub identity: u64,
    /// The seed random durations are drawn from.
    pub seed: u64,
    /// What happens after the last cue.
    pub loop_mode: LoopMode,
    /// The wall-clock anchor, if any.
    pub anchor: Option<WallClockAnchor>,
    /// Section names, indexed like [`ResolvedCue::section`].
    pub section_names: Vec<String>,
    /// Every cue, enabled or not.
    pub cues: Vec<ResolvedCue>,
}

/// Resolves one cue, or `None` if the indices are out of range.
pub fn resolve_cue(playlist: &Playlist, section: usize, cue: usize) -> Option<ResolvedCue> {
    let section_ref = playlist.sections.get(section)?;
    let cue_ref = section_ref.cues.get(cue)?;
    let levels: [(&CueSettings, Origin); 3] = [
        (&cue_ref.settings, Origin::Cue),
        (&section_ref.defaults, Origin::Section),
        (&playlist.defaults, Origin::Playlist),
    ];

    let (duration, duration_origin) = first_set(&levels, |s| s.duration.clone())
        .unwrap_or_else(|| (builtin_duration(), Origin::Builtin));
    let (advance, advance_origin) =
        first_set(&levels, |s| s.advance.clone()).unwrap_or((AdvanceMode::Timer, Origin::Builtin));
    let (transition, transition_origin) = first_set(&levels, |s| s.transition.clone())
        .unwrap_or_else(|| (builtin_transition(), Origin::Builtin));

    // Meta merges from the top down, so the more specific level wins a
    // clash and keeps the keys it did not mention.
    let mut meta = MetaOverrides::default();
    let mut meta_origin = Origin::Builtin;
    for (settings, origin) in levels.iter().rev() {
        if let Some(values) = &settings.meta {
            if !values.values.is_empty() {
                meta_origin = *origin;
            }
            for (name, value) in &values.values {
                meta.values.insert(name.clone(), value.clone());
            }
        }
    }

    Some(ResolvedCue {
        section,
        cue,
        enabled: cue_ref.enabled,
        scene: cue_ref.scene.clone(),
        preset: cue_ref.preset.clone(),
        duration,
        advance,
        transition,
        meta,
        overrides: cue_ref.overrides.clone(),
        origins: SettingOrigins {
            duration: duration_origin,
            advance: advance_origin,
            transition: transition_origin,
            meta: meta_origin,
        },
    })
}

/// The first level (most specific first) for which `get` yields a value.
fn first_set<T>(
    levels: &[(&CueSettings, Origin)],
    get: impl Fn(&CueSettings) -> Option<T>,
) -> Option<(T, Origin)> {
    levels
        .iter()
        .find_map(|(settings, origin)| get(settings).map(|value| (value, *origin)))
}

/// Resolves every cue of `playlist`.
pub fn resolve(playlist: &Playlist) -> ResolvedPlaylist {
    let mut cues = Vec::with_capacity(playlist.cue_count());
    for (section, section_ref) in playlist.sections.iter().enumerate() {
        for cue in 0..section_ref.cues.len() {
            if let Some(resolved) = resolve_cue(playlist, section, cue) {
                cues.push(resolved);
            }
        }
    }
    ResolvedPlaylist {
        name: playlist.name.clone(),
        identity: identity(playlist),
        seed: playlist.seed(),
        loop_mode: playlist.loop_mode,
        anchor: playlist.anchor,
        section_names: playlist.sections.iter().map(|s| s.name.clone()).collect(),
        cues,
    }
}

/// A stable hash of what makes a playlist *this* playlist for resuming:
/// its name, its sections' names, and each cue's scene and enabled flag, in
/// order.
///
/// Durations, transitions and overrides are deliberately left out. A
/// resume point names a section and cue index, so it stays valid however
/// long the cues run; editing a duration by hand between runs must not
/// throw away the saved position, while adding or reordering cues (which
/// would make the saved indices point elsewhere) must.
///
/// FNV-1a over a fixed byte encoding: the same on every platform and
/// toolchain, unlike `std`'s hashers, whose algorithm is unspecified.
pub fn identity(playlist: &Playlist) -> u64 {
    const OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut hash = OFFSET;
    let mut feed = |bytes: &[u8]| {
        for byte in bytes {
            hash ^= u64::from(*byte);
            hash = hash.wrapping_mul(PRIME);
        }
        // A separator no UTF-8 text contains, so ("ab","c") != ("a","bc").
        hash ^= 0xff;
        hash = hash.wrapping_mul(PRIME);
    };
    feed(playlist.name.as_bytes());
    for section in &playlist.sections {
        feed(b"section");
        feed(section.name.as_bytes());
        for cue in &section.cues {
            feed(cue.scene.to_text().as_bytes());
            feed(if cue.enabled { b"on" } else { b"off" });
        }
    }
    hash
}

/// Every distinct scene an enabled cue uses, in order of first use.
///
/// This is the app's precompile list: compile each one at load, before the
/// first cue plays, and no shader is ever compiled during playback.
/// Disabled cues are left out because they cannot play until the file is
/// edited and reloaded, which precompiles again.
pub fn distinct_scenes(playlist: &Playlist) -> Vec<SceneRef> {
    let mut seen: HashSet<&SceneRef> = HashSet::new();
    let mut scenes = Vec::new();
    for section in &playlist.sections {
        for cue in section.cues.iter().filter(|cue| cue.enabled) {
            if seen.insert(&cue.scene) {
                scenes.push(cue.scene.clone());
            }
        }
    }
    scenes
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Cue, Section, TransitionKind};
    use anyhow::{Result, anyhow};

    fn lib(name: &str) -> SceneRef {
        SceneRef::Library(name.to_string())
    }

    fn one_cue_playlist() -> Playlist {
        let mut playlist = Playlist::new("p");
        let mut section = Section::new("s");
        section.cues.push(Cue::new(lib("a")));
        playlist.sections.push(section);
        playlist
    }

    fn resolved(playlist: &Playlist) -> Result<ResolvedCue> {
        resolve_cue(playlist, 0, 0).ok_or_else(|| anyhow!("cue 0/0 should exist"))
    }

    fn set_cue(playlist: &mut Playlist, f: impl FnOnce(&mut Cue)) -> Result<()> {
        let cue = playlist
            .sections
            .first_mut()
            .and_then(|s| s.cues.first_mut())
            .ok_or_else(|| anyhow!("no cue"))?;
        f(cue);
        Ok(())
    }

    #[test]
    fn a_cue_that_names_only_a_scene_gets_the_builtins() -> Result<()> {
        let cue = resolved(&one_cue_playlist())?;
        assert_eq!(
            cue.duration,
            DurationSpec::Seconds(180.0),
            "built-in duration"
        );
        assert_eq!(cue.advance, AdvanceMode::Timer, "built-in advance");
        assert_eq!(
            cue.transition.kind,
            TransitionKind::Crossfade,
            "built-in transition"
        );
        assert_eq!(cue.origins.duration, Origin::Builtin, "origin of duration");
        assert_eq!(cue.origins.meta, Origin::Builtin, "no meta from anywhere");
        assert!(cue.meta.values.is_empty(), "no meta values");
        Ok(())
    }

    #[test]
    fn each_level_overrides_the_one_above() -> Result<()> {
        let mut playlist = one_cue_playlist();
        playlist.defaults.duration = Some(DurationSpec::Seconds(60.0));
        assert_eq!(
            resolved(&playlist)?.duration,
            DurationSpec::Seconds(60.0),
            "playlist level"
        );
        assert_eq!(
            resolved(&playlist)?.origins.duration,
            Origin::Playlist,
            "playlist origin"
        );

        if let Some(section) = playlist.sections.first_mut() {
            section.defaults.duration = Some(DurationSpec::Bars(8));
        }
        assert_eq!(
            resolved(&playlist)?.duration,
            DurationSpec::Bars(8),
            "section beats playlist"
        );
        assert_eq!(
            resolved(&playlist)?.origins.duration,
            Origin::Section,
            "section origin"
        );

        set_cue(&mut playlist, |cue| {
            cue.settings.duration = Some(DurationSpec::Beats(5))
        })?;
        assert_eq!(
            resolved(&playlist)?.duration,
            DurationSpec::Beats(5),
            "cue beats section"
        );
        assert_eq!(
            resolved(&playlist)?.origins.duration,
            Origin::Cue,
            "cue origin"
        );
        Ok(())
    }

    #[test]
    fn fields_inherit_independently() -> Result<()> {
        let mut playlist = one_cue_playlist();
        playlist.defaults.advance = Some(AdvanceMode::Manual);
        playlist.defaults.transition = Some(TransitionSpec::cut());
        set_cue(&mut playlist, |cue| {
            cue.settings.duration = Some(DurationSpec::Seconds(10.0));
        })?;
        let cue = resolved(&playlist)?;
        assert_eq!(
            cue.duration,
            DurationSpec::Seconds(10.0),
            "duration from the cue"
        );
        assert_eq!(
            cue.advance,
            AdvanceMode::Manual,
            "advance from the playlist"
        );
        assert_eq!(
            cue.transition,
            TransitionSpec::cut(),
            "transition from the playlist"
        );
        assert_eq!(cue.origins.duration, Origin::Cue, "duration origin");
        assert_eq!(cue.origins.advance, Origin::Playlist, "advance origin");
        Ok(())
    }

    #[test]
    fn a_cut_at_cue_level_replaces_the_inherited_transition_whole() -> Result<()> {
        let mut playlist = one_cue_playlist();
        playlist.defaults.transition = Some(TransitionSpec::crossfade(4.0));
        set_cue(&mut playlist, |cue| {
            cue.settings.transition = Some(TransitionSpec::cut())
        })?;
        let cue = resolved(&playlist)?;
        assert_eq!(
            cue.transition.kind,
            TransitionKind::Cut,
            "kind from the cue"
        );
        assert_eq!(
            cue.transition.duration,
            DurationSpec::Seconds(0.0),
            "the inherited 4s must not leak into a cut"
        );
        Ok(())
    }

    #[test]
    fn meta_merges_key_by_key_with_the_specific_level_winning() -> Result<()> {
        let meta = |pairs: &[(&str, f32)]| MetaOverrides {
            values: pairs
                .iter()
                .map(|(k, v)| ((*k).to_string(), ControlValue::Float(*v)))
                .collect(),
        };
        let mut playlist = one_cue_playlist();
        playlist.defaults.meta = Some(meta(&[("brightness", 1.0), ("hue", 0.1)]));
        if let Some(section) = playlist.sections.first_mut() {
            section.defaults.meta = Some(meta(&[("hue", 0.5)]));
        }
        set_cue(&mut playlist, |cue| {
            cue.settings.meta = Some(meta(&[("gamma", 2.0)]))
        })?;
        let cue = resolved(&playlist)?;
        assert_eq!(
            cue.meta.values.get("brightness"),
            Some(&ControlValue::Float(1.0)),
            "an untouched playlist key survives"
        );
        assert_eq!(
            cue.meta.values.get("hue"),
            Some(&ControlValue::Float(0.5)),
            "the section overrides the playlist's hue"
        );
        assert_eq!(
            cue.meta.values.get("gamma"),
            Some(&ControlValue::Float(2.0)),
            "the cue adds its own key"
        );
        assert_eq!(
            cue.origins.meta,
            Origin::Cue,
            "the most specific contributor"
        );
        Ok(())
    }

    #[test]
    fn overrides_and_preset_belong_to_the_cue_alone() -> Result<()> {
        let mut playlist = one_cue_playlist();
        set_cue(&mut playlist, |cue| {
            cue.preset = Some("slow".into());
            cue.overrides
                .insert("speed".into(), ControlValue::Float(0.6));
        })?;
        let cue = resolved(&playlist)?;
        assert_eq!(cue.preset.as_deref(), Some("slow"), "preset");
        assert_eq!(
            cue.overrides.get("speed"),
            Some(&ControlValue::Float(0.6)),
            "override"
        );
        Ok(())
    }

    #[test]
    fn resolve_keeps_authored_positions_and_disabled_cues() -> Result<()> {
        let mut playlist = Playlist::new("p");
        for name in ["one", "two"] {
            let mut section = Section::new(name);
            section.cues.push(Cue::new(lib(&format!("{name}-a"))));
            let mut off = Cue::new(lib(&format!("{name}-b")));
            off.enabled = false;
            section.cues.push(off);
            playlist.sections.push(section);
        }
        let all = resolve(&playlist);
        assert_eq!(all.cues.len(), 4, "disabled cues are resolved too");
        let positions: Vec<(usize, usize, bool)> = all
            .cues
            .iter()
            .map(|c| (c.section, c.cue, c.enabled))
            .collect();
        assert_eq!(
            positions,
            vec![(0, 0, true), (0, 1, false), (1, 0, true), (1, 1, false)],
            "authored indices and enabled flags"
        );
        assert_eq!(all.section_names, vec!["one", "two"], "section names");
        assert!(resolve_cue(&playlist, 2, 0).is_none(), "no third section");
        assert!(resolve_cue(&playlist, 0, 2).is_none(), "no third cue");
        Ok(())
    }

    #[test]
    fn identity_ignores_timing_but_not_structure() -> Result<()> {
        let mut base = one_cue_playlist();
        let before = identity(&base);
        base.defaults.duration = Some(DurationSpec::Seconds(5.0));
        set_cue(&mut base, |cue| {
            cue.settings.transition = Some(TransitionSpec::cut());
            cue.overrides.insert("x".into(), ControlValue::Float(1.0));
        })?;
        assert_eq!(
            identity(&base),
            before,
            "timing edits keep a resume point valid"
        );

        let mut more = base.clone();
        if let Some(section) = more.sections.first_mut() {
            section.cues.push(Cue::new(lib("b")));
        }
        assert_ne!(identity(&more), before, "an added cue changes identity");

        let mut renamed = base.clone();
        renamed.name = "other".into();
        assert_ne!(identity(&renamed), before, "a rename changes identity");

        let mut disabled = base;
        set_cue(&mut disabled, |cue| cue.enabled = false)?;
        assert_ne!(
            identity(&disabled),
            before,
            "disabling a cue changes identity"
        );
        Ok(())
    }

    #[test]
    fn identity_is_pinned() {
        // FNV-1a over the fixed encoding above. If this value moves, every
        // saved resume point stops matching its playlist.
        let mut playlist = Playlist::new("p");
        let mut section = Section::new("s");
        section.cues.push(Cue::new(lib("a")));
        playlist.sections.push(section);
        assert_eq!(
            identity(&playlist),
            0x3072_d250_76d1_3604,
            "identity hash changed"
        );
    }

    #[test]
    fn distinct_scenes_come_in_first_use_order_without_repeats_or_disabled() {
        let mut playlist = Playlist::new("p");
        let mut one = Section::new("one");
        for name in ["c", "a", "c", "b"] {
            one.cues.push(Cue::new(lib(name)));
        }
        let mut two = Section::new("two");
        two.cues.push(Cue::new(lib("a")));
        two.cues
            .push(Cue::new(SceneRef::Path("x/y.synScene".into())));
        let mut off = Cue::new(lib("never"));
        off.enabled = false;
        two.cues.push(off);
        two.cues
            .push(Cue::new(SceneRef::Path("x/y.synScene".into())));
        playlist.sections.push(one);
        playlist.sections.push(two);
        assert_eq!(
            distinct_scenes(&playlist),
            vec![
                lib("c"),
                lib("a"),
                lib("b"),
                SceneRef::Path("x/y.synScene".into())
            ],
            "first-use order, deduplicated, no disabled cues"
        );
    }
}
