// ============================================================================
// Module:       strobe_app::show::edit
// Description:  Editing a playlist from the LIST pane: one pure function per
//               change to a cue, applied to a copy, by authored position.
//
// Dependencies: strobe-playlist (model)
// ============================================================================

//! Cue edits (backlog U-07, D213).
//!
//! The LIST pane edits the playlist **as authored**: a cue is addressed by
//! its section and its place in that section (`ResolvedCue::section`,
//! `::cue`), not by its place in the timeline, which shuffle and disabled
//! cues make different. [`apply`] takes the playlist and returns a new one,
//! with the position the edited cue now has, or says in one line why the
//! edit cannot be made; the playlist it was given is never half changed.
//!
//! The app hands the result to `Show::replace_playlist`, the same path a
//! reload from the file takes: only scenes new to the show are compiled,
//! and the show keeps its place.
//!
//! An inheritable setting set to `None` is *inherited* again, from the
//! section or the playlist, which is how the pane's "Inherit" works; the
//! file then says nothing about it for that cue.

use strobe_playlist::{Cue, DurationSpec, Playlist, SceneRef, TransitionSpec};

/// Where a cue is, as authored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct At {
    /// The section's index.
    pub section: usize,
    /// The cue's index inside it.
    pub cue: usize,
}

/// One change to one cue.
#[derive(Debug, Clone, PartialEq)]
pub enum CueEdit {
    /// One place earlier in its section.
    MoveUp,
    /// One place later in its section.
    MoveDown,
    /// A copy straight after it.
    Duplicate,
    /// Gone.
    Delete,
    /// A new cue for this scene straight after it, inheriting everything.
    InsertAfter(SceneRef),
    /// Its length, or `None` to inherit.
    SetDuration(Option<DurationSpec>),
    /// How the show arrives at it, or `None` to inherit.
    SetTransition(Option<TransitionSpec>),
    /// The scene preset it loads, or `None` for the scene's own start.
    SetPreset(Option<String>),
    /// The picture or folder it shows behind `syn_Media`, or `None` for the
    /// scene's own (D217).
    SetMedia(Option<String>),
    /// Whether it plays.
    SetEnabled(bool),
}

/// `playlist` with `edit` made to the cue at `at`, and where that cue (or,
/// for a delete, its neighbour) now is. `Err` says why not, in one line.
pub fn apply(
    playlist: &Playlist,
    at: At,
    edit: &CueEdit,
) -> Result<(Playlist, Option<At>), String> {
    let mut next = playlist.clone();
    let section = next
        .sections
        .get_mut(at.section)
        .ok_or_else(|| "that cue's section is no longer in the playlist".to_string())?;
    let count = section.cues.len();
    if at.cue >= count {
        return Err("that cue is no longer in the playlist".to_string());
    }
    let same = Some(at);
    let gone = || "that cue is no longer in the playlist".to_string();
    let moved = |cue| Some(At { cue, ..at });
    let landed = match edit {
        CueEdit::MoveUp => {
            if at.cue == 0 {
                return Err("it is already first in its section".to_string());
            }
            section.cues.swap(at.cue, at.cue - 1);
            moved(at.cue - 1)
        }
        CueEdit::MoveDown => {
            if at.cue + 1 >= count {
                return Err("it is already last in its section".to_string());
            }
            section.cues.swap(at.cue, at.cue + 1);
            moved(at.cue + 1)
        }
        CueEdit::Duplicate => {
            let copy = section.cues.get(at.cue).cloned().ok_or_else(gone)?;
            section.cues.insert(at.cue + 1, copy);
            moved(at.cue + 1)
        }
        CueEdit::Delete => {
            section.cues.remove(at.cue);
            match section.cues.len() {
                0 => None,
                left => moved(at.cue.min(left - 1)),
            }
        }
        CueEdit::InsertAfter(scene) => {
            section.cues.insert(at.cue + 1, Cue::new(scene.clone()));
            moved(at.cue + 1)
        }
        CueEdit::SetDuration(duration) => {
            cue_at(section, at.cue)?.settings.duration = duration.clone();
            same
        }
        CueEdit::SetTransition(transition) => {
            cue_at(section, at.cue)?.settings.transition = transition.clone();
            same
        }
        CueEdit::SetPreset(preset) => {
            cue_at(section, at.cue)?.preset = preset
                .as_deref()
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_string);
            same
        }
        CueEdit::SetMedia(media) => {
            cue_at(section, at.cue)?.media = media
                .as_deref()
                .map(str::trim)
                .filter(|path| !path.is_empty())
                .map(std::path::PathBuf::from);
            same
        }
        CueEdit::SetEnabled(enabled) => {
            cue_at(section, at.cue)?.enabled = *enabled;
            same
        }
    };
    Ok((next, landed))
}

/// The cue at `index` in `section`.
fn cue_at(section: &mut strobe_playlist::Section, index: usize) -> Result<&mut Cue, String> {
    section
        .cues
        .get_mut(index)
        .ok_or_else(|| "that cue is no longer in the playlist".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, anyhow};
    use strobe_playlist::Section;

    fn playlist() -> Playlist {
        let mut playlist = Playlist::new("show");
        let mut one = Section::new("one");
        for name in ["a", "b", "c"] {
            one.cues.push(Cue::new(SceneRef::Library(name.to_string())));
        }
        let mut two = Section::new("two");
        two.cues.push(Cue::new(SceneRef::Library("d".to_string())));
        playlist.sections.push(one);
        playlist.sections.push(two);
        playlist
    }

    fn names(playlist: &Playlist, section: usize) -> Vec<String> {
        playlist
            .sections
            .get(section)
            .map(|s| s.cues.iter().map(|c| c.scene.to_text()).collect())
            .unwrap_or_default()
    }

    fn at(section: usize, cue: usize) -> At {
        At { section, cue }
    }

    #[test]
    fn moves_stay_in_their_section_and_follow_the_cue() -> Result<()> {
        let base = playlist();
        let (up, landed) = apply(&base, at(0, 1), &CueEdit::MoveUp).map_err(|e| anyhow!(e))?;
        assert_eq!(names(&up, 0), ["b", "a", "c"], "b moved up");
        assert_eq!(landed, Some(at(0, 0)), "and the selection follows it");
        assert!(
            apply(&base, at(0, 0), &CueEdit::MoveUp).is_err(),
            "the first cannot go up"
        );
        assert!(
            apply(&base, at(1, 0), &CueEdit::MoveDown).is_err(),
            "the last cannot go down"
        );
        assert_eq!(
            names(&base, 0),
            ["a", "b", "c"],
            "the original is untouched"
        );
        Ok(())
    }

    #[test]
    fn duplicate_insert_and_delete_land_where_expected() -> Result<()> {
        let base = playlist();
        let (dup, landed) = apply(&base, at(0, 0), &CueEdit::Duplicate).map_err(|e| anyhow!(e))?;
        assert_eq!(names(&dup, 0), ["a", "a", "b", "c"], "a copy after it");
        assert_eq!(landed, Some(at(0, 1)), "the copy is selected");

        let scene = SceneRef::Library("new".to_string());
        let (ins, landed) =
            apply(&base, at(0, 2), &CueEdit::InsertAfter(scene)).map_err(|e| anyhow!(e))?;
        assert_eq!(names(&ins, 0), ["a", "b", "c", "new"], "inserted after c");
        assert_eq!(landed, Some(at(0, 3)), "the new cue is selected");

        let (del, landed) = apply(&base, at(0, 2), &CueEdit::Delete).map_err(|e| anyhow!(e))?;
        assert_eq!(names(&del, 0), ["a", "b"], "c is gone");
        assert_eq!(landed, Some(at(0, 1)), "its neighbour is selected");
        let (empty, landed) = apply(&base, at(1, 0), &CueEdit::Delete).map_err(|e| anyhow!(e))?;
        assert!(names(&empty, 1).is_empty(), "a section can be emptied");
        assert_eq!(landed, None, "nothing left to select there");
        Ok(())
    }

    #[test]
    fn settings_set_and_inherit_again() -> Result<()> {
        let base = playlist();
        let three = DurationSpec::parse("3m").map_err(|e| anyhow!("{e}"))?;
        let (set, _) = apply(&base, at(0, 1), &CueEdit::SetDuration(Some(three.clone())))
            .map_err(|e| anyhow!(e))?;
        let cue = |p: &Playlist| p.sections[0].cues[1].clone();
        assert_eq!(cue(&set).settings.duration, Some(three), "set on the cue");
        let (back, _) =
            apply(&set, at(0, 1), &CueEdit::SetDuration(None)).map_err(|e| anyhow!(e))?;
        assert_eq!(cue(&back).settings.duration, None, "inherited again");

        let (cut, _) = apply(
            &base,
            at(0, 1),
            &CueEdit::SetTransition(Some(TransitionSpec::cut())),
        )
        .map_err(|e| anyhow!(e))?;
        assert_eq!(
            cue(&cut).settings.transition,
            Some(TransitionSpec::cut()),
            "transition"
        );
        let (preset, _) = apply(&base, at(0, 1), &CueEdit::SetPreset(Some("  ".into())))
            .map_err(|e| anyhow!(e))?;
        assert_eq!(cue(&preset).preset, None, "a blank preset is none");
        let (media, _) = apply(
            &base,
            at(0, 1),
            &CueEdit::SetMedia(Some(" loops/rain ".into())),
        )
        .map_err(|e| anyhow!(e))?;
        assert_eq!(
            cue(&media).media.as_deref(),
            Some(std::path::Path::new("loops/rain")),
            "media, trimmed"
        );
        let (none, _) = apply(&media, at(0, 1), &CueEdit::SetMedia(Some(String::new())))
            .map_err(|e| anyhow!(e))?;
        assert_eq!(
            cue(&none).media,
            None,
            "a blank media path is the scene's own"
        );
        let (off, _) =
            apply(&base, at(0, 1), &CueEdit::SetEnabled(false)).map_err(|e| anyhow!(e))?;
        assert!(!cue(&off).enabled, "disabled");
        assert!(
            apply(&base, at(5, 0), &CueEdit::Delete).is_err(),
            "a stale position is refused, not guessed"
        );
        Ok(())
    }
}
