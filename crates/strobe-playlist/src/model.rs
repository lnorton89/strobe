// ============================================================================
// Module:       strobe_playlist::model
// Description:  The playlist data model: playlists, sections, cues, the
//               settings they inherit, transitions, loop modes, shuffle
//               and the wall-clock anchor.
//
// Dependencies: crate::duration
// ============================================================================

//! The playlist model.
//!
//! This is the *authored* shape of a playlist: what a person typed or a
//! generator produced, with every inheritable field still optional. The
//! rest of the crate works from a flattened, fully resolved view of it
//! ([`crate::resolve`]); nothing downstream mutates a `Playlist`.
//!
//! # Inheritance
//!
//! Every setting resolves through **cue → section → playlist → built-in**,
//! so a cue that names only a scene is complete. [`CueSettings`] holds the
//! four inheritable fields, all optional; `None` means "ask the level
//! above".
//!
//! # Where a transition belongs
//!
//! A cue's `transition` describes how the show *arrives at that cue*, like
//! a slide's entrance effect. The first cue of a playlist has nothing to
//! arrive from and starts with a cut. When a looping playlist wraps, the
//! transition into its first cue is used, from its last.
//!
//! # Values the model keeps to itself
//!
//! [`ControlValue`] is this crate's own value type for control overrides
//! rather than anything from `strobe-scene`: the playlist file format must
//! not change when the scene crate's runtime types do. It mirrors the SSF
//! control kinds (a slider is a float, a toggle a bool, `xy` a vec2, a
//! colour a vec3, a dropdown an integer or a label) and the app converts
//! at the boundary.

use crate::duration::DurationSpec;
use std::collections::BTreeMap;
use std::path::PathBuf;

/// A whole playlist.
#[derive(Debug, Clone, PartialEq)]
pub struct Playlist {
    /// Shown in the UI, and part of the playlist's resume identity.
    pub name: String,
    /// Settings every cue inherits unless its section or the cue says
    /// otherwise.
    pub defaults: CueSettings,
    /// The cues, grouped. Played in order.
    pub sections: Vec<Section>,
    /// When the show starts each day, if it is tied to the clock.
    pub anchor: Option<WallClockAnchor>,
    /// What happens after the last cue.
    pub loop_mode: LoopMode,
    /// The seed for random durations, and the defaults the generator uses.
    pub shuffle: Option<ShuffleSpec>,
}

impl Playlist {
    /// An empty playlist called `name`.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            defaults: CueSettings::default(),
            sections: Vec::new(),
            anchor: None,
            loop_mode: LoopMode::Stop,
            shuffle: None,
        }
    }

    /// How many cues the playlist has, enabled or not.
    pub fn cue_count(&self) -> usize {
        self.sections.iter().map(|section| section.cues.len()).sum()
    }

    /// The seed random durations are drawn from: the shuffle seed, or 0.
    pub fn seed(&self) -> u64 {
        self.shuffle.as_ref().map_or(0, |shuffle| shuffle.seed)
    }
}

/// A named run of cues with settings of its own.
#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    /// Shown in the UI.
    pub name: String,
    /// Settings for this section's cues, over the playlist's.
    pub defaults: CueSettings,
    /// The cues, in play order.
    pub cues: Vec<Cue>,
}

impl Section {
    /// An empty section called `name`.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            defaults: CueSettings::default(),
            cues: Vec::new(),
        }
    }
}

/// One scene to play, with whatever it overrides.
#[derive(Debug, Clone, PartialEq)]
pub struct Cue {
    /// Which scene.
    pub scene: SceneRef,
    /// A named preset of the scene to load, if any.
    pub preset: Option<String>,
    /// Inheritable settings this cue overrides.
    pub settings: CueSettings,
    /// Scene control values to apply when the cue starts, by control name.
    pub overrides: BTreeMap<String, ControlValue>,
    /// A disabled cue stays in the file but is never played, scheduled or
    /// precompiled.
    pub enabled: bool,
}

impl Cue {
    /// An enabled cue that names only its scene.
    pub fn new(scene: SceneRef) -> Self {
        Self {
            scene,
            preset: None,
            settings: CueSettings::default(),
            overrides: BTreeMap::new(),
            enabled: true,
        }
    }
}

/// The fields a cue inherits. `None` means "use the level above".
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CueSettings {
    /// How long the cue lasts.
    pub duration: Option<DurationSpec>,
    /// What ends the cue.
    pub advance: Option<AdvanceMode>,
    /// How the show arrives at the cue.
    pub transition: Option<TransitionSpec>,
    /// Meta-control (post-processing) values; merged key by key down the
    /// levels rather than replaced.
    pub meta: Option<MetaOverrides>,
}

impl CueSettings {
    /// Whether every field is `None`.
    pub fn is_empty(&self) -> bool {
        self.duration.is_none()
            && self.advance.is_none()
            && self.transition.is_none()
            && self.meta.is_none()
    }
}

/// A scene named in a playlist: a library entry or a path.
///
/// In a file both are a string; one that has a path separator, starts with
/// `.` or `~`, or ends in `.synScene` is a path, anything else is a library
/// name ([`SceneRef::parse`]). A [`SceneRef::Path`] whose text would read as
/// a library name is written with a `./` prefix, so it stays a path.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum SceneRef {
    /// A scene in the library, by name or imported id (`warm-tunnel`,
    /// `ST-XsXXDn`).
    Library(String),
    /// A scene folder at a path.
    Path(PathBuf),
}

impl SceneRef {
    /// Reads the file form of a scene reference.
    pub fn parse(text: &str) -> Self {
        let looks_like_path = text.contains(['/', '\\'])
            || text.starts_with(['.', '~'])
            || text.to_ascii_lowercase().ends_with(".synscene");
        if looks_like_path {
            Self::Path(PathBuf::from(text))
        } else {
            Self::Library(text.to_string())
        }
    }

    /// The file form: the name, or the path (with a `./` guard if it would
    /// otherwise read back as a name).
    pub fn to_text(&self) -> String {
        match self {
            Self::Library(name) => name.clone(),
            Self::Path(path) => {
                let text = path.to_string_lossy().into_owned();
                match Self::parse(&text) {
                    Self::Path(_) => text,
                    Self::Library(_) => format!("./{text}"),
                }
            }
        }
    }
}

impl std::fmt::Display for SceneRef {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_text())
    }
}

/// A control value in an override.
///
/// Mirrors the SSF control kinds without depending on `strobe-scene`.
#[derive(Debug, Clone, PartialEq)]
pub enum ControlValue {
    /// A slider, knob or other scalar.
    Float(f32),
    /// A whole number, such as a dropdown index or a bang counter.
    Int(i64),
    /// A toggle.
    Bool(bool),
    /// An `xy` pad.
    Vec2([f32; 2]),
    /// An RGB colour, each channel 0..1.
    Color([f32; 3]),
    /// A dropdown label.
    Text(String),
}

impl ControlValue {
    /// The value as a float, for the kinds that are numbers.
    pub fn as_f32(&self) -> Option<f32> {
        match self {
            Self::Float(value) => Some(*value),
            Self::Int(value) => Some(*value as f32),
            Self::Bool(value) => Some(f32::from(u8::from(*value))),
            Self::Vec2(_) | Self::Color(_) | Self::Text(_) => None,
        }
    }
}

/// Meta-control overrides (brightness, hue, mirror and so on).
///
/// A placeholder until the meta stage exists in the renderer: names are
/// not checked, values are carried and merged, nothing applies them yet.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct MetaOverrides {
    /// Meta control name to value.
    pub values: BTreeMap<String, ControlValue>,
}

/// An audio event that can end a cue (Phase 7 detection feeds these).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioEventKind {
    /// A sharp rise in presence after a sustained low.
    Drop,
    /// A sustained collapse of bass presence.
    Breakdown,
}

/// What ends a cue.
#[derive(Debug, Clone, PartialEq)]
pub enum AdvanceMode {
    /// The cue's duration runs out.
    Timer,
    /// Only a person (next, jump) moves on; the duration is nominal.
    Manual,
    /// The named audio event arrives. No timer.
    AudioEvent(AudioEventKind),
    /// Whichever of these happens first.
    FirstOf(Vec<AdvanceMode>),
}

impl AdvanceMode {
    /// Whether the duration timer can end the cue.
    pub fn has_timer(&self) -> bool {
        match self {
            Self::Timer => true,
            Self::Manual | Self::AudioEvent(_) => false,
            Self::FirstOf(modes) => modes.iter().any(Self::has_timer),
        }
    }

    /// Whether `event` can end the cue.
    pub fn listens_for(&self, event: AudioEventKind) -> bool {
        match self {
            Self::Timer | Self::Manual => false,
            Self::AudioEvent(kind) => *kind == event,
            Self::FirstOf(modes) => modes.iter().any(|mode| mode.listens_for(event)),
        }
    }

    /// The same mode with nested `FirstOf` lists flattened, which is the
    /// only shape the file format can express.
    pub fn flattened(&self) -> Self {
        match self {
            Self::Timer | Self::Manual | Self::AudioEvent(_) => self.clone(),
            Self::FirstOf(modes) => {
                let mut flat = Vec::new();
                for mode in modes {
                    match mode.flattened() {
                        Self::FirstOf(inner) => flat.extend(inner),
                        other @ (Self::Timer | Self::Manual | Self::AudioEvent(_)) => {
                            flat.push(other);
                        }
                    }
                }
                Self::FirstOf(flat)
            }
        }
    }
}

/// How the show arrives at a cue.
#[derive(Debug, Clone, PartialEq)]
pub struct TransitionSpec {
    /// The effect.
    pub kind: TransitionKind,
    /// How long it takes. Ignored (zero) for a cut.
    pub duration: DurationSpec,
}

impl TransitionSpec {
    /// An instant cut.
    pub fn cut() -> Self {
        Self {
            kind: TransitionKind::Cut,
            duration: DurationSpec::Seconds(0.0),
        }
    }

    /// A crossfade lasting `seconds`.
    pub fn crossfade(seconds: f64) -> Self {
        Self {
            kind: TransitionKind::Crossfade,
            duration: DurationSpec::Seconds(seconds),
        }
    }
}

/// The transition effect.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionKind {
    /// Switch instantly.
    Cut,
    /// Mix the outgoing and incoming scenes.
    Crossfade,
    /// Fade out to black, then in.
    FadeThroughBlack,
    /// A custom GLSL transition (`sampler2D from, to; float progress`).
    Shader(PathBuf),
}

/// What happens after the last cue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoopMode {
    /// Play once and end.
    Stop,
    /// Start again from the first cue.
    Loop,
    /// Stay in the current section, repeating it, until told to move on.
    LoopSection,
}

/// Seeding for random durations and the generator's defaults.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShuffleSpec {
    /// The seed. Files hold it as an integer when it fits TOML's signed
    /// 64 bits and as a quoted decimal string above that.
    pub seed: u64,
    /// How many recent scenes the generator avoids repeating.
    pub no_repeat_window: usize,
}

/// A time of day the show starts at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WallClockAnchor {
    /// Local seconds after midnight, 0..86400.
    pub seconds_of_day: u32,
}

/// Seconds in a day.
pub(crate) const SECONDS_PER_DAY: u32 = 86_400;

impl WallClockAnchor {
    /// An anchor at `hours:minutes` local time, if that is a time of day.
    pub fn from_hm(hours: u32, minutes: u32) -> Option<Self> {
        if hours < 24 && minutes < 60 {
            Some(Self {
                seconds_of_day: hours * 3600 + minutes * 60,
            })
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::Result;

    #[test]
    fn scene_refs_split_names_from_paths() {
        assert_eq!(
            SceneRef::parse("warm-tunnel"),
            SceneRef::Library("warm-tunnel".into()),
            "a bare word is a library name"
        );
        assert_eq!(
            SceneRef::parse("ST-XsXXDn"),
            SceneRef::Library("ST-XsXXDn".into()),
            "an imported id is a library name"
        );
        for path in [
            "scenes/a.synScene",
            "C:\\scenes\\a",
            "./a",
            "~/a",
            "a.synScene",
            "../x",
        ] {
            assert_eq!(
                SceneRef::parse(path),
                SceneRef::Path(PathBuf::from(path)),
                "{path:?} should read as a path"
            );
        }
    }

    #[test]
    fn a_path_always_reads_back_as_a_path() {
        let odd = SceneRef::Path(PathBuf::from("plain"));
        let text = odd.to_text();
        assert_eq!(text, "./plain", "a bare-word path gets a guard");
        assert_eq!(
            SceneRef::parse(&text),
            SceneRef::Path(PathBuf::from("./plain")),
            "the guarded text should stay a path"
        );
    }

    #[test]
    fn advance_queries_see_through_first_of() {
        let mode = AdvanceMode::FirstOf(vec![
            AdvanceMode::Manual,
            AdvanceMode::AudioEvent(AudioEventKind::Drop),
        ]);
        assert!(!mode.has_timer(), "no timer member");
        assert!(mode.listens_for(AudioEventKind::Drop), "drop is listed");
        assert!(
            !mode.listens_for(AudioEventKind::Breakdown),
            "breakdown is not"
        );
        let with_timer = AdvanceMode::FirstOf(vec![AdvanceMode::Timer]);
        assert!(with_timer.has_timer(), "a timer member counts");
    }

    #[test]
    fn nested_first_of_flattens() {
        let nested = AdvanceMode::FirstOf(vec![
            AdvanceMode::Timer,
            AdvanceMode::FirstOf(vec![
                AdvanceMode::Manual,
                AdvanceMode::AudioEvent(AudioEventKind::Breakdown),
            ]),
        ]);
        assert_eq!(
            nested.flattened(),
            AdvanceMode::FirstOf(vec![
                AdvanceMode::Timer,
                AdvanceMode::Manual,
                AdvanceMode::AudioEvent(AudioEventKind::Breakdown),
            ]),
            "inner lists should be spliced into the outer one"
        );
    }

    #[test]
    fn control_values_expose_numbers() {
        assert_eq!(ControlValue::Float(0.5).as_f32(), Some(0.5), "float");
        assert_eq!(ControlValue::Int(3).as_f32(), Some(3.0), "int");
        assert_eq!(ControlValue::Bool(true).as_f32(), Some(1.0), "bool");
        assert_eq!(
            ControlValue::Vec2([1.0, 2.0]).as_f32(),
            None,
            "vec2 is not a scalar"
        );
    }

    #[test]
    fn anchors_reject_non_times() -> Result<()> {
        assert_eq!(
            WallClockAnchor::from_hm(9, 30).map(|a| a.seconds_of_day),
            Some(34_200),
            "09:30"
        );
        assert!(
            WallClockAnchor::from_hm(24, 0).is_none(),
            "24:00 is not a time of day"
        );
        assert!(
            WallClockAnchor::from_hm(0, 60).is_none(),
            "00:60 is not a time of day"
        );
        Ok(())
    }

    #[test]
    fn a_new_playlist_counts_cues_and_seeds_from_shuffle() {
        let mut playlist = Playlist::new("p");
        assert_eq!(playlist.cue_count(), 0, "empty");
        assert_eq!(playlist.seed(), 0, "no shuffle means seed 0");
        let mut section = Section::new("s");
        section.cues.push(Cue::new(SceneRef::Library("a".into())));
        playlist.sections.push(section);
        playlist.shuffle = Some(ShuffleSpec {
            seed: 7,
            no_repeat_window: 3,
        });
        assert_eq!(playlist.cue_count(), 1, "one cue");
        assert_eq!(playlist.seed(), 7, "shuffle seed");
    }
}
