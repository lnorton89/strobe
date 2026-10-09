// ============================================================================
// Module:       strobe_playlist::parse
// Description:  Playlist TOML to model, with every complaint carrying the
//               line, column and offending text.
//
// Dependencies: crate::model, crate::duration, crate::error, serde, toml
// ============================================================================

//! Reading a playlist.
//!
//! The file is deserialised into a "raw" mirror of the model in which every
//! value that needs checking is a [`Spanned`] (so it remembers its byte
//! range), then converted by [`Reader`], which turns a bad value into a
//! [`ParseError`] at that value's line and column. Going through serde for
//! the structure and by hand for the values is what lets an error inside a
//! duration string, inside an inline table, inside an array on line 14 say
//! exactly that. `deny_unknown_fields` is on everywhere, so a misspelt key
//! is an error, not a silently ignored setting.
//!
//! The format is the one in PLAN section 7.2. Beyond it:
//!
//! - a duration may also be a bare number of seconds (`duration = 90`);
//! - `advance` is `"timer"`, `"manual"`, `"audio-drop"`,
//!   `"audio-breakdown"`, or an array of those meaning "whichever first";
//! - a transition `{ kind = "crossfade" }` with no duration gets
//!   [`DEFAULT_TRANSITION_SECONDS`]; `{ kind = "shader", path = "x.glsl" }`
//!   names a custom transition;
//! - `anchor = "09:00"` (or `"09:00:30"`);
//! - control values are numbers, booleans, strings, `[x, y]` (a vec2) or
//!   `[r, g, b]` (a colour).

use crate::duration::DurationSpec;
use crate::error::{ParseError, line_column, snippet};
use crate::model::{
    AdvanceMode, AudioEventKind, ControlValue, Cue, CueSettings, LoopMode, MetaOverrides, Playlist,
    SceneRef, Section, ShuffleSpec, TransitionKind, TransitionSpec, WallClockAnchor,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::PathBuf;
use toml::{Spanned, Value};

/// The length given to a non-cut transition written without a duration.
pub const DEFAULT_TRANSITION_SECONDS: f64 = 2.0;

/// The name given to a playlist or section that has none.
const UNTITLED_PLAYLIST: &str = "Untitled playlist";

/// Characters of source quoted next to a syntax error.
const NEAR_LIMIT: usize = 40;

/// Parses playlist text.
pub fn parse(text: &str) -> Result<Playlist, ParseError> {
    let raw: RawPlaylist = toml::from_str(text).map_err(|error| toml_error(text, &error))?;
    Reader { text }.playlist(raw)
}

/// A TOML-level problem (syntax, unknown key, wrong type) as a
/// [`ParseError`].
pub(crate) fn toml_error(text: &str, error: &toml::de::Error) -> ParseError {
    let offset = error.span().map_or(0, |span| span.start);
    let (line, column) = line_column(text, offset);
    ParseError {
        line,
        column,
        message: error.message().trim().to_string(),
        near: snippet(text, offset, NEAR_LIMIT),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPlaylist {
    name: Option<Spanned<String>>,
    loop_mode: Option<Spanned<String>>,
    anchor: Option<Spanned<String>>,
    defaults: Option<RawSettings>,
    shuffle: Option<RawShuffle>,
    #[serde(default)]
    sections: Vec<RawSection>,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawSettings {
    duration: Option<Spanned<Value>>,
    advance: Option<Spanned<Value>>,
    transition: Option<Spanned<RawTransition>>,
    meta: Option<BTreeMap<String, Spanned<Value>>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawTransition {
    kind: Option<Spanned<String>>,
    duration: Option<Spanned<Value>>,
    path: Option<Spanned<String>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawShuffle {
    seed: Option<Spanned<Value>>,
    no_repeat_window: Option<Spanned<i64>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawSection {
    name: Option<Spanned<String>>,
    defaults: Option<RawSettings>,
    #[serde(default)]
    cues: Vec<RawCue>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawCue {
    scene: Spanned<String>,
    preset: Option<Spanned<String>>,
    duration: Option<Spanned<Value>>,
    advance: Option<Spanned<Value>>,
    transition: Option<Spanned<RawTransition>>,
    meta: Option<BTreeMap<String, Spanned<Value>>>,
    overrides: Option<BTreeMap<String, Spanned<Value>>>,
    enabled: Option<Spanned<bool>>,
}

/// Converts raw values into the model, reporting positions in `text`.
struct Reader<'a> {
    text: &'a str,
}

impl Reader<'_> {
    /// An error at byte `offset`.
    fn error(&self, offset: usize, message: impl Into<String>) -> ParseError {
        let (line, column) = line_column(self.text, offset);
        ParseError {
            line,
            column,
            message: message.into(),
            near: String::new(),
        }
    }

    fn playlist(&self, raw: RawPlaylist) -> Result<Playlist, ParseError> {
        let loop_mode = match &raw.loop_mode {
            None => LoopMode::Stop,
            Some(value) => match value.get_ref().as_str() {
                "stop" => LoopMode::Stop,
                "loop" => LoopMode::Loop,
                "loop-section" => LoopMode::LoopSection,
                other => {
                    return Err(self.error(
                        value.span().start,
                        format!(
                            "\"{other}\" is not a loop mode (expected stop, loop or loop-section)"
                        ),
                    ));
                }
            },
        };
        let anchor = raw
            .anchor
            .as_ref()
            .map(|value| self.anchor(value))
            .transpose()?;
        let defaults = match &raw.defaults {
            Some(settings) => self.settings(settings)?,
            None => CueSettings::default(),
        };
        let shuffle = raw
            .shuffle
            .as_ref()
            .map(|shuffle| self.shuffle(shuffle))
            .transpose()?;
        let mut sections = Vec::with_capacity(raw.sections.len());
        for (index, section) in raw.sections.iter().enumerate() {
            sections.push(self.section(section, index)?);
        }
        Ok(Playlist {
            name: raw
                .name
                .map_or_else(|| UNTITLED_PLAYLIST.to_string(), Spanned::into_inner),
            defaults,
            sections,
            anchor,
            loop_mode,
            shuffle,
        })
    }

    fn section(&self, raw: &RawSection, index: usize) -> Result<Section, ParseError> {
        let defaults = match &raw.defaults {
            Some(settings) => self.settings(settings)?,
            None => CueSettings::default(),
        };
        let mut cues = Vec::with_capacity(raw.cues.len());
        for cue in &raw.cues {
            cues.push(self.cue(cue)?);
        }
        Ok(Section {
            name: raw.name.as_ref().map_or_else(
                || format!("Section {}", index + 1),
                |name| name.get_ref().clone(),
            ),
            defaults,
            cues,
        })
    }

    fn cue(&self, raw: &RawCue) -> Result<Cue, ParseError> {
        let scene_text = raw.scene.get_ref();
        if scene_text.trim().is_empty() {
            return Err(self.error(raw.scene.span().start, "a cue's scene name is empty"));
        }
        let settings = self.settings_from(
            raw.duration.as_ref(),
            raw.advance.as_ref(),
            raw.transition.as_ref(),
            raw.meta.as_ref(),
        )?;
        let overrides = match &raw.overrides {
            Some(map) => self.values(map)?,
            None => BTreeMap::new(),
        };
        Ok(Cue {
            scene: SceneRef::parse(scene_text),
            preset: raw.preset.as_ref().map(|preset| preset.get_ref().clone()),
            settings,
            overrides,
            enabled: raw.enabled.as_ref().is_none_or(|flag| *flag.get_ref()),
        })
    }

    fn settings(&self, raw: &RawSettings) -> Result<CueSettings, ParseError> {
        self.settings_from(
            raw.duration.as_ref(),
            raw.advance.as_ref(),
            raw.transition.as_ref(),
            raw.meta.as_ref(),
        )
    }

    fn settings_from(
        &self,
        duration: Option<&Spanned<Value>>,
        advance: Option<&Spanned<Value>>,
        transition: Option<&Spanned<RawTransition>>,
        meta: Option<&BTreeMap<String, Spanned<Value>>>,
    ) -> Result<CueSettings, ParseError> {
        Ok(CueSettings {
            duration: duration.map(|value| self.duration(value)).transpose()?,
            advance: advance.map(|value| self.advance(value)).transpose()?,
            transition: transition.map(|value| self.transition(value)).transpose()?,
            meta: match meta {
                Some(map) => Some(MetaOverrides {
                    values: self.values(map)?,
                }),
                None => None,
            },
        })
    }

    /// A duration written as a string or a bare number of seconds.
    fn duration(&self, value: &Spanned<Value>) -> Result<DurationSpec, ParseError> {
        let start = value.span().start;
        let text = match value.get_ref() {
            Value::String(text) => text.clone(),
            Value::Integer(seconds) => format!("{seconds}s"),
            Value::Float(seconds) => format!("{seconds}s"),
            other
            @ (Value::Boolean(_) | Value::Datetime(_) | Value::Array(_) | Value::Table(_)) => {
                return Err(self.error(
                    start,
                    format!(
                        "{other} is not a duration (write a string like \"3m\" or a number of seconds)"
                    ),
                ));
            }
        };
        // Past the opening quote of a string; a bare number has no quote.
        let inside = usize::from(matches!(value.get_ref(), Value::String(_)));
        DurationSpec::parse(&text)
            .map_err(|error| self.error(start + inside + error.offset, error.to_string()))
    }

    fn advance(&self, value: &Spanned<Value>) -> Result<AdvanceMode, ParseError> {
        let start = value.span().start;
        let one = |name: &Value| -> Result<AdvanceMode, ParseError> {
            let Value::String(name) = name else {
                return Err(self.error(start, format!("{name} is not an advance mode")));
            };
            match name.as_str() {
                "timer" => Ok(AdvanceMode::Timer),
                "manual" => Ok(AdvanceMode::Manual),
                "audio-drop" => Ok(AdvanceMode::AudioEvent(AudioEventKind::Drop)),
                "audio-breakdown" => Ok(AdvanceMode::AudioEvent(AudioEventKind::Breakdown)),
                other => Err(self.error(
                    start + 1,
                    format!(
                        "\"{other}\" is not an advance mode (expected timer, manual, audio-drop, audio-breakdown, or a list of those)"
                    ),
                )),
            }
        };
        match value.get_ref() {
            Value::Array(items) => {
                if items.is_empty() {
                    return Err(self.error(start, "an advance list needs at least one mode"));
                }
                let modes = items.iter().map(one).collect::<Result<Vec<_>, _>>()?;
                Ok(AdvanceMode::FirstOf(modes))
            }
            other @ (Value::String(_)
            | Value::Integer(_)
            | Value::Float(_)
            | Value::Boolean(_)
            | Value::Datetime(_)
            | Value::Table(_)) => one(other),
        }
    }

    fn transition(&self, raw: &Spanned<RawTransition>) -> Result<TransitionSpec, ParseError> {
        let start = raw.span().start;
        let table = raw.get_ref();
        let Some(kind_value) = &table.kind else {
            return Err(self.error(
                start,
                "a transition needs a kind (cut, crossfade, fade-through-black or shader)",
            ));
        };
        let kind_start = kind_value.span().start;
        let kind = match kind_value.get_ref().as_str() {
            "cut" => TransitionKind::Cut,
            "crossfade" => TransitionKind::Crossfade,
            "fade-through-black" => TransitionKind::FadeThroughBlack,
            "shader" => match &table.path {
                Some(path) => TransitionKind::Shader(PathBuf::from(path.get_ref())),
                None => {
                    return Err(self.error(kind_start, "a shader transition needs a `path`"));
                }
            },
            other => {
                return Err(self.error(
                    kind_start,
                    format!(
                        "\"{other}\" is not a transition kind (expected cut, crossfade, fade-through-black or shader)"
                    ),
                ));
            }
        };
        if let (Some(path), false) = (&table.path, matches!(kind, TransitionKind::Shader(_))) {
            return Err(self.error(
                path.span().start,
                "`path` only applies to a shader transition",
            ));
        }
        let duration = match &table.duration {
            Some(value) => self.duration(value)?,
            None => match kind {
                TransitionKind::Cut => DurationSpec::Seconds(0.0),
                TransitionKind::Crossfade
                | TransitionKind::FadeThroughBlack
                | TransitionKind::Shader(_) => DurationSpec::Seconds(DEFAULT_TRANSITION_SECONDS),
            },
        };
        Ok(TransitionSpec { kind, duration })
    }

    fn values(
        &self,
        map: &BTreeMap<String, Spanned<Value>>,
    ) -> Result<BTreeMap<String, ControlValue>, ParseError> {
        map.iter()
            .map(|(name, value)| Ok((name.clone(), self.control_value(value)?)))
            .collect()
    }

    fn control_value(&self, value: &Spanned<Value>) -> Result<ControlValue, ParseError> {
        let start = value.span().start;
        let bad = |what: &dyn std::fmt::Display| {
            self.error(
                start,
                format!(
                    "{what} is not a control value (use a number, true/false, \"text\", [x, y] or [r, g, b])"
                ),
            )
        };
        let finite = |number: f64| -> Result<f32, ParseError> {
            if number.is_finite() {
                Ok(number as f32)
            } else {
                Err(self.error(start, "control values must be finite numbers"))
            }
        };
        match value.get_ref() {
            Value::Float(number) => Ok(ControlValue::Float(finite(*number)?)),
            Value::Integer(number) => Ok(ControlValue::Int(*number)),
            Value::Boolean(flag) => Ok(ControlValue::Bool(*flag)),
            Value::String(text) => Ok(ControlValue::Text(text.clone())),
            Value::Array(items) => {
                let mut numbers = Vec::with_capacity(items.len());
                for item in items {
                    match item {
                        Value::Float(number) => numbers.push(finite(*number)?),
                        Value::Integer(number) => numbers.push(finite(*number as f64)?),
                        Value::String(_)
                        | Value::Boolean(_)
                        | Value::Datetime(_)
                        | Value::Array(_)
                        | Value::Table(_) => return Err(bad(&value.get_ref())),
                    }
                }
                match numbers.as_slice() {
                    [x, y] => Ok(ControlValue::Vec2([*x, *y])),
                    [r, g, b] => Ok(ControlValue::Color([*r, *g, *b])),
                    _ => Err(bad(&value.get_ref())),
                }
            }
            other @ (Value::Datetime(_) | Value::Table(_)) => Err(bad(other)),
        }
    }

    fn shuffle(&self, raw: &RawShuffle) -> Result<ShuffleSpec, ParseError> {
        let seed = match &raw.seed {
            None => 0,
            Some(value) => {
                let start = value.span().start;
                match value.get_ref() {
                    Value::Integer(number) => u64::try_from(*number).map_err(|_| {
                        self.error(
                            start,
                            format!("{number} is not a seed (seeds are 0 or more)"),
                        )
                    })?,
                    Value::String(text) => text.trim().parse::<u64>().map_err(|_| {
                        self.error(
                            start + 1,
                            format!("\"{text}\" is not a seed (a whole number)"),
                        )
                    })?,
                    other @ (Value::Float(_)
                    | Value::Boolean(_)
                    | Value::Datetime(_)
                    | Value::Array(_)
                    | Value::Table(_)) => {
                        return Err(
                            self.error(start, format!("{other} is not a seed (a whole number)"))
                        );
                    }
                }
            }
        };
        let no_repeat_window = match &raw.no_repeat_window {
            None => 0,
            Some(value) => usize::try_from(*value.get_ref()).map_err(|_| {
                self.error(
                    value.span().start,
                    format!("{} is not a window size (0 or more)", value.get_ref()),
                )
            })?,
        };
        Ok(ShuffleSpec {
            seed,
            no_repeat_window,
        })
    }

    fn anchor(&self, value: &Spanned<String>) -> Result<WallClockAnchor, ParseError> {
        let text = value.get_ref();
        let bad = || {
            self.error(
                value.span().start + 1,
                format!("\"{text}\" is not a time of day (use HH:MM or HH:MM:SS)"),
            )
        };
        let parts: Vec<&str> = text.trim().split(':').collect();
        let number = |part: &str| -> Option<u32> {
            (part.len() == 2 && part.bytes().all(|b| b.is_ascii_digit()))
                .then(|| part.parse().ok())
                .flatten()
        };
        let (hours, minutes, seconds) = match parts.as_slice() {
            [h, m] => (number(h), number(m), Some(0)),
            [h, m, s] => (number(h), number(m), number(s)),
            _ => return Err(bad()),
        };
        match (hours, minutes, seconds) {
            (Some(h), Some(m), Some(s)) if h < 24 && m < 60 && s < 60 => Ok(WallClockAnchor {
                seconds_of_day: h * 3600 + m * 60 + s,
            }),
            _ => Err(bad()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, anyhow};

    fn err_of(text: &str) -> Result<ParseError> {
        match parse(text) {
            Ok(_) => Err(anyhow!("expected an error for:\n{text}")),
            Err(error) => Ok(error),
        }
    }

    #[test]
    fn the_plan_example_parses() -> Result<()> {
        let text = r#"
name = "Office lobby"
loop_mode = "loop"

[defaults]
duration   = "3m"
advance    = "timer"
transition = { kind = "crossfade", duration = "4s" }

[shuffle]
seed = 1337
no_repeat_window = 10

[[sections]]
name = "Morning"
defaults = { duration = "rand(2m, 4m)" }
cues = [
  { scene = "warm-tunnel" },
  { scene = "voronoi-geode", preset = "slow" },
  { scene = "ink-drift", duration = "32 bars", transition = { kind = "cut" } },
]

[[sections]]
name = "Afternoon"
defaults = { duration = "90s", transition = { kind = "fade-through-black", duration = "2s" } }
cues = [
  { scene = "lattice", overrides = { speed = 0.6 } },
  { scene = "ST-XsXXDn" },
]
"#;
        let playlist = parse(text)?;
        assert_eq!(playlist.name, "Office lobby", "name");
        assert_eq!(playlist.loop_mode, LoopMode::Loop, "loop mode");
        assert_eq!(
            playlist.shuffle,
            Some(ShuffleSpec {
                seed: 1337,
                no_repeat_window: 10
            }),
            "shuffle"
        );
        assert_eq!(
            playlist.defaults.duration,
            Some(DurationSpec::Seconds(180.0)),
            "default duration"
        );
        assert_eq!(playlist.sections.len(), 2, "two sections");
        assert_eq!(playlist.cue_count(), 5, "five cues");
        let ink = playlist
            .sections
            .first()
            .and_then(|s| s.cues.get(2))
            .ok_or_else(|| anyhow!("no ink-drift cue"))?;
        assert_eq!(
            ink.settings.duration,
            Some(DurationSpec::Bars(32)),
            "32 bars"
        );
        assert_eq!(ink.settings.transition, Some(TransitionSpec::cut()), "cut");
        let lattice = playlist
            .sections
            .get(1)
            .and_then(|s| s.cues.first())
            .ok_or_else(|| anyhow!("no lattice cue"))?;
        assert_eq!(
            lattice.overrides.get("speed"),
            Some(&ControlValue::Float(0.6)),
            "override value"
        );
        Ok(())
    }

    #[test]
    fn an_empty_file_is_an_empty_untitled_playlist() -> Result<()> {
        let playlist = parse("")?;
        assert_eq!(playlist.name, UNTITLED_PLAYLIST, "default name");
        assert!(playlist.sections.is_empty(), "no sections");
        assert_eq!(playlist.loop_mode, LoopMode::Stop, "default loop mode");
        Ok(())
    }

    #[test]
    fn a_bad_duration_is_reported_at_its_line_and_column() -> Result<()> {
        let text = "name = \"x\"\n\n[[sections]]\nname = \"s\"\ncues = [\n  { scene = \"a\", duration = \"3mn\" },\n]\n";
        let error = err_of(text)?;
        assert_eq!(error.line, 6, "line of the bad duration");
        assert_eq!(error.column, 30, "column of the 3 inside the quotes");
        assert!(
            error.to_string().contains("\"3mn\" is not a duration"),
            "the message should quote the text: {error}"
        );
        Ok(())
    }

    #[test]
    fn an_error_inside_rand_points_into_the_string() -> Result<()> {
        let error = err_of("[defaults]\nduration = \"rand(1m, 3mn)\"\n")?;
        assert_eq!(error.line, 2, "line");
        // `duration = "` is 12 characters; the 3 of `3mn` is at offset 9 in the string.
        assert_eq!(error.column, 12 + 9 + 1, "column of the offending argument");
        assert!(
            error.to_string().contains("\"3mn\""),
            "quotes the inner text: {error}"
        );
        Ok(())
    }

    #[test]
    fn numbers_are_seconds() -> Result<()> {
        let playlist = parse("[defaults]\nduration = 90\n")?;
        assert_eq!(
            playlist.defaults.duration,
            Some(DurationSpec::Seconds(90.0)),
            "integer seconds"
        );
        let playlist = parse("[defaults]\nduration = 1.5\n")?;
        assert_eq!(
            playlist.defaults.duration,
            Some(DurationSpec::Seconds(1.5)),
            "float seconds"
        );
        assert!(
            parse("[defaults]\nduration = -1\n").is_err(),
            "negative seconds"
        );
        assert!(
            parse("[defaults]\nduration = true\n").is_err(),
            "a boolean is not a duration"
        );
        Ok(())
    }

    #[test]
    fn unknown_keys_and_syntax_errors_carry_positions() -> Result<()> {
        let error = err_of("name = \"x\"\nlooop_mode = \"loop\"\n")?;
        assert_eq!(error.line, 2, "unknown key line");
        assert!(
            error.to_string().contains("looop_mode"),
            "names the key: {error}"
        );

        let error = err_of("name = \"x\"\n[[sections]\n")?;
        assert_eq!(error.line, 2, "syntax error line");
        assert!(
            error.near.starts_with("[[sections]"),
            "quotes the source: {error}"
        );

        let error = err_of("[[sections]]\ncues = [ { preset = \"x\" } ]\n")?;
        assert_eq!(error.line, 2, "a cue with no scene");
        assert!(
            error.to_string().contains("scene"),
            "names the missing field: {error}"
        );
        Ok(())
    }

    #[test]
    fn enumerated_values_name_what_was_expected() -> Result<()> {
        for (text, needle) in [
            ("loop_mode = \"forever\"", "\"forever\" is not a loop mode"),
            (
                "[defaults]\nadvance = \"tmer\"",
                "\"tmer\" is not an advance mode",
            ),
            ("[defaults]\nadvance = []", "at least one mode"),
            (
                "[defaults]\ntransition = { kind = \"wipe\" }",
                "\"wipe\" is not a transition kind",
            ),
            (
                "[defaults]\ntransition = { duration = \"1s\" }",
                "needs a kind",
            ),
            (
                "[defaults]\ntransition = { kind = \"shader\" }",
                "needs a `path`",
            ),
            (
                "[defaults]\ntransition = { kind = \"cut\", path = \"x\" }",
                "only applies to a shader",
            ),
            ("anchor = \"25:00\"", "\"25:00\" is not a time of day"),
            ("anchor = \"9:00\"", "is not a time of day"),
            ("[shuffle]\nseed = -3", "is not a seed"),
            ("[shuffle]\nno_repeat_window = -1", "not a window size"),
            (
                "[[sections]]\ncues = [ { scene = \"a\", overrides = { x = [1] } } ]",
                "not a control value",
            ),
            (
                "[[sections]]\ncues = [ { scene = \"a\", overrides = { x = { y = 1 } } } ]",
                "not a control value",
            ),
            (
                "[[sections]]\ncues = [ { scene = \"\" } ]",
                "scene name is empty",
            ),
        ] {
            let error = err_of(text)?;
            assert!(
                error.to_string().contains(needle),
                "for {text:?} expected a message containing {needle:?}, got: {error}"
            );
        }
        Ok(())
    }

    #[test]
    fn advance_lists_and_audio_events_parse() -> Result<()> {
        let playlist = parse(
            "[defaults]\nadvance = [\"timer\", \"audio-drop\"]\n[[sections]]\ncues = [ { scene = \"a\", advance = \"audio-breakdown\" } ]\n",
        )?;
        assert_eq!(
            playlist.defaults.advance,
            Some(AdvanceMode::FirstOf(vec![
                AdvanceMode::Timer,
                AdvanceMode::AudioEvent(AudioEventKind::Drop)
            ])),
            "list"
        );
        let cue = playlist.sections.first().and_then(|s| s.cues.first());
        assert_eq!(
            cue.and_then(|c| c.settings.advance.clone()),
            Some(AdvanceMode::AudioEvent(AudioEventKind::Breakdown)),
            "single event"
        );
        Ok(())
    }

    #[test]
    fn transitions_default_sensibly() -> Result<()> {
        let playlist = parse(
            "[defaults]\ntransition = { kind = \"crossfade\" }\n[[sections]]\ncues = [ { scene = \"a\", transition = { kind = \"shader\", path = \"t/wipe.glsl\", duration = \"1s\" } } ]\n",
        )?;
        assert_eq!(
            playlist.defaults.transition,
            Some(TransitionSpec::crossfade(DEFAULT_TRANSITION_SECONDS)),
            "a crossfade with no duration"
        );
        let cue = playlist.sections.first().and_then(|s| s.cues.first());
        assert_eq!(
            cue.and_then(|c| c.settings.transition.clone()),
            Some(TransitionSpec {
                kind: TransitionKind::Shader(PathBuf::from("t/wipe.glsl")),
                duration: DurationSpec::Seconds(1.0)
            }),
            "shader transition"
        );
        Ok(())
    }

    #[test]
    fn control_values_cover_the_ssf_kinds() -> Result<()> {
        let playlist = parse(
            "[[sections]]\ncues = [ { scene = \"a\", overrides = { f = 0.5, i = 3, b = true, p = [0.5, 1], c = [1, 0, 0.25], t = \"Wave\" } } ]\n",
        )?;
        let cue = playlist
            .sections
            .first()
            .and_then(|s| s.cues.first())
            .ok_or_else(|| anyhow!("no cue"))?;
        let get = |name: &str| cue.overrides.get(name).cloned();
        assert_eq!(get("f"), Some(ControlValue::Float(0.5)), "float");
        assert_eq!(get("i"), Some(ControlValue::Int(3)), "int");
        assert_eq!(get("b"), Some(ControlValue::Bool(true)), "bool");
        assert_eq!(get("p"), Some(ControlValue::Vec2([0.5, 1.0])), "vec2");
        assert_eq!(
            get("c"),
            Some(ControlValue::Color([1.0, 0.0, 0.25])),
            "colour"
        );
        assert_eq!(get("t"), Some(ControlValue::Text("Wave".into())), "text");
        Ok(())
    }

    #[test]
    fn anchors_with_and_without_seconds_parse() -> Result<()> {
        assert_eq!(
            parse("anchor = \"09:00\"")?.anchor,
            WallClockAnchor::from_hm(9, 0),
            "HH:MM"
        );
        assert_eq!(
            parse("anchor = \"09:00:30\"")?.anchor,
            Some(WallClockAnchor {
                seconds_of_day: 32_430
            }),
            "HH:MM:SS"
        );
        Ok(())
    }

    #[test]
    fn large_seeds_may_be_written_as_strings() -> Result<()> {
        let playlist = parse("[shuffle]\nseed = \"18446744073709551615\"\n")?;
        assert_eq!(
            playlist.shuffle.map(|s| s.seed),
            Some(u64::MAX),
            "a seed beyond TOML's i64"
        );
        Ok(())
    }

    #[test]
    fn disabled_cues_and_scene_paths_parse() -> Result<()> {
        let playlist =
            parse("[[sections]]\ncues = [ { scene = \"scenes/a.synScene\", enabled = false } ]\n")?;
        let cue = playlist
            .sections
            .first()
            .and_then(|s| s.cues.first())
            .ok_or_else(|| anyhow!("no cue"))?;
        assert!(!cue.enabled, "enabled = false");
        assert_eq!(
            cue.scene,
            SceneRef::Path(PathBuf::from("scenes/a.synScene")),
            "path scene"
        );
        Ok(())
    }
}
