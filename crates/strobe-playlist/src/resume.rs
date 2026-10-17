// ============================================================================
// Module:       strobe_playlist::resume
// Description:  The resume point: playlist identity, section, cue, offset and
//               pass as a few lines of TOML, saved atomically.
//
// Dependencies: crate::error, crate::file, crate::model, crate::parse, serde,
//               toml
// ============================================================================

//! Resuming after a restart.
//!
//! The app saves a [`ResumePoint`] every few seconds; after a crash, a power
//! cut or a deliberate restart, [`PlaylistClock::restore`] puts playback
//! back on the same cue at the same offset.
//!
//! The point names the cue by its *authored* position (section index, cue
//! index within the section), not by the timeline's, so a disabled cue added
//! or removed in the file does not shift it. It carries the playlist's
//! [`identity`](crate::resolve::identity) so a point saved for one show is
//! not applied to another: adding, removing or reordering cues changes the
//! identity, and editing a duration does not.
//!
//! The file is TOML, five lines, safe to read by eye:
//!
//! ```toml
//! identity = "1d0b8b7d3c5a6f21"
//! section = 1
//! cue = 4
//! pass = 0
//! offset = 37.5
//! ```
//!
//! The identity is a hex string because TOML integers are signed 64-bit.
//!
//! [`PlaylistClock::restore`]: crate::PlaylistClock::restore

use crate::error::{ParseError, PlaylistError};
use crate::file::write_atomic;
use crate::model::Playlist;
use crate::parse::toml_error;
use serde::Deserialize;
use std::path::Path;

/// Where playback was.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ResumePoint {
    /// The playlist's identity hash when this was saved.
    pub identity: u64,
    /// Authored section index.
    pub section: usize,
    /// Authored cue index inside the section.
    pub cue: usize,
    /// Which pass over the playlist (0 on the first).
    pub pass: u64,
    /// Seconds into the cue.
    pub offset: f64,
}

/// Why a resume point could not be applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ResumeError {
    /// The point was saved for a different playlist.
    #[error(
        "the resume point belongs to a different playlist ({saved:016x}, this one is {current:016x})"
    )]
    IdentityMismatch {
        /// Identity in the saved point.
        saved: u64,
        /// Identity of the playlist now loaded.
        current: u64,
    },
    /// The saved section and cue are beyond the end of the playlist.
    #[error("the resume point names cue {cue} of section {section}, which no longer exists")]
    CueMissing {
        /// Saved section index.
        section: usize,
        /// Saved cue index.
        cue: usize,
    },
    /// Every cue at or after the point has failed to compile.
    #[error("no playable cue at or after the resume point")]
    NothingPlayable,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    identity: String,
    section: u32,
    cue: u32,
    #[serde(default)]
    pass: u32,
    #[serde(default)]
    offset: f64,
}

impl ResumePoint {
    /// The point as TOML text.
    pub fn to_toml(&self) -> String {
        format!(
            "identity = \"{:016x}\"\nsection = {}\ncue = {}\npass = {}\noffset = {:?}\n",
            self.identity, self.section, self.cue, self.pass, self.offset
        )
    }

    /// Reads a point from TOML text.
    pub fn from_toml(text: &str) -> Result<Self, ParseError> {
        let raw: Raw = toml::from_str(text).map_err(|error| toml_error(text, &error))?;
        let fail = |message: String| ParseError {
            line: 1,
            column: 1,
            message,
            near: String::new(),
        };
        let identity = u64::from_str_radix(raw.identity.trim(), 16).map_err(|_| {
            fail(format!(
                "\"{}\" is not a playlist identity (16 hex digits)",
                raw.identity
            ))
        })?;
        if !(raw.offset.is_finite() && raw.offset >= 0.0) {
            return Err(fail(format!("{} is not an offset in seconds", raw.offset)));
        }
        Ok(Self {
            identity,
            section: raw.section as usize,
            cue: raw.cue as usize,
            pass: u64::from(raw.pass),
            offset: raw.offset,
        })
    }

    /// This point carried from `old` into `new`, an edited version of the
    /// same playlist (a hot reload, a LIST pane edit), so it still names the
    /// cue it was saved on (D432).
    ///
    /// The authored position alone is not enough: a cue inserted, deleted,
    /// duplicated or moved above the point's cue shifts it, and the
    /// position then names a neighbour. So the cue is looked for by what it
    /// is: unchanged at the same position, the point stays; else the
    /// nearest cue in `new` equal to it in every field (in a section of the
    /// same name first, then by distance) is the one; else the cue itself
    /// was edited or removed, and the point keeps its position, as before.
    /// The offset and the pass are kept.
    #[must_use]
    pub fn follow(self, old: &Playlist, new: &Playlist) -> Self {
        let Some((section_name, live)) = old
            .sections
            .get(self.section)
            .and_then(|section| Some((section.name.as_str(), section.cues.get(self.cue)?)))
        else {
            return self;
        };
        let unchanged = new
            .sections
            .get(self.section)
            .and_then(|section| section.cues.get(self.cue))
            .is_some_and(|cue| cue == live);
        if unchanged {
            return self;
        }
        let nearest = new
            .sections
            .iter()
            .enumerate()
            .flat_map(|(section, authored)| {
                authored
                    .cues
                    .iter()
                    .enumerate()
                    .filter(|(_, cue)| *cue == live)
                    .map(move |(cue, _)| (section, cue, authored.name == section_name))
            })
            .min_by_key(|&(section, cue, same_name)| {
                (
                    !same_name,
                    section.abs_diff(self.section),
                    cue.abs_diff(self.cue),
                )
            });
        match nearest {
            Some((section, cue, _)) => Self {
                section,
                cue,
                ..self
            },
            None => self,
        }
    }

    /// Writes the point to `path`, atomically.
    pub fn save(&self, path: &Path) -> Result<(), PlaylistError> {
        write_atomic(path, &self.to_toml())
    }

    /// Reads a point from `path`.
    pub fn load(path: &Path) -> Result<Self, PlaylistError> {
        let text = std::fs::read_to_string(path).map_err(|source| PlaylistError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(Self::from_toml(&text)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Cue, SceneRef, Section};
    use anyhow::Result;

    fn point() -> ResumePoint {
        ResumePoint {
            identity: 0x1d0b_8b7d_3c5a_6f21,
            section: 1,
            cue: 4,
            pass: 2,
            offset: 37.5,
        }
    }

    #[test]
    fn the_text_form_is_five_readable_lines_and_round_trips() -> Result<()> {
        let text = point().to_toml();
        assert_eq!(
            text,
            "identity = \"1d0b8b7d3c5a6f21\"\nsection = 1\ncue = 4\npass = 2\noffset = 37.5\n",
            "layout"
        );
        assert_eq!(ResumePoint::from_toml(&text)?, point(), "round trip");
        Ok(())
    }

    #[test]
    fn the_full_identity_range_survives() -> Result<()> {
        for identity in [0, 1, u64::MAX, 0x8000_0000_0000_0000] {
            let original = ResumePoint {
                identity,
                ..point()
            };
            assert_eq!(
                ResumePoint::from_toml(&original.to_toml())?,
                original,
                "identity {identity:x}"
            );
        }
        Ok(())
    }

    #[test]
    fn whole_second_offsets_print_as_floats() -> Result<()> {
        let original = ResumePoint {
            offset: 12.0,
            ..point()
        };
        assert!(
            original.to_toml().contains("offset = 12.0"),
            "a float, not an integer"
        );
        assert_eq!(
            ResumePoint::from_toml(&original.to_toml())?,
            original,
            "and reads back"
        );
        Ok(())
    }

    #[test]
    fn damaged_files_are_errors_not_guesses() {
        for bad in [
            "",
            "identity = \"zz\"\nsection = 0\ncue = 0\n",
            "identity = \"0000000000000001\"\nsection = -1\ncue = 0\n",
            "identity = \"0000000000000001\"\nsection = 0\ncue = 0\noffset = -3.0\n",
            "identity = \"0000000000000001\"\nsection = 0\ncue = 0\nextra = 1\n",
            "identity = \"0000000000000001\"\nsection = 0\ncu",
        ] {
            assert!(
                ResumePoint::from_toml(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
    }

    /// Sections of cues named by their scenes.
    fn playlist_of(sections: &[(&str, &[&str])]) -> Playlist {
        let mut playlist = Playlist::new("show");
        for (name, scenes) in sections {
            let mut section = Section::new(*name);
            for scene in *scenes {
                section
                    .cues
                    .push(Cue::new(SceneRef::Library((*scene).into())));
            }
            playlist.sections.push(section);
        }
        playlist
    }

    fn at(section: usize, cue: usize) -> ResumePoint {
        ResumePoint {
            section,
            cue,
            ..point()
        }
    }

    fn place(point: ResumePoint) -> (usize, usize) {
        (point.section, point.cue)
    }

    #[test]
    fn a_point_follows_its_cue_when_cues_above_it_move() {
        let old = playlist_of(&[("s", &["a", "b", "c", "d"])]);
        // Playing c, 37.5 s in.
        let live = at(0, 2);
        for (edit, new, expected) in [
            (
                "deleted above",
                playlist_of(&[("s", &["b", "c", "d"])]),
                (0, 1),
            ),
            (
                "inserted above",
                playlist_of(&[("s", &["x", "a", "b", "c", "d"])]),
                (0, 3),
            ),
            (
                "moved up",
                playlist_of(&[("s", &["a", "c", "b", "d"])]),
                (0, 1),
            ),
            (
                "a section added before",
                playlist_of(&[("intro", &["c", "x"]), ("s", &["a", "b", "c", "d"])]),
                (1, 2),
            ),
            (
                "a cue after it changed",
                playlist_of(&[("s", &["a", "b", "c", "x"])]),
                (0, 2),
            ),
        ] {
            let followed = live.follow(&old, &new);
            assert_eq!(place(followed), expected, "{edit}");
            assert_eq!(followed.offset, live.offset, "{edit}: the offset is kept");
        }
    }

    #[test]
    fn a_point_on_a_cue_that_was_edited_or_removed_keeps_its_position() {
        let old = playlist_of(&[("s", &["a", "b", "c"])]);
        let mut edited = old.clone();
        if let Some(cue) = edited
            .sections
            .first_mut()
            .and_then(|section| section.cues.get_mut(1))
        {
            cue.preset = Some("full".into());
        }
        assert_eq!(
            place(at(0, 1).follow(&old, &edited)),
            (0, 1),
            "the live cue's own edit"
        );
        let removed = playlist_of(&[("s", &["a", "c"])]);
        assert_eq!(
            place(at(0, 1).follow(&old, &removed)),
            (0, 1),
            "the live cue is gone: its position, which is now the next cue"
        );
        assert_eq!(
            place(at(5, 0).follow(&old, &removed)),
            (5, 0),
            "a point outside the old playlist is left for the restore to refuse"
        );
    }

    #[test]
    fn save_and_load_use_the_filesystem_atomically() -> Result<()> {
        let dir =
            std::env::temp_dir().join(format!("strobe-playlist-resume-{}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("playback.toml");
        point().save(&path)?;
        assert_eq!(ResumePoint::load(&path)?, point(), "load after save");
        let replaced = ResumePoint {
            offset: 99.0,
            ..point()
        };
        replaced.save(&path)?;
        assert_eq!(
            ResumePoint::load(&path)?,
            replaced,
            "second save replaces the first"
        );
        let missing = ResumePoint::load(&dir.join("nope.toml"));
        assert!(
            missing.is_err(),
            "a missing file is an error the caller can ignore"
        );
        std::fs::remove_dir_all(&dir)?;
        Ok(())
    }
}
