// ============================================================================
// Module:       strobe_app::ui::side
// Description:  The persistent side panel: the live preview, the status strip
//               under it, the fav slots, and the Controls, Playlist and
//               Presets tabs.
//
// Dependencies: egui; super::{chrome, inspector, model, perform, playlist,
//               presets, theme, widgets, icon}, crate::{icon, workspace}
// ============================================================================

//! The side panel (UI_DESIGN section 7, D274).
//!
//! Synesthesia keeps one column on the right of every view: the preview,
//! a status strip, and tabs for the playlist and the controls. Strobe's
//! side panel is that column, drawn from the same model every other pane
//! reads:
//!
//! ```text
//! +--------------------------------+
//! |          PREVIEW (16:9)        |
//! | * 60 fps  )) ====  128 BPM  [][]|
//! | FAV [1][2][3][4][5][6][7][8]   |
//! | CONTROLS | PLAYLIST | PRESETS  |
//! | (the tab's body, scrolling)    |
//! +--------------------------------+
//! ```
//!
//! - The **preview** is the scene's own output texture, fitted to the
//!   panel's width ([`super::perform::preview_in`]), with the LIVE,
//!   BLACKOUT and output-hidden badges the large preview has.
//! - The **status strip** answers the stage questions in one glance: the
//!   frame rate, whether audio is arriving (a level meter, not a word),
//!   the tempo, and the output window's two buttons.
//! - The **fav slots** are the eight quick-launch slots (D176), always in
//!   reach rather than inside a tab.
//! - The **tabs**: *Controls* is the inspector (the scene's controls by
//!   group, then the output-wide meta group); *Playlist* is the loaded
//!   show's cue list with a jump on double-click, and a button to the LIST
//!   view for editing; *Presets* recalls, saves and renames the scene's
//!   presets.
//!
//! The panel is [`Pane::Inspector`](crate::workspace::Pane::Inspector) in
//! the pane model: open, it is docked or a drawer; closed, the shell draws a
//! handle strip that reopens it. The tab is the operator's and is saved.

use super::chrome::Readouts;
use super::icon as icons;
use super::model::{Action, Model};
use super::theme::{self, SPACE_SM, SPACE_XS};
use super::widgets;
use super::{inspector, perform, playlist, presets};
use crate::icon::Icon;
use crate::palette::Palette;
use crate::workspace::Pane;
use egui::{Align, Layout, Sense, Ui, UiBuilder, Vec2};

/// The side panel's tabs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SideTab {
    /// The playing scene's controls and the meta group.
    #[default]
    Controls,
    /// The loaded playlist's cues.
    Playlist,
    /// The scene's presets.
    Presets,
}

impl SideTab {
    /// Every tab, in the order the bar shows them.
    pub const ALL: [Self; 3] = [Self::Controls, Self::Playlist, Self::Presets];

    /// The name saved in the preferences file.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Controls => "controls",
            Self::Playlist => "playlist",
            Self::Presets => "presets",
        }
    }

    /// The tab saved as `key`; `None` for a name this build does not know.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tab| tab.key() == key)
    }

    /// The tab's label.
    pub const fn label(self) -> &'static str {
        match self {
            Self::Controls => "Controls",
            Self::Playlist => "Playlist",
            Self::Presets => "Presets",
        }
    }

    /// The tab's icon.
    pub const fn icon(self) -> Icon {
        match self {
            Self::Controls => Icon::Controls,
            Self::Playlist => Icon::Playlist,
            Self::Presets => Icon::Star,
        }
    }
}

/// The largest share of the panel's height the preview takes, so the tabs
/// keep room in a short window.
const PREVIEW_SHARE: f32 = 0.38;

/// The shortest the preview is drawn: enough to tell a scene from black.
const PREVIEW_MIN: f32 = 64.0;

/// The height of the status strip.
const STRIP_HEIGHT: f32 = 26.0;

/// Below this panel width the status strip drops the BPM and the tabs drop
/// their words.
const TIGHT: f32 = 300.0;

/// Draws the panel's contents.
pub fn show(
    ui: &mut Ui,
    model: &mut Model<'_>,
    readouts: &Readouts,
    theme: &Palette,
    actions: &mut Vec<Action>,
) {
    preview(ui, model, theme, actions);
    ui.add_space(SPACE_XS);
    status_strip(ui, model, readouts, theme, actions);
    ui.add_space(SPACE_XS);
    presets::fav_row(ui, model, theme, actions);
    ui.add_space(SPACE_XS);
    tab_bar(ui, model, theme);
    ui.add_space(SPACE_SM);
    match model.ui.side_tab {
        SideTab::Controls => inspector::show(ui, model, theme, actions),
        SideTab::Playlist => playlist_tab(ui, model, theme, actions),
        SideTab::Presets => presets_tab(ui, model, theme, actions),
    }
}

/// The preview, 16:9 at the panel's width, capped so the tabs keep room.
fn preview(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    let width = ui.available_width();
    let height = (width * 9.0 / 16.0)
        .min(ui.available_height() * PREVIEW_SHARE)
        .max(PREVIEW_MIN);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        perform::preview_in(ui, model, theme, actions, true);
    });
}

/// Frame rate, audio level, tempo, and the output buttons.
fn status_strip(
    ui: &mut Ui,
    model: &mut Model<'_>,
    readouts: &Readouts,
    theme: &Palette,
    actions: &mut Vec<Action>,
) {
    let tight = ui.available_width() < TIGHT;
    ui.horizontal(|ui| {
        ui.set_height(STRIP_HEIGHT);
        // The buttons first, against the right edge, so the readings take
        // what is left and give way before the buttons do.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            let visible = model.output_visible;
            if widgets::icon_button(
                ui,
                theme,
                Icon::Output,
                if visible {
                    "Hide the output window. The scene keeps rendering."
                } else {
                    "Show the output window"
                },
            )
            .clicked()
            {
                actions.push(Action::SetOutputVisible(!visible));
            }
            if widgets::icon_button(
                ui,
                theme,
                Icon::Fullscreen,
                "Fullscreen on the chosen monitor; shows the output if it is hidden (F11)",
            )
            .clicked()
            {
                actions.push(Action::ToggleFullscreen);
            }
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                fps_reading(ui, model, readouts, theme);
                audio_reading(ui, model, theme, tight);
            });
        });
    });
}

/// A dot and the frame rate: green while the output keeps up.
fn fps_reading(ui: &mut Ui, model: &Model<'_>, readouts: &Readouts, theme: &Palette) {
    let fps = model.stats.fps();
    let colour = if fps >= 50.0 {
        theme.success
    } else if fps >= 25.0 {
        theme.warning
    } else {
        theme.text_faint
    };
    let (dot, _) = ui.allocate_exact_size(Vec2::splat(10.0), Sense::hover());
    ui.painter()
        .circle_filled(dot.center(), 3.5, theme::rgb(colour));
    ui.label(
        egui::RichText::new(readouts.fps())
            .small()
            .color(theme::rgb(theme.text_muted)),
    )
    .on_hover_text("Output frame rate");
}

/// The audio icon, a level meter and the tempo.
fn audio_reading(ui: &mut Ui, model: &Model<'_>, theme: &Palette, tight: bool) {
    let audio = &model.audio;
    let colour = if audio.live {
        theme.success
    } else if audio.can_capture {
        theme.warning
    } else {
        theme.text_faint
    };
    ui.add_space(SPACE_XS);
    icons::show(ui, Icon::Audio, icons::DISCLOSURE + 2.0, theme::rgb(colour));
    let level = audio_level(audio.input_rms);
    let room = (ui.available_width() - if tight { 0.0 } else { 64.0 }).max(0.0);
    let meter_width = room.clamp(24.0, 56.0);
    let fill = if audio.live {
        theme.accent
    } else {
        theme.text_faint
    };
    widgets::meter(ui, theme, level, fill, meter_width)
        .on_hover_text(format!("{}. The bar is the input level.", audio.source));
    if !tight {
        let bpm = if audio.bpm > 0.0 {
            format!("{:.0} BPM", audio.bpm)
        } else {
            "-- BPM".to_string()
        };
        ui.label(
            egui::RichText::new(bpm)
                .small()
                .color(theme::rgb(theme.text_muted)),
        )
        .on_hover_text("Tempo from the audio input, or the last tap (T)");
    }
}

/// An input RMS as a meter fraction: logarithmic, so a quiet room and a
/// loud one both move the bar, and silence is empty.
pub fn audio_level(rms: f32) -> f32 {
    if !rms.is_finite() || rms <= 0.0 {
        return 0.0;
    }
    // -60 dB is empty, 0 dB is full.
    let db = 20.0 * rms.log10();
    ((db + 60.0) / 60.0).clamp(0.0, 1.0)
}

/// The tab bar: equal cells, the chosen one underlined in the accent.
fn tab_bar(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette) {
    let width = ui.available_width();
    let labelled = width >= TIGHT;
    let cell = width / SideTab::ALL.len() as f32;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 28.0), Sense::hover());
    let mut chosen = None;
    for (index, tab) in SideTab::ALL.into_iter().enumerate() {
        let cell_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left() + cell * index as f32, rect.top()),
            Vec2::new(cell, rect.height()),
        );
        let id = egui::Id::new(("side-tab", tab.key()));
        let response = ui
            .interact(cell_rect, id, Sense::click())
            .on_hover_text(format!("{} tab", tab.label()));
        let active = model.ui.side_tab == tab;
        let fill = widgets::hover_fill(
            ui,
            response.id,
            response.hovered(),
            theme.panel,
            theme.raised,
        );
        let painter = ui.painter();
        painter.rect_filled(cell_rect, egui::CornerRadius::same(theme::RADIUS), fill);
        let colour = if active { theme.accent } else { theme.text_muted };
        let glyph = icons::INLINE;
        let font = egui::FontId::new(10.5, egui::FontFamily::Proportional);
        let galley = painter.layout_no_wrap(
            if labelled {
                tab.label().to_uppercase()
            } else {
                String::new()
            },
            font,
            theme::rgb(colour),
        );
        let content = glyph + if labelled { SPACE_XS + galley.size().x } else { 0.0 };
        let x = cell_rect.center().x - content / 2.0;
        icons::paint(
            painter,
            egui::Rect::from_center_size(
                egui::pos2(x + glyph / 2.0, cell_rect.center().y),
                Vec2::splat(glyph),
            ),
            tab.icon(),
            theme::rgb(colour),
        );
        if labelled {
            painter.galley(
                egui::pos2(
                    x + glyph + SPACE_XS,
                    cell_rect.center().y - galley.size().y / 2.0,
                ),
                galley,
                theme::rgb(colour),
            );
        }
        // The underline grows out from the centre as the choice moves.
        let grown = super::motion::toggle(ui.ctx(), id.with("under"), active, super::motion::QUICK);
        if grown > 0.0 {
            let underline = egui::Rect::from_center_size(
                egui::pos2(cell_rect.center().x, cell_rect.bottom() - 1.0),
                Vec2::new((cell_rect.width() - SPACE_SM) * grown, 2.0),
            );
            painter.rect_filled(underline, egui::CornerRadius::same(1), theme::rgb(theme.accent));
        }
        if response.clicked() {
            chosen = Some(tab);
        }
    }
    ui.painter().hline(
        rect.x_range(),
        rect.bottom(),
        egui::Stroke::new(1.0, theme::rgb(theme.border)),
    );
    if let Some(tab) = chosen {
        model.ui.side_tab = tab;
    }
}

/// The Playlist tab: the loaded show's cues, and the way to the LIST view.
fn playlist_tab(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    let Some(show) = model.playlist.show else {
        widgets::empty_state(
            ui,
            theme,
            Icon::Playlist,
            "No playlist loaded",
            "Open LIST to load a file or generate a show. Without one, Next and Previous step through the library.",
        );
        if widgets::button(ui, theme, Some(Icon::Playlist), "Open LIST").clicked() {
            model.ui.panes.open(Pane::Playlist);
        }
        return;
    };
    let rows = show.rows().len();
    ui.horizontal(|ui| {
        let name = widgets::truncated(
            ui,
            show.name(),
            egui::TextStyle::Body.resolve(ui.style()),
            theme.text,
            (ui.available_width() - 120.0).max(60.0),
        );
        let (rect, response) = ui.allocate_exact_size(name.size(), Sense::hover());
        ui.painter().galley(rect.min, name, theme::rgb(theme.text));
        response.on_hover_text(format!("{rows} cues. Double-click a cue to jump to it."));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if widgets::button(ui, theme, Some(Icon::Playlist), "Edit")
                .on_hover_text("Open the LIST view: cue settings, Generate and Save")
                .clicked()
            {
                model.ui.panes.open(Pane::Playlist);
            }
        });
    });
    ui.add_space(SPACE_XS);
    // The LIST view may draw the same list in the same frame: its ids are
    // kept apart here.
    ui.push_id("side-playlist", |ui| {
        playlist::timeline(ui, model, show, theme, actions);
    });
}

/// The Presets tab: the preset row, then every preset as a row to recall.
fn presets_tab(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    if model.controls.is_none() {
        widgets::empty_state(
            ui,
            theme,
            Icon::Star,
            "No scene playing",
            "Presets belong to a scene. Load one from the library.",
        );
        return;
    }
    presets::preset_row(ui, model, theme, actions);
    let names = model.remote.presets;
    let active = model.remote.active_preset;
    if names.is_empty() {
        ui.label(
            egui::RichText::new("No presets saved for this scene. Set the controls and save them above.")
                .small()
                .color(theme::rgb(theme.text_muted)),
        );
        return;
    }
    egui::ScrollArea::vertical()
        .id_salt("side-presets")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for name in names {
                let is_active = active == Some(name.as_str());
                let width = ui.available_width();
                let (rect, response) = ui.allocate_exact_size(
                    Vec2::new(width, theme::row_height() * 0.75),
                    Sense::click(),
                );
                let id = egui::Id::new(("preset-row", name));
                let fill = widgets::selection_fill(
                    ui,
                    theme,
                    id,
                    is_active,
                    response.hovered(),
                    theme.panel,
                );
                ui.painter()
                    .rect_filled(rect, egui::CornerRadius::same(theme::RADIUS), fill);
                if is_active {
                    widgets::accent_bar(
                        ui,
                        ui.painter(),
                        id,
                        rect,
                        widgets::Bar::Inset,
                        theme.accent,
                    );
                }
                let colour = if is_active { theme.accent } else { theme.text };
                let galley = widgets::truncated(
                    ui,
                    name,
                    egui::TextStyle::Body.resolve(ui.style()),
                    colour,
                    width - SPACE_SM * 3.0,
                );
                ui.painter().galley(
                    egui::pos2(
                        rect.left() + SPACE_SM + SPACE_XS,
                        rect.center().y - galley.size().y / 2.0,
                    ),
                    galley,
                    theme::rgb(colour),
                );
                if response
                    .on_hover_text("Recall this preset: the controls ease to its values")
                    .clicked()
                    && !is_active
                {
                    actions.push(Action::RecallPreset(name.clone()));
                }
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_audio_level_is_logarithmic_and_bounded() {
        assert_eq!(audio_level(0.0), 0.0, "silence");
        assert_eq!(audio_level(f32::NAN), 0.0, "nonsense");
        assert_eq!(audio_level(2.0), 1.0, "clipped");
        let quiet = audio_level(0.01);
        let loud = audio_level(0.1);
        assert!(quiet > 0.0 && quiet < loud && loud < 1.0, "{quiet} {loud}");
    }

    #[test]
    fn side_tab_keys_round_trip() {
        for tab in SideTab::ALL {
            assert_eq!(SideTab::from_key(tab.key()), Some(tab), "{tab:?}");
        }
        assert_eq!(SideTab::from_key("nope"), None, "unknown key");
    }
}
