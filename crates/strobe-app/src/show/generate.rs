// ============================================================================
// Module:       strobe_app::show::generate
// Description:  The Generate dialog's form, parsed into a GenerateSpec, and
//               the library snapshot the generator draws from.
//
// Dependencies: strobe-playlist (generate, DurationSpec); crate::library
// ============================================================================

//! The Generate dialog's data (PLAN section 7.3).
//!
//! The form is text, because that is what a person types into a field, and
//! [`GenerateForm::spec`] turns it into the pure generator's
//! [`GenerateSpec`] or says in one line what is wrong with which field. It
//! lives in the UI state; the pane edits it, the app parses it and runs
//! `strobe_playlist::generate` (a pure function of the library snapshot,
//! the spec and the seed), and the pane shows the preview before anything is
//! added to the playlist.
//!
//! ## The pool: tags, then name words
//!
//! The "pool" box takes words. [`snapshot`] gives every scene its `TAGS`
//! from `scene.json` (D205) and then its **name words** as further tags
//! (`05-domain-warp` is also `domain` and `warp`; numbers and words under
//! three letters are not), so a library nobody has tagged still generates.
//! Pool words are normalised like tags (`3D` is `3d`, `audio_reactive` is
//! `audio-reactive`). A word that equals a whole scene name picks that
//! scene. An empty pool is the whole library. The preview says which rule
//! put each scene in the pool (`strobe_playlist::Why`).

use crate::library::Library;
use strobe_playlist::{
    DurationSpec, GenerateError, GenerateSpec, LibraryScene, LibrarySnapshot, PoolFilter,
    TransitionSpec, Why,
};

/// How many of the library's tags an empty-pool message lists.
const TAGS_SHOWN: usize = 12;

/// Fewest letters in a name word for it to count as a tag.
const MIN_TAG_LETTERS: usize = 3;

/// The transition the generated cues use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransitionChoice {
    /// Leave it to the playlist and section defaults.
    #[default]
    Inherit,
    /// Cut.
    Cut,
    /// Crossfade.
    Crossfade,
    /// Fade through black.
    FadeThroughBlack,
}

impl TransitionChoice {
    /// Every choice, in the order the dialog offers them.
    pub const ALL: [TransitionChoice; 4] = [
        Self::Inherit,
        Self::Cut,
        Self::Crossfade,
        Self::FadeThroughBlack,
    ];

    /// The chip's label.
    pub fn label(self) -> &'static str {
        match self {
            Self::Inherit => "Inherit",
            Self::Cut => "Cut",
            Self::Crossfade => "Crossfade",
            Self::FadeThroughBlack => "Fade through black",
        }
    }
}

/// What the dialog's fields hold.
#[derive(Debug, Clone, PartialEq)]
pub struct GenerateForm {
    /// The new section's name.
    pub section_name: String,
    /// Target length: `30m`, `1h30m`, `4h`.
    pub target: String,
    /// Tags or scene names, separated by commas or spaces. Empty is all.
    pub pool: String,
    /// The duration rule each cue draws from: `3m`, `rand(2m, 4m)`, `32 bars`.
    pub duration: String,
    /// The transition rule.
    pub transition: TransitionChoice,
    /// The transition's length, when it has one.
    pub transition_length: String,
    /// How many recent scenes not to repeat.
    pub window: String,
    /// The seed; the same seed gives the same section.
    pub seed: String,
}

impl Default for GenerateForm {
    fn default() -> Self {
        Self {
            section_name: "Generated".to_string(),
            target: "30m".to_string(),
            pool: String::new(),
            duration: "rand(2m, 4m)".to_string(),
            transition: TransitionChoice::Inherit,
            transition_length: "4s".to_string(),
            window: "5".to_string(),
            seed: "1".to_string(),
        }
    }
}

impl GenerateForm {
    /// Parses the fields. `Err` is one line naming the field.
    pub fn spec(&self) -> Result<GenerateSpec, String> {
        let name = self.section_name.trim();
        if name.is_empty() {
            return Err("Section name: give the new section a name".to_string());
        }
        let target = parse_seconds("Target length", &self.target)?;
        if target <= 0.0 {
            return Err("Target length: must be longer than zero".to_string());
        }
        let duration =
            DurationSpec::parse(&self.duration).map_err(|e| format!("Cue duration: {e}"))?;
        let window: usize = self.window.trim().parse().map_err(|_| {
            format!(
                "No-repeat window: \"{}\" is not a whole number",
                self.window
            )
        })?;
        let seed: u64 = self
            .seed
            .trim()
            .parse()
            .map_err(|_| format!("Seed: \"{}\" is not a whole number", self.seed))?;
        let transitions = match self.transition {
            TransitionChoice::Inherit => Vec::new(),
            TransitionChoice::Cut => vec![TransitionSpec::cut()],
            TransitionChoice::Crossfade => {
                vec![TransitionSpec::crossfade(parse_seconds(
                    "Transition length",
                    &self.transition_length,
                )?)]
            }
            TransitionChoice::FadeThroughBlack => {
                let seconds = parse_seconds("Transition length", &self.transition_length)?;
                vec![TransitionSpec {
                    kind: strobe_playlist::TransitionKind::FadeThroughBlack,
                    duration: DurationSpec::Seconds(seconds),
                }]
            }
        };
        let words: Vec<&str> = self
            .pool
            .split([',', ' ', ';'])
            .map(str::trim)
            .filter(|word| !word.is_empty())
            .collect();
        let mut spec = GenerateSpec::new(name, target, seed);
        spec.pool = PoolFilter {
            names: words.iter().map(|word| word.to_lowercase()).collect(),
            tags: strobe_scene::tags::normalized(words.iter().copied()),
            ..PoolFilter::default()
        };
        spec.duration = duration;
        spec.transitions = transitions;
        spec.no_repeat_window = window;
        Ok(spec)
    }
}

/// A duration field that must come out as plain seconds (a target or a
/// transition length, where bars make no sense).
fn parse_seconds(field: &str, text: &str) -> Result<f64, String> {
    match DurationSpec::parse(text) {
        Ok(DurationSpec::Seconds(seconds)) => Ok(seconds),
        Ok(_) => Err(format!(
            "{field}: \"{}\" must be a time such as 90s, 30m or 1h30m",
            text.trim()
        )),
        Err(error) => Err(format!("{field}: {error}")),
    }
}

/// The library as the generator sees it: each scene's tags, then its name
/// words (see the module docs).
pub fn snapshot(library: &Library) -> LibrarySnapshot {
    LibrarySnapshot {
        scenes: library
            .entries
            .iter()
            .map(|entry| {
                let mut tags = entry.tags.clone();
                for word in entry
                    .lower
                    .split(|c: char| !c.is_alphabetic())
                    .filter(|word| word.chars().count() >= MIN_TAG_LETTERS)
                {
                    if !tags.iter().any(|tag| tag == word) {
                        tags.push(word.to_string());
                    }
                }
                LibraryScene {
                    name: entry.name.clone(),
                    tags,
                    folder: String::new(),
                }
            })
            .collect(),
    }
}

/// The generator's error as the dialog says it: an empty pool lists the
/// tags the library does have.
pub fn explain(error: &GenerateError, library: &Library) -> String {
    match error {
        GenerateError::EmptyPool => {
            let counts = tag_counts(library);
            if counts.is_empty() {
                "No scene matches the pool, and no scene in the library has tags: use words from \
                 scene names, or leave the pool empty for the whole library"
                    .to_string()
            } else {
                let listed: Vec<String> = counts
                    .iter()
                    .take(TAGS_SHOWN)
                    .map(|(tag, count)| format!("{tag} ({count})"))
                    .collect();
                let more = counts.len().saturating_sub(TAGS_SHOWN);
                let tail = if more > 0 {
                    format!(", and {more} more")
                } else {
                    String::new()
                };
                format!(
                    "No scene matches the pool. Tags in this library: {}{tail}",
                    listed.join(", ")
                )
            }
        }
        GenerateError::BadTarget | GenerateError::TooManyCues => error.to_string(),
    }
}

/// One line saying why the pool holds what it holds: `tag dark 5, by name
/// 2`, most common reason first.
pub fn pool_summary(pool: &[(String, Why)]) -> String {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for (_, why) in pool {
        let label = why.to_string();
        match counts.iter_mut().find(|(known, _)| *known == label) {
            Some((_, count)) => *count += 1,
            None => counts.push((label, 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    counts
        .iter()
        .map(|(label, count)| format!("{label} {count}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The library's tags, most used first (then by name), for saying what a
/// pool could have asked for when it matched nothing.
pub fn tag_counts(library: &Library) -> Vec<(String, usize)> {
    let mut counts: Vec<(String, usize)> = Vec::new();
    for tag in library.entries.iter().flat_map(|entry| &entry.tags) {
        match counts.iter_mut().find(|(known, _)| known == tag) {
            Some((_, count)) => *count += 1,
            None => counts.push((tag.clone(), 1)),
        }
    }
    counts.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    counts
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::library::Entry;
    use anyhow::{Result, anyhow};
    use std::path::PathBuf;

    fn library() -> Library {
        Library {
            dir: PathBuf::from("scenes"),
            entries: ["01-aurora-gradient", "04-voronoi-glass", "05-domain-warp"]
                .iter()
                .map(|n| Entry::new((*n).to_string(), PathBuf::from(n)))
                .collect(),
        }
    }

    #[test]
    fn the_default_form_is_a_valid_half_hour() -> Result<()> {
        let spec = GenerateForm::default().spec().map_err(|e| anyhow!(e))?;
        assert_eq!(spec.target_seconds, 1800.0, "30m");
        assert_eq!(spec.no_repeat_window, 5, "window");
        assert!(spec.transitions.is_empty(), "inherit");
        Ok(())
    }

    #[test]
    fn a_bad_field_is_named_in_the_error() {
        let bad = |change: fn(&mut GenerateForm)| {
            let mut form = GenerateForm::default();
            change(&mut form);
            form.spec().err().unwrap_or_default()
        };
        assert!(bad(|f| f.target = "soon".into()).starts_with("Target length"));
        assert!(bad(|f| f.target = "32 bars".into()).contains("must be a time"));
        assert!(bad(|f| f.duration = "3mn".into()).starts_with("Cue duration"));
        assert!(bad(|f| f.window = "-1".into()).starts_with("No-repeat window"));
        assert!(bad(|f| f.seed = "x".into()).starts_with("Seed"));
        assert!(bad(|f| f.section_name = "  ".into()).starts_with("Section name"));
        assert!(
            bad(|f| {
                f.transition = TransitionChoice::Crossfade;
                f.transition_length = "??".into();
            })
            .starts_with("Transition length")
        );
    }

    #[test]
    fn name_words_are_the_tags_and_a_pool_word_picks_by_either() -> Result<()> {
        let snap = snapshot(&library());
        let warp = snap
            .scenes
            .iter()
            .find(|s| s.name == "05-domain-warp")
            .ok_or_else(|| anyhow!("missing"))?;
        assert_eq!(warp.tags, vec!["domain", "warp"], "numbers are not tags");

        let form = GenerateForm {
            pool: "warp, voronoi-glass".into(),
            ..GenerateForm::default()
        };
        let spec = form.spec().map_err(|e| anyhow!(e))?;
        let made = strobe_playlist::generate(&snap, &spec).map_err(|e| anyhow!("{e}"))?;
        assert_eq!(
            made.pool_size, 1,
            "only the warp scene is tagged warp; the other word is no exact name"
        );
        Ok(())
    }

    #[test]
    fn real_tags_come_first_and_pool_words_are_normalised() -> Result<()> {
        let mut library = library();
        library.entries = vec![
            Entry::new("01-aurora-gradient".into(), PathBuf::from("a")).with_tags(&["Calm", "2D"]),
            Entry::new("09-sdf-cathedral".into(), PathBuf::from("c")).with_tags(&[
                "3D",
                "dark",
                "audio reactive",
            ]),
            Entry::new("05-domain-warp".into(), PathBuf::from("w")).with_tags(&["calm"]),
        ];
        let snap = snapshot(&library);
        let tags: Vec<&[String]> = snap.scenes.iter().map(|s| s.tags.as_slice()).collect();
        assert_eq!(
            tags.first().copied(),
            Some(
                &[
                    "calm".to_string(),
                    "2d".into(),
                    "aurora".into(),
                    "gradient".into()
                ][..]
            ),
            "tags, then name words not already there"
        );
        let pool = |words: &str| -> Result<Vec<String>> {
            let form = GenerateForm {
                pool: words.into(),
                ..GenerateForm::default()
            };
            let spec = form.spec().map_err(|e| anyhow!(e))?;
            let made = strobe_playlist::generate(&snap, &spec).map_err(|e| anyhow!("{e}"))?;
            Ok(made
                .pool
                .iter()
                .map(|(name, why)| format!("{name}: {why}"))
                .collect())
        };
        assert_eq!(
            pool("CALM")?,
            ["01-aurora-gradient: tag calm", "05-domain-warp: tag calm"],
            "a tag in any case"
        );
        assert_eq!(
            pool("audio_reactive, 3D")?,
            ["09-sdf-cathedral: tag audio-reactive"],
            "spellings normalise; the first of the pool's tags the scene carries is named"
        );
        assert_eq!(
            pool_summary(&[
                ("a".into(), Why::Tag("calm".into())),
                ("b".into(), Why::Name),
                ("c".into(), Why::Tag("calm".into())),
            ]),
            "tag calm 2, by name 1",
            "most common reason first"
        );
        let empty = explain(&GenerateError::EmptyPool, &library);
        assert!(
            empty.contains("calm (2), 2d (1)"),
            "an empty pool lists the library's tags: {empty}"
        );
        assert_eq!(
            tag_counts(&library),
            [
                ("calm".to_string(), 2),
                ("2d".to_string(), 1),
                ("3d".to_string(), 1),
                ("audio-reactive".to_string(), 1),
                ("dark".to_string(), 1)
            ],
            "most used first, then by name"
        );
        Ok(())
    }

    #[test]
    fn the_same_form_gives_the_same_section() -> Result<()> {
        let snap = snapshot(&library());
        let spec = GenerateForm::default().spec().map_err(|e| anyhow!(e))?;
        let a = strobe_playlist::generate(&snap, &spec).map_err(|e| anyhow!("{e}"))?;
        let b = strobe_playlist::generate(&snap, &spec).map_err(|e| anyhow!("{e}"))?;
        assert_eq!(a.section, b.section, "pure in its seed");
        assert!(
            (a.estimated_seconds - 1800.0).abs() < 1.0,
            "half an hour: {}",
            a.estimated_seconds
        );
        Ok(())
    }
}
