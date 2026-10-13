// ============================================================================
// Module:       strobe_app::show::view
// Description:  The playlist as the LIST pane draws it: one row per cue with
//               its computed start, length and transition, as plain text.
//
// Dependencies: strobe-playlist (Timeline)
// ============================================================================

//! Rows for the timeline.
//!
//! The pane must not walk a [`Timeline`] and format numbers on every
//! repaint of a 4-hour, 80-cue show, so the rows are built once when a
//! playlist is loaded or edited and the pane only reads them. What changes
//! while the show plays (the current cue, a failed cue, the time left) is
//! asked of the clock each frame and is not in a row.

use strobe_playlist::{Length, Timeline, TransitionKind};

/// One cue, ready to draw.
#[derive(Debug, Clone, PartialEq)]
pub struct CueRow {
    /// Index in the timeline (what `JumpTo` takes).
    pub index: usize,
    /// Authored section index.
    pub section: usize,
    /// Authored index of the cue inside its section (what an edit
    /// addresses, D213).
    pub cue: usize,
    /// The section's name.
    pub section_name: String,
    /// The scene as written in the playlist.
    pub scene: String,
    /// Seconds from the start of the playlist.
    pub start: f64,
    /// Seconds at the fallback tempo.
    pub duration: f64,
    /// `3:00`, or `32 bars` when the length is counted in beats.
    pub length_label: String,
    /// `crossfade 4s`, `cut`, `fade-through-black 2s`, `shader name 1s`.
    pub transition_label: String,
    /// Nothing but a person or an audio event ends this cue.
    pub open_ended: bool,
    /// How many control overrides the cue sets.
    pub overrides: usize,
    /// The wall-clock time the cue starts, for an anchored playlist.
    pub wall: Option<String>,
    /// Where the length and the transition came from: the cue itself, its
    /// section, the playlist, or the built-in default.
    pub origins: strobe_playlist::SettingOrigins,
    /// The preset the cue names, if any.
    pub preset: Option<String>,
}

/// The rows of `timeline`, in play order.
pub fn rows(timeline: &Timeline) -> Vec<CueRow> {
    timeline
        .cues()
        .iter()
        .enumerate()
        .map(|(index, cue)| {
            let resolved = &cue.resolved;
            let transition_label = match &resolved.transition.kind {
                TransitionKind::Cut => "cut".to_string(),
                TransitionKind::Crossfade => {
                    format!("crossfade {}", clock(cue.transition_length.seconds(120.0)))
                }
                TransitionKind::FadeThroughBlack => format!(
                    "fade-through-black {}",
                    clock(cue.transition_length.seconds(120.0))
                ),
                TransitionKind::Shader(path) => format!(
                    "{} {}",
                    path.file_stem().map_or_else(
                        || "shader".to_string(),
                        |s| s.to_string_lossy().into_owned()
                    ),
                    clock(cue.transition_length.seconds(120.0))
                ),
            };
            CueRow {
                index,
                section: resolved.section,
                cue: resolved.cue,
                section_name: timeline
                    .section_name(resolved.section)
                    .unwrap_or_default()
                    .to_string(),
                scene: resolved.scene.to_text(),
                start: cue.start,
                duration: cue.duration,
                length_label: match cue.length {
                    Length::Seconds(seconds) => clock(seconds),
                    Length::Beats(beats) if beats % 4 == 0 => format!("{} bars", beats / 4),
                    Length::Beats(beats) => format!("{beats} beats"),
                },
                transition_label,
                open_ended: cue.open_ended,
                overrides: resolved.overrides.len(),
                wall: timeline.wall_time(index).map(|t| t.to_string()),
                origins: resolved.origins,
                preset: resolved.preset.clone(),
            }
        })
        .collect()
}

/// `m:ss`, or `h:mm:ss` from an hour up; `--:--` for nonsense.
pub fn clock(seconds: f64) -> String {
    if !seconds.is_finite() || seconds < 0.0 {
        return "--:--".to_string();
    }
    let total = seconds.round() as u64;
    let (h, m, s) = (total / 3600, total % 3600 / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::{Result, anyhow};
    use strobe_playlist::{
        Cue, DurationSpec, Playlist, SceneRef, Section, TimelineConfig, TransitionSpec,
        WallClockAnchor,
    };

    fn playlist() -> Result<Playlist> {
        let mut playlist = Playlist::new("show");
        playlist.anchor = WallClockAnchor::from_hm(9, 0);
        let mut section = Section::new("Morning");
        let mut first = Cue::new(SceneRef::Library("a".into()));
        first.settings.duration = Some(DurationSpec::Seconds(90.0));
        first.settings.transition = Some(TransitionSpec::cut());
        let mut second = Cue::new(SceneRef::Library("b".into()));
        second.settings.duration = Some(DurationSpec::parse("32 bars").map_err(|e| anyhow!(e))?);
        second.settings.transition = Some(TransitionSpec::crossfade(4.0));
        second
            .overrides
            .insert("amount".into(), strobe_playlist::ControlValue::Float(0.5));
        section.cues.push(first);
        section.cues.push(second);
        playlist.sections.push(section);
        Ok(playlist)
    }

    #[test]
    fn rows_carry_starts_labels_and_wall_times() -> Result<()> {
        let timeline = Timeline::from_playlist(&playlist()?, &TimelineConfig::default());
        let rows = rows(&timeline);
        assert_eq!(rows.len(), 2, "two cues");
        assert_eq!(rows[0].length_label, "1:30", "seconds read as m:ss");
        assert_eq!(rows[0].transition_label, "cut", "a cut");
        assert_eq!(rows[0].wall.as_deref(), Some("09:00:00"), "anchored start");
        assert_eq!(rows[1].length_label, "32 bars", "bars stay bars");
        assert_eq!(rows[1].transition_label, "crossfade 0:04", "a crossfade");
        assert!(
            (rows[1].start - 90.0).abs() < 1e-9,
            "starts when the first ends"
        );
        assert_eq!(rows[1].overrides, 1, "one override");
        assert_eq!(rows[1].section_name, "Morning", "section name");
        Ok(())
    }

    #[test]
    fn the_clock_reads_like_a_clock() {
        assert_eq!(clock(0.0), "0:00");
        assert_eq!(clock(59.6), "1:00");
        assert_eq!(clock(3725.0), "1:02:05");
        assert_eq!(clock(f64::NAN), "--:--");
        assert_eq!(clock(-1.0), "--:--");
    }
}
