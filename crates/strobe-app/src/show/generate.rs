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
//! ## The pool, without tags
//!
//! The scene format has no tag metadata yet, so the "pool" box takes words
//! and [`snapshot`] gives every scene its **name words as tags**
//! (`05-domain-warp` is tagged `domain` and `warp`; numbers and words under
//! three letters are not tags). A word that equals a whole scene name picks
//! that scene. An empty pool is the whole library. When real tags exist the
//! snapshot is the only thing that changes.

use crate::library::Library;
use strobe_playlist::{
    DurationSpec, GenerateSpec, LibraryScene, LibrarySnapshot, PoolFilter, TransitionSpec,
};

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
        let words: Vec<String> = self
            .pool
            .split([',', ' ', ';'])
            .map(|word| word.trim().to_lowercase())
            .filter(|word| !word.is_empty())
            .collect();
        let mut spec = GenerateSpec::new(name, target, seed);
        spec.pool = PoolFilter {
            names: words.clone(),
            tags: words,
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

/// The library as the generator sees it, with name words as tags.
pub fn snapshot(library: &Library) -> LibrarySnapshot {
    LibrarySnapshot {
        scenes: library
            .entries
            .iter()
            .map(|entry| LibraryScene {
                name: entry.name.clone(),
                tags: entry
                    .lower
                    .split(|c: char| !c.is_alphabetic())
                    .filter(|word| word.chars().count() >= MIN_TAG_LETTERS)
                    .map(str::to_string)
                    .collect(),
                folder: String::new(),
            })
            .collect(),
    }
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

        let mut form = GenerateForm::default();
        form.pool = "warp, voronoi-glass".into();
        let spec = form.spec().map_err(|e| anyhow!(e))?;
        let made = strobe_playlist::generate(&snap, &spec).map_err(|e| anyhow!("{e}"))?;
        assert_eq!(
            made.pool_size, 1,
            "only the warp scene is tagged warp; the other word is no exact name"
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
