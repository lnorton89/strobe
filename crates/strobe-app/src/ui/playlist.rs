// ============================================================================
// Module:       strobe_app::ui::playlist
// Description:  The LIST pane: the load field, the playlist's timeline with
//               each cue's computed start, length and state, the cue
//               commands, and the Generate dialog.
//
// Dependencies: egui; super::{model, theme, widgets, icon}, crate::{show,
//               icon, workspace}
// ============================================================================

//! The Playlist pane (UI_DESIGN section 13), its top half in
//! `super::form`'s grammar (UI review step 3, D470).
//!
//! With nothing loaded it is a page: open a playlist file (typed, or with
//! Browse, the system's dialog), or generate one from the library. Loaded:
//!
//! ```text
//! PLAYLIST                                                    x
//! (an error banner, or the compile progress)
//! [ Example: lobby                     [Open...] [Save] [Unload] ]
//! [   12 cues . 42:10 . loops . anchored 09:00                    ]
//! [+30 s] [Restart] [Skip section]                  [Generate...]
//! ( the Generate form, when open )
//! > 1  aurora-gradient            3:00       NOW
//!      0:00  crossfade 0:04 . Morning        09:00:00
//!   2  voronoi-glass              3:00       NEXT
//! ```
//!
//! The pane edits nothing itself: loading, jumping, generating and saving
//! are [`Action`]s the app carries out, because they touch files, scenes
//! and the clock. What it shows is read off the [`Show`](crate::show::Show):
//! the rows (built once per playlist), the clock (current cue, next cue,
//! failed cues) and the precompile progress.
//!
//! Every cue shows its state in words and an icon, never colour alone:
//! `NOW`, `NEXT`, `skipped` (with the reason on hover), or nothing. A cue
//! whose scene failed to compile stays in the list, struck out in meaning
//! if not in glyphs, so the timeline still matches the file.

use super::form::{self, Row};
use super::icon as icons;
use super::model::{Action, Model};
use super::theme::{self, SPACE_MD, SPACE_SM, SPACE_XS};
use super::widgets::{self, Tone};
use crate::icon::Icon;
use crate::palette::Palette;
use crate::show::generate::TransitionChoice;
use crate::show::view::{self, CueRow};
use crate::show::{Phase, Show};
use crate::workspace::Pane;
use egui::{CornerRadius, Rect, Sense, Ui, Vec2};

/// Cue lines the Generate preview lists.
const PREVIEW_LINES: usize = 6;

/// Draws the pane's contents.
pub fn show(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    if widgets::pane_header(ui, theme, Icon::Playlist, Pane::Playlist.title(), |_| {}) {
        model.ui.panes.close(Pane::Playlist);
    }
    let show = model.playlist.show;
    let Some(show) = show else {
        empty(ui, model, theme, actions);
        return;
    };
    status(ui, model, theme);
    summary(ui, model, show, theme, actions);
    if show.phase() == Phase::Compiling {
        compiling(ui, show, theme);
    }
    command_row(ui, model, show, theme, actions);
    external_change(ui, model, theme, actions);
    if model.playlist.ui.generate_open {
        generate_card(ui, model, theme, actions);
    }
    cue_card(ui, model, show, theme, actions);
    disabled_cues(ui, show, theme, actions);
    ui.add_space(SPACE_XS);
    timeline(ui, model, show, theme, actions);
}

/// The cues the playlist keeps but skips, which the timeline does not list,
/// each with Enable (D213). Folded away until opened; absent when there
/// are none.
fn disabled_cues(ui: &mut Ui, show: &Show, theme: &Palette, actions: &mut Vec<Action>) {
    let sections = &show.playlist().sections;
    let count = sections
        .iter()
        .flat_map(|section| &section.cues)
        .filter(|cue| !cue.enabled)
        .count();
    if count == 0 {
        return;
    }
    egui::CollapsingHeader::new(
        egui::RichText::new(format!("Disabled cues ({count})"))
            .small()
            .color(theme::rgb(theme.text_muted)),
    )
    .id_salt("disabled-cues")
    .show(ui, |ui| {
        for (s, section) in sections.iter().enumerate() {
            for (c, cue) in section
                .cues
                .iter()
                .enumerate()
                .filter(|(_, cue)| !cue.enabled)
            {
                ui.horizontal(|ui| {
                    if ui
                        .small_button("Enable")
                        .on_hover_text("Play this cue again (Save to keep the change)")
                        .clicked()
                    {
                        actions.push(Action::EditCue {
                            at: crate::show::edit::At { section: s, cue: c },
                            edit: crate::show::edit::CueEdit::SetEnabled(true),
                        });
                    }
                    ui.label(
                        egui::RichText::new(format!("{} · {}", section.name, cue.scene.to_text()))
                            .small()
                            .color(theme::rgb(theme.text_faint)),
                    );
                });
            }
        }
    });
}

/// The playlist file row: the path, typed or chosen with Browse, and Open.
/// Enter in the field opens too.
fn open_row(
    group: &mut form::Group<'_>,
    model: &mut Model<'_>,
    theme: &Palette,
    actions: &mut Vec<Action>,
) {
    let picking = model.picking;
    group.wide_row(
        Row::new("Playlist file").description(
            "A .toml playlist. One with an error is refused, and says where the error is.",
        ),
        |ui| {
            let ready = !model.playlist.ui.path.trim().is_empty();
            let go = ui
                .add_enabled_ui(ready, |ui| widgets::primary_button(ui, theme, "Open"))
                .inner
                .clicked();
            if widgets::browse_button(ui, theme, picking) {
                actions.push(Action::Browse(crate::picker::Purpose::Playlist));
            }
            let width = ui.available_size_before_wrap().x;
            let edit = form::field(
                ui,
                theme,
                &mut model.playlist.ui.path,
                "playlists/example.toml",
                width,
            );
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let path = model.playlist.ui.path.trim();
            if (go || enter) && !path.is_empty() {
                actions.push(Action::LoadPlaylist(path.to_string()));
            }
        },
    );
}

/// The last load's outcome.
fn status(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette) {
    let mut dismissed = false;
    if let Some((is_error, message)) = &model.playlist.ui.status {
        let tone = if *is_error { Tone::Error } else { Tone::Info };
        widgets::banner(ui, theme, tone, message, |ui| {
            dismissed = widgets::button(ui, theme, None, "Dismiss").clicked();
        });
        ui.add_space(SPACE_XS);
    }
    if dismissed {
        model.playlist.ui.status = None;
    }
}

/// The file changed on disk while there were unsaved edits (D213): both
/// are kept until the operator picks one.
fn external_change(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    let Some(path) = model.playlist.ui.external_change.clone() else {
        return;
    };
    let message = format!(
        "{path} changed on disk. Your unsaved edits are kept: Save writes them over the file, \
         or take the file's version instead."
    );
    widgets::banner(ui, theme, Tone::Warning, &message, |ui| {
        if widgets::button(ui, theme, None, "Use the file")
            .on_hover_text("Throw away the unsaved edits and load the file as it is now")
            .clicked()
        {
            actions.push(Action::ReloadPlaylistFromDisk);
        }
    });
    ui.add_space(SPACE_XS);
}

/// Where an inherited setting came from, in words.
fn origin_word(origin: strobe_playlist::Origin) -> &'static str {
    match origin {
        strobe_playlist::Origin::Cue => "set on this cue",
        strobe_playlist::Origin::Section => "from the section",
        strobe_playlist::Origin::Playlist => "from the playlist",
        strobe_playlist::Origin::Builtin => "the default",
    }
}

/// The selected cue: its settings, where each comes from, and the edits
/// (D213). Every change is an [`Action::EditCue`]; the app rebuilds the
/// playlist and the show keeps its place.
fn cue_card(
    ui: &mut Ui,
    model: &mut Model<'_>,
    show: &Show,
    theme: &Palette,
    actions: &mut Vec<Action>,
) {
    use crate::show::edit::{At, CueEdit};
    let Some(row) = model
        .playlist
        .ui
        .selected
        .and_then(|index| show.rows().get(index))
    else {
        return;
    };
    let at = At {
        section: row.section,
        cue: row.cue,
    };
    let rev = show.rev();
    let draft = &mut model.playlist.ui.cue;
    if draft.for_cue != Some((at, rev)) {
        let resolved = show
            .playlist()
            .sections
            .get(at.section)
            .and_then(|s| s.cues.get(at.cue));
        draft.for_cue = Some((at, rev));
        draft.length = resolved
            .and_then(|c| c.settings.duration.as_ref())
            .map(ToString::to_string)
            .unwrap_or_default();
        let transition = resolved.and_then(|c| c.settings.transition.as_ref());
        draft.transition = match transition.map(|t| &t.kind) {
            None => TransitionChoice::Inherit,
            Some(strobe_playlist::TransitionKind::Cut) => TransitionChoice::Cut,
            Some(strobe_playlist::TransitionKind::Crossfade) => TransitionChoice::Crossfade,
            Some(
                strobe_playlist::TransitionKind::FadeThroughBlack
                | strobe_playlist::TransitionKind::Shader(_),
            ) => TransitionChoice::FadeThroughBlack,
        };
        draft.transition_length = transition
            .map(|t| t.duration.to_string())
            .unwrap_or_else(|| "4s".to_string());
        draft.preset = row.preset.clone().unwrap_or_default();
        draft.media = row.media.clone().unwrap_or_default();
    }
    let mut edits: Vec<CueEdit> = Vec::new();
    let mut problem: Option<String> = None;
    let title = match model.playlist.ui.also.len() {
        0 => format!("Cue {}: {}", row.index + 1, row.scene),
        more => format!(
            "Cue {}: {} and {more} more (settings go to all)",
            row.index + 1,
            row.scene
        ),
    };
    let picked = model
        .ui
        .browser
        .selected
        .and_then(|index| model.library.entries.get(index))
        .map(|entry| entry.name.clone());
    let length_now = format!(
        "Now {}, {}.",
        row.length_label,
        origin_word(row.origins.duration)
    );
    let transition_now = format!(
        "Now {}, {}.",
        row.transition_label,
        origin_word(row.origins.transition)
    );
    form::inset(ui, theme, |card| {
        card.block(|ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new(&title)
                        .font(theme::heading_font())
                        .color(theme::rgb(theme.text)),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    for (label, edit, tip) in [
                        (
                            "Delete",
                            CueEdit::Delete,
                            "Remove this cue (Save to keep the change)",
                        ),
                        (
                            "Disable",
                            CueEdit::SetEnabled(false),
                            "Keep it in the file but skip it; it is listed under Disabled cues",
                        ),
                        ("Duplicate", CueEdit::Duplicate, "A copy straight after it"),
                        ("Down", CueEdit::MoveDown, "One place later in its section"),
                        ("Up", CueEdit::MoveUp, "One place earlier in its section"),
                    ] {
                        if widgets::button(ui, theme, None, label)
                            .on_hover_text(tip)
                            .clicked()
                        {
                            edits.push(edit);
                        }
                    }
                });
            });
        });
        card.row(Row::new("Length").description(&length_now), |ui| {
            if row.origins.duration == strobe_playlist::Origin::Cue
                && widgets::button(ui, theme, None, "Inherit")
                    .on_hover_text("Take the section's or the playlist's length again")
                    .clicked()
            {
                edits.push(CueEdit::SetDuration(None));
            }
            let set = widgets::button(ui, theme, None, "Set").clicked();
            let length = form::field(ui, theme, &mut draft.length, &row.length_label, 120.0);
            let enter = length.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if set || enter {
                let text = draft.length.trim();
                if text.is_empty() {
                    edits.push(CueEdit::SetDuration(None));
                } else {
                    match strobe_playlist::DurationSpec::parse(text) {
                        Ok(spec) => edits.push(CueEdit::SetDuration(Some(spec))),
                        Err(error) => problem = Some(format!("Length: {error}")),
                    }
                }
            }
        });
        card.wide_row(Row::new("Transition").description(&transition_now), |ui| {
            if widgets::button(ui, theme, None, "Set").clicked() {
                let seconds = || -> Result<strobe_playlist::DurationSpec, String> {
                    strobe_playlist::DurationSpec::parse(&draft.transition_length)
                        .map_err(|e| format!("Transition length: {e}"))
                };
                let spec = match draft.transition {
                    TransitionChoice::Inherit => Ok(None),
                    TransitionChoice::Cut => Ok(Some(strobe_playlist::TransitionSpec::cut())),
                    TransitionChoice::Crossfade => seconds().map(|duration| {
                        Some(strobe_playlist::TransitionSpec {
                            kind: strobe_playlist::TransitionKind::Crossfade,
                            duration,
                        })
                    }),
                    TransitionChoice::FadeThroughBlack => seconds().map(|duration| {
                        Some(strobe_playlist::TransitionSpec {
                            kind: strobe_playlist::TransitionKind::FadeThroughBlack,
                            duration,
                        })
                    }),
                };
                match spec {
                    Ok(spec) => edits.push(CueEdit::SetTransition(spec)),
                    Err(why) => problem = Some(why),
                }
            }
            form::field(ui, theme, &mut draft.transition_length, "4s", 70.0);
            // A list, not four buttons in a row: the row widened the
            // card past the pane and cut off Set (U-09).
            egui::ComboBox::from_id_salt("cue-transition")
                .selected_text(draft.transition.label())
                .width(130.0)
                .show_ui(ui, |ui| {
                    for choice in TransitionChoice::ALL {
                        ui.selectable_value(&mut draft.transition, choice, choice.label());
                    }
                });
        });
        card.row(
            Row::new("Preset")
                .description("The scene starts from this preset; empty is its own start."),
            |ui| {
                if widgets::button(ui, theme, None, "Set").clicked() {
                    edits.push(CueEdit::SetPreset(Some(draft.preset.clone())));
                }
                form::field(ui, theme, &mut draft.preset, "the scene's own start", 160.0);
            },
        );
        card.row(
            Row::new("Media").description(
                "A picture or folder of pictures behind syn_Media while this cue plays; a \
                 relative path is looked for beside the playlist.",
            ),
            |ui| {
                if widgets::button(ui, theme, None, "Set").clicked() {
                    edits.push(CueEdit::SetMedia(Some(draft.media.clone())));
                }
                form::field(ui, theme, &mut draft.media, "the scene's own", 160.0);
            },
        );
        let after = match &picked {
            Some(name) => format!("Adds {name}, the scene selected in the library."),
            None => "Select a scene in the library to add it after this cue.".to_string(),
        };
        card.row(Row::new("Add a scene after").description(&after), |ui| {
            let add = ui
                .add_enabled_ui(picked.is_some(), |ui| {
                    widgets::button(ui, theme, Some(Icon::Library), "Add after")
                })
                .inner;
            if add.clicked()
                && let Some(name) = &picked
            {
                edits.push(CueEdit::InsertAfter(strobe_playlist::SceneRef::Library(
                    name.clone(),
                )));
            }
        });
        card.wide_row(
            Row::new("Import a shader after").description(
                "Imported into the library and added as a cue after this one; one already in \
                 the library is used as it is.",
            ),
            |ui| {
                let text = draft.shadertoy.trim().to_string();
                let import = ui
                    .add_enabled_ui(!text.is_empty(), |ui| {
                        widgets::button(ui, theme, Some(Icon::Import), "Import after")
                    })
                    .inner;
                if import.clicked() {
                    actions.push(Action::ImportCueAfter { at, text });
                    draft.shadertoy.clear();
                }
                let width = ui.available_size_before_wrap().x;
                form::field(
                    ui,
                    theme,
                    &mut draft.shadertoy,
                    "Shadertoy URL or id",
                    width,
                );
            },
        );
        if let Some(why) = &problem {
            card.block(|ui| {
                ui.label(
                    egui::RichText::new(why)
                        .font(theme::description_font())
                        .color(theme::rgb(theme.danger)),
                );
            });
        }
    });
    let also: Vec<At> = model
        .playlist
        .ui
        .also
        .iter()
        .filter_map(|index| show.rows().get(*index))
        .map(|row| At {
            section: row.section,
            cue: row.cue,
        })
        .collect();
    for edit in edits {
        let shared = matches!(
            edit,
            CueEdit::SetDuration(_)
                | CueEdit::SetTransition(_)
                | CueEdit::SetPreset(_)
                | CueEdit::SetMedia(_)
                | CueEdit::SetEnabled(_)
        );
        if shared && !also.is_empty() {
            let mut ats = vec![at];
            ats.extend(also.iter().copied());
            actions.push(Action::EditCues { ats, edit });
        } else {
            actions.push(Action::EditCue { at, edit });
        }
    }
    ui.add_space(SPACE_XS);
}

/// Nothing loaded: a page with the two ways to get a playlist, opening a
/// file or generating one.
fn empty(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    egui::ScrollArea::vertical()
        .id_salt("playlist-empty")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            form::page(
                ui,
                theme,
                "Playlist",
                Some(
                    "A show plan: scenes in sections, each cue with its length and transition. \
                     With none loaded, Previous and Next step through the library.",
                ),
                |ui| {
                    status(ui, model, theme);
                    form::group(ui, theme, "Open a playlist", |group| {
                        open_row(group, model, theme, actions);
                    });
                    recent(ui, model, theme, actions);
                    form::group(ui, theme, "Or generate one", |group| {
                        let open = model.playlist.ui.generate_open;
                        group.row(
                            Row::new("Generate a section").description(
                                "Pick scenes from the library by tags, for a length, with a \
                                 transition, and start a playlist from them.",
                            ),
                            |ui| {
                                if widgets::tool_button(
                                    ui,
                                    theme,
                                    Icon::Playlist,
                                    "Generate...",
                                    open,
                                    "Build a section from the library",
                                )
                                .clicked()
                                {
                                    model.playlist.ui.generate_open = !open;
                                }
                            },
                        );
                        if model.playlist.ui.generate_open {
                            group.block(|ui| generate_form(ui, model, theme, actions));
                        }
                    });
                    ui.add_space(theme::SPACE_LG);
                },
            );
        });
}

/// The playlists opened last (D481), one row each: the file's name, its
/// folder, and Open. Nothing when there are none.
fn recent(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    if model.ui.recent_playlists.is_empty() {
        return;
    }
    form::group(ui, theme, "Recent", |group| {
        for path in &model.ui.recent_playlists {
            let file = std::path::Path::new(path);
            let name = file
                .file_stem()
                .map_or_else(|| path.clone(), |stem| stem.to_string_lossy().into_owned());
            let folder = file
                .parent()
                .map(|dir| dir.display().to_string())
                .filter(|dir| !dir.is_empty())
                .unwrap_or_else(|| "the working folder".to_string());
            group.row(Row::new(&name).description(&folder), |ui| {
                if widgets::button(ui, theme, None, "Open")
                    .on_hover_text(path.as_str())
                    .clicked()
                {
                    actions.push(Action::LoadPlaylist(path.clone()));
                }
            });
        }
    });
}

/// Name, size, loop and anchor, the file, and the file commands.
fn summary(
    ui: &mut Ui,
    model: &mut Model<'_>,
    show: &Show,
    theme: &Palette,
    actions: &mut Vec<Action>,
) {
    let timeline = show.clock().timeline();
    let loop_word = match timeline.loop_mode() {
        strobe_playlist::LoopMode::Stop => "plays once",
        strobe_playlist::LoopMode::Loop => "loops",
        strobe_playlist::LoopMode::LoopSection => "loops the section",
    };
    let anchor = timeline.wall_time(0).map_or_else(String::new, |t| {
        // 09:00:00 reads 09:00; only the seconds go, and only when zero.
        let text = t.to_string();
        format!(
            "  ·  anchored {}",
            text.strip_suffix(":00").unwrap_or(&text)
        )
    });
    let runtime = view::clock(timeline.total_runtime());
    let open = if timeline.has_open_ended() {
        " (some cues wait for you)"
    } else {
        ""
    };
    let meta = format!(
        "{} cues  ·  {runtime}{open}  ·  {loop_word}{anchor}",
        show.rows().len()
    );
    let file = show
        .source()
        .map_or_else(|| "Not saved yet".to_string(), |p| p.display().to_string());
    let picking = model.picking;
    let dirty = model.playlist.ui.dirty;
    form::inset(ui, theme, |card| {
        let mut row = Row::new(show.name())
            .description(&meta)
            .note(&file, theme.text_faint);
        if dirty {
            row = row.status(Icon::Info, "Unsaved", theme.warning);
        }
        card.wide_row(row, |ui| {
            if widgets::button(ui, theme, Some(Icon::Close), "Unload")
                .on_hover_text("Stop the playlist and go back to library order")
                .clicked()
            {
                actions.push(Action::UnloadPlaylist);
            }
            if widgets::button(ui, theme, Some(Icon::Copy), "Save")
                .on_hover_text(
                    "Write the playlist to its file. The file is rewritten from the model, so \
                     comments in it are not kept.",
                )
                .clicked()
            {
                actions.push(Action::SavePlaylist);
            }
            let open = ui
                .add_enabled_ui(!picking, |ui| {
                    widgets::button(ui, theme, Some(Icon::Folder), "Open...")
                        .on_hover_text("Open another playlist file")
                })
                .inner;
            if open.clicked() {
                actions.push(Action::Browse(crate::picker::Purpose::Playlist));
            }
        });
    });
    ui.add_space(SPACE_SM);
}

/// The precompile's progress bar, in words and a meter.
fn compiling(ui: &mut Ui, show: &Show, theme: &Palette) {
    let progress = show.progress();
    let fraction = if progress.total == 0 {
        1.0
    } else {
        progress.done as f32 / progress.total as f32
    };
    ui.horizontal(|ui| {
        icons::show(ui, Icon::Refresh, icons::INLINE, theme::rgb(theme.accent));
        ui.label(
            egui::RichText::new(format!(
                "Compiling {} of {}{}",
                progress.done,
                progress.total,
                if progress.failed > 0 {
                    format!(" ({} failed)", progress.failed)
                } else {
                    String::new()
                }
            ))
            .small(),
        );
    });
    let width = ui.available_width();
    widgets::meter(ui, theme, fraction, theme.accent, width);
    ui.add_space(SPACE_XS);
}

/// The cue commands and the file buttons.
fn command_row(
    ui: &mut Ui,
    model: &mut Model<'_>,
    show: &Show,
    theme: &Palette,
    actions: &mut Vec<Action>,
) {
    let playing = show.phase() == Phase::Ready && show.clock().current().is_some();
    ui.horizontal_wrapped(|ui| {
        ui.add_enabled_ui(playing, |ui| {
            if widgets::button(ui, theme, None, "+30 s")
                .on_hover_text("Give the current cue 30 more seconds (E)")
                .clicked()
            {
                actions.push(Action::ExtendCue);
            }
            if widgets::button(ui, theme, Some(Icon::Reset), "Restart")
                .on_hover_text("Start the current cue again (R)")
                .clicked()
            {
                actions.push(Action::RestartCue);
            }
            if widgets::button(ui, theme, Some(Icon::Next), "Skip section")
                .on_hover_text("Go to the first cue of the next section (Shift+Right)")
                .clicked()
            {
                actions.push(Action::SkipSection);
            }
        });
        let open = model.playlist.ui.generate_open;
        if widgets::tool_button(
            ui,
            theme,
            Icon::Playlist,
            "Generate...",
            open,
            "Add a generated section",
        )
        .clicked()
        {
            model.playlist.ui.generate_open = !open;
        }
    });
    ui.add_space(SPACE_XS);
}

/// The Generate form, in the loaded pane above the cue list.
fn generate_card(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    generate_form(ui, model, theme, actions);
    ui.add_space(SPACE_SM);
}

/// The Generate form: an inset card of setting rows, the buttons, and what
/// the last Preview made.
fn generate_form(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    let mut changed = false;
    let state = &mut *model.playlist.ui;
    form::inset(ui, theme, |rows| {
        let form = &mut state.form;
        let fields: [(&str, &mut String, &str, &str); 7] = [
            (
                "Section",
                &mut form.section_name,
                "The new section's name.",
                "Late set",
            ),
            (
                "Target length",
                &mut form.target,
                "How long the section runs: 30m, 1h30m, 4h.",
                "1h",
            ),
            (
                "Pool",
                &mut form.pool,
                "Tags or words to pick scenes by, comma separated: a scene's tags, a word in \
                 its name, or its whole name. Empty is the whole library.",
                "ambient, slow",
            ),
            (
                "Exclude",
                &mut form.exclude,
                "Tags or words never to pick, even if the pool asks for them.",
                "heavy, dark",
            ),
            (
                "Cue duration",
                &mut form.duration,
                "Each cue's length: 3m, 90s, 32 bars, rand(2m, 4m).",
                "3m",
            ),
            (
                "No-repeat",
                &mut form.window,
                "How many recent scenes not to play again.",
                "8",
            ),
            (
                "Seed",
                &mut form.seed,
                "The same seed gives the same section.",
                "1",
            ),
        ];
        for (title, text, description, hint) in fields {
            rows.row(Row::new(title).description(description), |ui| {
                let width = ui.available_size_before_wrap().x;
                changed |= form::field(ui, theme, text, hint, width).changed();
            });
        }
        rows.wide_row(
            Row::new("Transition").description("How generated cues arrive."),
            |ui| {
                // Right to left: the last choice first.
                for choice in TransitionChoice::ALL.into_iter().rev() {
                    if ui
                        .selectable_label(form.transition == choice, choice.label())
                        .clicked()
                    {
                        form.transition = choice;
                        changed = true;
                    }
                }
            },
        );
        if matches!(
            form.transition,
            TransitionChoice::Crossfade | TransitionChoice::FadeThroughBlack
        ) {
            rows.row(
                Row::new("Transition length").description("How long each transition takes."),
                |ui| {
                    changed |=
                        form::field(ui, theme, &mut form.transition_length, "0:04", 80.0).changed();
                },
            );
        }
        let can_add = matches!(state.preview, Some(Ok(_)));
        rows.block(|ui| {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::button(ui, theme, Some(Icon::Close), "Close").clicked() {
                    state.generate_open = false;
                }
                ui.add_enabled_ui(can_add, |ui| {
                    if widgets::button(ui, theme, Some(Icon::Check), "Add to playlist").clicked() {
                        actions.push(Action::GenerateCommit);
                    }
                });
                if widgets::primary_button(ui, theme, "Preview").clicked() {
                    actions.push(Action::GeneratePreview);
                }
            });
        });
    });
    if changed {
        // The preview was made from other fields.
        state.preview = None;
        state.excluded.clear();
    }
    preview_result(ui, model, theme);
}

/// What the last Preview made.
fn preview_result(ui: &mut Ui, model: &Model<'_>, theme: &Palette) {
    match &model.playlist.ui.preview {
        None => {}
        Some(Err(message)) => {
            ui.add_space(SPACE_XS);
            ui.label(
                egui::RichText::new(message)
                    .small()
                    .color(theme::rgb(theme.danger)),
            );
        }
        Some(Ok(made)) => {
            ui.add_space(SPACE_XS);
            ui.label(
                egui::RichText::new(format!(
                    "{} cues, about {}, from a pool of {} scene(s), no repeat within {}",
                    made.section.cues.len(),
                    view::clock(made.estimated_seconds),
                    made.pool_size,
                    made.effective_window
                ))
                .small()
                .color(theme::rgb(theme.success)),
            );
            let every: Vec<String> = made
                .pool
                .iter()
                .map(|(name, why)| format!("{name}: {why}"))
                .collect();
            ui.label(
                egui::RichText::new(format!(
                    "Pool: {}",
                    crate::show::generate::pool_summary(&made.pool)
                ))
                .small()
                .color(theme::rgb(theme.text_muted)),
            )
            .on_hover_text(every.join("\n"));
            let excluded = &model.playlist.ui.excluded;
            if !excluded.is_empty() {
                // Why the rest were skipped, so an exclusion is never a
                // mystery (D264).
                let each: Vec<String> = excluded
                    .iter()
                    .map(|(name, why)| format!("{name}: {why}"))
                    .collect();
                ui.label(
                    egui::RichText::new(format!(
                        "Skipped: {}",
                        crate::show::generate::pool_summary(excluded)
                    ))
                    .small()
                    .color(theme::rgb(theme.text_muted)),
                )
                .on_hover_text(each.join("\n"));
            }
            for cue in made.section.cues.iter().take(PREVIEW_LINES) {
                let length = cue
                    .settings
                    .duration
                    .as_ref()
                    .map_or_else(String::new, ToString::to_string);
                ui.label(
                    egui::RichText::new(format!("   {}   {length}", cue.scene))
                        .small()
                        .color(theme::rgb(theme.text_muted)),
                );
            }
            if made.section.cues.len() > PREVIEW_LINES {
                ui.label(
                    egui::RichText::new(format!(
                        "   and {} more",
                        made.section.cues.len() - PREVIEW_LINES
                    ))
                    .small()
                    .color(theme::rgb(theme.text_faint)),
                );
            }
        }
    }
}

/// The cue list, virtualised: a 4-hour, 80-cue show draws a screenful.
/// The cue list, virtualised, scrolled to the current cue once per cue.
/// The side panel's Playlist tab draws the same list (D283).
pub(super) fn timeline(
    ui: &mut Ui,
    model: &mut Model<'_>,
    show: &Show,
    theme: &Palette,
    actions: &mut Vec<Action>,
) {
    let rows = show.rows();
    if rows.is_empty() {
        widgets::empty_state(
            ui,
            theme,
            Icon::Playlist,
            "No cues",
            "Every cue in this playlist is disabled, or there are none. Use Generate to add a section.",
        );
        return;
    }
    let current = show.clock().current();
    let next = show.clock().peek_next();
    ui.spacing_mut().item_spacing.y = 0.0;
    let mut area = egui::ScrollArea::vertical()
        .id_salt("playlist-timeline")
        .auto_shrink([false, false]);
    if model.playlist.ui.scrolled_to != current
        && let Some(index) = current
    {
        model.playlist.ui.scrolled_to = current;
        let offset = (index as f32 * theme::row_height() - ui.available_height() / 2.0).max(0.0);
        area = area.vertical_scroll_offset(offset);
    }
    area.show_rows(ui, theme::row_height(), rows.len(), |ui, range| {
        for position in range {
            if let Some(row) = rows.get(position) {
                cue_row(ui, model, show, theme, row, current, next, actions);
            }
        }
    });
}

#[expect(
    clippy::too_many_arguments,
    reason = "paints one row from the pieces the list already holds"
)]
fn cue_row(
    ui: &mut Ui,
    model: &mut Model<'_>,
    show: &Show,
    theme: &Palette,
    row: &CueRow,
    current: Option<usize>,
    next: Option<usize>,
    actions: &mut Vec<Action>,
) {
    let is_now = current == Some(row.index);
    let is_next = next == Some(row.index) && !is_now;
    let failure = show.cue_failure(row.index);
    let selected = model.playlist.ui.selected == Some(row.index)
        || model.playlist.ui.also.contains(&row.index);
    let width = ui.available_width();
    let (rect, response) =
        ui.allocate_exact_size(Vec2::new(width, theme::row_height()), Sense::click());
    let id = egui::Id::new(("cue-row", row.index));
    let fill = widgets::selection_fill(ui, theme, id, selected, response.hovered(), theme.panel);
    ui.painter().rect_filled(rect, CornerRadius::ZERO, fill);
    // A scene dragged from the browser lands straight after this cue (D224).
    if response
        .dnd_hover_payload::<super::browser::LibraryDrag>()
        .is_some()
    {
        let line = egui::Rect::from_min_max(
            egui::pos2(rect.left(), rect.bottom() - 2.0),
            rect.right_bottom(),
        );
        ui.painter()
            .rect_filled(line, CornerRadius::ZERO, theme::rgb(theme.accent));
    }
    if let Some(dropped) = response.dnd_release_payload::<super::browser::LibraryDrag>() {
        actions.push(Action::EditCue {
            at: crate::show::edit::At {
                section: row.section,
                cue: row.cue,
            },
            edit: crate::show::edit::CueEdit::InsertAfter(strobe_playlist::SceneRef::Library(
                dropped.0.clone(),
            )),
        });
    }
    if is_now {
        widgets::accent_bar(
            ui,
            ui.painter(),
            id,
            rect,
            widgets::Bar::Flush,
            theme.accent,
        );
    }

    let inset = theme::density().row_inset();
    let swatch = egui::pos2(rect.left() + SPACE_MD + 3.0, rect.top() + inset + 8.0);
    ui.painter().circle_filled(
        swatch,
        5.0,
        theme::rgb(widgets::scene_colour(theme, &row.scene)),
    );
    let text_x = rect.left() + SPACE_MD + 18.0;
    let state_width = 64.0;
    let text_width = (rect.right() - text_x - SPACE_SM - state_width).max(40.0);
    let name_colour = if failure.is_some() {
        theme.text_faint
    } else if is_now {
        theme.accent
    } else {
        theme.text
    };
    let name = widgets::truncated(
        ui,
        &format!("{:>2}  {}", row.index + 1, row.scene),
        egui::TextStyle::Body.resolve(ui.style()),
        name_colour,
        text_width,
    );
    ui.painter().galley(
        egui::pos2(text_x, rect.top() + inset),
        name,
        theme::rgb(name_colour),
    );
    let detail = match (failure, &row.wall) {
        (Some(_), _) => format!(
            "skipped: its scene failed to compile   {}",
            row.section_name
        ),
        (None, Some(wall)) => format!(
            "at {} \u{b7} {} \u{b7} {} \u{b7} {} \u{b7} {wall}",
            view::clock(row.start),
            row.length_label,
            row.transition_label,
            row.section_name
        ),
        (None, None) => format!(
            "at {} \u{b7} {} \u{b7} {} \u{b7} {}",
            view::clock(row.start),
            row.length_label,
            row.transition_label,
            row.section_name
        ),
    };
    let detail_colour = if failure.is_some() {
        theme.danger
    } else {
        theme.text_muted
    };
    let galley = widgets::truncated(
        ui,
        &detail,
        egui::TextStyle::Small.resolve(ui.style()),
        detail_colour,
        text_width,
    );
    ui.painter().galley(
        egui::pos2(text_x, rect.bottom() - galley.size().y - inset - 1.0),
        galley,
        theme::rgb(detail_colour),
    );

    // The state, at the right edge: words and an icon.
    let state: Option<(Icon, &str, _)> = if failure.is_some() {
        Some((Icon::Error, "skipped", theme.danger))
    } else if is_now {
        Some((Icon::Play, "NOW", theme.accent))
    } else if is_next {
        Some((Icon::Next, "NEXT", theme.text_muted))
    } else if row.open_ended {
        Some((Icon::Pause, "manual", theme.text_faint))
    } else {
        None
    };
    if let Some((icon, word, colour)) = state {
        let right = rect.right() - SPACE_SM;
        icons::paint(
            ui.painter(),
            Rect::from_center_size(
                egui::pos2(right - state_width + 8.0, rect.top() + inset + 8.0),
                Vec2::splat(icons::DISCLOSURE),
            ),
            icon,
            theme::rgb(colour),
        );
        ui.painter().text(
            egui::pos2(right - state_width + 20.0, rect.top() + inset + 8.0),
            egui::Align2::LEFT_CENTER,
            word,
            egui::FontId::new(10.5, egui::FontFamily::Proportional),
            theme::rgb(colour),
        );
    }

    let hover = match failure {
        Some(reason) => format!("{}: {reason}", row.scene),
        None => format!(
            "Double-click to jump here. {} control override(s).",
            row.overrides
        ),
    };
    let response = response.on_hover_text(hover);
    if response.clicked() {
        let list = &mut model.playlist.ui;
        // Ctrl-click adds a cue to the selection or takes it out (D230).
        if ui.input(|input| input.modifiers.command) && list.selected.is_some() {
            if list.selected == Some(row.index) {
                list.selected = (!list.also.is_empty()).then(|| list.also.remove(0));
            } else if let Some(at) = list.also.iter().position(|i| *i == row.index) {
                list.also.remove(at);
            } else {
                list.also.push(row.index);
            }
        } else {
            list.selected = Some(row.index);
            list.also.clear();
        }
    }
    if response.double_clicked() && failure.is_none() {
        actions.push(Action::JumpToCue(row.index));
    }
}
