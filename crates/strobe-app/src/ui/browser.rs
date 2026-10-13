// ============================================================================
// Module:       strobe_app::ui::browser
// Description:  The scene browser pane: search, health filter, a virtualised
//               list, and Preview kept separate from Load.
//
// Dependencies: egui; super::{model, theme, widgets, state}, crate::library
// ============================================================================

//! The scene browser (UI_DESIGN section 12, UI_IMPLEMENTATION_PLAN
//! section 8).
//!
//! Opening the browser must not stop the show, and browsing must not change
//! it. Three verbs, kept visibly apart:
//!
//! - **Select** (a click, or Up/Down in the search field) only highlights.
//! - **Preview** prepares the selected scene in a second, small player and
//!   shows it in the card at the top of the pane. It never changes the live
//!   scene, the controls, the macros or the output.
//! - **Load** commits the scene (double-click, Enter in the search field, or
//!   the button). If the scene fails to prepare, the old scene stays live
//!   and the failure appears as a banner and a badge on the row.
//!
//! Each scene shows its health with an icon and a word, never colour alone:
//! `Ready` once it has loaded, `Failed` with the first line of why,
//! otherwise nothing is claimed. The list is virtualised
//! (`ScrollArea::show_rows`), so a library of thousands costs the same as a
//! library of ten, and the filtered indices are cached until the query,
//! the filter or the library changes.

use super::icon as icons;
use super::model::{Action, Model};
use super::theme::{self, ROW_HEIGHT, SPACE_SM, SPACE_XS};
use super::widgets;
use crate::icon::Icon;
use crate::library::{Health, HealthFilter};
use crate::palette::Palette;
use crate::workspace::Pane;
use egui::{Align, CornerRadius, Key, Layout, Rect, Sense, Ui, UiBuilder, Vec2};

/// The id of the search field, so a command can focus it.
pub const SEARCH_ID: &str = "browser-search";

/// Draws the pane's contents.
pub fn show(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    let panes = &mut model.ui.panes;
    let closed = widgets::pane_header(ui, theme, Icon::Library, Pane::Browser.title(), |ui| {
        if widgets::tool_button(
            ui,
            theme,
            Icon::Import,
            "Import",
            false,
            "Import a Shadertoy shader",
        )
        .clicked()
        {
            panes.open(Pane::Import);
        }
    });
    if closed {
        model.ui.panes.close(Pane::Browser);
    }
    search_row(ui, model, theme, actions);
    filter_row(ui, model, theme);
    ui.add_space(SPACE_XS);

    if let Some(error) = model.scan_error {
        widgets::banner(ui, theme, widgets::Tone::Error, error, |ui| {
            if widgets::button(ui, theme, Some(Icon::Refresh), "Rescan").clicked() {
                actions.push(Action::RescanLibrary);
            }
        });
        ui.add_space(SPACE_XS);
    }
    if let Some(preview) = model.scene_preview {
        preview_card(ui, theme, preview, model.playing, actions);
        ui.add_space(SPACE_SM);
    }

    if model.library.entries.is_empty() {
        empty_library(ui, model, theme, actions);
        return;
    }
    let visible = model
        .ui
        .browser
        .take_visible(model.library, model.health, model.library_rev);
    if visible.is_empty() {
        no_matches(ui, model, theme);
    } else {
        list(ui, model, theme, &visible, actions);
    }
    model.ui.browser.put_visible(visible);
}

fn search_row(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    let id = egui::Id::new(SEARCH_ID);
    // Whether the field had focus when the frame began: Esc is its own
    // business while it does (clear first, then leave).
    let had_focus = ui.ctx().memory(|m| m.has_focus(id));
    let escape = ui.input(|i| i.key_pressed(Key::Escape));
    if had_focus && escape && !model.ui.browser.query.is_empty() {
        model.ui.browser.query.clear();
        ui.ctx().memory_mut(|m| m.request_focus(id));
    }
    let response = egui::Frame::new()
        .fill(theme::rgb(theme.app))
        .stroke(egui::Stroke::new(
            1.0,
            theme::rgb(if had_focus {
                theme.accent
            } else {
                theme.border
            }),
        ))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(theme::margin_xy(SPACE_SM, SPACE_XS))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                icons::show(
                    ui,
                    Icon::Search,
                    icons::INLINE,
                    theme::rgb(theme.text_muted),
                );
                let clear_width = if model.ui.browser.query.is_empty() {
                    0.0
                } else {
                    22.0
                };
                let edit = ui.add(
                    egui::TextEdit::singleline(&mut model.ui.browser.query)
                        .id(id)
                        .frame(egui::Frame::NONE)
                        .hint_text("Search names and tags   Ctrl+F")
                        .desired_width(ui.available_width() - clear_width - SPACE_SM),
                );
                if !model.ui.browser.query.is_empty() {
                    let (rect, response) =
                        ui.allocate_exact_size(Vec2::splat(16.0), Sense::click());
                    icons::paint(
                        ui.painter(),
                        rect,
                        Icon::Close,
                        theme::rgb(if response.hovered() {
                            theme.text
                        } else {
                            theme.text_muted
                        }),
                    );
                    if response.on_hover_text("Clear the search").clicked() {
                        model.ui.browser.query.clear();
                    }
                }
                edit
            })
            .inner
        })
        .inner;

    if model.ui.browser.focus_search {
        model.ui.browser.focus_search = false;
        response.request_focus();
    }
    // Keyboard-first: the arrows move the selection, Enter loads it.
    if response.has_focus() || response.lost_focus() {
        let (down, up, enter) = ui.input(|i| {
            (
                i.key_pressed(Key::ArrowDown),
                i.key_pressed(Key::ArrowUp),
                i.key_pressed(Key::Enter),
            )
        });
        let visible = model
            .ui
            .browser
            .take_visible(model.library, model.health, model.library_rev);
        let count = visible.len();
        let position = model
            .ui
            .browser
            .selected
            .and_then(|s| visible.iter().position(|v| *v == s));
        let mut next = position;
        if down && count > 0 {
            next = Some(position.map_or(0, |p| (p + 1).min(count - 1)));
        }
        if up && count > 0 {
            next = Some(position.map_or(0, |p| p.saturating_sub(1)));
        }
        if next != position {
            model.ui.browser.selected = next.and_then(|p| visible.get(p).copied());
            model.ui.browser.scroll_to = next;
        }
        if enter && response.lost_focus() {
            let target = model
                .ui
                .browser
                .selected
                .filter(|s| visible.contains(s))
                .or_else(|| visible.first().copied());
            if let Some(index) = target {
                model.ui.browser.selected = Some(index);
                actions.push(Action::Load(index));
            }
            response.request_focus();
        }
        model.ui.browser.put_visible(visible);
    }
}

fn filter_row(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette) {
    ui.horizontal_wrapped(|ui| {
        for filter in HealthFilter::ALL {
            let active = model.ui.browser.filter == filter;
            let tip = match filter {
                HealthFilter::All => "Every scene",
                HealthFilter::Ready => "Scenes that have loaded successfully this run",
                HealthFilter::Failed => "Scenes that failed to load: they need work",
                HealthFilter::Favourites => "Scenes you starred",
            };
            if widgets::tool_button(ui, theme, filter_icon(filter), filter.label(), active, tip)
                .clicked()
            {
                model.ui.browser.filter = filter;
            }
        }
    });
    let shown = model
        .ui
        .browser
        .visible(model.library, model.health, model.library_rev)
        .len();
    ui.label(
        egui::RichText::new(format!("{shown} of {} scenes", model.library.entries.len()))
            .small()
            .color(theme::rgb(theme.text_muted)),
    );
}

fn filter_icon(filter: HealthFilter) -> Icon {
    match filter {
        HealthFilter::All => Icon::Library,
        HealthFilter::Ready => Icon::Check,
        HealthFilter::Failed => Icon::Warning,
        HealthFilter::Favourites => Icon::Star,
    }
}

fn empty_library(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    let dir = model.library.dir.display().to_string();
    ui.add_space(theme::SPACE_LG);
    ui.vertical_centered(|ui| {
        icons::show(ui, Icon::Folder, 28.0, theme::rgb(theme.text_faint));
        ui.add_space(SPACE_SM);
        ui.label(egui::RichText::new("No scenes found").size(15.0));
        ui.add_space(SPACE_XS);
        ui.label(
            egui::RichText::new(&dir)
                .small()
                .color(theme::rgb(theme.text_muted)),
        );
        ui.add_space(SPACE_SM);
        ui.label(
            egui::RichText::new(
                "Put .synScene folders in that folder, or import a Shadertoy shader.",
            )
            .small()
            .color(theme::rgb(theme.text_muted)),
        );
        ui.add_space(theme::SPACE_MD);
        ui.horizontal(|ui| {
            if widgets::button(ui, theme, Some(Icon::Import), "Import").clicked() {
                model.ui.panes.open(Pane::Import);
            }
            if widgets::button(ui, theme, Some(Icon::Refresh), "Rescan").clicked() {
                actions.push(Action::RescanLibrary);
            }
        });
    });
}

fn no_matches(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette) {
    ui.add_space(theme::SPACE_LG);
    ui.vertical_centered(|ui| {
        icons::show(ui, Icon::Search, 28.0, theme::rgb(theme.text_faint));
        ui.add_space(SPACE_SM);
        ui.label(egui::RichText::new("No scene matches").size(15.0));
        ui.add_space(SPACE_SM);
        if widgets::button(ui, theme, Some(Icon::Close), "Clear search and filter").clicked() {
            model.ui.browser.query.clear();
            model.ui.browser.filter = HealthFilter::All;
        }
    });
}

fn list(
    ui: &mut Ui,
    model: &mut Model<'_>,
    theme: &Palette,
    visible: &[usize],
    actions: &mut Vec<Action>,
) {
    ui.spacing_mut().item_spacing.y = 0.0;
    model.ui.thumbs.pump(ui.ctx());
    // Rows keep one layout: a picture slot for all of them when any scene
    // has a picture (D212), the colour swatch otherwise.
    let pictures = model.library.entries.iter().any(|e| e.thumbnail.is_some());
    let mut area = egui::ScrollArea::vertical().auto_shrink([false, false]);
    if let Some(row) = model.ui.browser.scroll_to.take() {
        let centre = (row as f32 * ROW_HEIGHT - ui.available_height() / 2.0).max(0.0);
        area = area.vertical_scroll_offset(centre);
    }
    area.show_rows(ui, ROW_HEIGHT, visible.len(), |ui, range| {
        for position in range {
            let Some(&index) = visible.get(position) else {
                continue;
            };
            row(ui, model, theme, index, pictures, actions);
        }
    });
}

/// One scene's row: swatch, name, health, and the verbs for a selected row.
fn row(
    ui: &mut Ui,
    model: &mut Model<'_>,
    theme: &Palette,
    index: usize,
    pictures: bool,
    actions: &mut Vec<Action>,
) {
    let Some(entry) = model.library.entries.get(index) else {
        return;
    };
    let unknown = Health::Unknown;
    let health = model.health.get(index).unwrap_or(&unknown);
    let playing = model.playing == Some(index);
    let selected = model.ui.browser.selected == Some(index);
    let previewed = model.scene_preview.is_some_and(|p| p.index == index);
    let loading = model.loading == Some(entry.name.as_str());

    let width = ui.available_width();
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, ROW_HEIGHT), Sense::click());
    let id = egui::Id::new(("scene-row", &entry.name));
    let fill = widgets::selection_fill(ui, theme, id, selected, response.hovered(), theme.panel);
    ui.painter().rect_filled(rect, CornerRadius::ZERO, fill);
    if playing {
        widgets::accent_bar(
            ui,
            ui.painter(),
            id,
            rect,
            widgets::Bar::Flush,
            theme.accent,
        );
    }

    // The picture or the identity swatch, then name and status line.
    let colour = theme::rgb(widgets::scene_colour(theme, &entry.name));
    let text_x = if pictures {
        const PICTURE: Vec2 = Vec2::new(56.0, 31.5);
        let slot = Rect::from_min_size(
            egui::pos2(
                rect.left() + theme::SPACE_SM,
                rect.center().y - PICTURE.y / 2.0,
            ),
            PICTURE,
        );
        let texture = entry
            .thumbnail
            .as_deref()
            .and_then(|path| model.ui.thumbs.get(path));
        match texture {
            Some((id, size)) => {
                // Fit inside the slot, centred, keeping the aspect.
                let scale = (PICTURE.x / size[0]).min(PICTURE.y / size[1]);
                let fitted = Rect::from_center_size(
                    slot.center(),
                    Vec2::new(size[0] * scale, size[1] * scale),
                );
                ui.painter()
                    .rect_filled(slot, CornerRadius::same(3), theme::rgb(theme.app));
                ui.painter().image(
                    id,
                    fitted,
                    Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                    egui::Color32::from_white_alpha(255),
                );
            }
            None => {
                ui.painter()
                    .rect_filled(slot, CornerRadius::same(3), theme::rgb(theme.raised));
                ui.painter().circle_filled(slot.center(), 5.0, colour);
            }
        }
        slot.right() + SPACE_SM
    } else {
        let swatch = egui::pos2(rect.left() + theme::SPACE_MD + 3.0, rect.center().y);
        ui.painter().circle_filled(swatch, 5.0, colour);
        rect.left() + theme::SPACE_MD + 18.0
    };
    let buttons = if selected { 142.0 } else { 0.0 };
    // The star sits just left of the buttons and always has its slot, so
    // the name does not reflow when a row is hovered (D211).
    const STAR: f32 = 20.0;
    let text_width = (rect.right() - text_x - SPACE_SM - buttons - STAR - SPACE_XS).max(40.0);
    let name_colour = if playing { theme.accent } else { theme.text };
    let font = egui::TextStyle::Body.resolve(ui.style());
    let name = widgets::truncated(ui, &entry.name, font, name_colour, text_width);
    ui.painter().galley(
        egui::pos2(text_x, rect.top() + 6.0),
        name,
        theme::rgb(name_colour),
    );
    status_line(
        ui, theme, health, playing, loading, previewed, text_x, rect, text_width,
    );

    if let Health::Failed(reason) = health {
        response.clone().on_hover_text(reason);
    } else if response.hovered() && !entry.tags.is_empty() {
        // Built only while hovered: a row never allocates per frame.
        response
            .clone()
            .on_hover_text(format!("Tags: {}", entry.tags.join(", ")));
    }
    if response.clicked() {
        model.ui.browser.selected = Some(index);
    }
    if response.double_clicked() && !playing {
        actions.push(Action::Load(index));
    }

    let favourite = model.ui.browser.is_favourite(&entry.name);
    let star = Rect::from_center_size(
        egui::pos2(
            rect.right() - buttons - SPACE_SM - STAR / 2.0,
            rect.center().y,
        ),
        Vec2::splat(STAR),
    );
    let star_response = ui.interact(star, id.with("star"), Sense::click());
    if favourite || response.hovered() || star_response.hovered() || selected {
        let colour = if favourite {
            theme.warning
        } else if star_response.hovered() {
            theme.text_muted
        } else {
            theme.text_faint
        };
        icons::paint(
            ui.painter(),
            Rect::from_center_size(star.center(), Vec2::splat(icons::DISCLOSURE + 2.0)),
            Icon::Star,
            theme::rgb(colour),
        );
    }
    let star_response = star_response.on_hover_text(if favourite {
        "Remove from favourites"
    } else {
        "Add to favourites"
    });
    if star_response.clicked() {
        let name = entry.name.clone();
        model.ui.browser.toggle_favourite(&name);
    }

    if selected {
        let area = Rect::from_min_max(
            egui::pos2(rect.right() - buttons - SPACE_XS, rect.top() + 9.0),
            egui::pos2(rect.right() - SPACE_SM, rect.bottom() - 9.0),
        );
        ui.scope_builder(
            UiBuilder::new()
                .max_rect(area)
                .layout(Layout::right_to_left(Align::Center)),
            |ui| {
                if playing {
                    if widgets::button(ui, theme, Some(Icon::Refresh), "Reload")
                        .on_hover_text("Reload this scene from disk")
                        .clicked()
                    {
                        actions.push(Action::Reload);
                    }
                } else {
                    if widgets::button(ui, theme, Some(Icon::Play), "Load")
                        .on_hover_text(
                            "Make this scene live. If it fails, the current scene stays.",
                        )
                        .clicked()
                    {
                        actions.push(Action::Load(index));
                    }
                    if widgets::button(ui, theme, Some(Icon::Output), "Preview")
                        .on_hover_text("Prepare this scene here without changing what is live")
                        .clicked()
                    {
                        actions.push(Action::Preview(index));
                    }
                }
            },
        );
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "paints into a row whose geometry the caller already has"
)]
fn status_line(
    ui: &mut Ui,
    theme: &Palette,
    health: &Health,
    playing: bool,
    loading: bool,
    previewed: bool,
    x: f32,
    rect: Rect,
    width: f32,
) {
    let (icon, text, colour) = if loading {
        (Icon::Refresh, "Preparing...".to_string(), theme.accent)
    } else if playing {
        (Icon::Play, "Playing".to_string(), theme.accent)
    } else if previewed {
        (Icon::Output, "Previewing".to_string(), theme.info)
    } else {
        match health {
            Health::Unknown => (Icon::Folder, health.label().to_string(), theme.text_faint),
            Health::Ready => (Icon::Check, health.label().to_string(), theme.success),
            Health::Failed(reason) => (
                Icon::Error,
                format!("Failed: {}", reason.lines().next().unwrap_or("")),
                theme.danger,
            ),
        }
    };
    let y = rect.bottom() - 11.0;
    icons::paint(
        ui.painter(),
        Rect::from_center_size(egui::pos2(x + 6.0, y), Vec2::splat(icons::DISCLOSURE)),
        icon,
        theme::rgb(colour),
    );
    let galley = widgets::truncated(
        ui,
        &text,
        egui::TextStyle::Small.resolve(ui.style()),
        colour,
        width - 16.0,
    );
    ui.painter().galley(
        egui::pos2(x + 16.0, y - galley.size().y / 2.0),
        galley,
        theme::rgb(colour),
    );
}

/// The card at the top of the pane that shows the previewed scene.
fn preview_card(
    ui: &mut Ui,
    theme: &Palette,
    preview: super::model::ScenePreview<'_>,
    playing: Option<usize>,
    actions: &mut Vec<Action>,
) {
    egui::Frame::new()
        .fill(theme::rgb(theme.raised))
        .stroke(egui::Stroke::new(1.0, theme::rgb(theme.info)))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(theme::margin(SPACE_SM))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                widgets::chip(ui, "PREVIEW", theme.info, theme.text_on_accent);
                ui.label(egui::RichText::new(preview.name).small());
            });
            ui.add_space(SPACE_XS);
            let width = ui.available_width();
            let height = width / preview.image.aspect.max(0.1);
            let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
            ui.painter()
                .rect_filled(rect, CornerRadius::same(2), theme::rgb(theme.app));
            let uv = Rect::from_min_max(egui::pos2(0.0, 1.0), egui::pos2(1.0, 0.0));
            ui.painter().image(
                preview.image.texture,
                rect,
                uv,
                egui::Color32::from_white_alpha(255),
            );
            ui.add_space(SPACE_XS);
            ui.label(
                egui::RichText::new("Not live. The output and the controls are unchanged.")
                    .small()
                    .color(theme::rgb(theme.text_muted)),
            );
            ui.horizontal(|ui| {
                if playing != Some(preview.index)
                    && widgets::primary_button(ui, theme, "Load this scene").clicked()
                {
                    actions.push(Action::Load(preview.index));
                }
                if widgets::button(ui, theme, Some(Icon::Close), "Close preview").clicked() {
                    actions.push(Action::ClosePreview);
                }
            });
        });
}
