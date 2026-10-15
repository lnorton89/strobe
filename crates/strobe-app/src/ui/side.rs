// ============================================================================
// Module:       strobe_app::ui::side
// Description:  The persistent side panel: the live preview, the status strip
//               under it, and the Playlist, Controls, Presets and Edit
//               icon tabs.
//
// Dependencies: egui; super::{chrome, inspector, model, perform, playlist,
//               presets, theme, widgets, icon}, crate::{icon, workspace}
// ============================================================================

//! The side panel (UI_DESIGN section 7, D283).
//!
//! Synesthesia keeps one column on the right of every view: one preview, a
//! status strip, and icon tabs for the playlist, the controls and the
//! edit panel. Strobe's side panel is that column, drawn from the same
//! model every other pane reads:
//!
//! ```text
//! +--------------------------------+
//! |          PREVIEW (16:9)        |
//! | [out][full] * 60 fps ===== [cam]|
//! | FAV [1][2][3][4][5][6][7][8]   |
//! |  LIST  |  CTRL  |  PRESET | EDIT|
//! | (the tab's body, scrolling)    |
//! +--------------------------------+
//! ```
//!
//! - The **preview** is the scene's own output texture, fitted to the
//!   panel's width ([`super::perform::preview_in`]), with the LIVE,
//!   BLACKOUT and output-hidden badges the large preview has. A panel too
//!   short to give it [`PREVIEW_MIN`] leaves it out, so a drawer in a
//!   small window still has room for the tab's body.
//! - The **status strip** answers the stage questions in one glance, in
//!   Synesthesia's order: the output window's two buttons, the frame rate
//!   with a dot (green while the output keeps up), a horizontal audio level
//!   bar (a meter, not a word) with the tempo, and the snapshot button,
//!   which keeps the live frame as the scene's picture.
//! - The **fav slots** (D176) are under the strip, in reach from every
//!   tab, as Synesthesia keeps them; a panel shorter than [`FAVS_ABOVE`]
//!   moves them to the top of the Presets tab.
//! - The **tabs** are icons, with their names in tooltips and, when the
//!   panel is wide, beside them. *Playlist* is the loaded show's cue list
//!   with a jump on double-click, and a button to the LIST view for editing.
//!   *Controls* is the scene's controls by group, then the output-wide meta
//!   group. *Presets* is the preset row and the preset list. *Edit* is the scene's tags, picture, media and Generate
//!   controls, and the way to the Editor.
//!
//! The panel is [`Pane::Inspector`](crate::workspace::Pane::Inspector) in
//! the pane model: open, it is docked or a drawer; closed, the shell draws a
//! handle strip that reopens it. The tab is the operator's and is saved.
//!
//! While the main view is the library on its Controls page (D291) and the
//! panel is docked beside it, the controls are there, large, and the panel
//! does not draw them twice: its Controls tab is dimmed and, if it was the
//! chosen one, the panel shows Presets until the main view goes back to
//! Scenes. The saved choice is not changed. A drawer covers the main view,
//! so it keeps its Controls tab.

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
    /// The loaded playlist's cues.
    Playlist,
    /// The playing scene's controls and the meta group.
    #[default]
    Controls,
    /// The fav slots and the scene's presets.
    Presets,
    /// The scene's tags, picture, media, Generate and the Editor.
    Edit,
}

impl SideTab {
    /// Every tab, in the order the bar shows them.
    pub const ALL: [Self; 4] = [Self::Playlist, Self::Controls, Self::Presets, Self::Edit];

    /// The name saved in the preferences file.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Playlist => "playlist",
            Self::Controls => "controls",
            Self::Presets => "presets",
            Self::Edit => "edit",
        }
    }

    /// The tab saved as `key`; `None` for a name this build does not know.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tab| tab.key() == key)
    }

    /// The tab's caption beside its icon in a wide panel.
    pub const fn caption(self) -> &'static str {
        match self {
            Self::Playlist => "PLAYLIST",
            Self::Controls => "CONTROLS",
            Self::Presets => "PRESETS",
            Self::Edit => "EDIT",
        }
    }

    /// What the tab is for, as its tooltip says it.
    pub const fn tooltip(self) -> &'static str {
        match self {
            Self::Playlist => "Playlist: the loaded show's cues",
            Self::Controls => "Controls: the playing scene's controls",
            Self::Presets => "Presets: fav slots and the scene's saved looks",
            Self::Edit => "Edit: tags, picture, media, Generate controls, the Editor",
        }
    }

    /// The tab's icon.
    pub const fn icon(self) -> Icon {
        match self {
            Self::Playlist => Icon::Playlist,
            Self::Controls => Icon::Controls,
            Self::Presets => Icon::Star,
            Self::Edit => Icon::Editor,
        }
    }
}

/// The largest share of the panel's height the preview takes, so the tabs
/// keep room in a short window.
const PREVIEW_SHARE: f32 = 0.38;

/// The shortest the preview is drawn: enough to tell a scene from black.
/// A panel too short to give it this much at [`PREVIEW_SHARE`] leaves it
/// out.
const PREVIEW_MIN: f32 = 64.0;

/// The least height the panel needs to keep the fav slots above the tabs.
/// Below it they are the top of the Presets tab (D283).
const FAVS_ABOVE: f32 = 360.0;

/// The height of the status strip.
const STRIP_HEIGHT: f32 = 26.0;

/// Below this panel width the status strip drops the BPM.
const TIGHT: f32 = 300.0;

/// Below this panel width the tabs are icons alone.
const LABELLED: f32 = 400.0;

/// Draws the panel's contents. `docked` is whether the panel sits beside
/// the main view rather than over it as a drawer.
pub fn show(
    ui: &mut Ui,
    model: &mut Model<'_>,
    readouts: &Readouts,
    theme: &Palette,
    actions: &mut Vec<Action>,
    docked: bool,
) {
    // The fav slots sit above the tabs, in reach from every tab, while the
    // panel is tall enough to keep room for the tab's body; in a shorter
    // one they are the top of the Presets tab.
    let favs_above = ui.available_height() >= FAVS_ABOVE;
    // A drawer in a short window has no room for both the picture and the
    // tab's body: the controls win, and PLAY still has the picture.
    let height = ui.available_height() * PREVIEW_SHARE;
    if height >= PREVIEW_MIN {
        preview(ui, model, theme, actions, height);
        ui.add_space(SPACE_XS);
    }
    status_strip(ui, model, readouts, theme, actions);
    ui.add_space(SPACE_XS);
    if favs_above {
        presets::fav_row(ui, model, theme, actions);
        ui.add_space(SPACE_XS);
    }
    // The controls are in the main view beside the panel: it shows the
    // presets rather than the same controls again (D291). A drawer covers
    // the main view, so it keeps its own Controls tab.
    let controls_in_main = docked && model.ui.controls_in_main();
    let shown = if controls_in_main && model.ui.side_tab == SideTab::Controls {
        SideTab::Presets
    } else {
        model.ui.side_tab
    };
    tab_bar(ui, model, theme, shown, controls_in_main);
    ui.add_space(SPACE_SM);
    match shown {
        SideTab::Playlist => playlist_tab(ui, model, theme, actions),
        SideTab::Controls => inspector::show(ui, model, theme, actions, inspector::Part::Controls),
        SideTab::Presets => presets_tab(ui, model, theme, actions, !favs_above),
        SideTab::Edit => inspector::show(ui, model, theme, actions, inspector::Part::Edit),
    }
}

/// The preview, 16:9 at the panel's width, at most `cap` high so the tabs
/// keep room.
fn preview(
    ui: &mut Ui,
    model: &mut Model<'_>,
    theme: &Palette,
    actions: &mut Vec<Action>,
    cap: f32,
) {
    let width = ui.available_width();
    let height = (width * 9.0 / 16.0).min(cap);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
        ui.set_clip_rect(rect.intersect(ui.clip_rect()));
        perform::preview_in(ui, model, theme, actions, true);
    });
}

/// The status strip, in Synesthesia's order: the output and fullscreen
/// buttons, the frame rate with its dot, the audio level bar and the tempo,
/// and the snapshot button.
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
        fps_reading(ui, model, readouts, theme);
        // The snapshot keeps its place at the right edge; the level bar
        // takes what is left between.
        let reserve = SNAPSHOT + ui.spacing().item_spacing.x + if tight { 0.0 } else { BPM_ROOM };
        audio_reading(ui, model, readouts, theme, tight, reserve);
        // Not a scene of Synesthesia's: its folder is not written.
        let playing = model
            .playing
            .and_then(|index| model.library.entries.get(index));
        let can = playing.is_some_and(|entry| !entry.read_only());
        let tip = match playing {
            None => "Snapshot: load a scene first",
            Some(entry) if entry.read_only() => {
                "Snapshot: this scene is Synesthesia's own and read-only. Copy it to your library first."
            }
            Some(_) => {
                "Snapshot: keep what is on screen now as this scene's picture in the library"
            }
        };
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.add_enabled_ui(can, |ui| {
                if widgets::icon_button(ui, theme, Icon::Camera, tip).clicked() {
                    actions.push(Action::CaptureThumbnail);
                }
            })
            .response
            .on_disabled_hover_text(tip);
        });
    });
}

/// The width the snapshot button takes, and the room kept for the tempo.
const SNAPSHOT: f32 = 28.0;
const BPM_ROOM: f32 = 60.0;

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

/// The audio icon, a level bar that fills the room left after `reserve`,
/// and the tempo.
fn audio_reading(
    ui: &mut Ui,
    model: &Model<'_>,
    readouts: &Readouts,
    theme: &Palette,
    tight: bool,
    reserve: f32,
) {
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
    let room = (ui.available_width() - reserve - icons::DISCLOSURE).max(0.0);
    let meter_width = room.clamp(24.0, 220.0);
    let fill = if audio.live {
        theme.accent
    } else {
        theme.text_faint
    };
    // The tooltip's words are built only while it shows.
    widgets::meter(ui, theme, level, fill, meter_width).on_hover_ui(|ui| {
        ui.label(format!("{}. The bar is the input level.", audio.source));
    });
    if !tight {
        ui.label(
            egui::RichText::new(readouts.bpm())
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

/// The tab bar: icon tabs in equal cells, the `shown` one a filled tile
/// with an accent underline. The names are in the tooltips, and beside the
/// icons when the panel is wide enough for them. With `controls_in_main`
/// the Controls tab is dimmed: the main view has them (D291).
fn tab_bar(
    ui: &mut Ui,
    model: &mut Model<'_>,
    theme: &Palette,
    shown: SideTab,
    controls_in_main: bool,
) {
    let width = ui.available_width();
    let labelled = width >= LABELLED;
    let cell = width / SideTab::ALL.len() as f32;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 28.0), Sense::hover());
    let mut chosen = None;
    for (index, tab) in SideTab::ALL.into_iter().enumerate() {
        let cell_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left() + cell * index as f32, rect.top()),
            Vec2::new(cell, rect.height()),
        );
        let id = egui::Id::new(("side-tab", tab.key()));
        let elsewhere = controls_in_main && tab == SideTab::Controls;
        let tooltip = if elsewhere {
            "Controls: shown large in the main view now. Switch it to Scenes to have them here."
        } else {
            tab.tooltip()
        };
        let response = super::help::hovered(ui.interact(cell_rect, id, Sense::click()), tooltip);
        let active = shown == tab;
        let fill = widgets::hover_fill(
            ui,
            response.id,
            response.hovered(),
            if active { theme.raised } else { theme.panel },
            theme.raised,
        );
        let painter = ui.painter();
        painter.rect_filled(cell_rect, egui::CornerRadius::same(theme::RADIUS), fill);
        let colour = if active {
            theme.accent
        } else if elsewhere {
            theme.text_faint
        } else {
            theme.text_muted
        };
        let glyph = icons::INLINE;
        let font = egui::FontId::new(10.5, egui::FontFamily::Proportional);
        // Laid out only when it is drawn; egui caches the galley.
        let galley = labelled
            .then(|| painter.layout_no_wrap(tab.caption().to_owned(), font, theme::rgb(colour)));
        let content = glyph + galley.as_ref().map_or(0.0, |g| SPACE_XS + g.size().x);
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
        if let Some(galley) = galley {
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
            painter.rect_filled(
                underline,
                egui::CornerRadius::same(1),
                theme::rgb(theme.accent),
            );
        }
        if response.clicked() && !elsewhere {
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
        response.on_hover_ui(|ui| {
            ui.label(format!("{rows} cues. Double-click a cue to jump to it."));
        });
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

/// The Presets tab: the fav slots when the panel is too short to keep them
/// above the tabs (`favs`), the preset row, then every preset as a row to
/// recall.
fn presets_tab(
    ui: &mut Ui,
    model: &mut Model<'_>,
    theme: &Palette,
    actions: &mut Vec<Action>,
    favs: bool,
) {
    // The fav slots first: they hold scenes, so they are there with or
    // without a playing one.
    if favs {
        presets::fav_row(ui, model, theme, actions);
        ui.add_space(SPACE_SM);
    }
    if model.controls.is_none() {
        widgets::empty_state(
            ui,
            theme,
            Icon::Star,
            "No scene playing",
            "Presets belong to a scene. Click one in the library to make it live.",
        );
        return;
    }
    presets::preset_row(ui, model, theme, actions);
    let names = model.remote.presets;
    let active = model.remote.active_preset;
    if names.is_empty() {
        ui.label(
            egui::RichText::new(
                "No presets saved for this scene. Set the controls and save them above.",
            )
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
