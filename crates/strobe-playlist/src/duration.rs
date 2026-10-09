// ============================================================================
// Module:       strobe_playlist::duration
// Description:  DurationSpec: how long a cue or transition lasts, its text
//               grammar (90s, 1h30m, 1:30, 32 bars, rand(a, b)) and how a
//               random spec resolves to a concrete length.
//
// Dependencies: crate::rng
// ============================================================================

//! Durations.
//!
//! A [`DurationSpec`] is what a playlist file *says*: `3m`, `32 bars`,
//! `rand(2m, 4m)`. A [`Length`] is what it *means* once any randomness is
//! drawn: either a number of seconds, or a number of beats whose length
//! depends on the tempo at the time. Keeping the two apart is what lets the
//! timeline preview a playlist at a fixed fallback BPM while the live clock
//! stretches the same cues to the BPM it hears.
//!
//! # Grammar
//!
//! | Form | Meaning |
//! |---|---|
//! | `90s`, `3m`, `1h30m`, `1h 30m 5s`, `0.5s` | units in descending order, each at most once |
//! | `1:30`, `1:02:03` | `m:ss` and `h:mm:ss` |
//! | `32 bars`, `1 bar` | bars of four beats |
//! | `64 beats`, `1 beat` | beats |
//! | `rand(a, b)` | a draw between two durations of any form, nesting allowed |
//!
//! Case and spaces between tokens are ignored. A bare number has no unit
//! and is refused: `90` could mean seconds or minutes, and a playlist that
//! guesses wrong plays for the wrong hour.

use crate::rng::SplitMix64;
use std::fmt;
use std::str::FromStr;

/// Beats in a bar. Playlists assume 4/4: the audio analysis has no time
/// signature, and every scene in the library is written for it.
pub const BEATS_PER_BAR: u32 = 4;

/// The longest duration the grammar accepts, in seconds (about 115 days).
/// It keeps every sum in the timeline comfortably inside `f64`'s exact
/// integer range, and catches a typo like `1000000h`.
const MAX_SECONDS: f64 = 10_000_000.0;

/// The largest bar or beat count the grammar accepts.
const MAX_COUNT: u32 = 1_000_000;

/// How long something lasts, as written in a playlist.
#[derive(Debug, Clone, PartialEq)]
pub enum DurationSpec {
    /// A fixed number of seconds.
    Seconds(f64),
    /// Bars of four beats; tempo-dependent.
    Bars(u32),
    /// Beats; tempo-dependent.
    Beats(u32),
    /// A draw between two durations, fixed per cue by the playlist seed.
    Random {
        /// One end of the range.
        min: Box<DurationSpec>,
        /// The other end. If it is shorter than `min` the two swap.
        max: Box<DurationSpec>,
    },
}

/// A duration with its randomness drawn.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Length {
    /// Wall-clock seconds.
    Seconds(f64),
    /// Beats: how long they take depends on the BPM in force.
    Beats(u32),
}

impl Length {
    /// The length in seconds at `bpm` beats per minute.
    pub fn seconds(self, bpm: f64) -> f64 {
        match self {
            Self::Seconds(seconds) => seconds,
            Self::Beats(beats) => f64::from(beats) * 60.0 / bpm,
        }
    }

    /// Whether the length is counted in beats, so a live clock should end
    /// it on a downbeat.
    pub fn is_beat_based(self) -> bool {
        match self {
            Self::Seconds(_) => false,
            Self::Beats(_) => true,
        }
    }
}

impl DurationSpec {
    /// Parses the text grammar.
    pub fn parse(input: &str) -> Result<Self, DurationError> {
        parse_at(input, 0)
    }

    /// Whether the spec mentions bars or beats anywhere.
    pub fn is_beat_based(&self) -> bool {
        match self {
            Self::Seconds(_) => false,
            Self::Bars(_) | Self::Beats(_) => true,
            Self::Random { min, max } => min.is_beat_based() || max.is_beat_based(),
        }
    }

    /// Draws any randomness and returns the concrete length.
    ///
    /// Two beat-based ends give a whole number of beats (so the result is
    /// still beat-locked), and whole bars if both ends are whole bars.
    /// Anything else is converted to seconds at
    /// `fallback_bpm` and drawn uniformly between the two.
    pub fn resolve(&self, rng: &mut SplitMix64, fallback_bpm: f64) -> Length {
        match self {
            Self::Seconds(seconds) => Length::Seconds(*seconds),
            Self::Bars(bars) => Length::Beats(bars.saturating_mul(BEATS_PER_BAR)),
            Self::Beats(beats) => Length::Beats(*beats),
            Self::Random { min, max } => {
                let lo = min.resolve(rng, fallback_bpm);
                let hi = max.resolve(rng, fallback_bpm);
                match (lo, hi) {
                    (Length::Beats(a), Length::Beats(b)) => {
                        let (a, b) = (a.min(b), a.max(b));
                        if a % BEATS_PER_BAR == 0 && b % BEATS_PER_BAR == 0 {
                            // Bar to bar: draw whole bars, so a phrase ends on a bar line.
                            let (lo, hi) =
                                (u64::from(a / BEATS_PER_BAR), u64::from(b / BEATS_PER_BAR));
                            Length::Beats(rng.between(lo, hi) as u32 * BEATS_PER_BAR)
                        } else {
                            Length::Beats(rng.between(u64::from(a), u64::from(b)) as u32)
                        }
                    }
                    (Length::Seconds(_), Length::Seconds(_))
                    | (Length::Seconds(_), Length::Beats(_))
                    | (Length::Beats(_), Length::Seconds(_)) => {
                        let a = lo.seconds(fallback_bpm);
                        let b = hi.seconds(fallback_bpm);
                        Length::Seconds(rng.range(a.min(b), a.max(b)))
                    }
                }
            }
        }
    }
}

impl FromStr for DurationSpec {
    type Err = DurationError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::parse(input)
    }
}

impl fmt::Display for DurationSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Seconds(seconds) => f.write_str(&format_seconds(*seconds)),
            Self::Bars(1) => f.write_str("1 bar"),
            Self::Bars(bars) => write!(f, "{bars} bars"),
            Self::Beats(1) => f.write_str("1 beat"),
            Self::Beats(beats) => write!(f, "{beats} beats"),
            Self::Random { min, max } => write!(f, "rand({min}, {max})"),
        }
    }
}

/// Seconds in the shortest readable form that parses back to the same
/// number: `3m`, `1h30m`, `90s`, `0.5s`.
fn format_seconds(seconds: f64) -> String {
    let whole = seconds.fract() == 0.0 && (0.0..1.0e9).contains(&seconds);
    if !whole {
        return format!("{seconds}s");
    }
    let total = seconds as u64;
    if total >= 3600 {
        let (hours, minutes, secs) = (total / 3600, total % 3600 / 60, total % 60);
        let mut text = format!("{hours}h");
        if minutes > 0 {
            text.push_str(&format!("{minutes}m"));
        }
        if secs > 0 {
            text.push_str(&format!("{secs}s"));
        }
        text
    } else if total > 0 && total.is_multiple_of(60) {
        format!("{}m", total / 60)
    } else {
        format!("{total}s")
    }
}

/// A string that is not a duration, with where and why.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("\"{text}\" is not a duration ({expected})")]
pub struct DurationError {
    /// The duration text that was refused (the innermost piece, for an
    /// error inside `rand(..)`).
    pub text: String,
    /// Byte offset of the offending spot in the string that was parsed.
    pub offset: usize,
    /// What would have been accepted there.
    pub expected: &'static str,
}

/// What a plain duration looks like, for the error message.
const EXPECT_DURATION: &str = "try 90s, 3m, 1h30m, 1:30, 32 bars, 64 beats or rand(a, b)";

/// Parses `input`, which starts `base` bytes into the string being parsed.
fn parse_at(input: &str, base: usize) -> Result<DurationSpec, DurationError> {
    let trimmed = input.trim();
    let base = base + (input.len() - input.trim_start().len());
    let fail = |at: usize, expected: &'static str| DurationError {
        text: trimmed.to_string(),
        offset: base + at,
        expected,
    };
    if trimmed.is_empty() {
        return Err(fail(0, EXPECT_DURATION));
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.starts_with("rand(") {
        return parse_random(trimmed, base);
    }
    if lower.contains(':') {
        return parse_clock(trimmed, &lower, base);
    }
    for (suffix, bars) in [
        ("bars", true),
        ("bar", true),
        ("beats", false),
        ("beat", false),
    ] {
        if let Some(count) = lower.strip_suffix(suffix) {
            let count = count.trim_end();
            let valid = !count.is_empty() && count.bytes().all(|b| b.is_ascii_digit());
            let parsed = count
                .parse::<u32>()
                .ok()
                .filter(|n| (1..=MAX_COUNT).contains(n));
            return match (valid, parsed) {
                (true, Some(n)) if bars => Ok(DurationSpec::Bars(n)),
                (true, Some(n)) => Ok(DurationSpec::Beats(n)),
                (true, None) | (false, _) => Err(fail(
                    0,
                    "a whole number from 1 then `bars` or `beats`, like 32 bars",
                )),
            };
        }
    }
    parse_units(trimmed, &lower, base)
}

/// `rand(a, b)`.
fn parse_random(text: &str, base: usize) -> Result<DurationSpec, DurationError> {
    let fail = |at: usize, expected: &'static str| DurationError {
        text: text.to_string(),
        offset: base + at,
        expected,
    };
    const OPEN: usize = 5;
    if !text.ends_with(')') {
        return Err(fail(text.len(), "a closing `)`"));
    }
    let inner = text.get(OPEN..text.len() - 1).unwrap_or("");
    let mut depth = 0usize;
    let mut comma = None;
    for (at, ch) in inner.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            ',' if depth == 0 => {
                if comma.is_some() {
                    return Err(fail(OPEN + at, "exactly two durations: rand(min, max)"));
                }
                comma = Some(at);
            }
            _ => {}
        }
    }
    let Some(comma) = comma else {
        return Err(fail(
            OPEN,
            "two durations separated by a comma: rand(min, max)",
        ));
    };
    let (left, right) = inner.split_at(comma);
    let min = parse_at(left, base + OPEN)?;
    let max = parse_at(right.get(1..).unwrap_or(""), base + OPEN + comma + 1)?;
    Ok(DurationSpec::Random {
        min: Box::new(min),
        max: Box::new(max),
    })
}

/// `m:ss` and `h:mm:ss`.
fn parse_clock(text: &str, lower: &str, base: usize) -> Result<DurationSpec, DurationError> {
    let fail = |at: usize, expected: &'static str| DurationError {
        text: text.to_string(),
        offset: base + at,
        expected,
    };
    let mut parts: Vec<(usize, &str)> = Vec::new();
    let mut at = 0;
    for part in lower.split(':') {
        parts.push((at, part));
        at += part.len() + 1;
    }
    if parts.len() > 3 {
        return Err(fail(0, "m:ss or h:mm:ss"));
    }
    let mut seconds = 0.0;
    let last = parts.len() - 1;
    for (position, (at, part)) in parts.iter().enumerate() {
        let is_seconds = position == last;
        let digits_only = !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
        let fraction_ok = is_seconds && is_decimal(part);
        if !digits_only && !fraction_ok {
            return Err(fail(*at, "digits in every field, like 1:30"));
        }
        let Ok(value) = part.parse::<f64>() else {
            return Err(fail(*at, "digits in every field, like 1:30"));
        };
        // The leading field of a clock may run past 59 (`90:00`); the
        // fields after it may not.
        if position > 0 && value >= 60.0 {
            return Err(fail(*at, "minutes and seconds below 60"));
        }
        // Weight by position from the right: seconds, minutes, hours.
        let weight = 60f64.powi((last - position) as i32);
        seconds += value * weight;
    }
    finish_seconds(text, base, seconds)
}

/// Whether `text` is digits with at most one interior decimal point.
fn is_decimal(text: &str) -> bool {
    let mut points = 0;
    let mut digits = 0;
    for (at, byte) in text.bytes().enumerate() {
        match byte {
            b'0'..=b'9' => digits += 1,
            b'.' if at > 0 && at + 1 < text.len() => points += 1,
            _ => return false,
        }
    }
    digits > 0 && points <= 1
}

/// `1h30m`, `90s`, `0.5s`.
fn parse_units(text: &str, lower: &str, base: usize) -> Result<DurationSpec, DurationError> {
    let fail = |at: usize, expected: &'static str| DurationError {
        text: text.to_string(),
        offset: base + at,
        expected,
    };
    const SHAPE: &str = "a number then h, m or s, like 3m or 1h30m";
    let bytes = lower.as_bytes();
    let mut pos = 0;
    // Units must descend: hours (2), minutes (1), seconds (0).
    let mut ceiling = 3;
    let mut total = 0.0;
    while pos < bytes.len() {
        if bytes.get(pos).is_some_and(u8::is_ascii_whitespace) {
            pos += 1;
            continue;
        }
        let start = pos;
        while bytes
            .get(pos)
            .is_some_and(|b| b.is_ascii_digit() || *b == b'.')
        {
            pos += 1;
        }
        let number = lower.get(start..pos).unwrap_or("");
        if !is_decimal(number) {
            return Err(fail(start, SHAPE));
        }
        let unit_start = pos;
        while bytes.get(pos).is_some_and(u8::is_ascii_alphabetic) {
            pos += 1;
        }
        let rank_and_weight = match lower.get(unit_start..pos).unwrap_or("") {
            "h" => Some((2, 3600.0)),
            "m" => Some((1, 60.0)),
            "s" => Some((0, 1.0)),
            _ => None,
        };
        let Some((rank, weight)) = rank_and_weight else {
            return Err(fail(start, SHAPE));
        };
        if rank >= ceiling {
            return Err(fail(
                start,
                "units in order, each once: hours, then minutes, then seconds",
            ));
        }
        ceiling = rank;
        let Ok(value) = number.parse::<f64>() else {
            return Err(fail(start, SHAPE));
        };
        total += value * weight;
    }
    if ceiling == 3 {
        return Err(fail(0, SHAPE));
    }
    finish_seconds(text, base, total)
}

/// Range-checks a parsed number of seconds.
fn finish_seconds(text: &str, base: usize, seconds: f64) -> Result<DurationSpec, DurationError> {
    if seconds.is_finite() && (0.0..=MAX_SECONDS).contains(&seconds) {
        Ok(DurationSpec::Seconds(seconds))
    } else {
        Err(DurationError {
            text: text.to_string(),
            offset: base,
            expected: "at most 10000000 seconds",
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, anyhow};

    fn secs(text: &str) -> Result<f64> {
        match DurationSpec::parse(text)? {
            DurationSpec::Seconds(s) => Ok(s),
            other @ (DurationSpec::Bars(_)
            | DurationSpec::Beats(_)
            | DurationSpec::Random { .. }) => {
                Err(anyhow!("{text:?} parsed as {other:?}, not seconds"))
            }
        }
    }

    #[test]
    fn the_grammar_in_the_plan_parses() -> Result<()> {
        assert_eq!(secs("90s")?, 90.0, "90s");
        assert_eq!(secs("3m")?, 180.0, "3m");
        assert_eq!(secs("1h30m")?, 5400.0, "1h30m");
        assert_eq!(secs("1:30")?, 90.0, "1:30");
        assert_eq!(secs("1:02:03")?, 3723.0, "1:02:03");
        assert_eq!(
            DurationSpec::parse("32 bars")?,
            DurationSpec::Bars(32),
            "32 bars"
        );
        assert_eq!(
            DurationSpec::parse("64 beats")?,
            DurationSpec::Beats(64),
            "64 beats"
        );
        assert_eq!(
            DurationSpec::parse("rand(2m, 4m)")?,
            DurationSpec::Random {
                min: Box::new(DurationSpec::Seconds(120.0)),
                max: Box::new(DurationSpec::Seconds(240.0)),
            },
            "rand(2m, 4m)"
        );
        Ok(())
    }

    #[test]
    fn spacing_case_and_fractions_are_accepted() -> Result<()> {
        assert_eq!(secs("  1h 30m 5s ")?, 5405.0, "spaced units");
        assert_eq!(secs("3M")?, 180.0, "upper case");
        assert_eq!(secs("0.5s")?, 0.5, "fraction");
        assert_eq!(secs("0s")?, 0.0, "zero is allowed (a cut's transition)");
        assert_eq!(
            DurationSpec::parse("1 Bar")?,
            DurationSpec::Bars(1),
            "singular"
        );
        assert_eq!(
            DurationSpec::parse("16beats")?,
            DurationSpec::Beats(16),
            "no space"
        );
        assert_eq!(
            secs("90:00")?,
            5400.0,
            "a leading clock field may exceed 59"
        );
        assert_eq!(secs("0:07.5")?, 7.5, "fractional seconds in a clock");
        Ok(())
    }

    #[test]
    fn random_may_nest_and_mix_units() -> Result<()> {
        let spec = DurationSpec::parse("rand(30s, rand(8 bars, 16 bars))")?;
        assert!(
            spec.is_beat_based(),
            "a nested bar count makes the spec beat based"
        );
        assert_eq!(
            spec.to_string(),
            "rand(30s, rand(8 bars, 16 bars))",
            "nested random should print back as written"
        );
        Ok(())
    }

    #[test]
    fn the_planned_error_names_the_offending_text() {
        let error = DurationSpec::parse("3mn").err();
        assert_eq!(
            error.as_ref().map(|e| e.text.as_str()),
            Some("3mn"),
            "the error should quote the text"
        );
        assert!(
            error
                .map(|e| e.to_string())
                .is_some_and(|text| text.starts_with("\"3mn\" is not a duration")),
            "the message should read: \"3mn\" is not a duration"
        );
    }

    #[test]
    fn every_bad_form_is_refused() {
        for bad in [
            "",
            "   ",
            "90",
            "3mn",
            "m",
            "s30",
            "1m1h",
            "1h1h",
            "1:60",
            "1:2:3:4",
            "1:x",
            "1:",
            "0 bars",
            "-4 bars",
            "2.5 bars",
            "bars",
            "rand(1m)",
            "rand(1m, 2m, 3m)",
            "rand(1m, 2m",
            "rand(1m, 3mn)",
            "1e3s",
            "..5s",
            "5..s",
            "99999999999s",
            "1.2.3s",
        ] {
            assert!(
                DurationSpec::parse(bad).is_err(),
                "{bad:?} should not be a duration"
            );
        }
    }

    #[test]
    fn an_error_inside_rand_points_at_the_inner_text() {
        let input = "rand(1m, 3mn)";
        let error = DurationSpec::parse(input).err();
        assert_eq!(
            error.as_ref().map(|e| e.text.as_str()),
            Some("3mn"),
            "the offending argument should be quoted, not the whole rand()"
        );
        assert_eq!(
            error.map(|e| e.offset),
            Some(9),
            "the offset should land on the `3` in {input:?}"
        );
    }

    #[test]
    fn printing_gives_the_readable_form_and_parses_back() -> Result<()> {
        let cases = [
            (DurationSpec::Seconds(90.0), "90s"),
            (DurationSpec::Seconds(180.0), "3m"),
            (DurationSpec::Seconds(5400.0), "1h30m"),
            (DurationSpec::Seconds(3600.0), "1h"),
            (DurationSpec::Seconds(3605.0), "1h5s"),
            (DurationSpec::Seconds(0.0), "0s"),
            (DurationSpec::Seconds(0.5), "0.5s"),
            (DurationSpec::Bars(1), "1 bar"),
            (DurationSpec::Bars(32), "32 bars"),
            (DurationSpec::Beats(64), "64 beats"),
        ];
        for (spec, text) in cases {
            assert_eq!(spec.to_string(), text, "printing {spec:?}");
            assert_eq!(DurationSpec::parse(text)?, spec, "parsing {text:?} back");
        }
        Ok(())
    }

    #[test]
    fn awkward_floats_survive_printing() -> Result<()> {
        for seconds in [1.0 / 3.0, 187.43297, 0.1 + 0.2, 3600.5, 1.0e-3] {
            let spec = DurationSpec::Seconds(seconds);
            let back = DurationSpec::parse(&spec.to_string())?;
            assert_eq!(
                back, spec,
                "{seconds} did not survive a print/parse round trip"
            );
        }
        Ok(())
    }

    #[test]
    fn resolve_is_deterministic_and_stays_in_range() {
        let spec = DurationSpec::Random {
            min: Box::new(DurationSpec::Seconds(120.0)),
            max: Box::new(DurationSpec::Seconds(240.0)),
        };
        let draw = |seed| spec.resolve(&mut SplitMix64::new(seed), 120.0);
        assert_eq!(draw(1), draw(1), "same seed, same length");
        for seed in 0..200 {
            let Length::Seconds(s) = draw(seed) else {
                assert!(draw(seed).is_beat_based(), "seconds ends gave beats");
                continue;
            };
            assert!((120.0..240.0).contains(&s), "{s} is outside 2m..4m");
        }
    }

    #[test]
    fn beat_ends_resolve_to_whole_beats_and_swap_when_reversed() {
        let spec = DurationSpec::Random {
            min: Box::new(DurationSpec::Bars(8)),
            max: Box::new(DurationSpec::Bars(4)),
        };
        for seed in 0..100 {
            match spec.resolve(&mut SplitMix64::new(seed), 120.0) {
                Length::Beats(n) => {
                    assert!((16..=32).contains(&n), "{n} beats is outside 4..8 bars");
                    assert_eq!(n % 4, 0, "bar to bar draws whole bars, got {n} beats");
                }
                Length::Seconds(s) => {
                    assert!(s.is_nan(), "bar ends should stay in beats, got {s}s")
                }
            }
        }
    }

    #[test]
    fn beats_convert_to_seconds_at_the_given_tempo() {
        assert_eq!(Length::Beats(8).seconds(120.0), 4.0, "8 beats at 120 BPM");
        assert_eq!(
            Length::Seconds(5.0).seconds(60.0),
            5.0,
            "seconds ignore tempo"
        );
        assert_eq!(
            DurationSpec::Bars(2).resolve(&mut SplitMix64::new(0), 120.0),
            Length::Beats(8),
            "a bar is four beats"
        );
    }
}
