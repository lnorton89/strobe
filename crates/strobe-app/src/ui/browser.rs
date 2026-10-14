// ============================================================================
// Module:       strobe_app::ui::browser
// Description:  The scene library, the default main view: search, health
//               filter, a card size, a virtualised grid of thumbnail cards,
//               and Preview kept separate from Load.
//
// Dependencies: egui; super::{model, theme, widgets, state, thumbs, tags},
//               crate::library
// ============================================================================

//! The scene library (UI_DESIGN section 12, D273).
//!
//! Synesthesia's library is a grid of pictures with a search over it, and
//! so is this: the main view the control window opens on. Opening the
//! library must not stop the show, and browsing must not change it. Three
//! verbs, kept visibly apart:
//!
//! - **Select** (a click, or Up/Down in the search field) only highlights.
//! - **Preview** prepares the selected scene in a second, small player and
//!   shows it in the card above the grid. It never changes the live scene,
//!   the controls, the macros or the output.
//! - **Load** commits the scene (double-click, Enter in the search field, or
//!   the button on the selected card). If the scene fails to prepare, the
//!   old scene stays live and the failure appears as a banner and on the
//!   card.
//!
//! ```text
//! [search names and tags            x]   12 of 12   [All][Ready][Failed][Fav]  [S][M][L]
//! +----------+ +----------+ +----------+ +----------+
//! | picture  | | picture  | | picture  | | picture  |
//! | Name   * | | Name   * | | Name   * | | Name   * |
//! | Playing  | | Ready    | | tags...  | | Failed:..|
//! +----------+ +----------+ +----------+ +----------+
//! ```
//!
//! Each card shows the scene's picture (`IMAGE_PATH`, decoded off the UI
//! thread, D212; or the frame the operator kept, D222), its name, its star,
//! and its health as an icon and a word, never colour alone: `Playing`,
//! `Ready` once it has loaded, `Failed` with the first line of why, and
//! otherwise its tags. A scene with no picture shows its identity colour
//! and initial, so the grid stays a grid. The grid is virtualised by row
//! (`ScrollArea::show_rows`), so a library of thousands costs the same as a
//! library of ten, and the filtered indices are cached until the query, the
//! filter or the library changes.
//!
//! The selected scene's tags sit above the grid, with the Inspector's tag
//! row, so a scene can be tagged without playing it (D264).

use super::icon as icons;
use super::model::{Action, Model};
use super::tags;
use super::theme::{self, SPACE_MD, SPACE_SM, SPACE_XS};
use super::widgets;
use crate::icon::Icon;
use crate::library::{Health, HealthFilter};
use crate::palette::Palette;
use crate::workspace::Pane;
use egui::{Align, CornerRadius, Key, Layout, Rect, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2};

/// The id of the search field, so a command can focus it.
pub const SEARCH_ID: &str = "browser-search";

/// How large the grid's cards are (D273). One setting for the grid, saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum CardSize {
    /// The most scenes on screen.
    Small,
    /// The default.
    #[default]
    Medium,
    /// Pictures a person can judge from across a desk.
    Large,
}

impl CardSize {
    /// Every size, in the order the switch shows them.
    pub const ALL: [Self; 3] = [Self::Small, Self::Medium, Self::Large];

    /// The name saved in the preferences file.
    pub const fn key(self) -> &'static str {
        match self {
            Self::Small => "small",
            Self::Medium => "medium",
            Self::Large => "large",
        }
    }

    /// The size saved as `key`; `None` for a name this build does not know.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|size| size.key() == key)
    }

    /// The switch's one-letter label.
    pub const fn letter(self) -> &'static str {
        match self {
            Self::Small => "S",
            Self::Medium => "M",
            Self::Large => "L",
        }
    }

    /// The card width the grid aims for; columns are as many as fit, and
    /// the cards stretch to share the row.
    pub const fn width(self) -> f32 {
        match self {
            Self::Small => 150.0,
            Self::Medium => 190.0,
            Self::Large => 250.0,
        }
    }
}

/// The caption band's height under a card's picture: two lines of text and
/// their insets, at the density in force.
fn caption_height() -> f32 {
    theme::row_height() - 2.0
}

/// The gap between cards.
const GAP: f32 = SPACE_SM;

/// How many columns of `card` fit in `width`, at least one.
pub fn columns(width: f32, card: CardSize) -> usize {
    if !width.is_finite() || width <= 0.0 {
        return 1;
    }
    (((width + GAP) / (card.width() + GAP)).floor() as usize).max(1)
}

/// Draws the library.
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
        if widgets::icon_button(ui, theme, Icon::Refresh, "List the scene folder again").clicked() {
            actions.push(Action::RescanLibrary);
        }
    });
    if closed {
        model.ui.panes.close(Pane::Browser);
    }
    search_row(ui, model, theme, actions);
    filter_row(ui, model, theme);
    selected_tags(ui, model, theme, actions);
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
        grid(ui, model, theme, &visible, actions);
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
        .stroke(Stroke::new(
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

/// The count, the health filters and the card-size switch.
fn filter_row(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette) {
    let shown = model
        .ui
        .browser
        .visible(model.library, model.health, model.library_rev)
        .len();
    let total = model.library.entries.len();
    // Below this width the filters are icons alone, with their words in
    // their tooltips.
    const LABELLED: f32 = 520.0;
    let labelled = ui.available_width() >= LABELLED;
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(format!("{shown} of {total}"))
                .small()
                .color(theme::rgb(theme.text_muted)),
        )
        .on_hover_text("Scenes shown of scenes in the library");
        for filter in HealthFilter::ALL {
            let active = model.ui.browser.filter == filter;
            let tip = match filter {
                HealthFilter::All => "Every scene",
                HealthFilter::Ready => "Scenes that have loaded successfully this run",
                HealthFilter::Failed => "Scenes that failed to load: they need work",
                HealthFilter::Favourites => "Scenes you starred",
            };
            if widgets::tool_button_fitted(
                ui,
                theme,
                filter_icon(filter),
                filter.label(),
                active,
                tip,
                labelled,
            )
            .clicked()
            {
                model.ui.browser.filter = filter;
            }
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            // Largest first, so they read S M L left to right.
            for size in CardSize::ALL.iter().rev() {
                let active = model.ui.browser.cards == *size;
                let (rect, response) =
                    ui.allocate_exact_size(Vec2::new(24.0, 26.0), Sense::click());
                let fill = if active {
                    theme::rgb(theme.accent_soft)
                } else {
                    widgets::hover_fill(ui, response.id, response.hovered(), theme.raised, theme.hover)
                };
                ui.painter()
                    .rect_filled(rect, CornerRadius::same(theme::RADIUS), fill);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    size.letter(),
                    egui::TextStyle::Small.resolve(ui.style()),
                    theme::rgb(if active { theme.accent } else { theme.text }),
                );
                if response
                    .on_hover_text(format!("{} cards", size.key()))
                    .clicked()
                {
                    model.ui.browser.cards = *size;
                }
            }
        });
    });
}

/// The selected scene's tags, and the field that edits them (D264).
fn selected_tags(ui: &mut Ui, model: &mut Model<'_>, theme: &Palette, actions: &mut Vec<Action>) {
    let library = model.library;
    let Some(entry) = model
        .ui
        .browser
        .selected
        .and_then(|index| library.entries.get(index))
    else {
        return;
    };
    ui.add_space(SPACE_XS);
    tags::row(
        ui,
        theme,
        tags::Place {
            scene: &entry.name,
            named: true,
        },
        &entry.tags,
        &mut model.ui.browser_tag_draft,
        |_| {},
        actions,
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

/// The grid: as many columns as fit, the cards stretched to share the row,
/// virtualised by row.
fn grid(
    ui: &mut Ui,
    model: &mut Model<'_>,
    theme: &Palette,
    visible: &[usize],
    actions: &mut Vec<Action>,
) {
    ui.spacing_mut().item_spacing.y = 0.0;
    model.ui.thumbs.pump(ui.ctx());
    let size = model.ui.browser.cards;
    // The scroll bar's room is kept out of the width, so a row never
    // outgrows the pane when the bar appears.
    let width = ui.available_width() - theme::SPACE_SM * 2.0;
    let columns = columns(width, size);
    let card_width = (width - GAP * (columns as f32 - 1.0)) / columns as f32;
    let card_height = card_width * 9.0 / 16.0 + caption_height();
    let row_height = card_height + GAP;
    let rows = visible.len().div_ceil(columns);
    let mut area = egui::ScrollArea::vertical()
        .id_salt("library-grid")
        .auto_shrink([false, false]);
    if let Some(position) = model.ui.browser.scroll_to.take() {
        let row = position / columns;
        let centre = (row as f32 * row_height - ui.available_height() / 2.0).max(0.0);
        area = area.vertical_scroll_offset(centre);
    }
    area.show_rows(ui, row_height, rows, |ui, range| {
        for row in range {
            // The row's place is taken first, so nothing a card draws in a
            // child can move the cursor; the cards are then painted into
            // it.
            let (row_rect, _) =
                ui.allocate_exact_size(Vec2::new(width, row_height), Sense::hover());
            for column in 0..columns {
                let Some(&index) = visible.get(row * columns + column) else {
                    break;
                };
                let rect = Rect::from_min_size(
                    egui::pos2(
                        row_rect.left() + column as f32 * (card_width + GAP),
                        row_rect.top(),
                    ),
                    Vec2::new(card_width, card_height),
                );
                card(ui, model, theme, index, rect, actions);
            }
        }
    });
}

/// A library scene being dragged out of the browser (D224): its folder
/// name, as a playlist names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LibraryDrag(pub String);

/// While a scene is dragged out of the browser, its name follows the
/// pointer, so it is clear what will land.
pub fn drag_label(ctx: &egui::Context, theme: &Palette) {
    let Some(payload) = egui::DragAndDrop::payload::<LibraryDrag>(ctx) else {
        return;
    };
    let Some(pointer) = ctx.pointer_latest_pos() else {
        return;
    };
    egui::Area::new(egui::Id::new("library-drag"))
        .order(egui::Order::Tooltip)
        .fixed_pos(pointer + Vec2::new(12.0, 8.0))
        .interactable(false)
        .show(ctx, |ui| {
            egui::Frame::new()
                .fill(theme::rgb(theme.raised))
                .stroke(Stroke::new(1.0, theme::rgb(theme.accent)))
                .corner_radius(CornerRadius::same(theme::RADIUS))
                .inner_margin(theme::margin_xy(SPACE_SM, SPACE_XS))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(format!("+ {}", payload.0))
                            .color(theme::rgb(theme.text)),
                    );
                });
        });
}

/// One scene's card: picture, name, star, status, and the verbs when it is
/// selected.
fn card(
    ui: &mut Ui,
    model: &mut Model<'_>,
    theme: &Palette,
    index: usize,
    rect: Rect,
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
    let id = egui::Id::new(("scene-card", &entry.name));

    let response = ui.interact(rect, id, Sense::click_and_drag());
    // A card dragged to the LIST view becomes a cue there (D224).
    if response.drag_started() {
        response.dnd_set_drag_payload(LibraryDrag(entry.name.clone()));
    }
    let painter = ui.painter();
    let fill = widgets::selection_fill(ui, theme, id, selected, response.hovered(), theme.raised);
    painter.rect_filled(rect, CornerRadius::same(theme::RADIUS), fill);

    // The picture, or the identity colour and the initial.
    let picture = Rect::from_min_size(
        rect.min,
        Vec2::new(rect.width(), rect.height() - caption_height()),
    );
    let colour = widgets::scene_colour(theme, &entry.name);
    let radius = CornerRadius {
        nw: theme::RADIUS,
        ne: theme::RADIUS,
        sw: 0,
        se: 0,
    };
    let texture = entry
        .thumbnail
        .as_deref()
        .and_then(|path| model.ui.thumbs.get(path));
    match texture {
        Some((texture, size)) => {
            painter.rect_filled(picture, radius, theme::rgb(theme.app));
            // Fit inside, centred, keeping the aspect.
            let scale = (picture.width() / size[0]).min(picture.height() / size[1]);
            let fitted = Rect::from_center_size(
                picture.center(),
                Vec2::new(size[0] * scale, size[1] * scale),
            );
            ui.painter_at(picture).image(
                texture,
                fitted,
                Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::from_white_alpha(255),
            );
        }
        None => {
            painter.rect_filled(picture, radius, theme::translucent(colour, 48));
            // The first letter, or failing that the first digit: a
            // library numbered `01-aurora` reads A, not 0.
            let initial: String = entry
                .name
                .chars()
                .find(char::is_ascii_alphabetic)
                .or_else(|| entry.name.chars().find(|c| c.is_alphanumeric()))
                .map_or_else(String::new, |c| c.to_uppercase().collect());
            painter.text(
                picture.center(),
                egui::Align2::CENTER_CENTER,
                initial,
                egui::FontId::new(
                    (picture.height() * 0.4).clamp(14.0, 44.0),
                    egui::FontFamily::Proportional,
                ),
                theme::rgb(colour),
            );
        }
    }
    // The state over the picture's top-left corner, as the preview's badge.
    let badge_area = Rect::from_min_size(
        picture.left_top() + Vec2::splat(SPACE_XS),
        Vec2::new(picture.width() - SPACE_XS * 2.0, 22.0),
    );
    if playing || loading || previewed {
        // A child that is not merged back: a badge must not move the grid's
        // cursor.
        let mut badges = ui.new_child(UiBuilder::new().max_rect(badge_area));
        badges.scope(|ui| {
            ui.horizontal(|ui| {
                if loading {
                    widgets::status_chip_over(ui, Icon::Refresh, "PREPARING", theme.accent, Some(theme.app));
                } else if playing {
                    widgets::status_chip_over(ui, Icon::Play, "PLAYING", theme.accent, Some(theme.app));
                } else {
                    widgets::status_chip_over(ui, Icon::Output, "PREVIEW", theme.info, Some(theme.app));
                }
            });
        });
    }

    // The caption band: name and star on the first line, status on the
    // second.
    let band = Rect::from_min_max(picture.left_bottom(), rect.max);
    const STAR: f32 = 20.0;
    let inset = theme::density().row_inset() - 2.0;
    let text_x = band.left() + SPACE_SM;
    let text_width = (band.width() - SPACE_SM * 2.0 - STAR - SPACE_XS).max(30.0);
    let name_colour = if playing { theme.accent } else { theme.text };
    let name = widgets::truncated(
        ui,
        &entry.name,
        egui::TextStyle::Body.resolve(ui.style()),
        name_colour,
        text_width,
    );
    ui.painter()
        .galley(egui::pos2(text_x, band.top() + inset), name, theme::rgb(name_colour));
    status_line(
        ui,
        theme,
        health,
        entry.tags.as_slice(),
        (playing, loading, previewed),
        text_x,
        band,
        band.width() - SPACE_SM * 2.0,
    );

    // A selected card is outlined in the accent; a playing one in the
    // accent too, thinner, so it is found in the grid at a glance.
    if selected || playing {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(theme::RADIUS),
            Stroke::new(if selected { 2.0 } else { 1.0 }, theme::rgb(theme.accent)),
            StrokeKind::Inside,
        );
    } else {
        ui.painter().rect_stroke(
            rect,
            CornerRadius::same(theme::RADIUS),
            Stroke::new(1.0, theme::rgb(theme.border)),
            StrokeKind::Inside,
        );
    }

    if let Health::Failed(reason) = health {
        response.clone().on_hover_text(reason);
    } else if response.hovered() && !entry.tags.is_empty() {
        // Built only while hovered: a card never allocates per frame.
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
    let name_for_menu = entry.name.clone();
    let favourite = model.ui.browser.is_favourite(&entry.name);
    let mut menu_choice = None;
    response.context_menu(|ui| {
        if !playing && ui.button("Load").clicked() {
            menu_choice = Some(Menu::Load);
            ui.close();
        }
        if !playing && ui.button("Preview without loading").clicked() {
            menu_choice = Some(Menu::Preview);
            ui.close();
        }
        if ui
            .button(if favourite {
                "Remove from favourites"
            } else {
                "Add to favourites"
            })
            .clicked()
        {
            menu_choice = Some(Menu::Star);
            ui.close();
        }
        if ui.button("Open in Editor").clicked() {
            menu_choice = Some(Menu::Edit);
            ui.close();
        }
    });
    match menu_choice {
        Some(Menu::Load) => actions.push(Action::Load(index)),
        Some(Menu::Preview) => actions.push(Action::Preview(index)),
        Some(Menu::Star) => model.ui.browser.toggle_favourite(&name_for_menu),
        Some(Menu::Edit) => actions.push(Action::OpenEditor(Some(name_for_menu.clone()))),
        None => {}
    }

    // The star, at the right of the name, always in its slot (D211).
    let star = Rect::from_center_size(
        egui::pos2(band.right() - SPACE_SM - STAR / 2.0, band.top() + inset + 8.0),
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
        model.ui.browser.toggle_favourite(&name_for_menu);
    }

    // The verbs, over the picture's bottom edge, on the selected card.
    if selected {
        let verbs = Rect::from_min_max(
            egui::pos2(picture.left() + SPACE_XS, picture.bottom() - 28.0 - SPACE_XS),
            egui::pos2(picture.right() - SPACE_XS, picture.bottom() - SPACE_XS),
        );
        let mut verb_ui = ui.new_child(
            UiBuilder::new()
                .max_rect(verbs)
                .layout(Layout::right_to_left(Align::Center)),
        );
        verb_ui.scope(
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
                    // A narrow card has room for one verb; Preview stays in
                    // the right-click menu.
                    if verbs.width() >= 150.0
                        && widgets::button(ui, theme, Some(Icon::Output), "Preview")
                            .on_hover_text(
                                "Prepare this scene here without changing what is live",
                            )
                            .clicked()
                    {
                        actions.push(Action::Preview(index));
                    }
                }
            },
        );
    }
}

/// What a card's context menu asked for.
#[derive(Clone, Copy)]
enum Menu {
    Load,
    Preview,
    Star,
    Edit,
}

/// The caption's second line: the scene's state, or its tags.
#[expect(
    clippy::too_many_arguments,
    reason = "paints into a card whose geometry the caller already has"
)]
fn status_line(
    ui: &mut Ui,
    theme: &Palette,
    health: &Health,
    tags: &[String],
    (playing, loading, previewed): (bool, bool, bool),
    x: f32,
    band: Rect,
    width: f32,
) {
    let (icon, text, colour) = if loading {
        (Some(Icon::Refresh), "Preparing...".to_string(), theme.accent)
    } else if playing {
        (Some(Icon::Play), "Playing".to_string(), theme.accent)
    } else if previewed {
        (Some(Icon::Output), "Previewing".to_string(), theme.info)
    } else {
        match health {
            Health::Unknown if !tags.is_empty() => (None, tags.join(", "), theme.text_muted),
            Health::Unknown => (None, String::new(), theme.text_faint),
            Health::Ready => (Some(Icon::Check), health.label().to_string(), theme.success),
            Health::Failed(reason) => (
                Some(Icon::Error),
                format!("Failed: {}", reason.lines().next().unwrap_or("")),
                theme.danger,
            ),
        }
    };
    if text.is_empty() {
        return;
    }
    let y = band.bottom() - theme::density().row_inset() - 5.0;
    let mut text_x = x;
    if let Some(icon) = icon {
        icons::paint(
            ui.painter(),
            Rect::from_center_size(egui::pos2(x + 6.0, y), Vec2::splat(icons::DISCLOSURE)),
            icon,
            theme::rgb(colour),
        );
        text_x += 16.0;
    }
    let galley = widgets::truncated(
        ui,
        &text,
        egui::TextStyle::Small.resolve(ui.style()),
        colour,
        width - (text_x - x),
    );
    ui.painter().galley(
        egui::pos2(text_x, y - galley.size().y / 2.0),
        galley,
        theme::rgb(colour),
    );
}

/// The card above the grid that shows the previewed scene: its picture at
/// the left, the words and the verbs beside it.
fn preview_card(
    ui: &mut Ui,
    theme: &Palette,
    preview: super::model::ScenePreview<'_>,
    playing: Option<usize>,
    actions: &mut Vec<Action>,
) {
    egui::Frame::new()
        .fill(theme::rgb(theme.raised))
        .stroke(Stroke::new(1.0, theme::rgb(theme.info)))
        .corner_radius(CornerRadius::same(theme::RADIUS))
        .inner_margin(theme::margin(SPACE_SM))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            // The picture a third of the width, never taller than a card.
            let width = (ui.available_width() / 3.0).clamp(96.0, 240.0);
            let height = width / preview.image.aspect.max(0.1);
            ui.horizontal(|ui| {
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
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        widgets::chip(ui, "PREVIEW", theme.info, theme.text_on_accent);
                        ui.label(egui::RichText::new(preview.name).strong());
                    });
                    ui.label(
                        egui::RichText::new("Not live. The output and the controls are unchanged.")
                            .small()
                            .color(theme::rgb(theme.text_muted)),
                    );
                    ui.add_space(SPACE_XS);
                    ui.horizontal_wrapped(|ui| {
                        if playing != Some(preview.index)
                            && widgets::primary_button(ui, theme, "Load this scene").clicked()
                        {
                            actions.push(Action::Load(preview.index));
                        }
                        if widgets::button(ui, theme, Some(Icon::Close), "Close preview").clicked()
                        {
                            actions.push(Action::ClosePreview);
                        }
                    });
                });
            });
        });
    // Keep the module's wider gap in use for the band under the card.
    ui.add_space(SPACE_MD - SPACE_SM);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_follow_the_width_and_never_reach_zero() {
        assert_eq!(columns(0.0, CardSize::Medium), 1, "no room is one column");
        assert_eq!(columns(f32::NAN, CardSize::Medium), 1, "nonsense");
        assert!(
            columns(1200.0, CardSize::Small) > columns(1200.0, CardSize::Large),
            "smaller cards, more columns"
        );
        assert!(
            columns(1200.0, CardSize::Medium) >= 6,
            "a 1200-wide grid holds six medium cards, as Synesthesia's does at 1920"
        );
    }

    #[test]
    fn card_size_keys_round_trip() {
        for size in CardSize::ALL {
            assert_eq!(CardSize::from_key(size.key()), Some(size), "{size:?}");
        }
        assert_eq!(CardSize::from_key("huge"), None, "unknown key");
    }
}
