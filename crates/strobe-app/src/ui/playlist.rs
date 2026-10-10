// ============================================================================
// Module:       strobe_app::ui::playlist
// Description:  The LIST pane: the load field, the playlist's timeline with
//               each cue's computed start, length and state, the cue
//               commands, and the Generate dialog.
//
// Dependencies: egui; super::{model, theme, widgets, icon}, crate::{show,
//               icon, workspace}
// ============================================================================

//! The Playlist pane (UI_DESIGN section 13).
//!
//! ```text
//! PLAYLIST                                                    x
//! [ playlists/example.toml                    ] [Load]
//! (an error banner, or the compile progress)
//! Example: lobby   12 cues   42:10   loop   anchored 09:00
//! [+30 s] [Restart] [Skip section] [Generate...] [Save] [Unload]
//! ( the Generate card, when open )
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

use super::icon as icons;
use super::model::{Action, Model};
use super::theme::{self, ROW_HEIGHT, SPACE_MD, SPACE_SM, SPACE_XS};
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
    load_row(ui, model, theme, actions);
    status(ui, model, theme);
    let show = model.playlist.show;
    let Some(show) = show else {
        empty(ui, model, theme, actions);
        return;
    };
    summary(ui, show, theme);
    if show.phase() == Phase::Compiling {
        compiling(ui, show, theme);
    }
    command_row(ui, model, show, theme, actions);
    if model.playlist.ui.generate_open {
        generate_card(ui, model, theme, actions);
    }
    ui.add_space(SPACE_XS);
    timeline(ui, model, show, theme, actions);
}

/// The path field and its button. Enter in the field loads too.
fn load_row(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        let button_width = 64.0;
        let edit = ui.add(
            egui::TextEdit::singleline(&mut model.playlist.ui.path)
                .hint_text("playlists/example.toml")
                .desired_width(ui.available_width() - button_width - SPACE_SM),
        );
        let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        let go = widgets::button(ui, theme, None, "Load")
            .on_hover_text("Load this playlist file. A file with an error is refused and the current playlist stays.")
            .clicked();
        let path = model.playlist.ui.path.trim();
        if (go || enter) && !path.is_empty() {
            actions.push(Action::LoadPlaylist(path.to_string()));
        }
    });
    ui.add_space(SPACE_XS);
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

/// Nothing loaded: say what to do, and still offer Generate.
fn empty(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        if widgets::button(ui, theme, Some(Icon::Playlist), "Generate...")
            .on_hover_text("Build a section from the library and start a playlist from it")
            .clicked()
        {
            model.playlist.ui.generate_open = !model.playlist.ui.generate_open;
        }
    });
    if model.playlist.ui.generate_open {
        generate_card(ui, model, theme, actions);
        return;
    }
    widgets::empty_state(
        ui,
        theme,
        Icon::Playlist,
        "No playlist loaded",
        "Type a playlist file's path above and press Load, or start the app with --playlist <file>. \
         Until then Previous and Next step through the library.",
    );
}

/// Name, size, loop and anchor.
fn summary(ui: &mut Ui, show: &Show, theme: &Palette) {
    let timeline = show.clock().timeline();
    let loop_word = match timeline.loop_mode() {
        strobe_playlist::LoopMode::Stop => "plays once",
        strobe_playlist::LoopMode::Loop => "loops",
        strobe_playlist::LoopMode::LoopSection => "loops the section",
    };
    let anchor = timeline.wall_time(0).map_or_else(String::new, |t| {
        format!("   anchored {}", t.to_string().trim_end_matches(":00"))
    });
    let runtime = view::clock(timeline.total_runtime());
    let open = if timeline.has_open_ended() {
        " (some cues wait for you)"
    } else {
        ""
    };
    egui::Frame::new()
        .fill(theme::rgb(theme.raised))
        .stroke(egui::Stroke::new(1.0, theme::rgb(theme.border)))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(theme::margin(SPACE_SM))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(egui::RichText::new(show.name()).strong());
            ui.label(
                egui::RichText::new(format!(
                    "{} cues   {runtime}{open}   {loop_word}{anchor}",
                    show.rows().len()
                ))
                .small()
                .color(theme::rgb(theme.text_muted)),
            );
            let file = show
                .source()
                .map_or_else(|| "not saved yet".to_string(), |p| p.display().to_string());
            ui.label(
                egui::RichText::new(file)
                    .small()
                    .color(theme::rgb(theme.text_faint)),
            );
        });
    ui.add_space(SPACE_XS);
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
        if widgets::button(ui, theme, Some(Icon::Playlist), "Generate...")
            .on_hover_text("Add a generated section")
            .clicked()
        {
            model.playlist.ui.generate_open = !model.playlist.ui.generate_open;
        }
        let save = if model.playlist.ui.dirty {
            "Save *"
        } else {
            "Save"
        };
        if widgets::button(ui, theme, Some(Icon::Copy), save)
            .on_hover_text(
                "Write the playlist to its file. The file is rewritten from the model, \
                 so comments in it are not kept.",
            )
            .clicked()
        {
            actions.push(Action::SavePlaylist);
        }
        if widgets::button(ui, theme, Some(Icon::Close), "Unload")
            .on_hover_text("Stop the playlist and go back to library order")
            .clicked()
        {
            actions.push(Action::UnloadPlaylist);
        }
    });
}

/// The Generate dialog, as a card in the pane.
fn generate_card(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    egui::Frame::new()
        .fill(theme::rgb(theme.raised))
        .stroke(egui::Stroke::new(1.0, theme::rgb(theme.accent)))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(theme::margin(SPACE_SM))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            widgets::section(ui, theme, "Generate a section");
            let state = &mut *model.playlist.ui;
            let form = &mut state.form;
            let mut changed = false;
            egui::Grid::new("generate-fields")
                .num_columns(2)
                .spacing([SPACE_SM, SPACE_XS])
                .show(ui, |ui| {
                    let mut field = |ui: &mut Ui, label: &str, text: &mut String, tip: &str| {
                        ui.label(
                            egui::RichText::new(label)
                                .small()
                                .color(theme::rgb(theme.text_muted)),
                        );
                        let edit = ui
                            .add(egui::TextEdit::singleline(text).desired_width(f32::INFINITY))
                            .on_hover_text(tip);
                        changed |= edit.changed();
                        ui.end_row();
                    };
                    field(ui, "Section", &mut form.section_name, "The new section's name");
                    field(ui, "Target length", &mut form.target, "How long the section should run: 30m, 1h30m, 4h");
                    field(
                        ui,
                        "Pool",
                        &mut form.pool,
                        "Words to pick scenes by, comma separated: a scene's name, or a word in it \
                         (there are no tags yet, so name words stand in). Empty is the whole library.",
                    );
                    field(ui, "Cue duration", &mut form.duration, "Each cue's length: 3m, 90s, 32 bars, rand(2m, 4m)");
                    field(ui, "No-repeat", &mut form.window, "How many recent scenes not to repeat");
                    field(ui, "Seed", &mut form.seed, "The same seed gives the same section");
                });
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    egui::RichText::new("Transition")
                        .small()
                        .color(theme::rgb(theme.text_muted)),
                );
                for choice in TransitionChoice::ALL {
                    let on = form.transition == choice;
                    if widgets::tool_button(ui, theme, Icon::Playlist, choice.label(), on, "How generated cues arrive")
                        .clicked()
                    {
                        form.transition = choice;
                        changed = true;
                    }
                }
            });
            if matches!(
                form.transition,
                TransitionChoice::Crossfade | TransitionChoice::FadeThroughBlack
            ) {
                ui.horizontal(|ui| {
                    ui.label(
                        egui::RichText::new("Transition length")
                            .small()
                            .color(theme::rgb(theme.text_muted)),
                    );
                    changed |= ui
                        .add(egui::TextEdit::singleline(&mut form.transition_length).desired_width(60.0))
                        .changed();
                });
            }
            if changed {
                // The preview was made from other fields.
                state.preview = None;
            }
            ui.add_space(SPACE_XS);
            let can_add = matches!(state.preview, Some(Ok(_)));
            ui.horizontal(|ui| {
                if widgets::primary_button(ui, theme, "Preview").clicked() {
                    actions.push(Action::GeneratePreview);
                }
                ui.add_enabled_ui(can_add, |ui| {
                    if widgets::button(ui, theme, Some(Icon::Check), "Add to playlist").clicked() {
                        actions.push(Action::GenerateCommit);
                    }
                });
                if widgets::button(ui, theme, Some(Icon::Close), "Close").clicked() {
                    state.generate_open = false;
                }
            });
            preview_result(ui, model, theme);
        });
    ui.add_space(SPACE_XS);
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
fn timeline(
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
        let offset = (index as f32 * ROW_HEIGHT - ui.available_height() / 2.0).max(0.0);
        area = area.vertical_scroll_offset(offset);
    }
    area.show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
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
    let selected = model.playlist.ui.selected == Some(row.index);
    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, ROW_HEIGHT), Sense::click());
    let id = egui::Id::new(("cue-row", row.index));
    let fill = widgets::selection_fill(ui, theme, id, selected, response.hovered(), theme.panel);
    ui.painter().rect_filled(rect, CornerRadius::ZERO, fill);
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

    let swatch = egui::pos2(rect.left() + SPACE_MD + 3.0, rect.top() + 14.0);
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
        egui::pos2(text_x, rect.top() + 6.0),
        name,
        theme::rgb(name_colour),
    );
    let detail = match (failure, &row.wall) {
        (Some(_), _) => format!(
            "skipped: its scene failed to compile   {}",
            row.section_name
        ),
        (None, Some(wall)) => format!(
            "{} {}  {}  {}  {wall}",
            view::clock(row.start),
            row.length_label,
            row.transition_label,
            row.section_name
        ),
        (None, None) => format!(
            "{}  {}  {}  {}",
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
        egui::pos2(text_x, rect.bottom() - galley.size().y - 7.0),
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
                egui::pos2(right - state_width + 8.0, rect.top() + 14.0),
                Vec2::splat(icons::DISCLOSURE),
            ),
            icon,
            theme::rgb(colour),
        );
        ui.painter().text(
            egui::pos2(right - state_width + 20.0, rect.top() + 14.0),
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
        model.playlist.ui.selected = Some(row.index);
    }
    if response.double_clicked() && failure.is_none() {
        actions.push(Action::JumpToCue(row.index));
    }
}
