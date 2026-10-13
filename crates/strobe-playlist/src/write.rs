// ============================================================================
// Module:       strobe_playlist::write
// Description:  Model to playlist TOML: deterministic, laid out the way
//               PLAN section 7.2 shows it, and read back to an equal model.
//
// Dependencies: crate::model, crate::duration
// ============================================================================

//! Writing a playlist.
//!
//! The writer is hand-rolled rather than going through `toml::to_string`
//! because the file is meant to be read and edited by a person: one cue per
//! line, settings as inline tables next to what they apply to, a fixed key
//! order, and blank lines between blocks. The same model always gives the
//! same bytes, so a saved playlist diffs cleanly in version control.
//!
//! ```toml
//! name = "Office lobby"
//! loop_mode = "loop"
//!
//! [defaults]
//! duration = "3m"
//! transition = { kind = "crossfade", duration = "4s" }
//!
//! [[sections]]
//! name = "Morning"
//! cues = [
//!   { scene = "warm-tunnel" },
//!   { scene = "ink-drift", duration = "32 bars", transition = { kind = "cut" } },
//! ]
//! ```
//!
//! Round trip: `parse(write(p)) == p` for every playlist whose numbers are
//! finite and whose `AdvanceMode::FirstOf` lists are not nested (the file
//! can only express a flat list; [`AdvanceMode::flattened`] is the
//! normal form). The tests assert it.

use crate::model::{
    AdvanceMode, AudioEventKind, ControlValue, Cue, CueSettings, LoopMode, Playlist, Section,
    TransitionKind, TransitionSpec, WallClockAnchor,
};
use std::fmt::Write as _;

/// Playlist text for `playlist`.
pub fn to_toml(playlist: &Playlist) -> String {
    let mut out = String::new();
    push_line(&mut out, &format!("name = {}", quote(&playlist.name)));
    push_line(
        &mut out,
        &format!("loop_mode = {}", quote(loop_mode_name(playlist.loop_mode))),
    );
    if let Some(anchor) = playlist.anchor {
        push_line(
            &mut out,
            &format!("anchor = {}", quote(&anchor_text(anchor))),
        );
    }

    let defaults = settings_pairs(&playlist.defaults);
    if !defaults.is_empty() {
        out.push('\n');
        push_line(&mut out, "[defaults]");
        for (key, value) in defaults {
            push_line(&mut out, &format!("{key} = {value}"));
        }
    }

    if let Some(shuffle) = &playlist.shuffle {
        out.push('\n');
        push_line(&mut out, "[shuffle]");
        let seed = match i64::try_from(shuffle.seed) {
            Ok(seed) => seed.to_string(),
            // TOML integers are signed 64-bit; larger seeds travel as text.
            Err(_) => quote(&shuffle.seed.to_string()),
        };
        push_line(&mut out, &format!("seed = {seed}"));
        push_line(
            &mut out,
            &format!("no_repeat_window = {}", shuffle.no_repeat_window),
        );
    }

    for section in &playlist.sections {
        out.push('\n');
        write_section(&mut out, section);
    }
    out
}

fn write_section(out: &mut String, section: &Section) {
    push_line(out, "[[sections]]");
    push_line(out, &format!("name = {}", quote(&section.name)));
    let defaults = settings_pairs(&section.defaults);
    if !defaults.is_empty() {
        push_line(out, &format!("defaults = {}", inline_table(&defaults)));
    }
    if section.cues.is_empty() {
        push_line(out, "cues = []");
        return;
    }
    push_line(out, "cues = [");
    for cue in &section.cues {
        push_line(out, &format!("  {},", inline_table(&cue_pairs(cue))));
    }
    push_line(out, "]");
}

/// `key = value` pairs for a cue, in file order.
fn cue_pairs(cue: &Cue) -> Vec<(String, String)> {
    let mut pairs = vec![("scene".to_string(), quote(&cue.scene.to_text()))];
    if let Some(preset) = &cue.preset {
        pairs.push(("preset".to_string(), quote(preset)));
    }
    if let Some(media) = &cue.media {
        pairs.push(("media".to_string(), quote(&media.to_string_lossy())));
    }
    pairs.extend(settings_pairs(&cue.settings));
    if !cue.overrides.is_empty() {
        pairs.push(("overrides".to_string(), value_table(&cue.overrides)));
    }
    if !cue.enabled {
        pairs.push(("enabled".to_string(), "false".to_string()));
    }
    pairs
}

/// `key = value` pairs for the inheritable settings that are set.
fn settings_pairs(settings: &CueSettings) -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    if let Some(duration) = &settings.duration {
        pairs.push(("duration".to_string(), quote(&duration.to_string())));
    }
    if let Some(advance) = &settings.advance {
        pairs.push(("advance".to_string(), advance_value(advance)));
    }
    if let Some(transition) = &settings.transition {
        pairs.push(("transition".to_string(), transition_value(transition)));
    }
    if let Some(meta) = &settings.meta {
        pairs.push(("meta".to_string(), value_table(&meta.values)));
    }
    pairs
}

fn advance_value(advance: &AdvanceMode) -> String {
    match advance.flattened() {
        AdvanceMode::FirstOf(modes) => {
            let names: Vec<String> = modes
                .iter()
                .map(|mode| quote(&advance_name(mode)))
                .collect();
            format!("[{}]", names.join(", "))
        }
        single @ (AdvanceMode::Timer | AdvanceMode::Manual | AdvanceMode::AudioEvent(_)) => {
            quote(&advance_name(&single))
        }
    }
}

/// The file name of a non-list advance mode.
fn advance_name(mode: &AdvanceMode) -> String {
    match mode {
        AdvanceMode::Timer => "timer".to_string(),
        AdvanceMode::Manual => "manual".to_string(),
        AdvanceMode::AudioEvent(AudioEventKind::Drop) => "audio-drop".to_string(),
        AdvanceMode::AudioEvent(AudioEventKind::Breakdown) => "audio-breakdown".to_string(),
        // Flattened away by the caller; kept total so the match is.
        AdvanceMode::FirstOf(_) => "timer".to_string(),
    }
}

fn transition_value(transition: &TransitionSpec) -> String {
    let (kind, path) = match &transition.kind {
        TransitionKind::Cut => ("cut", None),
        TransitionKind::Crossfade => ("crossfade", None),
        TransitionKind::FadeThroughBlack => ("fade-through-black", None),
        TransitionKind::Shader(path) => ("shader", Some(path.to_string_lossy().into_owned())),
    };
    let mut pairs = vec![("kind".to_string(), quote(kind))];
    if let Some(path) = path {
        pairs.push(("path".to_string(), quote(&path)));
    }
    let is_cut = matches!(transition.kind, TransitionKind::Cut);
    let zero = transition.duration == crate::duration::DurationSpec::Seconds(0.0);
    // A cut takes no time, so its duration is only written if someone set one.
    if !(is_cut && zero) {
        pairs.push((
            "duration".to_string(),
            quote(&transition.duration.to_string()),
        ));
    }
    inline_table(&pairs)
}

fn value_table(values: &std::collections::BTreeMap<String, ControlValue>) -> String {
    let pairs: Vec<(String, String)> = values
        .iter()
        .map(|(name, value)| (key(name), control_value(value)))
        .collect();
    inline_table(&pairs)
}

fn control_value(value: &ControlValue) -> String {
    match value {
        ControlValue::Float(number) => float(f64::from(*number), &format!("{number:?}")),
        ControlValue::Int(number) => number.to_string(),
        ControlValue::Bool(flag) => flag.to_string(),
        ControlValue::Vec2([x, y]) => format!("[{}, {}]", f32_text(*x), f32_text(*y)),
        ControlValue::Color([r, g, b]) => {
            format!("[{}, {}, {}]", f32_text(*r), f32_text(*g), f32_text(*b))
        }
        ControlValue::Text(text) => quote(text),
    }
}

/// An `f32` as a TOML float, using its shortest round-tripping form.
fn f32_text(number: f32) -> String {
    float(f64::from(number), &format!("{number:?}"))
}

/// `text` (Rust's debug form of a float, which always has a `.` or an
/// exponent) unless the value is not finite, which TOML spells differently.
fn float(number: f64, text: &str) -> String {
    if number.is_nan() {
        "nan".to_string()
    } else if number.is_infinite() {
        if number > 0.0 { "inf" } else { "-inf" }.to_string()
    } else {
        text.to_string()
    }
}

fn inline_table(pairs: &[(String, String)]) -> String {
    if pairs.is_empty() {
        return "{}".to_string();
    }
    let body: Vec<String> = pairs.iter().map(|(k, v)| format!("{k} = {v}")).collect();
    format!("{{ {} }}", body.join(", "))
}

fn loop_mode_name(mode: LoopMode) -> &'static str {
    match mode {
        LoopMode::Stop => "stop",
        LoopMode::Loop => "loop",
        LoopMode::LoopSection => "loop-section",
    }
}

fn anchor_text(anchor: WallClockAnchor) -> String {
    let seconds = anchor.seconds_of_day;
    let (hours, minutes, secs) = (seconds / 3600, seconds % 3600 / 60, seconds % 60);
    if secs == 0 {
        format!("{hours:02}:{minutes:02}")
    } else {
        format!("{hours:02}:{minutes:02}:{secs:02}")
    }
}

/// A bare key if it can be one, else a quoted key.
fn key(name: &str) -> String {
    let bare = !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if bare { name.to_string() } else { quote(name) }
}

/// A TOML basic string.
fn quote(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for ch in text.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            control if control.is_control() => {
                let _ = write!(out, "\\u{:04X}", u32::from(control));
            }
            plain => out.push(plain),
        }
    }
    out.push('"');
    out
}

fn push_line(out: &mut String, line: &str) {
    out.push_str(line);
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::duration::DurationSpec;
    use crate::model::{MetaOverrides, SceneRef, ShuffleSpec};
    use crate::parse::parse;
    use anyhow::{Result, anyhow};
    use std::path::PathBuf;

    /// A playlist that touches every field of the model.
    pub(crate) fn kitchen_sink() -> Playlist {
        let mut playlist = Playlist::new("Office \"lobby\" \\ weekday");
        playlist.loop_mode = LoopMode::LoopSection;
        playlist.anchor = WallClockAnchor::from_hm(9, 0);
        playlist.defaults = CueSettings {
            duration: Some(DurationSpec::Seconds(180.0)),
            advance: Some(AdvanceMode::FirstOf(vec![
                AdvanceMode::Timer,
                AdvanceMode::AudioEvent(AudioEventKind::Drop),
            ])),
            transition: Some(TransitionSpec::crossfade(4.0)),
            meta: Some(MetaOverrides {
                values: [("brightness".to_string(), ControlValue::Float(1.1))].into(),
            }),
        };
        playlist.shuffle = Some(ShuffleSpec {
            seed: u64::MAX,
            no_repeat_window: 10,
        });

        let mut morning = Section::new("Morning");
        morning.defaults.duration = Some(DurationSpec::Random {
            min: Box::new(DurationSpec::Seconds(120.0)),
            max: Box::new(DurationSpec::Bars(64)),
        });
        morning.defaults.meta = Some(MetaOverrides::default());
        morning
            .cues
            .push(Cue::new(SceneRef::Library("warm-tunnel".into())));
        let mut cue = Cue::new(SceneRef::Library("voronoi-geode".into()));
        cue.preset = Some("slow".into());
        cue.media = Some(PathBuf::from("media/loop"));
        morning.cues.push(cue);
        let mut cue = Cue::new(SceneRef::Path(PathBuf::from("scenes/ink.synScene")));
        cue.settings.duration = Some(DurationSpec::Bars(32));
        cue.settings.transition = Some(TransitionSpec::cut());
        cue.settings.advance = Some(AdvanceMode::Manual);
        cue.enabled = false;
        morning.cues.push(cue);

        let mut afternoon = Section::new("Afternoon");
        afternoon.defaults.transition = Some(TransitionSpec {
            kind: TransitionKind::FadeThroughBlack,
            duration: DurationSpec::Seconds(2.0),
        });
        let mut cue = Cue::new(SceneRef::Library("lattice".into()));
        cue.overrides
            .insert("speed".into(), ControlValue::Float(0.6));
        cue.overrides.insert("steps".into(), ControlValue::Int(4));
        cue.overrides
            .insert("mirror".into(), ControlValue::Bool(true));
        cue.overrides
            .insert("pad".into(), ControlValue::Vec2([0.25, -1.0]));
        cue.overrides
            .insert("tint".into(), ControlValue::Color([1.0, 0.5, 0.0]));
        cue.overrides
            .insert("shape".into(), ControlValue::Text("Wave \"A\"".into()));
        cue.overrides
            .insert("odd name".into(), ControlValue::Float(1.0e-7));
        cue.settings.transition = Some(TransitionSpec {
            kind: TransitionKind::Shader(PathBuf::from("transitions/wipe.glsl")),
            duration: DurationSpec::Beats(8),
        });
        cue.settings.advance = Some(AdvanceMode::AudioEvent(AudioEventKind::Breakdown));
        afternoon.cues.push(cue);
        afternoon
            .cues
            .push(Cue::new(SceneRef::Library("ST-XsXXDn".into())));

        playlist.sections.push(morning);
        playlist.sections.push(afternoon);
        playlist.sections.push(Section::new("Empty"));
        playlist
    }

    #[test]
    fn round_trip_gives_an_equal_model() -> Result<()> {
        let original = kitchen_sink();
        let text = to_toml(&original);
        let back = parse(&text).map_err(|e| anyhow!("{e}\n---\n{text}"))?;
        assert_eq!(
            back, original,
            "parse(write(p)) differs from p; text was:\n{text}"
        );
        Ok(())
    }

    #[test]
    fn writing_is_deterministic_and_idempotent() -> Result<()> {
        let original = kitchen_sink();
        let once = to_toml(&original);
        assert_eq!(once, to_toml(&original), "same model, different bytes");
        let twice = to_toml(&parse(&once)?);
        assert_eq!(once, twice, "write(parse(write(p))) is not write(p)");
        Ok(())
    }

    #[test]
    fn the_layout_matches_the_plan_example() {
        let mut playlist = Playlist::new("Office lobby");
        playlist.loop_mode = LoopMode::Loop;
        playlist.defaults.duration = Some(DurationSpec::Seconds(180.0));
        playlist.defaults.transition = Some(TransitionSpec::crossfade(4.0));
        playlist.shuffle = Some(ShuffleSpec {
            seed: 1337,
            no_repeat_window: 10,
        });
        let mut section = Section::new("Morning");
        section.defaults.duration = Some(DurationSpec::Random {
            min: Box::new(DurationSpec::Seconds(120.0)),
            max: Box::new(DurationSpec::Seconds(240.0)),
        });
        section
            .cues
            .push(Cue::new(SceneRef::Library("warm-tunnel".into())));
        let mut cue = Cue::new(SceneRef::Library("ink-drift".into()));
        cue.settings.duration = Some(DurationSpec::Bars(32));
        cue.settings.transition = Some(TransitionSpec::cut());
        section.cues.push(cue);
        playlist.sections.push(section);

        let expected = r#"name = "Office lobby"
loop_mode = "loop"

[defaults]
duration = "3m"
transition = { kind = "crossfade", duration = "4s" }

[shuffle]
seed = 1337
no_repeat_window = 10

[[sections]]
name = "Morning"
defaults = { duration = "rand(2m, 4m)" }
cues = [
  { scene = "warm-tunnel" },
  { scene = "ink-drift", duration = "32 bars", transition = { kind = "cut" } },
]
"#;
        assert_eq!(to_toml(&playlist), expected, "layout drifted from PLAN 7.2");
    }

    #[test]
    fn awkward_floats_round_trip_exactly() -> Result<()> {
        let mut playlist = Playlist::new("floats");
        let mut section = Section::new("s");
        let mut cue = Cue::new(SceneRef::Library("a".into()));
        let values = [
            0.1f32,
            0.3,
            1.0 / 3.0,
            16_777_216.0,
            1.0e-30,
            3.4e38,
            -0.0,
            123.456_79,
        ];
        for (i, value) in values.iter().enumerate() {
            cue.overrides
                .insert(format!("v{i}"), ControlValue::Float(*value));
        }
        section.cues.push(cue);
        playlist.sections.push(section);
        let text = to_toml(&playlist);
        let back = parse(&text)?;
        assert_eq!(back, playlist, "a float changed in transit:\n{text}");
        Ok(())
    }

    #[test]
    fn nested_advance_lists_are_written_flat() -> Result<()> {
        let mut playlist = Playlist::new("n");
        playlist.defaults.advance = Some(AdvanceMode::FirstOf(vec![
            AdvanceMode::Timer,
            AdvanceMode::FirstOf(vec![AdvanceMode::Manual]),
        ]));
        let back = parse(&to_toml(&playlist))?;
        assert_eq!(
            back.defaults.advance,
            Some(AdvanceMode::FirstOf(vec![
                AdvanceMode::Timer,
                AdvanceMode::Manual
            ])),
            "the normal form is a flat list"
        );
        Ok(())
    }

    #[test]
    fn a_default_playlist_writes_just_its_header() -> Result<()> {
        let text = to_toml(&Playlist::new("tiny"));
        assert_eq!(
            text, "name = \"tiny\"\nloop_mode = \"stop\"\n",
            "minimal output"
        );
        assert_eq!(parse(&text)?, Playlist::new("tiny"), "and reads back");
        Ok(())
    }

    #[test]
    fn strings_are_escaped() {
        assert_eq!(
            quote("a\"b\\c\nd"),
            "\"a\\\"b\\\\c\\nd\"",
            "quotes, backslashes, newlines"
        );
        assert_eq!(quote("\u{1}"), "\"\\u0001\"", "control characters");
        assert_eq!(key("plain_key-1"), "plain_key-1", "bare key");
        assert_eq!(key("has space"), "\"has space\"", "quoted key");
    }
}
