// ============================================================================
// Module:       strobe_app::ui::command_palette
// Description:  The Ctrl+K palette: a search field over every command, pane,
//               layout, scene and preset, drawn over a scrim, run with the
//               keyboard or a click.
//
// Dependencies: egui; crate::{command_palette, palette, workspace};
//               super::{keys, model, theme, motion, widgets}
// ============================================================================

//! The command palette, drawn.
//!
//! [`crate::command_palette`] decides what is listed and in what order; this
//! module only draws it and reads the keys. Up and Down move the highlight,
//! Enter runs it, Escape or a click outside closes the palette, and Ctrl+K
//! toggles it. Running a command goes through [`super::keys::run`], the same
//! code its shortcut runs, so the two can never disagree. A scene's Load and
//! Preview and a preset's Apply push the same actions as the browser's and
//! the preset row's buttons (D262).
//!
//! The list is kept in [`PaletteUi`] and built again only when what it lists
//! changes, and ranked again only when the query does, so an open palette
//! over a large library allocates nothing per frame for its entries.
//!
//! While the palette is open it owns the keyboard, as the modal does: the
//! shell passes it to [`super::keys::handle`] as a modal, so typing `b` into
//! the field never also blacks out the show.

use super::keys;
use super::model::{Action, Model};
use super::motion;
use super::theme::{self, SPACE_MD, SPACE_SM, SPACE_XS};
use super::widgets;
use crate::command_palette::{Availability, Item, Listing, Sources, Target, moved};
use crate::palette::Palette;
use crate::workspace::Pane;
use egui::{Align2, Context, CornerRadius, Key, Modifiers, Sense, Vec2};

/// The card's width: room for a command name, its group and its shortcut on
/// one line.
const CARD_WIDTH: f32 = 520.0;

/// The least room kept between the card and the window's sides.
const CARD_MARGIN: f32 = 24.0;

/// How far below the top of the window the card sits, so it reads as
/// attached to the title bar rather than floating mid-screen.
const TOP_OFFSET: f32 = 72.0;

/// The most rows shown at once; the list scrolls with the highlight.
const MAX_ROWS: usize = 10;

/// The card's height apart from its rows: the field, the hint line, the
/// margins and a gap below, so a short window shows fewer rows rather than
/// running the card off the screen (U-09).
const CARD_CHROME: f32 = 110.0;

/// One row's height.
const ROW: f32 = 30.0;

/// The scrim's opacity: lighter than the modal's, since nothing is at stake.
const SCRIM_ALPHA: u8 = 110;

/// What the palette keeps between frames. Presentation state; never saved.
#[derive(Debug, Default)]
pub struct PaletteUi {
    /// The palette is showing.
    pub open: bool,
    /// The search text.
    pub query: String,
    /// The highlighted row, an index into the ranked list.
    pub selected: Option<usize>,
    /// Set by the frame that opened it: the field takes focus, and the
    /// Ctrl+K that opened it must not also close it.
    pub just_opened: bool,
    /// The entries and their ranking, kept until what they come from
    /// changes (D262).
    listing: Listing,
}

impl PaletteUi {
    /// Opens it empty, or closes it.
    pub fn toggle(&mut self) {
        if self.open {
            self.close();
        } else {
            self.open = true;
            self.query.clear();
            self.selected = Some(0);
            self.just_opened = true;
            self.listing.clear();
        }
    }

    /// Opens it with `query` already typed.
    pub fn open_with(&mut self, query: &str) {
        self.open = true;
        query.clone_into(&mut self.query);
        self.selected = Some(0);
        self.just_opened = true;
        self.listing.clear();
    }

    /// Closes it.
    pub fn close(&mut self) {
        self.open = false;
        self.query.clear();
        self.selected = None;
        self.just_opened = false;
    }
}

/// What the keyboard asked of the palette this frame.
#[derive(Default)]
struct Keys {
    up: bool,
    down: bool,
    enter: bool,
    close: bool,
}

/// Draws the palette when it is open, and runs what is chosen.
pub fn draw(ctx: &Context, theme: &Palette, model: &mut Model<'_>, actions: &mut Vec<Action>) {
    if !model.ui.command_palette.open {
        return;
    }
    let availability = Availability {
        playlist: model.transport.cue.is_some(),
        can_step: model.transport.can_step,
        playing: model.controls.is_some(),
        generating: model.generate.running.is_some(),
    };
    let sources = Sources {
        bindings: model.remote.bindings,
        availability,
        library: model.library,
        library_rev: model.library_rev,
        playing: model.playing,
        presets: model.remote.presets,
    };
    let ui_state = &mut *model.ui;
    let layouts = &ui_state.layouts;
    let state = &mut ui_state.command_palette;
    let just_opened = std::mem::take(&mut state.just_opened);

    // Consumed before the field is drawn, so it never sees Enter (which
    // would end the edit) or the arrows.
    let keys = ctx.input_mut(|input| Keys {
        up: input.consume_key(Modifiers::NONE, Key::ArrowUp),
        down: input.consume_key(Modifiers::NONE, Key::ArrowDown),
        enter: input.consume_key(Modifiers::NONE, Key::Enter),
        close: input.consume_key(Modifiers::NONE, Key::Escape)
            || (!just_opened && input.consume_key(Modifiers::COMMAND, Key::K)),
    });

    state
        .listing
        .update(sources, layouts.keys(), &state.query, &mut state.selected);
    let count = state.listing.ranked().len();
    if keys.up {
        state.selected = moved(state.selected, -1, count);
    }
    if keys.down {
        state.selected = moved(state.selected, 1, count);
    }

    let progress = motion::transition(ctx, egui::Id::new("palette-card"), true, motion::QUICK);
    let screen = ctx.content_rect();
    // The scrim is painted on the foreground layer, as the modal's is, so
    // it dims every panel; a separate area under it swallows the click that
    // misses the card, and that click closes the palette.
    ctx.layer_painter(egui::LayerId::new(
        egui::Order::Foreground,
        egui::Id::new("palette-scrim-paint"),
    ))
    .rect_filled(
        screen,
        CornerRadius::ZERO,
        theme::translucent(theme.app, (f32::from(SCRIM_ALPHA) * progress) as u8),
    );
    let clicked_outside = egui::Area::new(egui::Id::new("palette-scrim"))
        .order(egui::Order::Middle)
        .fixed_pos(screen.min)
        .show(ctx, |ui| {
            ui.allocate_exact_size(screen.size(), Sense::click())
                .1
                .clicked()
        })
        .inner;

    let mut chosen: Option<usize> = None;
    egui::Area::new(egui::Id::new("palette-card"))
        .order(egui::Order::Foreground)
        .anchor(Align2::CENTER_TOP, Vec2::new(0.0, TOP_OFFSET))
        .show(ctx, |ui| {
            ui.set_opacity(progress);
            egui::Frame::new()
                .fill(theme::rgb(theme.raised))
                .stroke(egui::Stroke::new(1.0, theme::rgb(theme.border)))
                .corner_radius(CornerRadius::same(theme::RADIUS_LG))
                .inner_margin(theme::margin(SPACE_MD))
                .shadow(egui::epaint::Shadow {
                    offset: [0, 8],
                    blur: 24,
                    spread: 0,
                    // Alpha-only: a shadow is lighting, not a theme colour.
                    color: egui::Color32::from_black_alpha(96),
                })
                .show(ui, |ui| {
                    // Narrower than the card in a small window, so it never
                    // runs off the screen (U-09).
                    ui.set_width(CARD_WIDTH.min(screen.width() - 2.0 * CARD_MARGIN).max(0.0));
                    let field = ui.add(
                        egui::TextEdit::singleline(&mut state.query)
                            .id(egui::Id::new("palette-query"))
                            .hint_text("Type a command, pane, scene or preset")
                            .desired_width(f32::INFINITY),
                    );
                    if just_opened {
                        field.request_focus();
                    }
                    if field.changed() {
                        state.listing.update(
                            sources,
                            layouts.keys(),
                            &state.query,
                            &mut state.selected,
                        );
                        state.selected = (!state.listing.ranked().is_empty()).then_some(0);
                    }
                    ui.add_space(SPACE_SM);
                    let fit = ((screen.height() - TOP_OFFSET - CARD_CHROME) / ROW).floor();
                    let shown = if fit.is_finite() && fit >= 1.0 {
                        (fit as usize).min(MAX_ROWS)
                    } else {
                        1
                    };
                    if let Some(row) = rows(
                        ui,
                        theme,
                        shown,
                        state.listing.items(),
                        state.listing.ranked(),
                        state.selected,
                        &state.query,
                    ) {
                        state.selected = Some(row);
                        chosen = Some(row);
                    }
                    ui.add_space(SPACE_XS);
                    ui.label(
                        egui::RichText::new("Up and Down to move, Enter to run, Esc to close")
                            .color(theme::rgb(theme.text_faint))
                            .text_style(egui::TextStyle::Small),
                    );
                });
        });

    if keys.enter {
        chosen = state.selected;
    }
    if keys.close || clicked_outside {
        state.close();
        return;
    }
    let Some(target) = chosen
        .and_then(|row| state.listing.ranked().get(row))
        .and_then(|index| state.listing.items().get(*index))
        .filter(|item| item.unavailable.is_none())
        .map(|item| item.target.clone())
    else {
        return;
    };
    state.close();
    match target {
        Target::Command(command) => keys::run(command, model, actions),
        Target::Pane(pane) => model.ui.panes.open(pane),
        Target::Layout(name) => {
            model.ui.apply_layout(&name);
        }
        Target::ControlDesktop => actions.push(Action::ControlAnotherDesktop),
        Target::LoadScene(index) => actions.push(Action::Load(index)),
        Target::PreviewScene(index) => {
            // The preview is shown in the browser's card.
            model.ui.panes.open(Pane::Browser);
            actions.push(Action::Preview(index));
        }
        Target::ApplyPreset(name) => actions.push(Action::RecallPreset(name)),
    }
}

/// Draws the visible rows. Returns the row clicked, if one was.
fn rows(
    ui: &mut egui::Ui,
    theme: &Palette,
    shown: usize,
    all: &[Item],
    ranked: &[usize],
    selected: Option<usize>,
    query: &str,
) -> Option<usize> {
    if ranked.is_empty() {
        ui.label(
            egui::RichText::new(format!("Nothing matches \u{201c}{}\u{201d}", query.trim()))
                .color(theme::rgb(theme.text_muted)),
        );
        return None;
    }
    // A window of rows that keeps the highlight in view.
    let highlight = selected.unwrap_or(0);
    let first = highlight
        .saturating_sub(shown.saturating_sub(1))
        .min(ranked.len().saturating_sub(shown));
    let mut clicked = None;
    for (row, index) in ranked.iter().enumerate().skip(first).take(shown) {
        let Some(item) = all.get(*index) else {
            continue;
        };
        let (rect, response) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), ROW), Sense::click());
        let active = Some(row) == selected;
        if active || response.hovered() {
            let fill = if active { theme.selection } else { theme.hover };
            ui.painter()
                .rect_filled(rect, CornerRadius::same(theme::RADIUS), theme::rgb(fill));
        }
        let text = if item.unavailable.is_some() {
            theme.text_faint
        } else if active {
            theme.selection_text
        } else {
            theme.text
        };
        let body = egui::TextStyle::Body.resolve(ui.style());
        let small = egui::TextStyle::Small.resolve(ui.style());
        // Right to left, so a narrow window shortens the title rather than
        // drawing it over the reason or the shortcut (U-09).
        let right = item
            .unavailable
            .or(item.shortcut.as_deref())
            .unwrap_or_default();
        let right = ui.painter().text(
            rect.right_center() - Vec2::new(SPACE_SM, 0.0),
            Align2::RIGHT_CENTER,
            right,
            small.clone(),
            theme::rgb(theme.text_muted),
        );
        let room = (right.left() - SPACE_SM - (rect.left() + SPACE_SM)).max(0.0);
        let title = widgets::truncated(ui, &item.title, body, text, room);
        let title_width = title.size().x;
        let left = rect.left_center() + Vec2::new(SPACE_SM, -title.size().y / 2.0);
        ui.painter().galley(left, title, theme::rgb(text));
        let category =
            widgets::truncated(ui, item.category, small, theme.text_faint, f32::INFINITY);
        if title_width + SPACE_SM + category.size().x <= room {
            let at = rect.left_center()
                + Vec2::new(SPACE_SM + title_width + SPACE_SM, -category.size().y / 2.0);
            ui.painter()
                .galley(at, category, theme::rgb(theme.text_faint));
        }
        if response.clicked() {
            clicked = Some(row);
        }
    }
    clicked
}
